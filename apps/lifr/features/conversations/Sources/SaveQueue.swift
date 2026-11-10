import ComposableArchitecture
import Foundation

/// Runs changes to the kept chats one at a time, in the order they're made,
/// so an older copy of a chat is never kept after a newer one.
final class SaveQueue: Sendable {
    private let last = LockIsolated<Task<Void, Never>?>(nil)

    /// Starts `work` once everything given before it has finished, and
    /// returns it to wait on.
    func enqueue(_ work: @escaping @Sendable () async throws -> Void) -> Task<Void, any Error> {
        last.withValue { last in
            let previous = last
            let task = Task {
                await previous?.value
                try await work()
            }
            last = Task { _ = await task.result }
            return task
        }
    }
}

extension SaveQueue: DependencyKey {
    static let liveValue = SaveQueue()
    static var testValue: SaveQueue { SaveQueue() }
}

extension DependencyValues {
    var saveQueue: SaveQueue {
        get { self[SaveQueue.self] }
        set { self[SaveQueue.self] = newValue }
    }
}
