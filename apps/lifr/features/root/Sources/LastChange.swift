import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient

/// The last change made to tasks on this device, which Undo reverts.
package struct LastChange: Equatable, Sendable {
    package var id: UUID
    /// What changed, in words, such as "“Run” is Done".
    package var message: String
    package var change: Change
    /// The tasks it touched, as it left them. Undo refuses once one of them
    /// has changed again, rather than overwrite that change.
    package var after: [UUID: LifrTask?]
    /// Whether the toast offers Undo. Lists already offer it for a reopened
    /// parent, in their own banner.
    package var showsToast: Bool

    /// `nil` for saves that change no task, such as a new comment or Settings.
    package init?(_ saved: Saved, id: UUID) {
        let change = saved.change
        guard !change.previous.isEmpty else { return nil }
        let workspace = saved.workspace
        self.id = id
        self.change = change
        after = Dictionary(uniqueKeysWithValues: change.previous.keys.map { ($0, workspace.tasks[$0]) })
        showsToast = change.reopenedParent == nil

        let event = change.events.first { $0.reason == nil } ?? change.events.first
        let taskID = event?.taskID ?? change.previous.keys.sorted { $0.uuidString < $1.uuidString }[0]
        let task = workspace.tasks[taskID] ?? change.previous[taskID] ?? nil
        let title = task.map { "“\($0.title)”" } ?? "A task"
        switch (event?.kind, event?.field) {
        case (.created?, _):
            message = "Added \(title)"
        case (.fieldChanged?, .status?):
            let status = event?.newValue.flatMap(TaskStatus.init(rawValue:))
            message = status.map { "\(title) is \($0.title)" } ?? "Changed \(title)"
        case (.movedToTrash?, _):
            message = "Moved \(title) to the Trash"
        default:
            message = "Changed \(title)"
        }
    }

    /// Whether every task is still as this change left it in `workspace`.
    package func isCurrent(in workspace: Workspace) -> Bool {
        after.allSatisfy { id, task in workspace.tasks[id] == task }
    }
}

extension SharedKey where Self == InMemoryKey<LastChange?>.Default {
    /// The last change, kept by the store client the screens use.
    package static var lastChange: Self {
        Self[.inMemory("lifrLastChange"), default: nil]
    }
}

extension StoreClient {
    /// This client, keeping each change made through it in `lastChange`.
    func recordingChanges(into lastChange: Shared<LastChange?>, uuid: UUIDGenerator) -> StoreClient {
        var client = self
        client.save = { [save] operation in
            let saved = try await save(operation)
            if let entry = LastChange(saved, id: uuid()) {
                lastChange.withLock { $0 = entry }
            }
            return saved
        }
        return client
    }
}
