import XCTest
@testable import DosiTrackerCore

/// The API client against a canned server (ports of the Windows agent's
/// `api.rs` tests, through URLProtocol instead of a local socket).
final class ApiClientTests: XCTestCase {
    private func client(_ server: MockServer, workspace: String = "acme", offlineAccess: Bool = true,
                        now: @escaping @Sendable () -> Date = { Date() }) -> ApiClient {
        ApiClient(
            baseURL: URL(string: server.baseURL.absoluteString + "/")!,
            account: ApiClient.Account(workspace: workspace, username: "alice@acme.com", password: "p@ss word"),
            session: MockServer.session(),
            requestsOfflineAccess: offlineAccess,
            userAgent: "DosiTracker/9.9.9 (macOS 14.5)",
            now: now
        )
    }

    private func sampleActivity() -> ActivityPayload {
        ActivityPayload(projectId: "0b9a7c55-2f37-4f40-a7e5-5a8f0f6b2d11", startedAt: Date().addingTimeInterval(-600),
                        endedAt: Date())
    }

    private func abp(_ code: String, _ message: String) -> String {
        "{\"error\":{\"code\":\"\(code)\",\"message\":\"\(message)\",\"details\":null}}"
    }

    // MARK: Sign-in

    func testSignInUsesThePasswordGrantWithTheTenantInTheQuery() async throws {
        let server = MockServer([.tokenOK()])
        try await client(server).signIn()
        let request = try XCTUnwrap(server.requests.first)
        XCTAssertEqual(request.method, "POST")
        XCTAssertEqual(request.url.absoluteString, "https://\(server.host)/connect/token?__tenant=acme",
                       "no double slash; the tenant travels in the query, not the body")
        XCTAssertEqual(request.header("Content-Type"), "application/x-www-form-urlencoded")
        XCTAssertEqual(request.formFields, [
            "grant_type": "password",
            "username": "alice@acme.com",
            "password": "p@ss word",
            "client_id": "Tracker_App",
            "scope": "Tracker offline_access",
        ])
        XCTAssertNil(request.formFields["__tenant"])
    }

    func testWorkspaceNamesArePercentEncodedAndAHostAccountHasNoTenant() async throws {
        let server = MockServer([.tokenOK(), .tokenOK()])
        try await client(server, workspace: " Acme Corp/EU ").signIn()
        try await client(server, workspace: "").signIn()
        XCTAssertTrue(server.requests[0].url.absoluteString.hasSuffix("/connect/token?__tenant=Acme%20Corp%2FEU"))
        XCTAssertNil(server.requests[1].url.query)
        XCTAssertEqual(PercentEncoding.encode("plain-name_1.x~"), "plain-name_1.x~")
    }

