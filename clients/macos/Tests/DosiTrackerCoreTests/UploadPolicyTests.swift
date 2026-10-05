import XCTest
@testable import DosiTrackerCore

/// The CONTRACT §1 "How agents treat upload responses" table, case by case as
/// the Windows agent's `upload_failures_are_classified_as_the_contract_says`.
final class UploadPolicyTests: XCTestCase {
    private func abp(_ code: String?, _ message: String) -> AbpError {
        AbpError(code: code, message: message)
    }

    private func kind(_ status: Int, _ error: AbpError? = nil, retryAfter: TimeInterval? = nil) -> String {
        switch UploadPolicy.decide(status: status, error: error, retryAfter: retryAfter) {
        case .uploaded:
            return "uploaded"
        case .retry(let plan):
            switch plan.reason {
            case .subscriptionInactive:
                return "subscription"
            case .clockAhead:
                return "clock"
            default:
                return "transient"
            }
        case .signInExpired:
            return "auth"
        case .stripCapturesAndResend:
            return "strip"
        case .verifyProject:
            return "project"
        case .park:
            return "park"
        }
    }

    func testSuccessRemovesTheRow() {
        XCTAssertEqual(kind(200), "uploaded")
        XCTAssertEqual(kind(201), "uploaded")
        XCTAssertEqual(UploadPolicy.decide(.accepted(status: 200)), .uploaded)
    }

    func testInfrastructureThrottlingAndCodelessErrorsAreTransient() {
        for status in [404, 405, 408, 429, 500, 502, 503] {
            XCTAssertEqual(kind(status), "transient", "\(status)")
        }
        XCTAssertEqual(kind(400), "transient", "a proxy's bare 400")
        XCTAssertEqual(kind(403, AbpError(message: "Forbidden")), "transient", "403 without a code")
        XCTAssertEqual(kind(422, abp("Tracker:SomethingNew", "?")), "transient", "an unknown business error")
        // A 5xx is transient even with a business code.
        XCTAssertEqual(kind(500, abp(UploadPolicy.Code.activityOverlaps, "x")), "transient")
        // A framework error code (authorization) is not a verdict on the block.
        XCTAssertEqual(kind(403, abp("Volo.Authorization:010001", "Authorization failed!")), "transient")
    }

    func testSpecialCasesGetTheirOwnHandling() {
        XCTAssertEqual(kind(402, abp(UploadPolicy.Code.subscriptionInactive, "Inactive")), "subscription")
        XCTAssertEqual(kind(402), "transient", "402 without a code")
        XCTAssertEqual(kind(403, abp(UploadPolicy.Code.notProjectMember, "Not a member")), "project")
        XCTAssertEqual(kind(409, abp(UploadPolicy.Code.projectArchived, "Archived")), "project")
        XCTAssertEqual(kind(425, abp(UploadPolicy.Code.activityInFuture, "In the future")), "clock")
        XCTAssertEqual(kind(413), "strip")
        XCTAssertEqual(kind(413, abp(UploadPolicy.Code.captureTooLarge, "Too big")), "strip")
        XCTAssertEqual(kind(400, abp(UploadPolicy.Code.invalidCaptureData, "Bad image")), "strip")
    }

    func testDefinitiveRejectionsOfTheBlockAreParked() {
        XCTAssertEqual(kind(409, abp(UploadPolicy.Code.activityOverlaps, "Overlaps")), "park")
        XCTAssertEqual(kind(409, abp(UploadPolicy.Code.clientIdConflict, "Conflict")), "park")
        XCTAssertEqual(kind(422, abp(UploadPolicy.Code.activityTooLong, "Too long")), "park")
        XCTAssertEqual(kind(422, abp(UploadPolicy.Code.activityTooOld, "Too old")), "park")
        XCTAssertEqual(kind(422, abp(UploadPolicy.Code.timeRangeInvalid, "Bad range")), "park")
        let validation = AbpError(message: "Your request is not valid!", validationErrors: [
            AbpError.ValidationError(message: "Description is too long.", members: ["description"]),
        ])
        XCTAssertEqual(kind(400, validation), "park")
        // An older server answers 403 for every business error: the code decides.
        XCTAssertEqual(kind(403, abp(UploadPolicy.Code.activityTooLong, "Too long")), "park")
        // Validation errors on another status are not a verdict.
        XCTAssertEqual(kind(422, validation), "transient")
    }

    func testTheServerMessageIsKeptAndRetryAfterIsHonoured() {
        let validation = AbpError(message: "Your request is not valid!", validationErrors: [
            AbpError.ValidationError(message: "Description is too long.", members: ["description"]),
        ])
        XCTAssertEqual(UploadPolicy.decide(status: 400, error: validation, retryAfter: nil),
                       .park(reason: "Your request is not valid! Description is too long."))
        XCTAssertEqual(UploadPolicy.decide(status: 429, error: nil, retryAfter: 30),
                       .retry(RetryPlan(reason: .throttled, after: 30)))
        XCTAssertEqual(UploadPolicy.decide(status: 503, error: nil, retryAfter: 120),
                       .retry(RetryPlan(reason: .serverError(status: 503), after: 120)))
        XCTAssertEqual(UploadPolicy.decide(status: 403, error: abp(UploadPolicy.Code.notProjectMember, "Not a member"),
                                           retryAfter: nil),
                       .verifyProject(code: UploadPolicy.Code.notProjectMember, message: "Not a member"))
        XCTAssertEqual(UploadPolicy.decide(status: 413, error: nil, retryAfter: nil),
                       .stripCapturesAndResend(reason: "HTTP 413"))
        XCTAssertEqual(UploadPolicy.decide(status: 409, error: AbpError(code: "  "), retryAfter: nil),
                       .retry(RetryPlan(reason: .unexpectedResponse(status: 409))), "a blank code is no code")
    }

