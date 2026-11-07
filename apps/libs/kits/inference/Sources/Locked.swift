import Synchronization

/// A value several tasks share, behind a lock. For InferenceKit and its
/// runtimes.
package final class Locked<Value: Sendable>: Sendable {
    private let mutex: Mutex<Value>

    package init(_ value: Value) { mutex = Mutex(value) }

    package var value: Value { mutex.withLock { $0 } }

    package func setValue(_ value: Value) { mutex.withLock { $0 = value } }

    package func withValue<Result>(_ body: (inout sending Value) throws -> sending Result) rethrows -> sending Result {
        try mutex.withLock(body)
    }
}
