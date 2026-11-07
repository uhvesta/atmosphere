import Synchronization

/// A value several tasks share, behind a lock.
final class Locked<Value: Sendable>: Sendable {
    private let mutex: Mutex<Value>

    init(_ value: Value) { mutex = Mutex(value) }

    var value: Value { mutex.withLock { $0 } }

    func withValue<Result>(_ body: (inout sending Value) throws -> sending Result) rethrows -> sending Result {
        try mutex.withLock(body)
    }
}
