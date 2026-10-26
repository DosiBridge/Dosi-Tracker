import XCTest
@testable import DosiTrackerCore

/// The wire shapes (CONTRACT §1-§3): exact camelCase keys, UTC "Z" dates with
/// milliseconds, nil values left out, and lenient decoding of what older
/// servers and older builds produce. Ports of the Windows agent's
/// `model.rs` tests.
final class ModelsCodingTests: XCTestCase {
    private func encode<T: Encodable>(_ value: T) throws -> Data {
        try WireJSON.encoder().encode(value)
    }

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try WireJSON.decoder().decode(type, from: Data(json.utf8))
    }

    private func block() -> ActivityPayload {
        ActivityPayload(projectId: "0b9a7c55-2f37-4f40-a7e5-5a8f0f6b2d11",
                        clientActivityId: "f1e2d3c4-b5a6-4978-8695-a4b3c2d1e0f9",
                        startedAt: utc("2026-10-05T10:00:00Z"), endedAt: utc("2026-10-05T10:10:00.250Z"))
    }

    func testAFullBlockUsesTheContractNamesAndShapes() throws {
        var payload = block()
        payload.productivity = 73
        payload.mouseClicks = 12
        payload.keyboardHits = 310
        payload.description = "Invoice export"
        payload.activeWindows = [WindowInfo(appName: "Xcode", windowTitle: "App.swift", seconds: 240, keyboardHits: 310,
                                            mouseClicks: 12)]
        payload.runningPrograms = [WindowInfo(appName: "Slack", windowTitle: "")]
        payload.timeline = [ActivityMinute(minute: 0, keyboardHits: 3, mouseClicks: 1, active: true, appName: "Xcode",
                                           activeSeconds: 42)]
        payload.screenshotPngBase64 = "/9j/screen"
        payload.screenshotThumbJpgBase64 = "/9j/thumb"
        payload.webcamJpgBase64 = "/9j/cam"
        payload.platform = "macos"
        payload.osVersion = "macOS 14.5 (23F79)"
        payload.agentVersion = "0.2.0"
        payload.deviceId = "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11"
        payload.deviceName = "Ana's MacBook Pro"
        payload.timeZone = "Asia/Dhaka"
        payload.utcOffsetMinutes = 360
        payload.idleSeconds = 120
        payload.endReason = .signOut
        payload.sentAt = utc("2026-10-05T10:10:01.5Z")
        payload.screenshotCapturedAt = utc("2026-10-05T10:04:10.123Z")
        payload.screenshotBlurred = true
        payload.injectedInputEvents = 4

        assertJSONEqual(try encode(payload), """
            {"projectId":"0b9a7c55-2f37-4f40-a7e5-5a8f0f6b2d11","clientActivityId":"f1e2d3c4-b5a6-4978-8695-a4b3c2d1e0f9",
             "startedAt":"2026-10-05T10:00:00.000Z","endedAt":"2026-10-05T10:10:00.250Z",
             "productivity":73,"mouseClicks":12,"keyboardHits":310,"description":"Invoice export",
             "activeWindows":[{"appName":"Xcode","windowTitle":"App.swift","seconds":240,"keyboardHits":310,"mouseClicks":12}],
             "runningPrograms":[{"appName":"Slack","windowTitle":""}],
             "timeline":[{"minute":0,"keyboardHits":3,"mouseClicks":1,"active":true,"appName":"Xcode","activeSeconds":42}],
             "screenshotPngBase64":"/9j/screen","screenshotThumbJpgBase64":"/9j/thumb","webcamJpgBase64":"/9j/cam",
             "platform":"macos","osVersion":"macOS 14.5 (23F79)","agentVersion":"0.2.0",
             "deviceId":"5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11","deviceName":"Ana's MacBook Pro",
             "timeZone":"Asia/Dhaka","utcOffsetMinutes":360,"idleSeconds":120,"endReason":"signout",
             "sentAt":"2026-10-05T10:10:01.500Z","screenshotCapturedAt":"2026-10-05T10:04:10.123Z",
             "screenshotBlurred":true,"injectedInputEvents":4}
            """)
    }

    func testUnsetOptionalFieldsAreLeftOutForOlderServers() throws {
        // An older server binds `productivity` to a non-nullable int: a null
        // would fail the whole upload, an absent field takes its default.
        let json = jsonDictionary(try encode(block()))
        for absent in ["productivity", "description", "screenshotPngBase64", "screenshotThumbJpgBase64", "webcamJpgBase64",
                       "platform", "osVersion", "agentVersion", "deviceId", "deviceName", "timeZone", "utcOffsetMinutes",
                       "idleSeconds", "endReason", "sentAt", "screenshotCapturedAt", "screenshotBlurred",
                       "injectedInputEvents"] {
            XCTAssertNil(json[absent], "\(absent) should be left out")
        }
        // The fields every server requires are always there.
        XCTAssertEqual(Set(json.keys), ["clientActivityId", "projectId", "startedAt", "endedAt", "mouseClicks",
                                        "keyboardHits", "activeWindows", "runningPrograms", "timeline"])
        // Backslash-free: base64 slashes are not escaped.
        var payload = block()
        payload.screenshotPngBase64 = "a/b+c="
        XCTAssertTrue(String(decoding: try encode(payload), as: UTF8.self).contains("\"a/b+c=\""))
    }

    func testDatesAreUTCWithMillisecondsAndRoundTrip() throws {
        let date = utc("2026-07-14T16:05:00.123+06:00")
        XCTAssertEqual(WireDate.string(from: date), "2026-07-14T10:05:00.123Z")
        var payload = block()
        payload.startedAt = date
        let back = try WireJSON.decoder().decode(ActivityPayload.self, from: try encode(payload))
        XCTAssertEqual(back.startedAt, date)
        XCTAssertEqual(back, payload)
    }

    func testEndReasonsUseTheWireNamesAndUnknownOnesAreDropped() throws {
        let names: [EndReason: String] = [
            .interval: "interval", .pause: "pause", .onBreak: "break", .idle: "idle", .lock: "lock", .sleep: "sleep",
            .project: "project", .task: "task", .signOut: "signout", .quit: "quit", .shutdown: "shutdown",
        ]
        XCTAssertEqual(names.count, EndReason.allCases.count)
        for (reason, wire) in names {
            XCTAssertEqual(reason.rawValue, wire)
            XCTAssertEqual(EndReason.parse(wire), reason)
        }
        XCTAssertEqual(EndReason.parse(" Lock "), .lock)
        XCTAssertNil(EndReason.parse("hibernate"))

        // A row queued by a newer build with an unknown reason still loads.
        var json = jsonDictionary(try encode(block()))
        json["endReason"] = "teleported"
        let data = try JSONSerialization.data(withJSONObject: json)
        XCTAssertNil(try WireJSON.decoder().decode(ActivityPayload.self, from: data).endReason)
    }

    func testPayloadsQueuedBeforeProductivityExistedStillLoad() throws {
        let legacy = """
            {"clientActivityId":"id","projectId":"p","startedAt":"2026-01-01T00:00:00Z","endedAt":"2026-01-01T00:10:00Z",
             "description":null,"mouseClicks":1,"keyboardHits":2,"activeWindows":[],"runningPrograms":[],
             "screenshotPngBase64":null,"webcamJpgBase64":null}
            """
        let payload = try decode(ActivityPayload.self, legacy)
        // Never measured: not "0% active".
        XCTAssertNil(payload.productivity)
        XCTAssertTrue(payload.timeline.isEmpty)
        XCTAssertNil(payload.screenshotThumbJpgBase64)
        XCTAssertNil(payload.deviceId)
        XCTAssertNil(payload.endReason)
    }

    func testPayloadsQueuedByThePreviousBuildKeepTheirMeasurements() throws {
        let previous = """
            {"clientActivityId":"id","projectId":"p","startedAt":"2026-01-01T00:00:00Z","endedAt":"2026-01-01T00:10:00Z",
             "description":null,"productivity":64,"mouseClicks":1,"keyboardHits":2,"activeWindows":[],"runningPrograms":[],
             "timeline":[{"minute":0,"keyboardHits":2,"mouseClicks":1,"active":true}],
             "screenshotPngBase64":"iVBORw0KGgo=","screenshotThumbJpgBase64":null,"webcamJpgBase64":null}
            """
        let payload = try decode(ActivityPayload.self, previous)
        XCTAssertEqual(payload.productivity, 64)
        XCTAssertNil(payload.timeline.first?.activeSeconds)
        XCTAssertTrue(payload.hasCaptures)
        XCTAssertEqual(payload.captureBytes, Int64("iVBORw0KGgo=".utf8.count))
    }

    func testWindowUsageIsSentInCamelCaseAndBareWindowsStayBare() throws {
        let used = WindowInfo(appName: "Xcode", windowTitle: "App.swift", seconds: 240, keyboardHits: 310, mouseClicks: 12)
        assertJSONEqual(try encode(used),
                        #"{"appName":"Xcode","windowTitle":"App.swift","seconds":240,"keyboardHits":310,"mouseClicks":12}"#)
        assertJSONEqual(try encode(WindowInfo(appName: "Slack", windowTitle: "")), #"{"appName":"Slack","windowTitle":""}"#)
        // The server's stored legacy shape decodes (and missing names are blank).
        XCTAssertEqual(try decode(WindowInfo.self, #"{"appName":"Finder"}"#), WindowInfo(appName: "Finder", windowTitle: ""))
    }

    func testTimelineMinutesCarryActiveSecondsOnlyWhenKnown() throws {
        let minute = ActivityMinute(minute: 0, keyboardHits: 3, mouseClicks: 1, active: true, appName: nil, activeSeconds: 42)
        assertJSONEqual(try encode(minute), #"{"minute":0,"keyboardHits":3,"mouseClicks":1,"active":true,"activeSeconds":42}"#)
        var legacy = minute
        legacy.activeSeconds = nil
        assertJSONEqual(try encode(legacy), #"{"minute":0,"keyboardHits":3,"mouseClicks":1,"active":true}"#)
    }

    func testUnmeasuredCountsTravelAsNullAndSurviveTheQueue() throws {
        var payload = block()
        payload.keyboardHits = nil
        payload.mouseClicks = 4
        payload.timeline = [ActivityMinute(minute: 0, keyboardHits: nil, mouseClicks: 4, active: true)]
        let data = try encode(payload)
        let json = jsonDictionary(data)
        XCTAssertTrue(json["keyboardHits"] is NSNull, "never left out, never a made-up 0")
        XCTAssertEqual(json["mouseClicks"] as? Int, 4)
        let minute = try XCTUnwrap((json["timeline"] as? [[String: Any]])?.first)
        XCTAssertTrue(minute["keyboardHits"] is NSNull)

        // A queued row reads back the same.
        let back = try WireJSON.decoder().decode(ActivityPayload.self, from: data)
        XCTAssertNil(back.keyboardHits)
        XCTAssertEqual(back.mouseClicks, 4)
        XCTAssertNil(back.timeline.first?.keyboardHits)

        // Rows queued by older builds, which always measured, read as 0.
        let older = #"{"projectId":"p","clientActivityId":"c","startedAt":"2026-10-05T10:00:00Z","endedAt":"2026-10-05T10:10:00Z"}"#
        let legacy = try WireJSON.decoder().decode(ActivityPayload.self, from: Data(older.utf8))
        XCTAssertEqual(legacy.keyboardHits, 0)
        XCTAssertEqual(legacy.mouseClicks, 0)
    }

    func testProjectPolicyFieldsDefaultLikeTheServer() throws {
        let olderServer = """
            [{"id":"p","title":"Website","intervalMinutes":10,"allowScreenshot":true,"allowWebcam":false,
              "allowKeyboard":true,"allowMouse":true,"allowActiveWindow":true,"allowRunningPrograms":true}]
            """
        var project = try XCTUnwrap(MyProject.decodeList(from: Data(olderServer.utf8)).first)
        XCTAssertEqual(project.idleThresholdMinutes, 5, "not 0, which would turn idle detection off")
        XCTAssertFalse(project.blurScreenshots)
        XCTAssertEqual(project.idleThreshold, 300)
        XCTAssertEqual(project.interval, 600)

        let nulls = olderServer.replacingOccurrences(of: "}]", with: #","idleThresholdMinutes":null,"blurScreenshots":null}]"#)
        project = try XCTUnwrap(MyProject.decodeList(from: Data(nulls.utf8)).first)
        XCTAssertEqual(project.idleThresholdMinutes, 5)
        XCTAssertFalse(project.blurScreenshots)

        let newer = olderServer.replacingOccurrences(of: "}]", with: #","idleThresholdMinutes":0,"blurScreenshots":true}]"#)
        project = try XCTUnwrap(MyProject.decodeList(from: Data(newer.utf8)).first)
        XCTAssertEqual(project.idleThreshold, 0, "0 turns idle detection off")
        XCTAssertTrue(project.blurScreenshots)

        project.idleThresholdMinutes = 500
        XCTAssertEqual(project.idleThreshold, 3600)
        project.intervalMinutes = 1
        XCTAssertEqual(project.interval, 300)
    }

    func testAnUnreadableProjectIsSkippedNotFatal() throws {
        let list = """
            [{"title":"No id"},{"id":"p2","title":"Kept","intervalMinutes":15,"allowScreenshot":false,"allowWebcam":false,
              "allowKeyboard":true,"allowMouse":true,"allowActiveWindow":false,"allowRunningPrograms":false}]
            """
        let projects = try MyProject.decodeList(from: Data(list.utf8))
        XCTAssertEqual(projects.map { $0.id }, ["p2"])
        XCTAssertEqual(projects[0].allowedCaptures,
                       CapturePermissions(screenshot: false, webcam: false, keyboard: true, mouse: true,
                                          activeWindow: false, runningPrograms: false))
        XCTAssertThrowsError(try MyProject.decodeList(from: Data(#"{"items":[]}"#.utf8)))
    }

    func testHeartbeatsUseTheContractShapeAndFitTheServerLimits() throws {
        let heartbeat = Heartbeat(
            deviceId: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11",
            deviceName: "  " + String(repeating: "N", count: 200) + "  ",
            platform: "macos",
            osVersion: nil,
            agentVersion: "0.2.0",
            status: .signedOut,
            currentApp: "   ",
            timeZone: "Asia/Dhaka",
            utcOffsetMinutes: 9_999
        ).normalized()
        XCTAssertEqual(heartbeat.deviceName?.count, 128)
        XCTAssertNil(heartbeat.currentApp)
        XCTAssertEqual(heartbeat.utcOffsetMinutes, 840)

        let json = jsonDictionary(try encode(heartbeat))
        XCTAssertEqual(json["status"] as? String, "signedout")
        XCTAssertEqual(json["deviceId"] as? String, "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11")
        XCTAssertEqual(json["timeZone"] as? String, "Asia/Dhaka")
        XCTAssertNil(json["projectId"])
        XCTAssertNil(json["osVersion"])

        let wire: [PresenceStatus: String] = [
            .tracking: "tracking", .paused: "paused", .onBreak: "break", .idle: "idle", .locked: "locked",
            .choosing: "choosing", .signedOut: "signedout", .stopped: "stopped",
        ]
        XCTAssertEqual(wire.count, PresenceStatus.allCases.count)
        for (status, name) in wire {
            XCTAssertEqual(status.rawValue, name)
        }
    }

    func testAFullHeartbeatMatchesTheContract() throws {
        var heartbeat = Heartbeat(deviceId: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11", deviceName: "Mac", platform: "macos",
                                  osVersion: "macOS 14.5 (23F79)", agentVersion: "0.2.0", status: .tracking,
                                  projectId: "p1", statusSince: utc("2026-10-05T09:00:00Z"), currentApp: "Xcode",
                                  timeZone: "Asia/Dhaka", utcOffsetMinutes: 360)
        heartbeat.sentAt = utc("2026-10-05T10:00:00Z")
        assertJSONEqual(try encode(heartbeat), """
            {"deviceId":"5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11","deviceName":"Mac","platform":"macos",
             "osVersion":"macOS 14.5 (23F79)","agentVersion":"0.2.0","status":"tracking","projectId":"p1",
             "statusSince":"2026-10-05T09:00:00.000Z","currentApp":"Xcode","timeZone":"Asia/Dhaka",
             "utcOffsetMinutes":360,"sentAt":"2026-10-05T10:00:00.000Z"}
            """)
    }

    func testADeviceHeartbeatIsNormalized() {
        let heartbeat = Heartbeat(device: sampleDevice(), status: .idle, projectId: "p1", statusSince: nil,
                                  currentApp: "  Xcode  ", timeZone: TimeZone(identifier: "Asia/Dhaka")!,
                                  now: utc("2026-10-05T10:00:00Z"))
        XCTAssertEqual(heartbeat.deviceId, "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11")
        XCTAssertEqual(heartbeat.currentApp, "Xcode")
        XCTAssertEqual(heartbeat.platform, "macos")
        XCTAssertEqual(heartbeat.timeZone, "Asia/Dhaka")
        XCTAssertEqual(heartbeat.utcOffsetMinutes, 360)
    }

    func testTeamPresenceIsReadLeniently() throws {
        let list = """
            {"items":[
              {"userId":"u1","userName":"alice","name":"Alice","deviceId":"5B0C1B9E-8F2C-4A43-9A51-0C7F6F2D9B11",
               "deviceName":"DESKTOP-1","platform":"windows","osVersion":"Windows 11 Pro 23H2 (10.0.22631)",
               "agentVersion":"0.2.0","status":"tracking","projectId":"p1","projectTitle":"Website",
               "statusSince":"2026-10-05T09:00:00Z","lastSeenAt":"2026-10-05T10:00:30.5","online":true,
               "currentApp":"chrome.exe","timeZone":"Asia/Dhaka","utcOffsetMinutes":360},
              {"userId":"u1","userName":"alice","deviceId":"d2","status":"hibernating","online":false,"lastSeenAt":"later"},
              {"userName":"no ids"}
            ]}
            """
        let devices = try PresenceInfo.decodeList(from: Data(list.utf8))
        XCTAssertEqual(devices.count, 2, "an unreadable entry is skipped")
        let desktop = devices[0]
        XCTAssertEqual(desktop.presenceStatus, .tracking)
        XCTAssertTrue(desktop.online)
        XCTAssertEqual(desktop.statusSince, utc("2026-10-05T09:00:00Z"))
        XCTAssertEqual(desktop.lastSeenAt, utc("2026-10-05T10:00:30.5Z"), "an unzoned server time is UTC")
        XCTAssertTrue(desktop.isDevice("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11"))
        XCTAssertEqual(desktop.id, "u1/5B0C1B9E-8F2C-4A43-9A51-0C7F6F2D9B11")
        let other = devices[1]
        XCTAssertNil(other.presenceStatus, "a status from a newer server stays readable")
        XCTAssertEqual(other.status, "hibernating")
        XCTAssertNil(other.lastSeenAt)
        XCTAssertFalse(other.isDevice("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11"))
        // It round-trips through its own coding.
        XCTAssertEqual(try decode(PresenceInfo.self, String(decoding: try encode(desktop), as: UTF8.self)), desktop)
    }

    func testHeartbeatResultsAcceptUTCAndUnzonedServerTimes() throws {
        let zoned = try decode(HeartbeatResult.self, #"{"serverTime":"2026-10-05T10:00:00.123Z"}"#)
        let bare = try decode(HeartbeatResult.self, #"{"serverTime":"2026-10-05T10:00:00.123"}"#)
        XCTAssertNotNil(zoned.serverTime)
        XCTAssertEqual(zoned.serverTime, bare.serverTime)
        XCTAssertNil(try decode(HeartbeatResult.self, #"{"serverTime":"soon"}"#).serverTime)
        XCTAssertNil(try decode(HeartbeatResult.self, "{}").serverTime)
    }

    func testAbpErrorBodiesAreParsed() {
        let body = #"{"error":{"code":"Tracker:ActivityOverlaps","message":"Overlaps another block.","details":null,"data":{},"validationErrors":null}}"#
        let error = AbpError.parse(Data(body.utf8))
        XCTAssertEqual(error?.trimmedCode, "Tracker:ActivityOverlaps")
        XCTAssertEqual(error?.hasValidationErrors, false)
        XCTAssertEqual(error?.summary, "Overlaps another block.")

        let validation = """
            {"error":{"code":null,"message":"Your request is not valid!",
             "validationErrors":[{"message":"The field Productivity must be between 0 and 100.","members":["productivity"]}]}}
            """
        let invalid = AbpError.parse(Data(validation.utf8))
        XCTAssertNil(invalid?.trimmedCode)
        XCTAssertEqual(invalid?.hasValidationErrors, true)
        XCTAssertEqual(invalid?.summary, "Your request is not valid! The field Productivity must be between 0 and 100.")

        XCTAssertNil(AbpError.parse(Data("<html>Bad gateway</html>".utf8)))
        XCTAssertNil(AbpError.parse(Data(#"{"title":"Not Found","status":404}"#.utf8)))
        XCTAssertNil(AbpError.parse(Data(#"{"error":"invalid_grant"}"#.utf8)), "an OAuth body is not an ABP error")
        XCTAssertNil(AbpError.parse(Data()))
        XCTAssertNil(AbpError.parse(Data(#"{"error":{"code":"  "}}"#.utf8))?.trimmedCode)
        XCTAssertEqual(AbpError(code: "Tracker:X").displayMessage, "Tracker:X")
    }

    func testReportSummariesTolerateUnmeasuredProductivity() throws {
        let summary = try decode(ReportSummary.self, #"{"totalTrackedMinutes":90.5,"averageProductivity":null,"totalIdleMinutes":12}"#)
        XCTAssertEqual(summary.totalTrackedMinutes, 90.5)
        XCTAssertNil(summary.averageProductivity)
        XCTAssertEqual(summary.totalIdleMinutes, 12)
        XCTAssertEqual(summary.trackedSeconds, 5_430)
        XCTAssertEqual(summary.displayProductivity, 0)
        // An older server always measured.
        let older = try decode(ReportSummary.self, #"{"totalTrackedMinutes":5,"averageProductivity":61.2,"perUser":[]}"#)
        XCTAssertEqual(older.averageProductivity, 61.2)
        XCTAssertEqual(older.totalIdleMinutes, 0)
        XCTAssertEqual(older.displayProductivity, 61)
    }

    func testTokenResponsesAndErrorsAreRead() throws {
        let token = try JSONDecoder().decode(TokenResponse.self, from: Data(
            #"{"access_token":"abc","token_type":"Bearer","expires_in":3599,"refresh_token":"r1","scope":"Tracker offline_access"}"#.utf8))
        XCTAssertEqual(token, TokenResponse(accessToken: "abc", expiresIn: 3599, refreshToken: "r1"))

        let refused = OAuthErrorBody.parse(Data(#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#.utf8))
        XCTAssertTrue(AuthRejection.isRejection(status: 400, body: refused))
        XCTAssertEqual(AuthRejection(status: 400, body: refused).userMessage, "Invalid username or password!")
        XCTAssertEqual(OAuthErrorBody.parse(Data("not json".utf8)), OAuthErrorBody(error: nil, errorDescription: nil))
    }

    func testOnlyRefusedCredentialsAreARejectedSignIn() {
        let body = { (json: String) in OAuthErrorBody.parse(Data(json.utf8)) }
        XCTAssertTrue(AuthRejection.isRejection(status: 400, body: body(#"{"error":"invalid_grant"}"#)))
        XCTAssertTrue(AuthRejection.isRejection(status: 401, body: body("")))
        XCTAssertTrue(AuthRejection.isRejection(status: 403, body: body("")))
        // Not about the credentials: a proxy's bare 400, a wrong client, a
        // missing endpoint, throttling and server failures.
        XCTAssertFalse(AuthRejection.isRejection(status: 400, body: body("")))
        XCTAssertFalse(AuthRejection.isRejection(status: 400, body: body(#"{"error":"invalid_client"}"#)))
        XCTAssertFalse(AuthRejection.isRejection(status: 404, body: body("")))
        XCTAssertFalse(AuthRejection.isRejection(status: 429, body: body("")))
        XCTAssertFalse(AuthRejection.isRejection(status: 503, body: body("")))

        XCTAssertEqual(AuthRejection(status: 401, error: nil, errorDescription: nil).userMessage, "Incorrect email or password.")
        let locked = AuthRejection(status: 400, error: "invalid_grant",
                                   errorDescription: "The user account has been locked out due to invalid login attempts.")
        XCTAssertEqual(locked.kind, .lockedOut)
        XCTAssertEqual(locked.userMessage, "The user account has been locked out due to invalid login attempts.")
        let change = AuthRejection(status: 400, error: "invalid_grant", errorDescription: "ShouldChangePasswordOnNextLogin",
                                   changePasswordToken: "cpt")
        XCTAssertEqual(change.kind, .mustChangePassword)
        XCTAssertTrue(change.userMessage.contains("must be changed"))
        XCTAssertEqual(AuthRejection(status: 400, error: "invalid_grant",
                                     errorDescription: "You are not allowed to login! Your account is inactive.").kind, .notAllowed)

        XCTAssertTrue(TokenFailure(status: 429, error: nil, errorDescription: nil).userMessage.contains("Too many"))
        XCTAssertTrue(TokenFailure(status: 503, error: nil, errorDescription: nil).userMessage.contains("503"))
        XCTAssertEqual(TokenFailure(status: 400, error: "invalid_client", errorDescription: "Unknown client").userMessage,
                       "Unknown client")
        XCTAssertTrue(TokenFailure(status: 400, error: "invalid_scope", errorDescription: nil).isInvalidScope)
    }

    /// What ABP 10.5's token endpoint really sends for a deactivated, a
    /// locked-out and an unconfirmed account.
    func testAbpAccountRefusalsAreClassifiedByTheirErrorCode() {
        let body = { (json: String) in OAuthErrorBody.parse(Data(json.utf8)) }

        // Deactivated (e.g. removed from the workspace): Sign-in expired.
        let inactive = body(#"{"error":"account_inactive","error_description":"You are not allowed to login! Your account is inactive or needs to confirm your email/phone number."}"#)
        XCTAssertTrue(AuthRejection.isRejection(status: 400, body: inactive))
        let refusal = AuthRejection(status: 400, body: inactive)
        XCTAssertEqual(refusal.kind, .notAllowed)
        XCTAssertTrue(refusal.userMessage.contains("inactive"))
        XCTAssertEqual(UploadPolicy.decide(.failed(.authRejected(refusal))), .signInExpired(refusal))

        // Locked out for a while: the saved password is still right, retried.
        let locked = body(#"{"error":"account_locked","error_description":"The user account has been locked out due to invalid login attempts. Please wait a while and try again."}"#)
        XCTAssertFalse(AuthRejection.isRejection(status: 400, body: locked))
        let failure = TokenFailure(status: 400, error: locked.error, errorDescription: locked.errorDescription)
        XCTAssertTrue(failure.isAccountLocked)
        XCTAssertTrue(failure.userMessage.contains("locked out"))
        XCTAssertEqual(UploadPolicy.decide(.failed(.tokenFailed(failure))), .retry(RetryPlan(reason: .accountLocked)))
        XCTAssertTrue(TokenFailure(status: 400, error: "account_locked", errorDescription: nil).userMessage
            .contains("temporarily locked"))

        // Not confirmed: a readable message, not ABP's machine reason.
        let unconfirmed = AuthRejection(status: 400, error: "invalid_grant", errorDescription: "RequiresConfirmUser")
        XCTAssertEqual(unconfirmed.kind, .notAllowed)
        XCTAssertTrue(unconfirmed.userMessage.contains("not confirmed"))
    }
}
