import Foundation

/// Thin async wrapper around the Dosi-Tracker backend REST API.
///
/// Authentication uses the backend's OpenIddict token endpoint
/// (`POST /connect/token`, resource-owner password flow, client `Tracker_App`).
/// The bearer token is cached and refreshed via re-login shortly before expiry.
actor ApiClient {
    private static let clientId = "Tracker_App"
    private static let scope = "Tracker"

    private let baseURL: String
    private let username: String
    private let password: String
    /// Workspace (tenant) name; empty for a host account.
    private let tenant: String
    private let session: URLSession
    private var token: String?
    private var tokenExpiresAt = Date.distantPast

    init(baseURL: String, username: String, password: String, tenant: String = "") {
        self.baseURL = baseURL
        self.username = username
        self.password = password
        self.tenant = tenant
        self.session = URLSession(configuration: .ephemeral)
    }

    nonisolated var hasCredentials: Bool { !username.isEmpty }

    /// Upload outcome classification, so the sync loop can park permanently
    /// rejected payloads instead of letting them block the queue forever.
    enum SubmitOutcome {
        case ok
        case rejected(String)
        case transient(String)
    }

    private struct TokenResponse: Decodable {
        let accessToken: String
        let expiresIn: Int?

        enum CodingKeys: String, CodingKey {
            case accessToken = "access_token"
            case expiresIn = "expires_in"
        }
    }

    /// Authenticate against the OpenIddict token endpoint and cache the bearer token.
    /// The workspace (tenant) MUST travel as a query parameter: ABP's tenant
    /// resolvers read route/query/header/cookie, **not** the POST body. A
    /// body-only `__tenant` silently resolves to the host tenant.
    func login() async throws {
        var tokenPath = "/connect/token"
        if !tenant.isEmpty {
            let encoded = tenant.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? tenant
            tokenPath += "?__tenant=\(encoded)"
        }
        guard let url = URL(string: baseURL + tokenPath) else {
            throw ApiError.badConfiguration("invalid apiBaseURL: \(baseURL)")
        }

        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.httpBody = Self.formEncode([
            "grant_type": "password",
            "username": username,
            "password": password,
            "client_id": Self.clientId,
            "scope": Self.scope,
        ])

        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            let code = (response as? HTTPURLResponse)?.statusCode ?? -1
            throw ApiError.http(code)
        }

        let tokenResponse = try JSONDecoder().decode(TokenResponse.self, from: data)
        token = tokenResponse.accessToken
        // Refresh one minute early so requests never race the expiry.
        let ttl = max(60, (tokenResponse.expiresIn ?? 3600) - 60)
        tokenExpiresAt = Date().addingTimeInterval(TimeInterval(ttl))
    }

    private func ensureToken() async throws -> String {
        if token == nil || Date() >= tokenExpiresAt {
            try await login()
        }
        guard let token else { throw ApiError.notAuthenticated }
        return token
    }

    /// Active (non-archived) projects the user may track, with capture permissions.
    func projects() async throws -> [Project] {
        let data = try await get("/api/app/project/my-projects")
        return try JSONDecoder.iso.decode([Project].self, from: data)
    }

    /// Upload a completed activity time block.
    func submit(activity: Activity) async -> SubmitOutcome {
        do {
            let bearer = try await ensureToken()
            guard let url = URL(string: baseURL + "/api/app/activity") else {
                return .rejected("invalid apiBaseURL")
            }

            var request = URLRequest(url: url)
            request.httpMethod = "POST"
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization")
            request.httpBody = try JSONEncoder.iso.encode(activity)

            let (data, response) = try await session.data(for: request)
            guard let http = response as? HTTPURLResponse else {
                return .transient("no HTTP response")
            }

            if (200..<300).contains(http.statusCode) {
                return .ok
            }

            if http.statusCode == 401 {
                // Token revoked/expired server-side; force a re-login on the next attempt.
                token = nil
                return .transient("HTTP 401")
            }

            // Timeout / throttling are transient; other 4xx are permanent rejections.
            if (400..<500).contains(http.statusCode) && http.statusCode != 408 && http.statusCode != 429 {
                let body = String(data: data, encoding: .utf8) ?? ""
                return .rejected("HTTP \(http.statusCode): \(body.prefix(512))")
            }

            return .transient("HTTP \(http.statusCode)")
        } catch {
            return .transient(error.localizedDescription)
        }
    }

    // MARK: - Transport

    private func get(_ path: String) async throws -> Data {
        let bearer = try await ensureToken()
        guard let url = URL(string: baseURL + path) else {
            throw ApiError.badConfiguration("invalid apiBaseURL: \(baseURL)")
        }
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization")

        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            let code = (response as? HTTPURLResponse)?.statusCode ?? -1
            throw ApiError.http(code)
        }
        return data
    }

    private static func formEncode(_ fields: [String: String]) -> Data {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~")
        let encoded = fields.map { key, value in
            let k = key.addingPercentEncoding(withAllowedCharacters: allowed) ?? key
            let v = value.addingPercentEncoding(withAllowedCharacters: allowed) ?? value
            return "\(k)=\(v)"
        }
        return Data(encoded.joined(separator: "&").utf8)
    }

    enum ApiError: LocalizedError {
        case http(Int)
        case notAuthenticated
        case badConfiguration(String)

        var errorDescription: String? {
            switch self {
            case .http(let code): return "HTTP \(code)"
            case .notAuthenticated: return "not authenticated"
            case .badConfiguration(let message): return message
            }
        }
    }
}

extension JSONEncoder {
    static let iso: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        return encoder
    }()
}

extension JSONDecoder {
    /// ISO-8601 decoder that also accepts .NET's 7-digit fractional seconds
    /// (Foundation's built-in `.iso8601` strategy cannot parse fractions at all).
    static let iso: JSONDecoder = {
        let decoder = JSONDecoder()
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]

        decoder.dateDecodingStrategy = .custom { decoder in
            let container = try decoder.singleValueContainer()
            let raw = try container.decode(String.self)

            if let date = plain.date(from: raw) ?? fractional.date(from: raw) {
                return date
            }
            if let date = fractional.date(from: Self.normalizeFraction(raw)) {
                return date
            }
            throw DecodingError.dataCorruptedError(
                in: container, debugDescription: "Unparseable ISO8601 date: \(raw)")
        }
        return decoder
    }()

    /// Clamp the fractional-seconds part to exactly 3 digits (e.g. ".1234567" → ".123"),
    /// which is what ISO8601DateFormatter's `.withFractionalSeconds` expects.
    private static func normalizeFraction(_ raw: String) -> String {
        guard let dotIndex = raw.firstIndex(of: ".") else { return raw }

        var digits = ""
        var idx = raw.index(after: dotIndex)
        while idx < raw.endIndex, raw[idx].isNumber {
            digits.append(raw[idx])
            idx = raw.index(after: idx)
        }
        guard !digits.isEmpty else { return raw }

        let clamped = String((digits + "000").prefix(3))
        return String(raw[..<dotIndex]) + "." + clamped + String(raw[idx...])
    }
}
