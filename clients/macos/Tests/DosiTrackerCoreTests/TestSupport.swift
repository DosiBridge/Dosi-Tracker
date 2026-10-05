import Foundation
import XCTest
@testable import DosiTrackerCore

/// An arbitrary Unix second, as in the Windows agent's Rust tests.
let t0: Int64 = 1_790_000_000
let t0Ms: Int64 = t0 * 1000

/// The instant of a Unix second.
func at(_ second: Int64) -> Date {
    EpochTime.date(seconds: second)
}

/// The instant of a Unix millisecond.
func atMs(_ ms: Int64) -> Date {
    EpochTime.date(milliseconds: ms)
}

/// A wire date (`2026-10-05T10:00:00Z`); fails the test when unreadable.
func utc(_ text: String, file: StaticString = #filePath, line: UInt = #line) -> Date {
    guard let date = WireDate.date(from: text) else {
        XCTFail("unreadable date \(text)", file: file, line: line)
        return Date(timeIntervalSince1970: 0)
    }
    return date
}

/// A device as the tests describe it.
func sampleDevice() -> DeviceInfo {
    DeviceInfo(
        deviceId: "5B0C1B9E-8F2C-4A43-9A51-0C7F6F2D9B11",
        deviceName: "Ana's MacBook Pro",
        osVersion: "macOS 14.5 (23F79)",
        agentVersion: "9.9.9"
    )
}

/// A fresh path in the temporary directory (the file does not exist yet).
func temporaryFileURL(_ name: String = "dosi-tracker-test") -> URL {
    FileManager.default.temporaryDirectory
        .appendingPathComponent("\(name)-\(UUID().uuidString.lowercased()).db", isDirectory: false)
}

/// Remove a queue file and its SQLite sidecars.
func removeDatabase(at url: URL) {
    for suffix in [""] + Queue.sidecarSuffixes {
        try? FileManager.default.removeItem(atPath: url.path + suffix)
    }
}

/// The JSON object (or array) in `data`, for comparisons that do not depend
/// on key order.
func jsonObject(_ data: Data, file: StaticString = #filePath, line: UInt = #line) -> Any? {
    do {
        return try JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
    } catch {
        XCTFail("not JSON: \(error)", file: file, line: line)
        return nil
    }
}

/// The JSON object in `data` as a dictionary.
func jsonDictionary(_ data: Data, file: StaticString = #filePath, line: UInt = #line) -> [String: Any] {
    guard let dictionary = jsonObject(data, file: file, line: line) as? [String: Any] else {
        XCTFail("not a JSON object", file: file, line: line)
        return [:]
    }
    return dictionary
}

/// Assert that `data` holds exactly the JSON of `golden` (same keys, same
/// values, any key order).
func assertJSONEqual(_ data: Data, _ golden: String, file: StaticString = #filePath, line: UInt = #line) {
    guard let actual = jsonObject(data, file: file, line: line),
          let expected = jsonObject(Data(golden.utf8), file: file, line: line)
    else {
        return
    }
    // JSONSerialization builds Foundation objects, which compare deeply.
    let same = (actual as? NSObject)?.isEqual(expected) ?? false
    XCTAssertTrue(same, "JSON differs.\nactual:   \(String(decoding: data, as: UTF8.self))\nexpected: \(golden)",
                  file: file, line: line)
}

// MARK: - A canned HTTP server, through URLProtocol

/// One canned answer of a ``MockServer``.
struct MockReply {
    var status: Int = 200
    var headers: [String: String] = [:]
    var body: Data = Data()
    /// Fail without a response instead (offline, refused, …).
    var failure: URLError?
    /// Never answer (until the request is cancelled).
    var hangs = false

    static func json(_ status: Int, _ body: String, headers: [String: String] = [:]) -> MockReply {
        var all = headers
        all["Content-Type"] = "application/json"
        return MockReply(status: status, headers: all, body: Data(body.utf8))
    }

    static func empty(_ status: Int, headers: [String: String] = [:]) -> MockReply {
        MockReply(status: status, headers: headers)
    }

    static func tokenOK(_ token: String = "t0k3n", refreshToken: String? = nil, expiresIn: Int = 3600) -> MockReply {
        var body = "{\"access_token\":\"\(token)\",\"token_type\":\"Bearer\",\"expires_in\":\(expiresIn)"
        if let refresh = refreshToken {
            body += ",\"refresh_token\":\"\(refresh)\""
        }
        return json(200, body + "}")
    }

