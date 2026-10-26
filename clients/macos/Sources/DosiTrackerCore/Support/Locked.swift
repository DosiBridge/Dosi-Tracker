import Foundation

/// A value read and replaced under a lock, from any thread: for the few
/// settings an actor must take synchronously (the workspace an API client
/// acts in, who hears about its rotated tokens).
public final class Locked<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Value

    public init(_ value: Value) {
        self.value = value
    }

    /// The value now.
    public func get() -> Value {
        lock.lock()
        defer { lock.unlock() }
        return value
    }

    /// Replace the value.
    public func set(_ newValue: Value) {
        lock.lock()
        value = newValue
        lock.unlock()
    }
}