    func testFixedWaitsForSubscriptionAndClockProblems() {
        XCTAssertEqual(UploadPolicy.decide(status: 402, error: abp(UploadPolicy.Code.subscriptionInactive, "x"),
                                           retryAfter: 5),
                       .retry(RetryPlan(reason: .subscriptionInactive, after: 3600)))
        XCTAssertEqual(UploadPolicy.decide(status: 425, error: abp(UploadPolicy.Code.activityInFuture, "x"),
                                           retryAfter: nil),
                       .retry(RetryPlan(reason: .clockAhead, after: 300)))
        XCTAssertEqual(RetryReason.subscriptionInactive.userMessage, "Workspace subscription inactive")
        XCTAssertEqual(RetryReason.clockAhead.userMessage, "Your computer clock is ahead")
    }

    func testA401AfterAFreshSignInIsTransient() {
        XCTAssertEqual(UploadPolicy.decide(status: 401, error: nil, retryAfter: nil),
                       .retry(RetryPlan(reason: .unauthorized)))
    }

    func testClientErrorsMapToTheirHandling() {
        let rejection = AuthRejection(status: 400, error: "invalid_grant", errorDescription: "Invalid username or password!")
        XCTAssertEqual(UploadPolicy.decide(.failed(.authRejected(rejection))), .signInExpired(rejection))
        XCTAssertEqual(UploadPolicy.decide(.failed(.transport(.timedOut))), .retry(RetryPlan(reason: .offline)))
        XCTAssertEqual(UploadPolicy.decide(.failed(.tokenFailed(TokenFailure(status: 503, error: nil, errorDescription: nil)))),
                       .retry(RetryPlan(reason: .signInFailed(status: 503))))
        XCTAssertEqual(UploadPolicy.decide(.failed(.tokenFailed(TokenFailure(status: 429, error: nil, errorDescription: nil,
                                                                             retryAfter: 60)))),
                       .retry(RetryPlan(reason: .throttled, after: 60)))
        XCTAssertEqual(UploadPolicy.decide(.failed(.invalidResponse("?"))),
                       .retry(RetryPlan(reason: .unexpectedResponse(status: 0))))
        XCTAssertEqual(UploadPolicy.decide(.failed(.encoding("bad"))), .park(reason: "bad"))
        XCTAssertEqual(UploadPolicy.decide(.failed(.http(HTTPFailure(status: 409, error: abp(UploadPolicy.Code.activityOverlaps,
                                                                                            "Overlaps"))))),
                       .park(reason: "Overlaps"))
    }

    func testBackoffDoublesFromThirtySecondsToThirtyMinutes() {
        XCTAssertEqual(UploadBackoff.delay(afterConsecutiveFailures: 0), 0)
        XCTAssertEqual((1...8).map { UploadBackoff.delay(afterConsecutiveFailures: $0) },
                       [30, 60, 120, 240, 480, 960, 1_800, 1_800])
        XCTAssertEqual(UploadBackoff.delay(afterConsecutiveFailures: 1_000), 1_800)
        XCTAssertEqual(RetryPlan(reason: .offline).delay(consecutiveFailures: 3), 120)
        XCTAssertEqual(RetryPlan(reason: .throttled, after: 7).delay(consecutiveFailures: 3), 7)
    }

    func testTheRetryScheduleBacksOffAndResets() {
        var schedule = RetrySchedule()
        XCTAssertTrue(schedule.isDue(at: at(t0)))
        let first = schedule.failed(RetryPlan(reason: .offline), at: at(t0))
        XCTAssertEqual(first, at(t0 + 30))
        XCTAssertFalse(schedule.isDue(at: at(t0 + 29)))
        XCTAssertTrue(schedule.isDue(at: at(t0 + 30)))
        XCTAssertEqual(schedule.failed(RetryPlan(reason: .offline), at: at(t0 + 30)), at(t0 + 90))
        XCTAssertEqual(schedule.consecutiveFailures, 2)
        XCTAssertEqual(schedule.lastReason, .offline)
        // A server-requested wait wins over the backoff.
        XCTAssertEqual(schedule.failed(RetryPlan(reason: .subscriptionInactive, after: 3_600), at: at(t0 + 90)),
                       at(t0 + 3_690))
        schedule.reset()
        XCTAssertEqual(schedule.consecutiveFailures, 0)
        XCTAssertNil(schedule.nextAttemptAt)
        XCTAssertTrue(schedule.isDue(at: at(t0)))
    }
}
