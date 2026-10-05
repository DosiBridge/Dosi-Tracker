import Foundation

/// The error object of an ABP error response
/// (`{"error":{"code":"Tracker:…","message":"…","details":…,"validationErrors":[…]}}`).
public struct AbpError: Codable, Equatable, Sendable {
    public struct ValidationError: Codable, Equatable, Sendable {
        public var message: String?
        public var members: [String]?

        public init(message: String?, members: [String]?) {
            self.message = message
            self.members = members
        }
    }

    /// Business error code such as `Tracker:ActivityOverlaps`; nil for
    /// validation failures and framework errors without one.
    public var code: String?
    public var message: String?
    public var details: String?
    public var validationErrors: [ValidationError]?

    public init(code: String? = nil, message: String? = nil, details: String? = nil, validationErrors: [ValidationError]? = nil) {
        self.code = code
        self.message = message
        self.details = details
        self.validationErrors = validationErrors
    }

    private enum CodingKeys: String, CodingKey {
        case code, message, details, validationErrors
    }

    /// Lenient: a field of an unexpected type reads as nil.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        code = try? c.decodeIfPresent(String.self, forKey: .code)
        message = try? c.decodeIfPresent(String.self, forKey: .message)
        details = try? c.decodeIfPresent(String.self, forKey: .details)
        validationErrors = try? c.decodeIfPresent([ValidationError].self, forKey: .validationErrors)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encodeIfPresent(code, forKey: .code)
        try c.encodeIfPresent(message, forKey: .message)
        try c.encodeIfPresent(details, forKey: .details)
        try c.encodeIfPresent(validationErrors, forKey: .validationErrors)
    }

    /// The error of an ABP error body; nil when `data` is not one (an HTML
    /// page from a proxy, an empty body, an OAuth error, …).
    public static func parse(_ data: Data) -> AbpError? {
        guard !data.isEmpty, let envelope = try? JSONDecoder().decode(Envelope.self, from: data) else {
            return nil
        }
        return envelope.error
    }

    /// The business error code, if the server sent a non-blank one.
    public var trimmedCode: String? { code?.nonEmptyTrimmed }

    /// Whether the server named a business error code.
    public var hasCode: Bool { trimmedCode != nil }

    /// Whether this is a DataAnnotations validation failure.
    public var hasValidationErrors: Bool { !(validationErrors ?? []).isEmpty }

    /// One line for people: the message plus each validation error's message
    /// (what a parked row keeps), nil when the server sent no text.
    public var summary: String? {
        var parts: [String] = []
        if let message = message?.nonEmptyTrimmed {
            parts.append(message)
        }
        for error in validationErrors ?? [] {
            if let text = error.message?.nonEmptyTrimmed {
                parts.append(text)
            }
        }
        return parts.isEmpty ? nil : parts.joined(separator: " ")
    }

    /// ``summary``, else the code.
    public var displayMessage: String? { summary ?? trimmedCode }

    private struct Envelope: Decodable {
        let error: AbpError?
    }
}

/// Subset of `GET /api/app/reporting/summary` (`ReportSummaryDto`) the agent
/// shows: today's and this week's totals. Fields an older server does not
/// send default to 0 / nil.
public struct ReportSummary: Codable, Equatable, Sendable {
    public var totalActivities: Int
    public var totalTrackedMinutes: Double
    /// Idle minutes inside the tracked time (0 from older servers).
    public var totalIdleMinutes: Double
    /// Duration-weighted activity %; nil when nothing was measured.
    public var averageProductivity: Double?
    public var totalMouseClicks: Int64
    public var totalKeyboardHits: Int64

    public init(totalActivities: Int = 0, totalTrackedMinutes: Double = 0, totalIdleMinutes: Double = 0,
                averageProductivity: Double? = nil, totalMouseClicks: Int64 = 0, totalKeyboardHits: Int64 = 0) {
        self.totalActivities = totalActivities
        self.totalTrackedMinutes = totalTrackedMinutes
        self.totalIdleMinutes = totalIdleMinutes
        self.averageProductivity = averageProductivity
        self.totalMouseClicks = totalMouseClicks
        self.totalKeyboardHits = totalKeyboardHits
    }

    private enum CodingKeys: String, CodingKey {
        case totalActivities, totalTrackedMinutes, totalIdleMinutes, averageProductivity
        case totalMouseClicks, totalKeyboardHits
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        totalActivities = try c.decodeIfPresent(Int.self, forKey: .totalActivities) ?? 0
        totalTrackedMinutes = try c.decodeIfPresent(Double.self, forKey: .totalTrackedMinutes) ?? 0
        totalIdleMinutes = try c.decodeIfPresent(Double.self, forKey: .totalIdleMinutes) ?? 0
        averageProductivity = try c.decodeIfPresent(Double.self, forKey: .averageProductivity)
        totalMouseClicks = try c.decodeIfPresent(Int64.self, forKey: .totalMouseClicks) ?? 0
        totalKeyboardHits = try c.decodeIfPresent(Int64.self, forKey: .totalKeyboardHits) ?? 0
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(totalActivities, forKey: .totalActivities)
        try c.encode(totalTrackedMinutes, forKey: .totalTrackedMinutes)
        try c.encode(totalIdleMinutes, forKey: .totalIdleMinutes)
        try c.encodeIfPresent(averageProductivity, forKey: .averageProductivity)
        try c.encode(totalMouseClicks, forKey: .totalMouseClicks)
        try c.encode(totalKeyboardHits, forKey: .totalKeyboardHits)
    }

