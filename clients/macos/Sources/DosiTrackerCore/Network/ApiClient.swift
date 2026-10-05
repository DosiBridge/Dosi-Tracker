import Foundation

/// The backend REST API, on URLSession with async/await (port of the Windows
/// agent's `api.rs`).
///
/// Sign-in is the OpenIddict resource-owner password grant against
/// `POST /connect/token` with client `Tracker_App` and scope `Tracker`, plus
/// `offline_access`, which the seeded client allows (it has the
/// refresh-token grant): the refresh token renews the access token without
/// re-sending the password, and the saved password remains the fallback. A
/// server that refuses the scope (`invalid_scope`) is asked again without it.
/// The workspace travels as the `__tenant` query parameter of the token
/// request, exactly as on Windows (ABP's tenant resolvers do not read the
/// POST body); every other call carries the tenant inside the bearer token.
///
/// A 401 from any call drops the access token and signs in again once before
/// the call is retried. Every response's `Date` header updates
/// ``lastClockReading()``. Each call has a whole-request budget
/// (``requestTimeout``, ``uploadTimeout``), so a dead server can never hang
/// the worker.
public actor ApiClient {
    /// The credentials the client signs in with.
    public struct Account: Equatable, Sendable, CustomStringConvertible, CustomDebugStringConvertible {
        /// Workspace (tenant) name; empty for a host account.
        public var workspace: String
        public var username: String
        public var password: String

        public init(workspace: String, username: String, password: String) {
            self.workspace = workspace
            self.username = username
            self.password = password
        }

        /// The queue owner key of this account.
        public var ownerKey: String { Queue.ownerKey(workspace: workspace, username: username) }

        /// Never includes the password.
        public var description: String {
            let tenant = workspace.trimmingCharacters(in: .whitespacesAndNewlines)
            return "\(username) @ \(tenant.isEmpty ? "(host)" : tenant)"
        }

        public var debugDescription: String { description }
    }

    /// A password change the server demands at sign-in
    /// (``AuthRejection/Kind/mustChangePassword``).
    public struct PasswordChange: Equatable, Sendable {
        public var changePasswordToken: String
        public var newPassword: String

        public init(changePasswordToken: String, newPassword: String) {
            self.changePasswordToken = changePasswordToken
            self.newPassword = newPassword
        }
    }

    /// OpenIddict client seeded by the backend.
    public static let clientId = "Tracker_App"
    public static let apiScope = "Tracker"
    public static let offlineAccessScope = "offline_access"
    /// Whole-request budget for sign-in, projects and totals (also the idle
    /// timeout of every request).
    public static let requestTimeout: TimeInterval = 30
    /// Whole-request budget for an upload, which carries a screenshot (and
    /// maybe a webcam photo) over what may be a slow uplink.
    public static let uploadTimeout: TimeInterval = 120
    /// Longest `Retry-After` honoured: a broken or hostile value must not
    /// stall the queue for days.
    public static let maxRetryAfter: TimeInterval = 6 * 60 * 60

    private let serverURL: URL
    /// The API root as text, without a trailing slash (paths are appended as
    /// `/api/...`; a trailing slash would make every one of them `//api/...`).
    private let baseText: String
    private let owner: String
    private let session: URLSession
    private let userAgent: String
    private let now: @Sendable () -> Date

    private var account: Account
    private var requestsOfflineAccess: Bool
    private var accessToken: AccessToken?
    private var refreshToken: String?
    private var renewal: Task<Void, Error>?
    private var clock: ClockReading?
    /// The signed-in user's id, read once from the application configuration.
    private var userId: String?

    /// - Parameters:
    ///   - baseURL: the API root (``Config/normalizeServerURL(_:)``).
    ///   - requestsOfflineAccess: ask for a refresh token (`offline_access`).
    ///   - now: the device clock (injectable for tests).
    public init(
        baseURL: URL,
        account: Account,
        session: URLSession = ApiClient.makeSession(),
        requestsOfflineAccess: Bool = true,
        userAgent: String = ApiClient.defaultUserAgent(),
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        var text = baseURL.absoluteString.trimmingCharacters(in: .whitespacesAndNewlines)
        while text.hasSuffix("/") {
            text.removeLast()
        }
        self.serverURL = baseURL
        self.baseText = text
        self.owner = account.ownerKey
        self.session = session
        self.requestsOfflineAccess = requestsOfflineAccess
        self.userAgent = userAgent
        self.now = now
        self.account = account
    }

    /// A URLSession for the API: no cookies, no cache, no waiting for
    /// connectivity (offline must fail fast so the block stays queued).
    public static func makeSession(configuration: URLSessionConfiguration = .ephemeral) -> URLSession {
        configuration.timeoutIntervalForRequest = requestTimeout
        configuration.timeoutIntervalForResource = uploadTimeout
        configuration.waitsForConnectivity = false
        configuration.httpCookieAcceptPolicy = .never
        configuration.httpShouldSetCookies = false
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: configuration)
    }

    /// `DosiTracker/<version> (macOS <version>)`.
    public static func defaultUserAgent() -> String {
        AgentInfo.userAgent(version: AgentInfo.version(), osVersion: DeviceInfo.currentOSVersion())
    }

    /// The API root this client talks to.
    public nonisolated var baseURL: URL { serverURL }

    /// The queue owner key of the signed-in account, so rows are only ever
    /// uploaded under their owner's token.
    public nonisolated var ownerKey: String { owner }

    /// The account, with the new password after a successful password change.
    public func currentAccount() -> Account {
        account
    }

    /// Whether an access token is held (it may still have expired).
    public func hasAccessToken() -> Bool {
        accessToken != nil
    }

    /// The latest device/server clock comparison, from the `Date` header of
    /// the last response.
    public func lastClockReading() -> ClockReading? {
        clock
    }

    // MARK: Sign-in

    /// Sign in with the password (always a fresh password grant). Throws
    /// ``ApiError/authRejected(_:)`` when the server refuses the credentials;
    /// with ``AuthRejection/Kind/mustChangePassword`` call again with a
    /// ``PasswordChange`` built from its `changePasswordToken`.
    public func signIn(passwordChange: PasswordChange? = nil) async throws {
        accessToken = nil
        refreshToken = nil
        try await passwordGrant(passwordChange)
    }

    /// Forget the tokens (sign-out). The next call signs in again.
    public func signOut() {
        accessToken = nil
        refreshToken = nil
    }

    // MARK: API calls

    /// `GET /api/app/project/my-projects`: the non-archived projects the user
    /// tracks, with their capture policy.
    public func myProjects() async throws -> [MyProject] {
        let response = try await authorizedRequest(method: "GET", path: "/api/app/project/my-projects",
                                                   budget: ApiClient.requestTimeout)
        guard response.isSuccess else { throw ApiError.http(failure(of: response)) }
        do {
            return try MyProject.decodeList(from: response.data)
        } catch {
            throw ApiError.invalidResponse("The project list could not be read.")
        }
    }

    /// `POST /api/app/activity`: upload one block, with `sentAt` stamped from
    /// the device clock on each attempt (the server derives the clock skew
    /// from it). Never throws: classify the result with ``UploadPolicy``.
    public func uploadActivity(_ payload: ActivityPayload) async -> UploadResponse {
        let clockNow = now
        let encodeAttempt: () throws -> Data = {
            var outgoing = payload
            outgoing.sentAt = clockNow()
            return try ApiClient.encodeBody(outgoing, what: "block")
        }
        do {
            let response = try await authorizedRequest(method: "POST", path: "/api/app/activity",
                                                       body: encodeAttempt, budget: ApiClient.uploadTimeout)
            if response.isSuccess {
                return .accepted(status: response.status)
            }
            return .failed(.http(failure(of: response)))
        } catch let error as ApiError {
            return .failed(error)
        } catch {
            return .failed(.transport(TransportFailure(code: -1, description: error.localizedDescription)))
        }
    }

    /// `POST /api/app/presence/heartbeat` (normalized, `sentAt` stamped at
    /// send time), finished — a sign-in included — within `timeout` seconds
    /// or failed with a timeout: 1-2 s for the best-effort one sent when
    /// quitting. A failed heartbeat is never retried or queued.
    public func sendHeartbeat(_ heartbeat: Heartbeat,
                              timeout: TimeInterval = ApiClient.requestTimeout) async throws -> HeartbeatResult {
        let body = heartbeat.normalized()
        return try await ApiClient.withDeadline(timeout) {
            try await self.postHeartbeat(body, timeout: timeout)
        }
    }

    /// `GET /api/app/presence/team`: live device statuses — everyone's for
    /// callers with `Tracker.Activities.ViewAll`, otherwise the caller's own
    /// devices (e.g. to warn that tracking also runs on another computer).
    public func presence() async throws -> [PresenceInfo] {
        let response = try await authorizedRequest(method: "GET", path: "/api/app/presence/team",
                                                   budget: ApiClient.requestTimeout)
        guard response.isSuccess else { throw ApiError.http(failure(of: response)) }
        do {
            return try PresenceInfo.decodeList(from: response.data)
        } catch {
            throw ApiError.invalidResponse("The presence list could not be read.")
        }
    }

    /// The signed-in user's id (`currentUser.id` of `GET
    /// /api/abp/application-configuration`), fetched once per client: the
    /// totals must be scoped to the caller, not to the team an owner or
    /// manager may see.
    public func currentUserId() async throws -> String {
        if let known = userId {
            return known
        }
        let response = try await authorizedRequest(method: "GET", path: "/api/abp/application-configuration",
                                                   budget: ApiClient.requestTimeout)
        guard response.isSuccess else { throw ApiError.http(failure(of: response)) }
        guard let id = ApplicationConfiguration.currentUserId(from: response.data) else {
            throw ApiError.invalidResponse("The signed-in user could not be identified.")
        }
        userId = id
        return id
    }

    /// `GET /api/app/reporting/summary` for `[from, to)` of user `userId`
    /// (nil: the server's default scope), with every "day" counted in
    /// `timeZone` (IANA id; omitted when blank, and ignored by an older
    /// server).
    public func summary(from: Date, to: Date, timeZone: String?, userId: String? = nil) async throws -> ReportSummary {
        let query = ApiClient.summaryQuery(from: from, to: to, timeZone: timeZone, userId: userId)
        let response = try await authorizedRequest(method: "GET", path: "/api/app/reporting/summary",
                                                   query: query, budget: ApiClient.requestTimeout)
        guard response.isSuccess else { throw ApiError.http(failure(of: response)) }
        do {
            return try WireJSON.decoder().decode(ReportSummary.self, from: response.data)
        } catch {
            throw ApiError.invalidResponse("The totals could not be read.")
        }
    }

    /// The query of a summary request: `From`, `To` (ISO-8601 UTC with
    /// milliseconds) and, when given, `TimeZone` and `UserId`.
    static func summaryQuery(from: Date, to: Date, timeZone: String?, userId: String? = nil) -> [(String, String)] {
        var query = [("From", WireDate.string(from: from)), ("To", WireDate.string(from: to))]
        if let zone = timeZone?.nonEmptyTrimmed {
            query.append(("TimeZone", zone))
        }
        if let user = userId?.nonEmptyTrimmed {
            query.append(("UserId", user))
        }
        return query
    }

    // MARK: Tokens

    private var scope: String {
        requestsOfflineAccess ? "\(ApiClient.apiScope) \(ApiClient.offlineAccessScope)" : ApiClient.apiScope
    }

    private func passwordGrant(_ change: PasswordChange?) async throws {
        let token: TokenResponse
        do {
            token = try await requestToken(passwordFields(change))
        } catch ApiError.tokenFailed(let failure) where failure.isInvalidScope && requestsOfflineAccess {
            // This server does not grant refresh tokens to the client: sign
            // in with the API scope alone, as the Windows agent does.
            CoreLog.info("api", "the server refused offline_access; signing in without refresh tokens")
            requestsOfflineAccess = false
            token = try await requestToken(passwordFields(change))
        }
        store(token)
        if let change = change {
            account.password = change.newPassword
        }
    }

    private func passwordFields(_ change: PasswordChange?) -> [(String, String)] {
        var fields: [(String, String)] = [
            ("grant_type", "password"),
            ("username", account.username.trimmingCharacters(in: .whitespacesAndNewlines)),
            ("password", account.password),
            ("client_id", ApiClient.clientId),
            ("scope", scope),
        ]
        if let change = change {
            fields.append(("ChangePasswordToken", change.changePasswordToken))
            fields.append(("NewPassword", change.newPassword))
        }
        return fields
    }

    private func refreshGrant(_ token: String) async throws {
        let response = try await requestToken([
            ("grant_type", "refresh_token"),
            ("refresh_token", token),
            ("client_id", ApiClient.clientId),
        ])
        store(response)
    }

    private func store(_ token: TokenResponse) {
        // Renew a minute early so requests never race the expiry.
        let lifetime = TimeInterval(max((token.expiresIn ?? 3600) - 60, 60))
        accessToken = AccessToken(value: token.accessToken, expiresAt: now().addingTimeInterval(lifetime))
        if let refresh = token.refreshToken?.nonEmptyTrimmed {
            refreshToken = refresh
        }
    }

    private func validAccessToken() async throws -> String {
        if let token = accessToken, now() < token.expiresAt {
            return token.value
        }
        try await renewAccessToken()
        guard let token = accessToken else {
            throw ApiError.invalidResponse("The sign-in returned no access token.")
        }
        return token.value
    }

    /// One renewal at a time: concurrent callers wait for the same one.
    private func renewAccessToken() async throws {
        if let running = renewal {
            try await running.value
            return
        }
        let task = Task<Void, Error> {
            try await self.performRenewal()
        }
        renewal = task
        defer { renewal = nil }
        try await task.value
    }

    private func performRenewal() async throws {
        if let refresh = refreshToken {
            do {
                try await refreshGrant(refresh)
                return
            } catch ApiError.authRejected {
                // Expired or revoked: fall back to the saved password. Any
                // other failure (offline, 5xx) is the server's, not the
                // credentials', and is passed on.
                refreshToken = nil
            }
        }
        try await passwordGrant(nil)
    }

    private func tokenURL() throws -> URL {
        var text = baseText + "/connect/token"
        let workspace = account.workspace.trimmingCharacters(in: .whitespacesAndNewlines)
        if !workspace.isEmpty {
            text += "?__tenant=" + PercentEncoding.encode(workspace)
        }
        guard let url = URL(string: text) else {
            throw ApiError.invalidConfiguration("Invalid server address: \(baseText)")
        }
        return url
    }

    private func requestToken(_ fields: [(String, String)]) async throws -> TokenResponse {
        var request = URLRequest(url: try tokenURL())
        request.httpMethod = "POST"
        request.timeoutInterval = ApiClient.requestTimeout
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue(userAgent, forHTTPHeaderField: "User-Agent")
        request.httpBody = PercentEncoding.form(fields)

        let response = try await send(request, budget: ApiClient.requestTimeout)
        if response.isSuccess {
            guard let token = try? JSONDecoder().decode(TokenResponse.self, from: response.data),
                  !token.accessToken.isEmpty
            else {
                throw ApiError.invalidResponse("The server sent an unexpected sign-in response.")
            }
            return token
        }
        let body = OAuthErrorBody.parse(response.data)
        if AuthRejection.isRejection(status: response.status, body: body) {
            throw ApiError.authRejected(AuthRejection(status: response.status, body: body))
        }
        throw ApiError.tokenFailed(TokenFailure(
            status: response.status,
            error: body.error,
            errorDescription: body.errorDescription,
            retryAfter: retryAfter(of: response)
        ))
    }

    // MARK: Transport

    private func postHeartbeat(_ heartbeat: Heartbeat, timeout requestTimeout: TimeInterval) async throws -> HeartbeatResult {
        let clockNow = now
        let response = try await authorizedRequest(
            method: "POST",
            path: "/api/app/presence/heartbeat",
            body: {
                var outgoing = heartbeat
                outgoing.sentAt = clockNow()
                return try ApiClient.encodeBody(outgoing, what: "heartbeat")
            },
            budget: requestTimeout
        )
        guard response.isSuccess else { throw ApiError.http(failure(of: response)) }
        // The heartbeat counted even if its answer cannot be read.
        return (try? WireJSON.decoder().decode(HeartbeatResult.self, from: response.data)) ?? HeartbeatResult(serverTime: nil)
    }

    /// Send with the bearer token; on 401 drop it, sign in again and retry
    /// once. `body` is built for each attempt (so `sentAt` is current).
    private func authorizedRequest(method: String, path: String, query: [(String, String)] = [],
                                   body: (() throws -> Data)? = nil, budget: TimeInterval) async throws -> RawResponse {
        var response = try await sendAuthorized(method: method, path: path, query: query, body: body, budget: budget)
        if response.status == 401 {
            // Revoked or expired server-side.
            accessToken = nil
            response = try await sendAuthorized(method: method, path: path, query: query, body: body, budget: budget)
            if response.status == 401 {
                accessToken = nil
            }
        }
        return response
    }

    private func sendAuthorized(method: String, path: String, query: [(String, String)],
                                body: (() throws -> Data)?, budget: TimeInterval) async throws -> RawResponse {
        let token = try await validAccessToken()
        var text = baseText + path
        if !query.isEmpty {
            text += "?" + PercentEncoding.query(query)
        }
        guard let url = URL(string: text) else {
            throw ApiError.invalidConfiguration("Invalid server address: \(baseText)")
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.timeoutInterval = ApiClient.requestTimeout
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue(userAgent, forHTTPHeaderField: "User-Agent")
        if let body = body {
            request.httpBody = try body()
            request.setValue("application/json; charset=utf-8", forHTTPHeaderField: "Content-Type")
        }
        return try await send(request, budget: budget)
    }

    /// Perform one request within `budget` seconds, mapping every failure
    /// without a response to ``ApiError/transport(_:)``, and note the server
    /// clock.
    private func send(_ request: URLRequest, budget: TimeInterval) async throws -> RawResponse {
        let session = self.session
        let response: RawResponse
        do {
            response = try await ApiClient.withDeadline(budget) {
                try await ApiClient.perform(request, on: session)
            }
        } catch let error as ApiError {
            throw error
        } catch let error as URLError {
            throw ApiError.transport(TransportFailure(error))
        } catch is CancellationError {
            throw ApiError.transport(.cancelled)
        } catch {
            throw ApiError.transport(TransportFailure(code: -1, description: error.localizedDescription))
        }
        noteClock(response.dateHeader)
        return response
    }

    private static func perform(_ request: URLRequest, on session: URLSession) async throws -> RawResponse {
        let (data, urlResponse) = try await session.data(for: request)
        guard let http = urlResponse as? HTTPURLResponse else {
            throw ApiError.invalidResponse("The server did not answer over HTTP.")
        }
        return RawResponse(
            status: http.statusCode,
            data: data,
            dateHeader: http.value(forHTTPHeaderField: "Date"),
            retryAfterHeader: http.value(forHTTPHeaderField: "Retry-After")
        )
    }

    private func noteClock(_ dateHeader: String?) {
        guard let header = dateHeader, let reading = ClockReading(dateHeader: header, deviceNow: now()) else { return }
        clock = reading
    }

    private func failure(of response: RawResponse) -> HTTPFailure {
        HTTPFailure(status: response.status, error: AbpError.parse(response.data), retryAfter: retryAfter(of: response))
    }

    private func retryAfter(of response: RawResponse) -> TimeInterval? {
        ApiClient.retryAfterSeconds(
            response.retryAfterHeader,
            serverDate: response.dateHeader.flatMap { WireDate.httpDate(from: $0) },
            deviceNow: now()
        )
    }

    private static func encodeBody<T: Encodable>(_ value: T, what: String) throws -> Data {
        do {
            return try WireJSON.encoder().encode(value)
        } catch {
            throw ApiError.encoding("The \(what) could not be encoded: \(error)")
        }
    }

    /// How long a `Retry-After` header asks to wait: delta-seconds or an HTTP
    /// date. A date is measured against the server's own clock when it sent
    /// one (`serverDate`), so a wrong device clock does not distort the wait;
    /// a date in the past means now. Bounded by ``maxRetryAfter``; nil when
    /// absent or unreadable.
    public static func retryAfterSeconds(_ header: String?, serverDate: Date?, deviceNow: Date) -> TimeInterval? {
        guard let text = header?.nonEmptyTrimmed else { return nil }
        let wait: TimeInterval
        if let seconds = UInt64(text) {
            wait = TimeInterval(seconds)
        } else if let at = WireDate.httpDate(from: text) {
            wait = max(at.timeIntervalSince(serverDate ?? deviceNow), 0)
        } else {
            return nil
        }
        return min(wait, maxRetryAfter)
    }

    /// Run `operation`, failing with a transport timeout if it has not
    /// finished within `seconds`. The answer comes by the deadline even when
    /// the operation does not react to cancellation at once (e.g. it waits for
    /// a sign-in already in flight): it is cancelled and left to end on its
    /// own. Cancelling the caller fails the call with a cancellation.
    public static func withDeadline<T: Sendable>(_ seconds: TimeInterval,
                                                 _ operation: @escaping @Sendable () async throws -> T) async throws -> T {
        let race = DeadlineRace<T>()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<T, Error>) in
                race.start(continuation, seconds: seconds, operation: operation)
            }
        } onCancel: {
            race.finish(.failure(ApiError.transport(.cancelled)))
        }
    }
}

