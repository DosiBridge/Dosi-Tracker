import Foundation

/// The backend REST API, on URLSession with async/await (port of the Windows
/// agent's `api.rs`), per the cross-agent contract.
///
/// Sign-in is the OpenIddict resource-owner password grant against
/// `POST /connect/token` with client `Tracker_App` and scope `Tracker`, plus
/// `offline_access`, and this install's `device_id`. No workspace travels
/// with it: the account signs in, lists its workspaces
/// (``workspaces()``, `GET /api/app/workspaces/mine`) and names the one it
/// tracks in on every workspace-scoped call with the `X-Workspace-Id`
/// header. The password is used for that one grant and never kept: the
/// refresh token (rotated by the server, always the newest kept and reported
/// to ``observeRefreshTokens(_:)``) renews the access token, and once it is
/// refused the user must sign in again (``ApiError/authRejected(_:)``) — the
/// password is never re-sent silently. A server that refuses the
/// `offline_access` scope (`invalid_scope`) is asked again without it.
///
/// Requests never follow redirects (``makeSession(configuration:)``): a
/// bearer token or a block is never replayed to another address. A 401 from
/// any call drops the access token and renews it once before the call is
/// retried. Every response's `Date` header updates ``lastClockReading()``.
/// Each call has a whole-request budget (``requestTimeout``,
/// ``uploadTimeout``), so a dead server can never hang the worker.
public actor ApiClient {
    /// Who the client signs in as, from which install, in which workspace.
    public struct Identity: Equatable, Sendable, CustomStringConvertible, CustomDebugStringConvertible {
        /// Email or user name.
        public var username: String
        /// This install's id (a v4 UUID), sent with every token request.
        public var deviceId: String
        /// The workspace GUID named by `X-Workspace-Id`; nil until one is
        /// chosen.
        public var workspaceId: String?
        /// A saved refresh token to resume with (nil: sign in with the
        /// password first).
        public var refreshToken: String?

        public init(username: String, deviceId: String, workspaceId: String? = nil, refreshToken: String? = nil) {
            self.username = username
            self.deviceId = deviceId
            self.workspaceId = workspaceId
            self.refreshToken = refreshToken
        }

        /// Never includes the refresh token.
        public var description: String {
            "\(username) @ \(workspaceId?.nonEmptyTrimmed ?? "(no workspace)")"
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
    /// Names the workspace a workspace-scoped request acts in.
    public static let workspaceHeader = "X-Workspace-Id"
    /// Whole-request budget for sign-in, projects and totals (also the idle
    /// timeout of every request).
    public static let requestTimeout: TimeInterval = 30
    /// Whole-request budget for an upload, which carries a screenshot (and
    /// maybe a webcam photo) over what may be a slow uplink.
    public static let uploadTimeout: TimeInterval = 120
    /// Longest `Retry-After` honoured: a broken or hostile value must not
    /// stall the queue for days.
    public static let maxRetryAfter: TimeInterval = 6 * 60 * 60

    /// Whether a request carries `X-Workspace-Id`.
    enum Scope: Equatable, Sendable {
        /// The token endpoint, the workspace list, the ABP configuration.
        case unscoped
        /// The client's current workspace.
        case currentWorkspace
        /// A given workspace (a queued row's own).
        case workspace(String)
    }

    private let serverURL: URL
    /// The API root as text, without a trailing slash (paths are appended as
    /// `/api/...`; a trailing slash would make every one of them `//api/...`).
    private let baseText: String
    private let session: URLSession
    private let userAgent: String
    private let now: @Sendable () -> Date
    private let signedInUsername: String
    private let deviceId: String

    /// Set from any context (the worker switches workspace synchronously).
    private let workspace: Locked<String?>
    private let refreshObserver = Locked<(@Sendable (String) async -> Void)?>(nil)
    private var requestsOfflineAccess: Bool
    private var accessToken: AccessToken?
    private var refreshToken: String?
    private var renewal: Task<Void, Error>?
    private var clock: ClockReading?
    /// The signed-in user's id, read once from the application configuration.
    private var userId: String?

    /// - Parameters:
    ///   - baseURL: the API root (``Config/normalizeServerURL(_:allowInsecureHTTP:)``).
    ///   - requestsOfflineAccess: ask for a refresh token (`offline_access`).
    ///   - now: the device clock (injectable for tests).
    public init(
        baseURL: URL,
        identity: Identity,
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
        self.session = session
        self.requestsOfflineAccess = requestsOfflineAccess
        self.userAgent = userAgent
        self.now = now
        signedInUsername = identity.username.trimmingCharacters(in: .whitespacesAndNewlines)
        deviceId = identity.deviceId
        workspace = Locked(identity.workspaceId?.nonEmptyTrimmed)
        refreshToken = identity.refreshToken?.nonEmptyTrimmed
    }

    /// A URLSession for the API: no cookies, no cache, no waiting for
    /// connectivity (offline must fail fast so the block stays queued), and
    /// no redirects (``RedirectRefusal``).
    public static func makeSession(configuration: URLSessionConfiguration = .ephemeral) -> URLSession {
        configuration.timeoutIntervalForRequest = requestTimeout
        configuration.timeoutIntervalForResource = uploadTimeout
        configuration.waitsForConnectivity = false
        configuration.httpCookieAcceptPolicy = .never
        configuration.httpShouldSetCookies = false
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: configuration, delegate: RedirectRefusal(), delegateQueue: nil)
    }

    /// `DosiTracker/<version> (macOS <version>)`.
    public static func defaultUserAgent() -> String {
        AgentInfo.userAgent(version: AgentInfo.version(), osVersion: DeviceInfo.currentOSVersion())
    }

    /// The API root this client talks to.
    public nonisolated var baseURL: URL { serverURL }

    /// The user this client signs in as.
    public nonisolated var username: String { signedInUsername }

    /// The workspace workspace-scoped calls act in (nil: none chosen).
    public nonisolated func currentWorkspaceId() -> String? {
        workspace.get()
    }

    /// Act in workspace `id` from now on (nil: none — scoped calls then
    /// fail until one is chosen). The sign-in and its tokens stay.
    public nonisolated func useWorkspace(_ id: String?) {
        workspace.set(id?.nonEmptyTrimmed)
    }

    /// The newest refresh token, to save with the sign-in (nil when the
    /// server granted none).
    public func currentRefreshToken() -> String? {
        refreshToken
    }

    /// Call `observer` with each new refresh token the server rotates in, so
    /// the saved sign-in always holds the one that still works. Takes effect
    /// at once, before any request.
    public nonisolated func observeRefreshTokens(_ observer: (@Sendable (String) async -> Void)?) {
        refreshObserver.set(observer)
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

    /// Sign in with the password (always a fresh password grant; the password
    /// is not kept). Throws ``ApiError/authRejected(_:)`` when the server
    /// refuses the credentials; with ``AuthRejection/Kind/mustChangePassword``
    /// call again with a ``PasswordChange`` built from its
    /// `changePasswordToken`.
    public func signIn(password: String, passwordChange: PasswordChange? = nil) async throws {
        accessToken = nil
        refreshToken = nil
        userId = nil
        try await passwordGrant(password, change: passwordChange)
    }

    /// Forget the tokens (sign-out). Every later call fails with
    /// ``ApiError/authRejected(_:)`` until ``signIn(password:passwordChange:)``.
    public func signOut() {
        accessToken = nil
        refreshToken = nil
        userId = nil
    }

    // MARK: API calls

    /// `GET /api/app/workspaces/mine` (no workspace header): the workspaces
    /// the signed-in user belongs to, archived ones included (flagged).
    public func workspaces() async throws -> [Workspace] {
        let response = try await authorizedRequest(method: "GET", path: "/api/app/workspaces/mine", scope: .unscoped,
                                                   budget: ApiClient.requestTimeout)
        guard response.isSuccess else { throw ApiError.http(failure(of: response)) }
        do {
            return try Workspace.decodeList(from: response.data)
        } catch {
            throw ApiError.invalidResponse("The workspace list could not be read.")
        }
    }

    /// `GET /api/app/project/my-projects`: the non-archived projects the user
    /// tracks in the workspace (the current one unless `workspaceId` names
    /// another), with their capture policy.
    public func myProjects(workspaceId: String? = nil) async throws -> [MyProject] {
        let response = try await authorizedRequest(method: "GET", path: "/api/app/project/my-projects",
                                                   scope: ApiClient.scopeFor(workspaceId),
                                                   budget: ApiClient.requestTimeout)
        guard response.isSuccess else { throw ApiError.http(failure(of: response)) }
        do {
            return try MyProject.decodeList(from: response.data)
        } catch {
            throw ApiError.invalidResponse("The project list could not be read.")
        }
    }

    /// `POST /api/app/activity`: upload one block to its workspace (the
    /// current one unless `workspaceId` names another), with `sentAt` stamped
    /// from the device clock on each attempt (the server derives the clock
    /// skew from it). Never throws: classify the result with
    /// ``UploadPolicy``.
    public func uploadActivity(_ payload: ActivityPayload, workspaceId: String? = nil) async -> UploadResponse {
        let clockNow = now
        let encodeAttempt: () throws -> Data = {
            var outgoing = payload
            outgoing.sentAt = clockNow()
            return try ApiClient.encodeBody(outgoing, what: "block")
        }
        do {
            let response = try await authorizedRequest(method: "POST", path: "/api/app/activity",
                                                       scope: ApiClient.scopeFor(workspaceId), body: encodeAttempt,
                                                       budget: ApiClient.uploadTimeout)
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

    /// `POST /api/app/presence/heartbeat` in the current workspace
    /// (normalized, `sentAt` stamped at send time), finished — a token
    /// renewal included — within `timeout` seconds or failed with a timeout:
    /// 1-2 s for the best-effort one sent when quitting. A failed heartbeat
    /// is never retried or queued.
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
                                                   scope: .currentWorkspace, budget: ApiClient.requestTimeout)
        guard response.isSuccess else { throw ApiError.http(failure(of: response)) }
        do {
            return try PresenceInfo.decodeList(from: response.data)
        } catch {
            throw ApiError.invalidResponse("The presence list could not be read.")
        }
    }

    /// The signed-in user's id (`currentUser.id` of `GET
    /// /api/abp/application-configuration`, asked without a workspace),
    /// fetched once per sign-in: the totals must be scoped to the caller, not
    /// to the team an owner or manager may see.
    public func currentUserId() async throws -> String {
        if let known = userId {
            return known
        }
        let response = try await authorizedRequest(method: "GET", path: "/api/abp/application-configuration",
                                                   scope: .unscoped, budget: ApiClient.requestTimeout)
        guard response.isSuccess else { throw ApiError.http(failure(of: response)) }
        guard let id = ApplicationConfiguration.currentUserId(from: response.data) else {
            throw ApiError.invalidResponse("The signed-in user could not be identified.")
        }
        userId = id
        return id
    }

    /// `GET /api/app/reporting/summary` for `[from, to)` of user `userId`
    /// (nil: the server's default scope) in the current workspace, with every
    /// "day" counted in `timeZone` (IANA id; omitted when blank, and ignored
    /// by an older server).
    public func summary(from: Date, to: Date, timeZone: String?, userId: String? = nil) async throws -> ReportSummary {
        let query = ApiClient.summaryQuery(from: from, to: to, timeZone: timeZone, userId: userId)
        let response = try await authorizedRequest(method: "GET", path: "/api/app/reporting/summary",
                                                   query: query, scope: .currentWorkspace,
                                                   budget: ApiClient.requestTimeout)
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

    private static func scopeFor(_ workspaceId: String?) -> Scope {
        if let id = workspaceId?.nonEmptyTrimmed {
            return .workspace(id)
        }
        return .currentWorkspace
    }

    // MARK: Tokens

    private var tokenScope: String {
        requestsOfflineAccess ? "\(ApiClient.apiScope) \(ApiClient.offlineAccessScope)" : ApiClient.apiScope
    }

    private func passwordGrant(_ password: String, change: PasswordChange?) async throws {
        let token: TokenResponse
        do {
            token = try await requestToken(passwordFields(password, change: change))
        } catch ApiError.tokenFailed(let failure) where failure.isInvalidScope && requestsOfflineAccess {
            // This server does not grant refresh tokens to the client: sign
            // in with the API scope alone (the sign-in then lasts as long as
            // the access token).
            CoreLog.info("api", "the server refused offline_access; signing in without refresh tokens")
            requestsOfflineAccess = false
            token = try await requestToken(passwordFields(password, change: change))
        }
        store(token)
    }

    private func passwordFields(_ password: String, change: PasswordChange?) -> [(String, String)] {
        var fields: [(String, String)] = [
            ("grant_type", "password"),
            ("username", signedInUsername),
            ("password", password),
            ("client_id", ApiClient.clientId),
            ("scope", tokenScope),
            ("device_id", deviceId),
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
            ("device_id", deviceId),
        ])
        let previous = refreshToken
        store(response)
        if let rotated = refreshToken, rotated != previous, let observer = refreshObserver.get() {
            await observer(rotated)
        }
    }

    private func store(_ token: TokenResponse) {
        // Renew a minute early so requests never race the expiry.
        let lifetime = TimeInterval(max((token.expiresIn ?? 3600) - 60, 60))
        accessToken = AccessToken(value: token.accessToken, expiresAt: now().addingTimeInterval(lifetime))
        // The server rotates the refresh token: always keep the newest.
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

    /// Renew with the refresh token. Refused (expired, revoked, the account
    /// deactivated) or none to renew with: the sign-in has ended and the user
    /// must sign in again — never a silent password re-login. Any other
    /// failure (offline, 5xx) is the server's, not the sign-in's, and is
    /// passed on with the token kept.
    private func performRenewal() async throws {
        guard let refresh = refreshToken else {
            throw ApiError.authRejected(AuthRejection.sessionEnded)
        }
        do {
            try await refreshGrant(refresh)
        } catch ApiError.authRejected(let rejection) {
            refreshToken = nil
            accessToken = nil
            throw ApiError.authRejected(rejection)
        }
    }

    private func tokenURL() throws -> URL {
        guard let url = URL(string: baseText + "/connect/token") else {
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
            scope: .currentWorkspace,
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

    /// Send with the bearer token; on 401 drop it, renew it and retry once.
    /// `body` is built for each attempt (so `sentAt` is current).
    private func authorizedRequest(method: String, path: String, query: [(String, String)] = [], scope: Scope,
                                   body: (() throws -> Data)? = nil, budget: TimeInterval) async throws -> RawResponse {
        var response = try await sendAuthorized(method: method, path: path, query: query, scope: scope, body: body,
                                                budget: budget)
        if response.status == 401 {
            // Revoked or expired server-side.
            accessToken = nil
            response = try await sendAuthorized(method: method, path: path, query: query, scope: scope, body: body,
                                                budget: budget)
            if response.status == 401 {
                accessToken = nil
            }
        }
        return response
    }

    private func sendAuthorized(method: String, path: String, query: [(String, String)], scope: Scope,
                                body: (() throws -> Data)?, budget: TimeInterval) async throws -> RawResponse {
        let workspaceHeader: String?
        switch scope {
        case .unscoped:
            workspaceHeader = nil
        case .currentWorkspace:
            guard let current = workspace.get() else {
                throw ApiError.invalidConfiguration("No workspace is chosen.")
            }
            workspaceHeader = current
        case .workspace(let id):
            workspaceHeader = id
        }
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
        if let workspace = workspaceHeader {
            request.setValue(workspace, forHTTPHeaderField: ApiClient.workspaceHeader)
        }
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

/// Never follow a redirect (cross-agent contract): the bearer token, a block
/// or a password must never be replayed to an address nobody configured. The
/// 3xx answer is returned as it is and fails the call.
public final class RedirectRefusal: NSObject, URLSessionTaskDelegate, Sendable {
    public override init() {
        super.init()
    }

    public func urlSession(_ session: URLSession, task: URLSessionTask,
                           willPerformHTTPRedirection response: HTTPURLResponse,
                           newRequest request: URLRequest) async -> URLRequest? {
        CoreLog.warning("api", "the server answered HTTP \(response.statusCode) with a redirect; not followed")
        return nil
    }
}