    /// Tracked seconds, never negative.
    public var trackedSeconds: Double { max(totalTrackedMinutes, 0) * 60 }

    /// The average activity % for display, 0...100 (0 when not measured).
    public var displayProductivity: Int {
        Int(min(max(averageProductivity ?? 0, 0), 100))
    }
}

/// A successful `POST /connect/token` response.
public struct TokenResponse: Decodable, Equatable, Sendable {
    public let accessToken: String
    /// Seconds the access token is valid (3600 when not sent).
    public let expiresIn: Int?
    /// Present when `offline_access` was granted.
    public let refreshToken: String?

    public init(accessToken: String, expiresIn: Int?, refreshToken: String?) {
        self.accessToken = accessToken
        self.expiresIn = expiresIn
        self.refreshToken = refreshToken
    }

    private enum CodingKeys: String, CodingKey {
        case accessToken = "access_token"
        case expiresIn = "expires_in"
        case refreshToken = "refresh_token"
    }
}

/// OpenIddict's error body: `{"error":"invalid_grant","error_description":"…"}`
/// (ABP adds `changePasswordToken` when the password must be changed).
public struct OAuthErrorBody: Decodable, Equatable, Sendable {
    public let error: String?
    public let errorDescription: String?
    public let changePasswordToken: String?

    public init(error: String?, errorDescription: String?, changePasswordToken: String? = nil) {
        self.error = error
        self.errorDescription = errorDescription
        self.changePasswordToken = changePasswordToken
    }

    private enum CodingKeys: String, CodingKey {
        case error
        case errorDescription = "error_description"
        case changePasswordToken
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        error = try? c.decodeIfPresent(String.self, forKey: .error)
        errorDescription = try? c.decodeIfPresent(String.self, forKey: .errorDescription)
        changePasswordToken = try? c.decodeIfPresent(String.self, forKey: .changePasswordToken)
    }

    /// The body of a token-endpoint error; an unreadable body is all nils.
    public static func parse(_ data: Data) -> OAuthErrorBody {
        (try? JSONDecoder().decode(OAuthErrorBody.self, from: data))
            ?? OAuthErrorBody(error: nil, errorDescription: nil)
    }

    /// The trimmed, non-blank description.
    public var trimmedDescription: String? { errorDescription?.nonEmptyTrimmed }
}

