import XCTest
@testable import DosiTrackerCore

/// The API client against a canned server (ports of the Windows agent's
/// `api.rs` tests, through URLProtocol instead of a local socket), per the
/// cross-agent contract: no tenant on the token request, this install's
/// `device_id` with every grant, the workspace named by `X-Workspace-Id`.
final class ApiClientTests: XCTestCase {
    private let deviceId = "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11"
    private let acme = "3f2a1b0c-0000-4000-8000-00000000a001"
    private let globex = "3f2a1b0c-0000-4000-8000-00000000b002"

    private func client(_ server: MockServer, workspaceId: String? = "3f2a1b0c-0000-4000-8000-00000000a001",
                        refreshToken: String? = nil, offlineAccess: Bool = true,
                        now: @escaping @Sendable () -> Date = { Date() }) -> ApiClient {
        ApiClient(
            baseURL: URL(string: server.baseURL.absoluteString + "/")!,
            identity: ApiClient.Identity(username: " alice@acme.com ", deviceId: deviceId, workspaceId: workspaceId,
                                         refreshToken: refreshToken),
            session: MockServer.session(),
            requestsOfflineAccess: offlineAccess,
            userAgent: "DosiTracker/9.9.9 (macOS 14.5)",
            now: now
        )
    }

    /// A client signed in with the password (the first canned reply is the
    /// token).
    private func signedIn(_ server: MockServer, workspaceId: String? = "3f2a1b0c-0000-4000-8000-00000000a001")
        async throws -> ApiClient {
        let api = client(server, workspaceId: workspaceId)
        try await api.signIn(password: "p@ss word")
        return api
    }

    private func sampleActivity() -> ActivityPayload {
        ActivityPayload(projectId: "0b9a7c55-2f37-4f40-a7e5-5a8f0f6b2d11", startedAt: Date().addingTimeInterval(-600),
                        endedAt: Date())
    }

    private func abp(_ code: String, _ message: String) -> String {
        "{\"error\":{\"code\":\"\(code)\",\"message\":\"\(message)\",\"details\":null}}"
    }

    // MARK: Sign-in

    func testThePasswordGrantSendsNoTenantAndThisInstallsDeviceId() async throws {
        let server = MockServer([.tokenOK(refreshToken: "r1")])
        let api = client(server, workspaceId: nil)
        try await api.signIn(password: "p@ss word")
        let request = try XCTUnwrap(server.requests.first)
        XCTAssertEqual(request.method, "POST")
        XCTAssertEqual(request.url.absoluteString, "https://\(server.host)/connect/token",
                       "no double slash, and no __tenant: the workspace never travels with the sign-in")
        XCTAssertNil(request.url.query)
        XCTAssertEqual(request.header("Content-Type"), "application/x-www-form-urlencoded")
        XCTAssertNil(request.header(ApiClient.workspaceHeader))
        XCTAssertEqual(request.formFields, [
            "grant_type": "password",
            "username": "alice@acme.com",
            "password": "p@ss word",
            "client_id": "Tracker_App",
            "scope": "Tracker offline_access",
            "device_id": deviceId,
        ])
        let refresh = await api.currentRefreshToken()
        XCTAssertEqual(refresh, "r1")
    }

