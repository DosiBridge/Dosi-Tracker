import Foundation

/// A request that never got an HTTP response: offline, DNS, refused
/// connection, TLS failure, timeout, cancellation.
public struct TransportFailure: Equatable, Sendable {
    /// `URLError.Code` raw value (or -1 for a non-URL error).
    public let code: Int
    public let description: String

    public init(code: Int, description: String) {
        self.code = code
        self.description = description
    }

    public init(_ error: URLError) {
        self.init(code: error.code.rawValue, description: error.localizedDescription)
    }

    public static let timedOut = TransportFailure(code: URLError.Code.timedOut.rawValue, description: "The request timed out.")
    public static let cancelled = TransportFailure(code: URLError.Code.cancelled.rawValue, description: "The request was cancelled.")

    public var isTimeout: Bool { code == URLError.Code.timedOut.rawValue }
    public var isCancelled: Bool { code == URLError.Code.cancelled.rawValue }

    /// macOS App Transport Security refused a cleartext (`http://`)
    /// connection: the address must use https://.
    public var isInsecureConnectionRefused: Bool {
        code == URLError.Code.appTransportSecurityRequiresSecureConnection.rawValue
    }

    /// The server's certificate (or this Mac's trust settings) is the problem,
    /// not the network.
    public var isCertificateProblem: Bool {
        let codes: [URLError.Code] = [
            .serverCertificateUntrusted, .serverCertificateHasBadDate, .serverCertificateNotYetValid,
            .serverCertificateHasUnknownRoot, .secureConnectionFailed, .clientCertificateRejected,
            .clientCertificateRequired,
        ]
        return codes.contains { $0.rawValue == code }
    }
}

/// An API response that was not a success.
public struct HTTPFailure: Equatable, Sendable {
    public let status: Int
    /// The ABP error body, when the server sent one.
    public let error: AbpError?
    /// `Retry-After`, in seconds from now (bounded by
    /// ``ApiClient/maxRetryAfter``).
    public let retryAfter: TimeInterval?

    public init(status: Int, error: AbpError? = nil, retryAfter: TimeInterval? = nil) {
        self.status = status
        self.error = error
        self.retryAfter = retryAfter
    }
}

/// Why an ``ApiClient`` call failed.
public enum ApiError: Error, Equatable, Sendable {
    /// The server address cannot form a request URL.
    case invalidConfiguration(String)
    /// No HTTP response (network, TLS, timeout, cancellation).
    case transport(TransportFailure)
    /// The token endpoint refused the credentials (`invalid_grant`, 401,
    /// 403): sign-in expired. Only this signs the user out; everything else
    /// keeps the saved sign-in.
    case authRejected(AuthRejection)
    /// Signing in failed for another reason (throttling, a 5xx, an unexpected
    /// answer). Says nothing about the credentials.
    case tokenFailed(TokenFailure)
    /// A non-success API response (after one silent re-sign-in on 401).
    case http(HTTPFailure)
    /// A success response whose body could not be read.
    case invalidResponse(String)
    /// The request body could not be encoded (a local bug, never transient).
    case encoding(String)

    /// The saved credentials were refused.
    public var isAuthRejection: Bool {
        if case .authRejected = self { return true }
        return false
    }

    /// No response at all: the server could not be reached.
    public var isTransport: Bool {
        if case .transport = self { return true }
        return false
    }

    /// The HTTP status, when there was a response.
    public var status: Int? {
        switch self {
        case .http(let failure):
            return failure.status
        case .authRejected(let rejection):
            return rejection.status
        case .tokenFailed(let failure):
            return failure.status
        default:
            return nil
        }
    }

    /// What to tell the user (sign-in form, status line).
    public var userMessage: String {
        switch self {
        case .invalidConfiguration(let message):
            return message
        case .transport(let failure):
            if failure.isInsecureConnectionRefused {
                return "This server must be reached over https://. Change the server address to start with https://."
            }
            if failure.isCertificateProblem {
                return "The server's security certificate is not trusted. Check the server address."
            }
            if failure.isTimeout {
                return "The server took too long to answer. Check your connection."
            }
            return "Cannot reach the server. Check the server address and your connection."
        case .authRejected(let rejection):
            return rejection.userMessage
        case .tokenFailed(let failure):
            return failure.userMessage
        case .http(let failure):
            switch failure.status {
            case 429:
                return "The server is busy. Try again in a minute."
            case 500...599:
                return "The server had a problem (HTTP \(failure.status)). Try again shortly."
            default:
                return failure.error?.displayMessage ?? "Unexpected server response (HTTP \(failure.status))."
            }
        case .invalidResponse:
            return "Unexpected response from the server."
        case .encoding(let message):
            return message
        }
    }
}

/// The outcome of uploading one block, classified by ``UploadPolicy``.
public enum UploadResponse: Equatable, Sendable {
    /// 2xx: the server stored the block (or already had it).
    case accepted(status: Int)
    case failed(ApiError)
}

/// The device clock compared with the server's `Date` header (CONTRACT §1
/// "Clock check"). Timestamps are never rewritten; a large offset is shown.
public struct ClockReading: Equatable, Sendable {
    /// Beyond this many seconds the user is warned.
    public static let warningThreshold = 120

    /// Server time minus device time, in whole seconds (truncated, as on
    /// Windows): positive when this computer's clock is behind.
    public let offsetSeconds: Int
    /// When the reading was taken (device clock).
    public let measuredAt: Date

    public init(offsetSeconds: Int, measuredAt: Date) {
        self.offsetSeconds = offsetSeconds
        self.measuredAt = measuredAt
    }

    /// The reading from a response's `Date` header received at `deviceNow`.
    public init?(dateHeader: String, deviceNow: Date) {
        guard let server = WireDate.httpDate(from: dateHeader) else { return nil }
        let offset = server.timeIntervalSince(deviceNow)
        guard offset.isFinite, abs(offset) < 1e12 else { return nil }
        self.init(offsetSeconds: Int(offset), measuredAt: deviceNow)
    }

    /// Minutes the device clock is off by, once that is worth a warning
    /// (|offset| over 2 min): positive = behind the server, negative = ahead.
    public var skewMinutes: Int? {
        ClockReading.skewWarningMinutes(offsetSeconds: offsetSeconds)
    }

    public var deviceIsAhead: Bool { offsetSeconds < 0 }

    /// "Your computer clock is off by N min", when it is.
    public var warning: String? {
        guard let minutes = skewMinutes else { return nil }
        return "Your computer clock is off by \(abs(minutes)) min"
    }

    /// The warning minutes for an offset, if it exceeds
    /// ``warningThreshold`` (rounded half away from zero).
    public static func skewWarningMinutes(offsetSeconds: Int) -> Int? {
        guard abs(offsetSeconds) > warningThreshold else { return nil }
        return Int((Double(offsetSeconds) / 60).rounded())
    }
}