    func testRefusedCredentialsAreARejectedSignInWithTheServersReason() async throws {
        let server = MockServer([.json(400, #"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#)])
        do {
            try await client(server).signIn()
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
                try await client(server).signIn()
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
        let api = client(server)
        do {
            try await api.signIn()
            XCTFail("nothing answers")
        } catch let error as ApiError {
            XCTAssertTrue(error.isTransport)
            XCTAssertTrue(error.userMessage.contains("Cannot reach the server"))
        }
        let upload = await api.uploadActivity(sampleActivity())
        XCTAssertEqual(UploadPolicy.decide(upload), .retry(RetryPlan(reason: .offline)))
        do {
            _ = try await api.myProjects()
            XCTFail("nothing answers")
        } catch let error as ApiError {
            XCTAssertTrue(error.isTransport)
        }
    }

    func testAServerThatRefusesOfflineAccessIsAskedAgainWithoutIt() async throws {
        let server = MockServer([
            .json(400, #"{"error":"invalid_scope","error_description":"The specified 'scope' is invalid."}"#),
            .tokenOK(),
            .json(200, "[]"),
        ])
        let api = client(server)
        try await api.signIn()
        XCTAssertEqual(server.requests[0].formFields["scope"], "Tracker offline_access")
        XCTAssertEqual(server.requests[1].formFields["scope"], "Tracker")
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
            try await api.signIn()
        } catch ApiError.authRejected(let rejection) {
            XCTAssertEqual(rejection.kind, .mustChangePassword)
            let token = try XCTUnwrap(rejection.changePasswordToken)
            change = ApiClient.PasswordChange(changePasswordToken: token, newPassword: "N3w-secret")
        }
        try await api.signIn(passwordChange: try XCTUnwrap(change))
        let fields = server.requests[1].formFields
        XCTAssertEqual(fields["ChangePasswordToken"], "cpt-1")
        XCTAssertEqual(fields["NewPassword"], "N3w-secret")
        let account = await api.currentAccount()
        XCTAssertEqual(account.password, "N3w-secret", "the new password is what gets saved")
    }

    // MARK: Uploads

    func testA401UploadSignsInAgainAndResendsWithSentAt() async throws {
        let server = MockServer([
            .tokenOK(),
            .empty(401),
            .tokenOK(),
            .json(200, "{}", headers: ["Date": "Mon, 05 Oct 2026 10:00:00 GMT"]),
        ])
        let api = client(server)
        let result = await api.uploadActivity(sampleActivity())
        XCTAssertEqual(result, .accepted(status: 200))

        let requests = server.requests
        XCTAssertEqual(requests.count, 4)
        XCTAssertTrue(requests[0].url.absoluteString.hasSuffix("/connect/token?__tenant=acme"))
        XCTAssertEqual(requests[1].path, "/api/app/activity")
        XCTAssertEqual(requests[1].method, "POST")
        XCTAssertEqual(requests[2].path, "/connect/token")
        let upload = requests[3]
        XCTAssertEqual(upload.header("Authorization"), "Bearer t0k3n")
        XCTAssertEqual(upload.header("User-Agent"), "DosiTracker/9.9.9 (macOS 14.5)")
        XCTAssertTrue(upload.header("Content-Type")?.hasPrefix("application/json") ?? false)
        let json = jsonDictionary(upload.body)
        let sentAt = try XCTUnwrap(json["sentAt"] as? String, "sentAt is stamped at send time")
        XCTAssertNotNil(WireDate.date(from: sentAt))
        XCTAssertNil(json["productivity"], "unmeasured: left out")
        // The Date header of the last answer set the clock reading.
        let reading = await api.lastClockReading()
        XCTAssertNotNil(reading)
    }

    func testTheRefreshTokenRenewsTheAccessTokenBeforeThePasswordIsUsedAgain() async throws {
        let server = MockServer([
            .tokenOK("first", refreshToken: "r1"),
            .empty(401),
            .tokenOK("second", refreshToken: "r2"),
            .json(200, "{}"),
        ])
        let result = await client(server).uploadActivity(sampleActivity())
        XCTAssertEqual(result, .accepted(status: 200))
        let refresh = server.requests[2].formFields
        XCTAssertEqual(refresh["grant_type"], "refresh_token")
        XCTAssertEqual(refresh["refresh_token"], "r1")
        XCTAssertNil(refresh["password"], "the password is not re-sent")
        XCTAssertEqual(server.requests[3].header("Authorization"), "Bearer second")
    }

    func testARejectedRefreshTokenFallsBackToThePassword() async throws {
        let server = MockServer([
            .tokenOK("first", refreshToken: "r1"),
            .empty(401),
            .json(400, #"{"error":"invalid_grant","error_description":"The refresh token is no longer valid."}"#),
            .tokenOK("second"),
            .json(200, "{}"),
        ])
        let result = await client(server).uploadActivity(sampleActivity())
        XCTAssertEqual(result, .accepted(status: 200))
        XCTAssertEqual(server.requests[3].formFields["grant_type"], "password")
    }

    func testRefusedCredentialsDuringAnUploadMeanSignInExpired() async throws {
        let server = MockServer([
            .json(400, #"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#),
        ])
        let result = await client(server).uploadActivity(sampleActivity())
        guard case .signInExpired(let rejection) = UploadPolicy.decide(result) else {
            return XCTFail("expected sign-in expired, got \(result)")
        }
        XCTAssertEqual(rejection.userMessage, "Invalid username or password!")
    }

    func testAnOversizedUploadAsksForItsCapturesToBeStripped() async throws {
        let server = MockServer([.tokenOK(), .empty(413)])
        let result = await client(server).uploadActivity(sampleActivity())
        XCTAssertEqual(UploadPolicy.decide(result), .stripCapturesAndResend(reason: "HTTP 413"))
    }

    func testThrottledUploadsCarryTheRetryAfter() async throws {
        let server = MockServer([.tokenOK(), .empty(429, headers: ["Retry-After": "90"])])
        let result = await client(server).uploadActivity(sampleActivity())
        XCTAssertEqual(UploadPolicy.decide(result), .retry(RetryPlan(reason: .throttled, after: 90)))
    }

    func testBusinessErrorsKeepTheAbpBody() async throws {
        let server = MockServer([.tokenOK(), .json(409, abp("Tracker:ActivityOverlaps", "Overlaps another block."))])
        let result = await client(server).uploadActivity(sampleActivity())
        XCTAssertEqual(UploadPolicy.decide(result), .park(reason: "Overlaps another block."))
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
        let api = client(server)
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
        }
        XCTAssertTrue(server.requests[2].url.absoluteString.contains("TimeZone=Asia%2FDhaka"))
    }

    func testTheSummaryQuerySurvivesUrlEncoding() async throws {
        let server = MockServer([.tokenOK(), .json(200, #"{"totalTrackedMinutes":90.5,"averageProductivity":null}"#)])
        let from = utc("2026-09-29T18:00:00Z")
        let to = utc("2026-09-30T10:29:24.976Z")
        let summary = try await client(server).summary(from: from, to: to, timeZone: "America/Argentina/Buenos_Aires")
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
        let api = client(server)
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
        let heartbeat = Heartbeat(deviceId: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11", deviceName: "Mac", platform: "macos",
                                  agentVersion: "0.2.0", status: .tracking, projectId: "p1", currentApp: "Xcode",
                                  timeZone: "Asia/Dhaka", utcOffsetMinutes: 360)
        let result = try await client(server).sendHeartbeat(heartbeat, timeout: 5)
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
        let devices = try await client(server).presence()
        XCTAssertEqual(devices.map { $0.presenceStatus }, [.paused])
        XCTAssertEqual(server.requests[1].path, "/api/app/presence/team")
        XCTAssertEqual(server.requests[1].method, "GET")
    }

    func testHeartbeatsAreBoundedByTheirTimeout() async throws {
        let server = MockServer([.tokenOK(), .hanging()])
        let heartbeat = Heartbeat(deviceId: UUID().uuidString.lowercased(), status: .stopped)
        let started = Date()
        do {
            _ = try await client(server).sendHeartbeat(heartbeat, timeout: 0.8)
            XCTFail("the server never answers")
        } catch let error as ApiError {
            XCTAssertTrue(error.isTransport)
        }
        XCTAssertLessThan(Date().timeIntervalSince(started), 3)
    }

    func testAHeartbeatWhoseAnswerCannotBeReadStillCounts() async throws {
        let server = MockServer([.tokenOK(), .json(200, "not json")])
        let result = try await client(server).sendHeartbeat(Heartbeat(deviceId: "d", status: .idle), timeout: 5)
        XCTAssertNil(result.serverTime)
    }

    // MARK: Clock and Retry-After

    func testTheServerClockComesFromTheDateHeader() async throws {
        let device = utc("2026-10-05T10:00:00Z")
        let server = MockServer([.tokenOK(), .json(200, "[]", headers: ["Date": "Mon, 05 Oct 2026 10:03:30 GMT"])])
        let api = client(server, now: { device })
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

    func testTheAccountNeverPrintsItsPassword() {
        let account = ApiClient.Account(workspace: "Acme", username: "alice@acme.com", password: "s3cr3t")
        XCTAssertFalse("\(account)".contains("s3cr3t"))
        XCTAssertFalse(String(reflecting: account).contains("s3cr3t"))
        XCTAssertEqual(ApiClient.Account(workspace: " ", username: "root", password: "x").description, "root @ (host)")
    }
}
