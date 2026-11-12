import ComposableArchitecture
import Foundation

/// Runs changes to the kept chats one at a time, in the order they're made,
/// so an older copy of a chat is never kept after a newer one. A save of a
/// chat still waiting when a newer save of it is given doesn't run: the
/// newer one keeps the chat as it is now, so typing in a draft doesn't line
/// up a save for each keystroke.
package final class SaveQueue: Sendable {
    private struct Line {
        var last: Task<Void, Never>?
        var given = 0
        /// For each chat with a save waiting, the number of its newest.
        var newest: [UUID: Int] = [:]
    }

    private let line = LockIsolated(Line())

    package init() {}

    /// Starts `work` once everything given before it has finished, and
    /// returns it to wait on. With `chat`, `work` saves that chat, and it's
    /// left out if a newer save of the chat is given before it starts.
    package func enqueue(
        saving chat: UUID? = nil, _ work: @escaping @Sendable () async throws -> Void
    ) -> Task<Void, any Error> {
        line.withValue { line in
            line.given += 1
            let number = line.given
            if let chat { line.newest[chat] = number }
            let previous = line.last
            let task = Task { [self] in
                await previous?.value
                if let chat, !starts(chat, number) { return }
                try await work()
            }
            line.last = Task { _ = await task.result }
            return task
        }
    }

    /// Whether save `number` of `chat` is its newest, so it runs.
    private func starts(_ chat: UUID, _ number: Int) -> Bool {
        line.withValue { line in
            guard line.newest[chat] == number else { return false }
            line.newest[chat] = nil
            return true
        }
    }
}

extension SaveQueue: DependencyKey {
    package static let liveValue = SaveQueue()
    package static var testValue: SaveQueue { SaveQueue() }
}

extension DependencyValues {
    var saveQueue: SaveQueue {
        get { self[SaveQueue.self] }
        set { self[SaveQueue.self] = newValue }
    }
}
