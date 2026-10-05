import Foundation
import SQLite3

/// An SQLite failure: the (extended) result code and SQLite's message.
public struct QueueError: Error, Equatable, CustomStringConvertible {
    public let code: Int32
    public let message: String

    public init(code: Int32, message: String) {
        self.code = code
        self.message = message
    }

    /// The database is damaged (corrupt pages, or not SQLite at all), as
    /// opposed to locked, unreadable or out of disk space.
    public var isDamage: Bool {
        let primary = code & 0xFF
        return primary == SQLITE_CORRUPT || primary == SQLITE_NOTADB
    }

    public var description: String { "SQLite error \(code): \(message)" }
}

/// A value bound to a statement parameter.
enum SQLiteValue {
    case int(Int64)
    case text(String)
    case null
}

/// `SQLITE_TRANSIENT`: SQLite copies bound text before the call returns.
private let transientDestructor = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

/// A minimal wrapper over the SQLite C API (the system library, no external
/// dependency). One connection, used under the owner's lock.
final class SQLiteDatabase {
    private let handle: OpaquePointer

    init(path: String) throws {
        var db: OpaquePointer?
        let flags = SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX
        let status = sqlite3_open_v2(path, &db, flags, nil)
        guard status == SQLITE_OK, let opened = db else {
            let message = db.map { String(cString: sqlite3_errmsg($0)) } ?? "could not open \(path)"
            if let db = db {
                sqlite3_close_v2(db)
            }
            throw QueueError(code: status, message: message)
        }
        handle = opened
        sqlite3_extended_result_codes(handle, 1)
        // Another process holding a lock (a second copy starting) waits
        // instead of failing at once.
        sqlite3_busy_timeout(handle, 5000)
    }

    deinit {
        sqlite3_close_v2(handle)
    }

    /// Run one or more statements that return no rows (or whose rows are
    /// ignored).
    func execute(_ sql: String) throws {
        var errorMessage: UnsafeMutablePointer<CChar>?
        let status = sqlite3_exec(handle, sql, nil, nil, &errorMessage)
        guard status == SQLITE_OK else {
            let message = errorMessage.map { String(cString: $0) } ?? lastMessage
            sqlite3_free(errorMessage)
            throw QueueError(code: status, message: message)
        }
    }

    /// Run one statement with `bindings`; returns how many rows it changed.
    @discardableResult
    func run(_ sql: String, _ bindings: [SQLiteValue] = []) throws -> Int {
        let statement = try prepare(sql)
        defer { sqlite3_finalize(statement) }
        try bind(bindings, to: statement)
        let status = sqlite3_step(statement)
        guard status == SQLITE_DONE || status == SQLITE_ROW else {
            throw QueueError(code: status, message: lastMessage)
        }
        return Int(sqlite3_changes(handle))
    }

    /// Run a query and map each row.
    func query<T>(_ sql: String, _ bindings: [SQLiteValue] = [], _ map: (Row) throws -> T) throws -> [T] {
        let statement = try prepare(sql)
        defer { sqlite3_finalize(statement) }
        try bind(bindings, to: statement)
        var rows: [T] = []
        while true {
            let status = sqlite3_step(statement)
            if status == SQLITE_DONE { break }
            guard status == SQLITE_ROW else {
                throw QueueError(code: status, message: lastMessage)
            }
            rows.append(try map(Row(statement: statement)))
        }
        return rows
    }

    /// The first column of the first row as an integer (nil without a row or
    /// for NULL).
    func scalar(_ sql: String, _ bindings: [SQLiteValue] = []) throws -> Int64? {
        try query(sql, bindings) { $0.int64OrNil(0) }.first ?? nil
    }

    var lastInsertRowId: Int64 { sqlite3_last_insert_rowid(handle) }

    private var lastMessage: String { String(cString: sqlite3_errmsg(handle)) }

    private func prepare(_ sql: String) throws -> OpaquePointer {
        var statement: OpaquePointer?
        let status = sqlite3_prepare_v2(handle, sql, -1, &statement, nil)
        guard status == SQLITE_OK, let prepared = statement else {
            let message = lastMessage
            sqlite3_finalize(statement)
            throw QueueError(code: status, message: message)
        }
        return prepared
    }

    private func bind(_ values: [SQLiteValue], to statement: OpaquePointer) throws {
        for (offset, value) in values.enumerated() {
            let index = Int32(offset + 1)
            let status: Int32
            switch value {
            case .int(let number):
                status = sqlite3_bind_int64(statement, index, number)
            case .text(let text):
                status = sqlite3_bind_text(statement, index, text, -1, transientDestructor)
            case .null:
                status = sqlite3_bind_null(statement, index)
            }
            guard status == SQLITE_OK else {
                throw QueueError(code: status, message: lastMessage)
            }
        }
    }

    /// The current row of a stepping statement.
    struct Row {
        fileprivate let statement: OpaquePointer

        func int64(_ column: Int32) -> Int64 {
            sqlite3_column_int64(statement, column)
        }

        func int64OrNil(_ column: Int32) -> Int64? {
            sqlite3_column_type(statement, column) == SQLITE_NULL ? nil : sqlite3_column_int64(statement, column)
        }

        func text(_ column: Int32) -> String? {
            guard sqlite3_column_type(statement, column) != SQLITE_NULL,
                  let pointer = sqlite3_column_text(statement, column)
            else {
                return nil
            }
            return String(cString: pointer)
        }
    }
}