    static func offline() -> MockReply {
        MockReply(failure: URLError(.cannotConnectToHost))
    }

    static func hanging() -> MockReply {
        MockReply(hangs: true)
    }
}

/// A request as the mock server received it.
struct CapturedRequest {
    let method: String
    let url: URL
    let headers: [String: String]
    let body: Data

    var path: String { url.path }
    /// The raw (still percent-encoded) query.
    var query: String? { url.query }
    var bodyText: String { String(decoding: body, as: UTF8.self) }

    func header(_ name: String) -> String? {
        headers.first { $0.key.caseInsensitiveCompare(name) == .orderedSame }?.value
    }

    /// The decoded fields of a form body.
    var formFields: [String: String] {
        var fields: [String: String] = [:]
        for pair in bodyText.split(separator: "&") {
            let parts = pair.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            guard parts.count == 2 else { continue }
            let key = String(parts[0]).removingPercentEncoding ?? String(parts[0])
            let value = String(parts[1]).replacingOccurrences(of: "+", with: " ").removingPercentEncoding ?? String(parts[1])
            fields[key] = value
        }
        return fields
    }
}

/// Serves canned replies, in order, to requests for its own host.
final class MockServer: @unchecked Sendable {
    let host = "\(UUID().uuidString.lowercased()).dosi.test"
    private let lock = NSLock()
    private var replies: [MockReply]
    private var received: [CapturedRequest] = []

    init(_ replies: [MockReply]) {
        self.replies = replies
        MockServer.register(self)
    }

    deinit {
        MockServer.unregister(host)
    }

    var baseURL: URL { URL(string: "https://\(host)")! }

    var requests: [CapturedRequest] {
        lock.lock()
        defer { lock.unlock() }
        return received
    }

    /// Replies not used yet.
    var remainingReplies: Int {
        lock.lock()
        defer { lock.unlock() }
        return replies.count
    }

    fileprivate func next(for request: CapturedRequest) -> MockReply? {
        lock.lock()
        defer { lock.unlock() }
        received.append(request)
        return replies.isEmpty ? nil : replies.removeFirst()
    }

    private static let registryLock = NSLock()
    private static var registry: [String: MockServer] = [:]

    private static func register(_ server: MockServer) {
        registryLock.lock()
        defer { registryLock.unlock() }
        registry[server.host] = server
    }

    private static func unregister(_ host: String) {
        registryLock.lock()
        defer { registryLock.unlock() }
        registry[host] = nil
    }

    fileprivate static func server(for host: String?) -> MockServer? {
        guard let host = host else { return nil }
        registryLock.lock()
        defer { registryLock.unlock() }
        return registry[host.lowercased()]
    }

    /// A URLSession whose requests go to the mock servers.
    static func session() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MockURLProtocol.self]
        return ApiClient.makeSession(configuration: configuration)
    }
}

/// Routes requests to the ``MockServer`` registered for their host.
final class MockURLProtocol: URLProtocol {
    override class func canInit(with request: URLRequest) -> Bool {
        true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        guard let url = request.url else {
            client?.urlProtocol(self, didFailWithError: URLError(.badURL))
            return
        }
        let captured = CapturedRequest(
            method: request.httpMethod ?? "GET",
            url: url,
            headers: request.allHTTPHeaderFields ?? [:],
            body: MockURLProtocol.body(of: request)
        )
        guard let server = MockServer.server(for: url.host), let reply = server.next(for: captured) else {
            client?.urlProtocol(self, didFailWithError: URLError(.cannotConnectToHost))
            return
        }
        if reply.hangs {
            return
        }
        if let failure = reply.failure {
            client?.urlProtocol(self, didFailWithError: failure)
            return
        }
        guard let response = HTTPURLResponse(url: url, statusCode: reply.status, httpVersion: "HTTP/1.1",
                                             headerFields: reply.headers)
        else {
            client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
            return
        }
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        if !reply.body.isEmpty {
            client?.urlProtocol(self, didLoad: reply.body)
        }
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}

    /// The body of a request: URLSession hands it to protocols as a stream.
    private static func body(of request: URLRequest) -> Data {
        if let body = request.httpBody {
            return body
        }
        guard let stream = request.httpBodyStream else { return Data() }
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 16_384)
        while stream.hasBytesAvailable {
            let read = stream.read(&buffer, maxLength: buffer.count)
            if read <= 0 { break }
            data.append(buffer, count: read)
        }
        return data
    }
}