/// The token endpoint refused the credentials: OpenIddict `invalid_grant`
/// or ABP's `account_inactive` (HTTP 400), or HTTP 401 / 403 (CONTRACT §1). Wrong password, a locked or
/// inactive account, a required password change, …
///
/// For the agent this is the only failure that means "Sign-in expired"; a
/// network error, throttling or a 5xx keeps the saved sign-in.
public struct AuthRejection: Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        case invalidCredentials
        /// Too many failed sign-ins.
        case lockedOut
        /// The account is inactive or not confirmed.
        case notAllowed
        /// The administrator reset the password (or it expired): sign in
        /// again with a new one via ``ApiClient/PasswordChange``.
        case mustChangePassword
        case twoFactorRequired
    }

    public let status: Int
    /// OAuth error code (`invalid_grant`, …).
    public let error: String?
    /// OpenIddict / ABP description (`Invalid username or password!`, …).
    public let errorDescription: String?
    /// Present when ``kind`` is ``Kind/mustChangePassword``.
    public let changePasswordToken: String?

    public init(status: Int, error: String?, errorDescription: String?, changePasswordToken: String? = nil) {
        self.status = status
        self.error = error
        self.errorDescription = errorDescription
        self.changePasswordToken = changePasswordToken
    }

    public init(status: Int, body: OAuthErrorBody) {
        self.init(status: status, error: body.error, errorDescription: body.errorDescription,
                  changePasswordToken: body.changePasswordToken)
    }

    /// ABP 10.5's OAuth error for a deactivated (or not allowed) account,
    /// sent instead of `invalid_grant`.
    public static let accountInactiveError = "account_inactive"
    /// ABP 10.5's OAuth error for a locked-out account. Not a rejection: the
    /// saved password is still right, and the lockout runs out by itself.
    public static let accountLockedError = "account_locked"

    /// Whether a token-endpoint failure refuses the credentials:
    /// `invalid_grant`, ABP's `account_inactive` (the account was deactivated,
    /// e.g. removed from the workspace), 401 or 403. A proxy's bare 400,
    /// `invalid_client`, `account_locked` (temporary), a 404, throttling and
    /// server failures say nothing about them.
    public static func isRejection(status: Int, body: OAuthErrorBody) -> Bool {
        body.error == "invalid_grant" || body.error == accountInactiveError || status == 401 || status == 403
    }

    public var kind: Kind {
        switch error {
        case .some(AuthRejection.accountInactiveError):
            return .notAllowed
        case .some(AuthRejection.accountLockedError):
            return .lockedOut
        default:
            break
        }
        let description = errorDescription?.nonEmptyTrimmed ?? ""
        switch description {
        case "ShouldChangePasswordOnNextLogin", "PeriodicallyChangePassword":
            return .mustChangePassword
        case "RequiresTwoFactor":
            return .twoFactorRequired
        case "RequiresConfirmUser":
            return .notAllowed
        default:
            break
        }
        let lowered = description.lowercased()
        if lowered.contains("locked out") {
            return .lockedOut
        }
        if lowered.contains("not allowed") || lowered.contains("inactive") {
            return .notAllowed
        }
        return .invalidCredentials
    }

    /// What the sign-in form shows: the server's own description ("Invalid
    /// username or password!", "The user account has been locked out …"), so
    /// a locked-out user is not told to retry; friendly text for ABP's
    /// machine-readable reasons; "Incorrect email or password." when the
    /// server said nothing.
    public var userMessage: String {
        switch kind {
        case .mustChangePassword:
            return "Your password must be changed before you can sign in. Choose a new password."
        case .twoFactorRequired:
            return "This account uses two-factor sign-in, which the desktop app does not support yet."
        case .notAllowed:
            if error == AuthRejection.accountInactiveError {
                return "This account is inactive (deactivated, or its email is not confirmed). Ask your workspace administrator."
            }
            if errorDescription?.nonEmptyTrimmed == "RequiresConfirmUser" {
                return "This account is not confirmed yet. Confirm your email address, then sign in again."
            }
            return errorDescription?.nonEmptyTrimmed ?? "This account is not allowed to sign in."
        case .lockedOut:
            return AuthRejection.lockedOutMessage(errorDescription)
        case .invalidCredentials:
            return errorDescription?.nonEmptyTrimmed ?? "Incorrect email or password."
        }
    }

    /// What a locked-out account is told (the server's own words when it sent
    /// readable ones).
    static func lockedOutMessage(_ description: String?) -> String {
        if let text = description?.nonEmptyTrimmed, text.contains(" ") {
            return text
        }
        return "This account is temporarily locked after too many failed sign-ins. Try again later."
    }
}

/// A token-endpoint failure that is not a refusal of the credentials
/// (throttling, a server failure, an unexpected status or body). The saved
/// sign-in stays valid.
public struct TokenFailure: Equatable, Sendable {
    public let status: Int
    public let error: String?
    public let errorDescription: String?
    /// `Retry-After`, in seconds from now.
    public let retryAfter: TimeInterval?

    public init(status: Int, error: String?, errorDescription: String?, retryAfter: TimeInterval? = nil) {
        self.status = status
        self.error = error
        self.errorDescription = errorDescription
        self.retryAfter = retryAfter
    }

    /// The server does not grant a requested scope (`offline_access` on a
    /// deployment that does not allow refresh tokens).
    public var isInvalidScope: Bool { error == "invalid_scope" }

    /// ABP's `account_locked`: too many failed sign-ins. Temporary — the saved
    /// password is still right, so it is retried, never "Sign-in expired".
    public var isAccountLocked: Bool { error == AuthRejection.accountLockedError }

    /// What the sign-in form shows (worded as on Windows).
    public var userMessage: String {
        if isAccountLocked {
            return AuthRejection.lockedOutMessage(errorDescription)
        }
        if status == 429 {
            return "Too many sign-in attempts. Try again in a minute."
        }
        if status >= 500 {
            return "The server had a problem (HTTP \(status)). Try again shortly."
        }
        return errorDescription?.nonEmptyTrimmed ?? "Sign-in failed (HTTP \(status))."
    }
}

/// The part of ABP's `GET /api/abp/application-configuration` the agent reads:
/// `{"currentUser":{"isAuthenticated":true,"id":"…",…},…}`.
public enum ApplicationConfiguration {
    private struct Body: Decodable {
        let currentUser: CurrentUser?
    }

    private struct CurrentUser: Decodable {
        let isAuthenticated: Bool?
        let id: String?
    }

    /// The authenticated user's id; nil for an anonymous or unreadable answer.
    public static func currentUserId(from data: Data) -> String? {
        guard let body = try? JSONDecoder().decode(Body.self, from: data),
              let user = body.currentUser, user.isAuthenticated != false,
              let id = user.id?.nonEmptyTrimmed
        else {
            return nil
        }
        return id
    }
}
