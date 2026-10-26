import XCTest
@testable import DosiTrackerCore

/// Which failures point at the server address (the sign-in form then opens
/// the address field) and which say nothing about it.
final class ApiErrorTests: XCTestCase {
    func testFailuresThatMeanTheAddressIsWrongPointAtIt() {
        let pointing: [ApiError] = [
            .invalidConfiguration("Invalid server address: x"),
            .transport(TransportFailure(code: URLError.Code.cannotFindHost.rawValue, description: "no host")),
            .transport(TransportFailure(code: URLError.Code.serverCertificateUntrusted.rawValue, description: "cert")),
            .transport(.timedOut),
            .invalidResponse("an HTML page"),
            .http(HTTPFailure(status: 301)),
            .http(HTTPFailure(status: 404)),
            .tokenFailed(TokenFailure(status: 302, error: nil, errorDescription: nil)),
            .tokenFailed(TokenFailure(status: 404, error: nil, errorDescription: nil)),
        ]
        for error in pointing {
            XCTAssertTrue(error.concernsServerAddress, "\(error)")
        }
    }

    func testAnswersFromTheApiSayNothingAboutTheAddress() {
        let answered: [ApiError] = [
            .authRejected(AuthRejection.sessionEnded),
            .tokenFailed(TokenFailure(status: 429, error: nil, errorDescription: nil)),
            .tokenFailed(TokenFailure(status: 500, error: nil, errorDescription: nil)),
            .http(HTTPFailure(status: 403)),
            .http(HTTPFailure(status: 500)),
            .encoding("a local bug"),
        ]
        for error in answered {
            XCTAssertFalse(error.concernsServerAddress, "\(error)")
        }
    }
}
