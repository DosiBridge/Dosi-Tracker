import Foundation

/// Thin async wrapper around the Dosi-Tracker backend REST API.
actor ApiClient {
    private let baseURL: String
    private var token: String?
    private let session: URLSession

    init(baseURL: String) {
        self.baseURL = baseURL
        self.session = URLSession(configuration: .ephemeral)
    }

    func setToken(_ token: String) {
        self.token = token
    }

    struct LoginRequest: Codable {
        let username: String
        let password: String
    }

    func login(username: String, password: String) async throws -> AuthSession {
        let body = try JSONEncoder.iso.encode(LoginRequest(username: username, password: password))
        let data = try await post("/api/app/account/login", body: body, authed: false)
        let sessionObj = try JSONDecoder.iso.decode(AuthSession.self, from: data)
        self.token = sessionObj.accessToken
        return sessionObj
    }

    func projects() async throws -> [Project] {
        let data = try await get("/api/app/project/my-projects")
        return try JSONDecoder.iso.decode([Project].self, from: data)
    }

    func submit(activity: Activity) async throws {
        let body = try JSONEncoder.iso.encode(activity)
        _ = try await post("/api/app/activity", body: body, authed: true)
    }

    // MARK: - Transport

    private func get(_ path: String) async throws -> Data {
        var request = URLRequest(url: URL(string: baseURL + path)!)
        request.httpMethod = "GET"
        applyAuth(&request)
        return try await perform(request)
    }

    private func post(_ path: String, body: Data, authed: Bool) async throws -> Data {
        var request = URLRequest(url: URL(string: baseURL + path)!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = body
        if authed { applyAuth(&request) }
        return try await perform(request)
    }

    private func applyAuth(_ request: inout URLRequest) {
        if let token = token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
    }

    private func perform(_ request: URLRequest) async throws -> Data {
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            let code = (response as? HTTPURLResponse)?.statusCode ?? -1
            throw NSError(domain: "DosiTracker.Api", code: code,
                          userInfo: [NSLocalizedDescriptionKey: "HTTP \(code)"])
        }
        return data
    }
}

extension JSONEncoder {
    static let iso: JSONEncoder = {
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        return e
    }()
}

extension JSONDecoder {
    static let iso: JSONDecoder = {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return d
    }()
}