/// An operation racing its deadline: whichever ends first resumes the caller
/// (exactly once) and the other is cancelled.
private final class DeadlineRace<T: Sendable>: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<T, Error>?
    private var finished = false
    private var tasks: [Task<Void, Never>] = []

    func start(_ continuation: CheckedContinuation<T, Error>, seconds: TimeInterval,
               operation: @escaping @Sendable () async throws -> T) {
        lock.lock()
        if finished {
            // The caller was cancelled before the race began.
            lock.unlock()
            continuation.resume(throwing: ApiError.transport(.cancelled))
            return
        }
        self.continuation = continuation
        lock.unlock()

        let work = Task {
            do {
                self.finish(.success(try await operation()))
            } catch {
                self.finish(.failure(error))
            }
        }
        let timer = Task {
            do {
                // At most a day: keeps the nanosecond count far from overflow.
                try await Task.sleep(nanoseconds: UInt64(min(max(seconds, 0), 86_400) * 1_000_000_000))
            } catch {
                return
            }
            self.finish(.failure(ApiError.transport(.timedOut)))
        }

        lock.lock()
        let alreadyFinished = finished
        if !alreadyFinished {
            tasks = [work, timer]
        }
        lock.unlock()
        if alreadyFinished {
            work.cancel()
            timer.cancel()
        }
    }

    func finish(_ result: Result<T, Error>) {
        lock.lock()
        guard !finished else {
            lock.unlock()
            return
        }
        finished = true
        let continuation = self.continuation
        self.continuation = nil
        let running = tasks
        tasks = []
        lock.unlock()
        for task in running {
            task.cancel()
        }
        continuation?.resume(with: result)
    }
}

/// A bearer token and when to renew it.
private struct AccessToken: Sendable {
    let value: String
    let expiresAt: Date
}

/// An HTTP response, reduced to what the client reads.
struct RawResponse: Sendable {
    let status: Int
    let data: Data
    let dateHeader: String?
    let retryAfterHeader: String?

    var isSuccess: Bool { (200..<300).contains(status) }
}