    func testRefusedCredentialsAreARejectedSignInWithTheServersReason() async throws {
        let server = MockServer([.json(400, #"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#)])
        do {
            try await client(server).signIn(password: "wrong")
            XCTFail("the server refuses")
        } catch let error as ApiError {
            guard case .authRejected(let rejection) = error else { return XCTFail("\(error)") }
            XCTAssertEqual(rejection.userMessage, "Invalid username or password!")
            XCTAssertTrue(error.isAuthRejection)
        }
    }

    func testAServerErrorOrThrottlingIsNotARejectedSignIn() async throws {
        for reply in [MockReply.empty(503), .json(429, "{}", headers: ["Retry-After": "60"]), .json(400, #"{"error":"invalid_client"}"#)] {
            let server = MockServer([reply])
            do {
                try await client(server).signIn(password: "pw")
                XCTFail("the server fails")
            } catch let error as ApiError {
                guard case .tokenFailed(let failure) = error else { return XCTFail("\(error)") }
                XCTAssertEqual(failure.status, reply.status)
                XCTAssertFalse(error.isAuthRejection)
                if reply.status == 429 {
                    XCTAssertEqual(failure.retryAfter, 60)
                }
            }
        }
    }

    func testAnUnreachableServerIsOffline() async throws {
        let server = MockServer([.offline(), .offline(), .offline()])
        let api = client(server, refreshToken: "r1")
        do {
            try await api.signIn(password: "pw")
            XCTFail("nothing answers")
        } catch let error as ApiError {
            XCTAssertTrue(error.isTransport)
            XCTAssertTrue(error.userMessage.contains("Cannot reach the server"))
        }
        let resumed = client(server, refreshToken: "r1")
        let upload = await resumed.uploadActivity(sampleActivity())
        XCTAssertEqual(UploadPolicy.decide(upload), .retry(RetryPlan(reason: .offline)))
        do {
            _ = try await resumed.myProjects()
            XCTFail("nothing answers")
        } catch let error as ApiError {
            XCTAssertTrue(error.isTransport)
        }
        let kept = await resumed.currentRefreshToken()
        XCTAssertEqual(kept, "r1", "offline says nothing about the sign-in: the token is kept")
    }

    func testAServerThatRefusesOfflineAccessIsAskedAgainWithoutIt() async throws {
        let server = MockServer([
            .json(400, #"{"error":"invalid_scope","error_description":"The specified 'scope' is invalid."}"#),
            .tokenOK(),
            .json(200, "[]"),
        ])
        let api = try await signedIn(server)
        XCTAssertEqual(server.requests[0].formFields["scope"], "Tracker offline_access")
        XCTAssertEqual(server.requests[1].formFields["scope"], "Tracker")
        XCTAssertEqual(server.requests[1].formFields["device_id"], deviceId)
        _ = try await api.myProjects()
        XCTAssertEqual(server.requests.count, 3)
    }

    func testAMustChangePasswordAnswerCarriesItsTokenAndTheChangeIsSent() async throws {
        let server = MockServer([
            .json(400, #"{"error":"invalid_grant","error_description":"ShouldChangePasswordOnNextLogin","changePasswordToken":"cpt-1"}"#),
            .tokenOK(),
        ])
        let api = client(server)
        var change: ApiClient.PasswordChange?
        do {
            try await api.signIn(password: "old-pw")
        } catch ApiError.authRejected(let rejection) {
            XCTAssertEqual(rejection.kind, .mustChangePassword)
            let token = try XCTUnwrap(rejection.changePasswordToken)
            change = ApiClient.PasswordChange(changePasswordToken: token, newPassword: "N3w-secret")
        }
        try await api.signIn(password: "old-pw", passwordChange: try XCTUnwrap(change))
        let fields = server.requests[1].formFields
        XCTAssertEqual(fields["ChangePasswordToken"], "cpt-1")
        XCTAssertEqual(fields["NewPassword"], "N3w-secret")
        XCTAssertEqual(fields["device_id"], deviceId)
    }

    // MARK: Workspaces

    func testTheWorkspacesAreListedWithoutAWorkspaceHeader() async throws {
        let mine = """
            [{"id":"\(acme)","name":"Acme","role":"owner","planName":"Team","isOwner":true,"isArchived":false},
             {"id":"\(globex)","name":"Globex","role":"member","isOwner":false,"isArchived":true,"archivedTime":"2026-09-01T00:00:00Z"}]
            """
        let server = MockServer([.tokenOK(), .json(200, mine)])
        let api = try await signedIn(server, workspaceId: acme)
        let list = try await api.workspaces()
        let request = server.requests[1]
        XCTAssertEqual(request.method, "GET")
        XCTAssertEqual(request.path, "/api/app/workspaces/mine")
        XCTAssertNil(request.header(ApiClient.workspaceHeader), "the list is never scoped to a workspace")
        XCTAssertEqual(request.header("Authorization"), "Bearer t0k3n")
        XCTAssertEqual(list.map { $0.name }, ["Acme", "Globex"])
        XCTAssertEqual(list[0].roleLabel, "Owner")
        XCTAssertEqual(list[0].planName, "Team")
        XCTAssertTrue(list[1].isArchived)
        XCTAssertEqual(list[1].roleLabel, "Member")
        // ABP's paged shape reads too.
        XCTAssertEqual(try Workspace.decodeList(from: Data(#"{"items":[{"id":"w1"}]}"#.utf8)).first?.displayName,
                       "Unnamed workspace")
    }

    func testAWorkspaceIsChosenAsTheContractSays() {
        let a = Workspace(id: acme, name: "Acme", role: "owner")
        let b = Workspace(id: globex, name: "Globex", role: "member")
        let archived = Workspace(id: "w-archived", name: "Old", isArchived: true)
        // Exactly one active workspace: chosen for the user.
        XCTAssertEqual(WorkspaceSelection.resolve([a, archived], preferred: nil), .selected(a))
        // Several: the one tracked before (ids compare like GUIDs), else ask.
        XCTAssertEqual(WorkspaceSelection.resolve([a, b], preferred: globex.uppercased()), .selected(b))
        XCTAssertEqual(WorkspaceSelection.resolve([a, b], preferred: nil), .choose([a, b]))
        XCTAssertEqual(WorkspaceSelection.resolve([a, b], preferred: "gone"), .choose([a, b]))
        // Archived ones are never offered, even when asked for.
        XCTAssertEqual(WorkspaceSelection.resolve([a, b, archived], preferred: "w-archived"), .choose([a, b]))
        XCTAssertEqual(WorkspaceSelection.resolve([archived], preferred: "w-archived"), WorkspaceSelection.noWorkspace)
        XCTAssertEqual(WorkspaceSelection.resolve([], preferred: nil), WorkspaceSelection.noWorkspace)
    }

    /// After the server ended access to the workspace tracked in, the user
    /// picks the next one: never taken for them, not even the only one left.
    func testAfterALostWorkspaceTheUserAlwaysChooses() {
        let a = Workspace(id: acme, name: "Acme", role: "owner")
        let b = Workspace(id: globex, name: "Globex", role: "member")
        let archived = Workspace(id: "w-archived", name: "Old", isArchived: true)
        XCTAssertEqual(WorkspaceSelection.resolve([a, archived], preferred: nil, autoSelect: false), .choose([a]))
        XCTAssertEqual(WorkspaceSelection.resolve([a, b], preferred: acme, autoSelect: false), .choose([a, b]))
        XCTAssertEqual(WorkspaceSelection.resolve([archived], preferred: nil, autoSelect: false),
                       WorkspaceSelection.noWorkspace)
        XCTAssertEqual(WorkspaceSelection.resolve([], preferred: nil, autoSelect: false), WorkspaceSelection.noWorkspace)
    }

    func testEveryWorkspaceScopedCallNamesTheWorkspace() async throws {
        let server = MockServer([
            .tokenOK(),
            .json(200, "[]"),
            .json(200, "{}"),
            .json(200, #"{"serverTime":"2026-10-05T10:00:00Z"}"#),
            .json(200, #"{"totalTrackedMinutes":1}"#),
            .json(200, #"{"items":[]}"#),
            .json(200, #"{"currentUser":{"isAuthenticated":true,"id":"u1"}}"#),
            .json(200, "{}"),
        ])
        let api = try await signedIn(server, workspaceId: acme)
        _ = try await api.myProjects()
        _ = await api.uploadActivity(sampleActivity())
        _ = try await api.sendHeartbeat(Heartbeat(deviceId: deviceId, status: .tracking), timeout: 5)
        _ = try await api.summary(from: Date().addingTimeInterval(-60), to: Date(), timeZone: nil)
        _ = try await api.presence()
        _ = try await api.currentUserId()
        // A queued row goes to its own workspace, whatever the current one is.
        _ = await api.uploadActivity(sampleActivity(), workspaceId: globex)

        let requests = server.requests
        XCTAssertEqual(requests.map { $0.path }, [
            "/connect/token", "/api/app/project/my-projects", "/api/app/activity", "/api/app/presence/heartbeat",
            "/api/app/reporting/summary", "/api/app/presence/team", "/api/abp/application-configuration",
            "/api/app/activity",
        ])
        for request in requests[1...5] {
            XCTAssertEqual(request.header(ApiClient.workspaceHeader), acme, request.path)
        }
        XCTAssertNil(requests[0].header(ApiClient.workspaceHeader), "the token request")
        XCTAssertNil(requests[6].header(ApiClient.workspaceHeader), "the ABP configuration is not workspace-scoped")
        XCTAssertEqual(requests[7].header(ApiClient.workspaceHeader), globex)
    }

    func testSwitchingWorkspaceKeepsTheSignIn() async throws {
        let server = MockServer([.tokenOK(), .json(200, "[]"), .json(200, "[]")])
        let api = try await signedIn(server, workspaceId: acme)
        _ = try await api.myProjects()
        api.useWorkspace(globex)
        _ = try await api.myProjects()
        XCTAssertEqual(server.requests.count, 3, "no new sign-in")
        XCTAssertEqual(server.requests[2].header(ApiClient.workspaceHeader), globex)
        XCTAssertEqual(api.currentWorkspaceId(), globex)
    }

    func testAScopedCallWithoutAChosenWorkspaceNeverReachesTheServer() async throws {
        let server = MockServer([.tokenOK()])
        let api = try await signedIn(server, workspaceId: nil)
        do {
            _ = try await api.myProjects()
            XCTFail("no workspace is chosen")
        } catch let error as ApiError {
            guard case .invalidConfiguration = error else { return XCTFail("\(error)") }
        }
        XCTAssertEqual(server.requests.count, 1)
    }

    // MARK: Tokens

    func testAnExpiredTokenIsRenewedWithTheRotatedRefreshTokenAndTheDeviceId() async throws {
        let server = MockServer([
            .tokenOK("first", refreshToken: "r1"),
            .empty(401),
            .tokenOK("second", refreshToken: "r2"),
            .json(200, "{}"),
        ])
        let api = try await signedIn(server)
        let rotated = RotatedTokens()
        api.observeRefreshTokens { token in await rotated.add(token) }
        let result = await api.uploadActivity(sampleActivity())
        XCTAssertEqual(result, .accepted(status: 200))
        let refresh = server.requests[2].formFields
        XCTAssertEqual(refresh["grant_type"], "refresh_token")
        XCTAssertEqual(refresh["refresh_token"], "r1")
        XCTAssertEqual(refresh["device_id"], deviceId)
        XCTAssertNil(refresh["password"], "the password is never re-sent")
        XCTAssertNil(server.requests[2].header(ApiClient.workspaceHeader))
        XCTAssertEqual(server.requests[3].header("Authorization"), "Bearer second")
        let kept = await api.currentRefreshToken()
        XCTAssertEqual(kept, "r2", "the rotated refresh token is kept")
        let seen = await rotated.all
        XCTAssertEqual(seen, ["r2"], "and reported, so the saved sign-in holds it")
    }

    func testASavedRefreshTokenResumesWithoutThePassword() async throws {
        let server = MockServer([.tokenOK("resumed", refreshToken: "r9"), .json(200, "[]")])
        let api = client(server, refreshToken: "r8")
        _ = try await api.myProjects()
        let renewal = server.requests[0].formFields
        XCTAssertEqual(renewal["grant_type"], "refresh_token")
        XCTAssertEqual(renewal["refresh_token"], "r8")
        XCTAssertEqual(renewal["device_id"], deviceId)
        XCTAssertEqual(server.requests[1].header("Authorization"), "Bearer resumed")
    }

    /// A refused refresh token ends the sign-in: the user signs in again; the
    /// password is never tried behind their back.
    func testARefusedRefreshTokenMeansSignInExpiredNeverAPasswordRetry() async throws {
        let server = MockServer([
            .tokenOK("first", refreshToken: "r1"),
            .empty(401),
            .json(400, #"{"error":"invalid_grant","error_description":"The refresh token is no longer valid."}"#),
        ])
        let api = try await signedIn(server)
        let result = await api.uploadActivity(sampleActivity())
        guard case .signInExpired(let rejection) = UploadPolicy.decide(result) else {
            return XCTFail("expected sign-in expired, got \(result)")
        }
        XCTAssertEqual(rejection.userMessage, "The refresh token is no longer valid.")
        XCTAssertEqual(server.requests.count, 3, "no password grant follows")
        XCTAssertEqual(server.requests.filter { $0.formFields["grant_type"] == "password" }.count, 1,
                       "only the user's own sign-in")
        let kept = await api.currentRefreshToken()
        XCTAssertNil(kept, "the refused token is dropped")

        // Nothing is tried again until the user signs in.
        let again = await api.uploadActivity(sampleActivity())
        XCTAssertEqual(UploadPolicy.decide(again), .signInExpired(AuthRejection.sessionEnded))
        XCTAssertEqual(server.requests.count, 3)
    }

    func testWithoutARefreshTokenTheSignInEndsWithTheAccessToken() async throws {
        let server = MockServer([.tokenOK("only", expiresIn: 3600)])
        let clock = TestClock(utc("2026-10-05T10:00:00Z"))
        let api = client(server, now: { clock.now })
        try await api.signIn(password: "pw")
        // An hour later the access token has run out, and nothing can renew it.
        clock.advance(3600)
        let result = await api.uploadActivity(sampleActivity())
        XCTAssertEqual(UploadPolicy.decide(result), .signInExpired(AuthRejection.sessionEnded))
        XCTAssertEqual(server.requests.count, 1)
        XCTAssertTrue(AuthRejection.sessionEnded.userMessage.contains("Sign in again"))
    }

    func testSigningOutForgetsTheTokens() async throws {
        let server = MockServer([.tokenOK(refreshToken: "r1")])
        let api = try await signedIn(server)
        await api.signOut()
        let token = await api.currentRefreshToken()
        XCTAssertNil(token)
        let hasAccess = await api.hasAccessToken()
        XCTAssertFalse(hasAccess)
    }

    // MARK: Redirects

    func testRedirectsAreNeverFollowed() async throws {
        let session = ApiClient.makeSession()
        XCTAssertTrue(session.delegate is RedirectRefusal)
        let original = URL(string: "https://tracker.example.com/api/app/activity")!
        let task = session.dataTask(with: original)
        let response = try XCTUnwrap(HTTPURLResponse(url: original, statusCode: 302, httpVersion: "HTTP/1.1",
                                                     headerFields: ["Location": "https://elsewhere.example.net/"]))
        let next = await RedirectRefusal().urlSession(session, task: task, willPerformHTTPRedirection: response,
                                                      newRequest: URLRequest(url: URL(string: "https://elsewhere.example.net/")!))
        XCTAssertNil(next, "the 3xx answer is returned as it is")
        task.cancel()
        session.invalidateAndCancel()

        // A redirect answer fails the call with a clear reason.
        XCTAssertTrue(ApiError.http(HTTPFailure(status: 301)).userMessage.contains("redirect"))
        XCTAssertTrue(TokenFailure(status: 307, error: nil, errorDescription: nil).userMessage.contains("redirect"))
        guard case .retry = UploadPolicy.decide(status: 302, error: nil, retryAfter: nil) else {
            return XCTFail("a redirect is never a park")
        }
    }

    // MARK: Uploads

    func testA401UploadRenewsTheTokenAndResendsWithSentAt() async throws {
        let server = MockServer([
            .tokenOK(refreshToken: "r1"),
            .empty(401),
            .tokenOK(refreshToken: "r2"),
            .json(200, "{}", headers: ["Date": "Mon, 05 Oct 2026 10:00:00 GMT"]),
        ])
        let api = try await signedIn(server)
        let result = await api.uploadActivity(sampleActivity())
        XCTAssertEqual(result, .accepted(status: 200))

        let requests = server.requests
        XCTAssertEqual(requests.count, 4)
        XCTAssertEqual(requests[1].path, "/api/app/activity")
        XCTAssertEqual(requests[1].method, "POST")
        XCTAssertEqual(requests[2].path, "/connect/token")
        let upload = requests[3]
        XCTAssertEqual(upload.header("Authorization"), "Bearer t0k3n")
        XCTAssertEqual(upload.header("User-Agent"), "DosiTracker/9.9.9 (macOS 14.5)")
        XCTAssertEqual(upload.header(ApiClient.workspaceHeader), acme)
        XCTAssertTrue(upload.header("Content-Type")?.hasPrefix("application/json") ?? false)
        let json = jsonDictionary(upload.body)
        let sentAt = try XCTUnwrap(json["sentAt"] as? String, "sentAt is stamped at send time")
        XCTAssertNotNil(WireDate.date(from: sentAt))
        XCTAssertNil(json["productivity"], "unmeasured: left out")
        // The Date header of the last answer set the clock reading.
        let reading = await api.lastClockReading()
        XCTAssertNotNil(reading)
    }

    func testAnOversizedUploadAsksForItsCapturesToBeStripped() async throws {
        let server = MockServer([.tokenOK(), .empty(413)])
        let api = try await signedIn(server)
        let result = await api.uploadActivity(sampleActivity())
        XCTAssertEqual(UploadPolicy.decide(result), .stripCapturesAndResend(reason: "HTTP 413"))
    }

    func testThrottledUploadsCarryTheRetryAfter() async throws {
        let server = MockServer([.tokenOK(), .empty(429, headers: ["Retry-After": "90"])])
        let api = try await signedIn(server)
        let result = await api.uploadActivity(sampleActivity())
        XCTAssertEqual(UploadPolicy.decide(result), .retry(RetryPlan(reason: .throttled, after: 90)))
    }

    func testBusinessErrorsKeepTheAbpBody() async throws {
        let server = MockServer([.tokenOK(), .json(409, abp("Tracker:ActivityOverlaps", "Overlaps another block."))])
        let api = try await signedIn(server)
        let result = await api.uploadActivity(sampleActivity())
        XCTAssertEqual(UploadPolicy.decide(result), .park(reason: "Overlaps another block."))
    }

    func testARevokedDeviceOrALostWorkspaceIsAnAccessLoss() async throws {
        let cases: [(Int, String, AccessLoss.Kind)] = [
            (403, "Tracker:DeviceRevoked", .deviceRevoked),
            (403, "Tracker:NotWorkspaceMember", .notWorkspaceMember),
            (403, "Tracker:WorkspaceArchived", .workspaceArchived),
            (400, "Tracker:WorkspaceRequired", .workspaceRequired),
        ]
        for (status, code, kind) in cases {
            let server = MockServer([.tokenOK(), .json(status, abp(code, "Refused.")), .json(status, abp(code, "Refused."))])
            let api = try await signedIn(server)
            let result = await api.uploadActivity(sampleActivity())
            XCTAssertEqual(UploadPolicy.decide(result), .accessLost(AccessLoss(kind: kind, message: "Refused.")), code)
            do {
                _ = try await api.sendHeartbeat(Heartbeat(deviceId: deviceId, status: .tracking), timeout: 5)
                XCTFail("refused")
            } catch let error as ApiError {
                XCTAssertEqual(AccessLoss.of(error)?.kind, kind, code)
            }
        }
    }

    // MARK: Projects, totals, presence

    func testProjectsParseWithTheNewPolicyFieldsAndErrorsKeepTheAbpBody() async throws {
        let projects = """
            [{"id":"p1","title":"Website","intervalMinutes":10,"allowScreenshot":true,"allowWebcam":false,
              "allowKeyboard":true,"allowMouse":true,"allowActiveWindow":true,"allowRunningPrograms":true,
              "idleThresholdMinutes":15,"blurScreenshots":true}]
            """
        let server = MockServer([
            .tokenOK(),
            .json(200, projects),
            .json(403, abp("Volo.Authorization:010001", "Authorization failed!")),
        ])
        let api = try await signedIn(server)
        let list = try await api.myProjects()
        XCTAssertEqual(list.first?.idleThresholdMinutes, 15)
        XCTAssertEqual(list.first?.blurScreenshots, true)
        XCTAssertEqual(server.requests[1].path, "/api/app/project/my-projects")
        XCTAssertEqual(server.requests[1].method, "GET")

        do {
            _ = try await api.summary(from: Date().addingTimeInterval(-3600), to: Date(), timeZone: "Asia/Dhaka")
            XCTFail("the server refuses")
        } catch let error as ApiError {
            XCTAssertEqual(error.status, 403)
            XCTAssertEqual(error.userMessage, "Authorization failed!")
            XCTAssertNil(AccessLoss.of(error), "an authorization failure is not an access loss")
        }
        XCTAssertTrue(server.requests[2].url.absoluteString.contains("TimeZone=Asia%2FDhaka"))
    }

    func testTheSummaryQuerySurvivesUrlEncoding() async throws {
        let server = MockServer([.tokenOK(), .json(200, #"{"totalTrackedMinutes":90.5,"averageProductivity":null}"#)])
        let from = utc("2026-09-29T18:00:00Z")
        let to = utc("2026-09-30T10:29:24.976Z")
        let api = try await signedIn(server)
        let summary = try await api.summary(from: from, to: to,
                                                                     timeZone: "America/Argentina/Buenos_Aires")
        XCTAssertEqual(summary.totalTrackedMinutes, 90.5)

        // What the server sees after decoding must still parse to the same instants.
        let request = server.requests[1]
        XCTAssertEqual(request.path, "/api/app/reporting/summary")
        let items = URLComponents(url: request.url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let value = { (name: String) in items.first { $0.name == name }?.value }
        XCTAssertEqual(value("From").flatMap { WireDate.date(from: $0) }, from)
        XCTAssertEqual(value("To").flatMap { WireDate.date(from: $0) }, to)
        XCTAssertEqual(value("TimeZone"), "America/Argentina/Buenos_Aires")
        XCTAssertFalse(request.url.absoluteString.contains("+"), "no raw + that a server would read as a space")

        // No zone: the parameter is left out (the server's default applies).
        XCTAssertEqual(ApiClient.summaryQuery(from: from, to: to, timeZone: nil).count, 2)
        XCTAssertEqual(ApiClient.summaryQuery(from: from, to: to, timeZone: "  ").count, 2)
        XCTAssertEqual(ApiClient.summaryQuery(from: from, to: to, timeZone: nil, userId: "u1").last?.1, "u1")
    }

    func testTheTotalsAreScopedToTheSignedInUser() async throws {
        let userId = "3a1b2c3d-0000-4000-8000-000000000001"
        let server = MockServer([
            .tokenOK(),
            .json(200, #"{"currentUser":{"isAuthenticated":true,"id":"\#(userId)","userName":"bob"},"auth":{}}"#),
            .json(200, #"{"totalTrackedMinutes":120}"#),
            .json(200, #"{"totalTrackedMinutes":600}"#),
        ])
        let api = try await signedIn(server)
        let id = try await api.currentUserId()
        XCTAssertEqual(id, userId)
        let today = try await api.summary(from: Date().addingTimeInterval(-3600), to: Date(), timeZone: "UTC", userId: id)
        XCTAssertEqual(today.totalTrackedMinutes, 120)
        // Read once: the second call answers from memory.
        let again = try await api.currentUserId()
        XCTAssertEqual(again, userId)
        _ = try await api.summary(from: Date().addingTimeInterval(-7200), to: Date(), timeZone: "UTC", userId: id)
        XCTAssertEqual(server.requests[1].path, "/api/abp/application-configuration")
        for request in server.requests[2...] {
            XCTAssertEqual(request.path, "/api/app/reporting/summary")
            let items = URLComponents(url: request.url, resolvingAgainstBaseURL: false)?.queryItems ?? []
            XCTAssertEqual(items.first { $0.name == "UserId" }?.value, userId)
        }
        XCTAssertNil(ApplicationConfiguration.currentUserId(from: Data(#"{"currentUser":{"isAuthenticated":false}}"#.utf8)))
        XCTAssertNil(ApplicationConfiguration.currentUserId(from: Data("not json".utf8)))
    }

    func testHeartbeatsPostTheContractBodyAndReadTheServerTime() async throws {
        let server = MockServer([.tokenOK(), .json(200, #"{"serverTime":"2026-10-05T10:00:00Z"}"#)])
        let heartbeat = Heartbeat(deviceId: deviceId, deviceName: "Mac", platform: "macos",
                                  agentVersion: "0.2.0", status: .tracking, projectId: "p1", currentApp: "Xcode",
                                  timeZone: "Asia/Dhaka", utcOffsetMinutes: 360)
        let api = try await signedIn(server)
        let result = try await api.sendHeartbeat(heartbeat, timeout: 5)
        XCTAssertEqual(result.serverTime, utc("2026-10-05T10:00:00Z"))
        let request = server.requests[1]
        XCTAssertEqual(request.path, "/api/app/presence/heartbeat")
        XCTAssertEqual(request.method, "POST")
        let body = jsonDictionary(request.body)
        XCTAssertEqual(body["status"] as? String, "tracking")
        XCTAssertEqual(body["currentApp"] as? String, "Xcode")
        XCTAssertNotNil(body["sentAt"] as? String)
    }

    func testTeamPresenceIsListed() async throws {
        let server = MockServer([
            .tokenOK(),
            .json(200, #"{"items":[{"userId":"u1","userName":"alice","deviceId":"d1","status":"paused","online":true}]}"#),
        ])
        let api = try await signedIn(server)
        let devices = try await api.presence()
        XCTAssertEqual(devices.map { $0.presenceStatus }, [.paused])
        XCTAssertEqual(server.requests[1].path, "/api/app/presence/team")
        XCTAssertEqual(server.requests[1].method, "GET")
    }

    func testHeartbeatsAreBoundedByTheirTimeout() async throws {
        let server = MockServer([.tokenOK(), .hanging()])
        let heartbeat = Heartbeat(deviceId: UUID().uuidString.lowercased(), status: .stopped)
        let api = try await signedIn(server)
        let started = Date()
        do {
            _ = try await api.sendHeartbeat(heartbeat, timeout: 0.8)
            XCTFail("the server never answers")
        } catch let error as ApiError {
            XCTAssertTrue(error.isTransport)
        }
        XCTAssertLessThan(Date().timeIntervalSince(started), 3)
    }

    func testAHeartbeatWhoseAnswerCannotBeReadStillCounts() async throws {
        let server = MockServer([.tokenOK(), .json(200, "not json")])
        let api = try await signedIn(server)
        let result = try await api.sendHeartbeat(Heartbeat(deviceId: "d", status: .idle), timeout: 5)
        XCTAssertNil(result.serverTime)
    }

    // MARK: Clock and Retry-After

    func testTheServerClockComesFromTheDateHeader() async throws {
        let device = utc("2026-10-05T10:00:00Z")
        let server = MockServer([.tokenOK(), .json(200, "[]", headers: ["Date": "Mon, 05 Oct 2026 10:03:30 GMT"])])
        let api = client(server, now: { device })
        try await api.signIn(password: "pw")
        _ = try await api.myProjects()
        let reading = await api.lastClockReading()
        XCTAssertEqual(reading?.offsetSeconds, 210)
        XCTAssertEqual(reading?.skewMinutes, 4)
        XCTAssertEqual(reading?.warning, "Your computer clock is off by 4 min")

        XCTAssertEqual(ClockReading(dateHeader: "Mon, 05 Oct 2026 09:59:00 GMT", deviceNow: device)?.offsetSeconds, -60)
        XCTAssertNil(ClockReading(dateHeader: "yesterday", deviceNow: device))
        let close = ClockReading(offsetSeconds: -60, measuredAt: device)
        XCTAssertNil(close.skewMinutes, "within two minutes: no warning")
        XCTAssertNil(close.warning)
        let ahead = ClockReading(offsetSeconds: -7 * 60 - 20, measuredAt: device)
        XCTAssertEqual(ahead.skewMinutes, -7, "device clock 7 min ahead")
        XCTAssertTrue(ahead.deviceIsAhead)
        XCTAssertEqual(ClockReading.skewWarningMinutes(offsetSeconds: 121), 2)
        XCTAssertNil(ClockReading.skewWarningMinutes(offsetSeconds: 120))
    }

    func testRetryAfterTakesSecondsOrADateAndIsBounded() {
        let now = utc("2026-10-05T10:00:00Z")
        XCTAssertEqual(ApiClient.retryAfterSeconds("120", serverDate: nil, deviceNow: now), 120)
        XCTAssertEqual(ApiClient.retryAfterSeconds(" 0 ", serverDate: nil, deviceNow: now), 0)
        XCTAssertEqual(ApiClient.retryAfterSeconds("Mon, 05 Oct 2026 10:05:00 GMT", serverDate: nil, deviceNow: now), 300)
        // Measured against the server's clock when it is known.
        XCTAssertEqual(ApiClient.retryAfterSeconds("Mon, 05 Oct 2026 10:05:00 GMT",
                                                   serverDate: now.addingTimeInterval(120), deviceNow: now), 180)
        // In the past: retry now. Absurdly far: capped.
        XCTAssertEqual(ApiClient.retryAfterSeconds("Mon, 05 Oct 2026 09:00:00 GMT", serverDate: nil, deviceNow: now), 0)
        XCTAssertEqual(ApiClient.retryAfterSeconds("99999999", serverDate: nil, deviceNow: now), ApiClient.maxRetryAfter)
        XCTAssertNil(ApiClient.retryAfterSeconds("soon", serverDate: nil, deviceNow: now))
        XCTAssertNil(ApiClient.retryAfterSeconds(nil, serverDate: nil, deviceNow: now))
    }

    // MARK: Deadlines

    func testADeadlineReturnsOnTimeEvenWhenTheWorkIgnoresCancellation() async {
        let started = Date()
        do {
            _ = try await ApiClient.withDeadline(0.3) { () async throws -> Int in
                // Not cancellable: a blocking wait.
                Thread.sleep(forTimeInterval: 1.5)
                return 1
            }
            XCTFail("the deadline passes first")
        } catch let error as ApiError {
            XCTAssertEqual(error, .transport(.timedOut))
        } catch {
            XCTFail("\(error)")
        }
        XCTAssertLessThan(Date().timeIntervalSince(started), 1.2)
    }

    func testADeadlinePassesResultsAndErrorsThrough() async throws {
        let value = try await ApiClient.withDeadline(5) { () async throws -> Int in 42 }
        XCTAssertEqual(value, 42)
        do {
            _ = try await ApiClient.withDeadline(5) { () async throws -> Int in throw ApiError.encoding("x") }
            XCTFail("the error passes through")
        } catch let error as ApiError {
            XCTAssertEqual(error, .encoding("x"))
        }
    }

    func testTheIdentityNeverPrintsItsRefreshToken() {
        let identity = ApiClient.Identity(username: "alice@acme.com", deviceId: deviceId, workspaceId: acme,
                                          refreshToken: "refresh-fixture-9")
        XCTAssertFalse("\(identity)".contains("refresh-fixture-9"))
        XCTAssertFalse(String(reflecting: identity).contains("refresh-fixture-9"))
        XCTAssertEqual(ApiClient.Identity(username: "root", deviceId: deviceId).description, "root @ (no workspace)")
    }
}

/// Refresh tokens reported by a client, in order.
private actor RotatedTokens {
    private(set) var all: [String] = []

    func add(_ token: String) {
        all.append(token)
    }
}

/// A device clock the test moves.
private final class TestClock: @unchecked Sendable {
    private let lock = NSLock()
    private var current: Date

    init(_ start: Date) {
        current = start
    }

    var now: Date {
        lock.lock()
        defer { lock.unlock() }
        return current
    }

    func advance(_ seconds: TimeInterval) {
        lock.lock()
        current = current.addingTimeInterval(seconds)
        lock.unlock()
    }
}
