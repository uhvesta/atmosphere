import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient

/// The last change made to tasks, Projects or Avenues on this device,
/// which Undo reverts.
package struct LastChange: Equatable, Sendable {
    package var id: UUID
    /// What changed, in words, such as "“Run” is Done".
    package var message: String
    package var change: Change
    /// The journal entry Undo reverts, for a change to a Project or an
    /// Avenue, and whatever tasks went with it. A change to tasks alone
    /// is undone from `change`.
    package var entry: UUID?
    /// The records it touched, as it left them. Undo refuses once one of
    /// them has changed again, rather than overwrite that change.
    package var after: [RecordID: RecordValue?]
    /// Whether the toast offers Undo. Lists already offer it for a reopened
    /// parent, in their own banner.
    package var showsToast: Bool
    /// Where its save comes among the store's saves, so an earlier save
    /// that replies after it doesn't take its place.
    package var number: Int

    /// `nil` for saves that change no task, Project or Avenue, such as a
    /// new comment, Settings or what an Avenue shares with apps, and for
    /// Undo itself, which isn't a change to undo in turn.
    package init?(_ saved: Saved, id: UUID) {
        let change = saved.change
        guard change.reverts.isEmpty else { return nil }
        let workspace = saved.workspace
        let steps = change.entry.flatMap { entry in workspace.journal.first { $0.id == entry } }?.steps ?? []
        let place =
            steps.first { $0.record.kind == .avenue && Self.isUndoable($0) }
            ?? steps.first { $0.record.kind == .project && Self.isUndoable($0) }
        guard !change.previous.isEmpty || place != nil else { return nil }
        self.id = id
        self.change = change
        number = saved.number
        showsToast = change.reopenedParent == nil
        if let place, let entry = change.entry {
            self.entry = entry
            after = Dictionary(steps.map { ($0.record, workspace.value(of: $0.record)) }) { first, _ in first }
            // A plan adding a Project with its tasks reads as the tasks it
            // added; Undo takes the Project too.
            message =
                place.before == nil && !change.previous.isEmpty
                ? Self.message(for: change, in: workspace) : Self.message(for: place, in: workspace)
            return
        }
        entry = nil
        after = Dictionary(
            uniqueKeysWithValues: change.previous.keys.map {
                (RecordID(.task, $0), workspace.tasks[$0].map(RecordValue.task))
            })
        message = Self.message(for: change, in: workspace)
    }

    /// Whether every record is still as this change left it in `workspace`.
    package func isCurrent(in workspace: Workspace) -> Bool {
        after.allSatisfy { record, value in workspace.value(of: record) == value }
    }

    /// Whether `step` is one Undo offers: not only what an Avenue shares
    /// with apps, which is a setting, changed where it's shown.
    private static func isUndoable(_ step: JournalStep) -> Bool {
        step.fields.contains { $0.field != .agentAccess }
    }

    /// What a change to a Project or an Avenue did, in words.
    private static func message(for step: JournalStep, in workspace: Workspace) -> String {
        switch (step.before, step.after) {
        case (.project(let before)?, .project(let after)?):
            let name = "“\(after.name)”"
            if before.status != after.status {
                switch after.status {
                case .archived: return String(localized: "Archived \(name)")
                case .completed: return String(localized: "Completed \(name)")
                case .active: return String(localized: "Reopened \(name)")
                }
            }
            if before.avenueID != after.avenueID, let avenue = workspace.avenues[after.avenueID] {
                return String(localized: "Moved \(name) to \(avenue.name)")
            }
            return String(localized: "Changed \(name)")
        case (.avenue(let before)?, .avenue(let after)?):
            let name = "“\(after.name)”"
            switch (before.archivedAt, after.archivedAt) {
            case (nil, _?): return String(localized: "Archived \(name)")
            case (_?, nil): return String(localized: "Restored \(name)")
            default: return String(localized: "Changed \(name)")
            }
        case (nil, .project(let project)?): return String(localized: "Added “\(project.name)”")
        case (nil, .avenue(let avenue)?): return String(localized: "Added “\(avenue.name)”")
        default: return String(localized: "Changed \(step.record.kind == .avenue ? "an Avenue" : "a Project")")
        }
    }

    /// What a change to tasks did, in words.
    private static func message(for change: Change, in workspace: Workspace) -> String {
        let event = change.events.first { $0.reason == nil } ?? change.events.first
        let taskID = event?.taskID ?? change.previous.keys.sorted { $0.uuidString < $1.uuidString }[0]
        let task = workspace.tasks[taskID] ?? change.previous[taskID] ?? nil
        let title = task.map { "“\($0.title)”" } ?? "A task"
        switch (event?.kind, event?.field) {
        case (.created?, _):
            // A plan adds several tasks at once; its Sub tasks go with them.
            let created = Set(change.events.filter { $0.kind == .created }.map(\.taskID))
            let added = created.compactMap { workspace.tasks[$0] }
                .filter { task in task.parentID.map { !created.contains($0) } ?? true }
            // Where they went, when that's one place: a Project, a parent
            // task or an Avenue.
            let places = Set(
                added.map { task in
                    task.parentID.flatMap { workspace.tasks[$0].map { "“\($0.title)”" } }
                        ?? task.projectID.flatMap { workspace.projects[$0]?.name }
                        ?? workspace.avenues[task.avenueID]?.name
                })
            let place = places.count == 1 ? places.first ?? nil : nil
            switch (added.count > 1, place) {
            case (true, let place?): return String(localized: "Added \(added.count) tasks to \(place)")
            case (true, nil): return String(localized: "Added \(added.count) tasks")
            case (false, let place?): return String(localized: "Added \(title) to \(place)")
            case (false, nil): return String(localized: "Added \(title)")
            }
        case (.fieldChanged?, .status?):
            let status = event?.newValue.flatMap(TaskStatus.init(rawValue:))
            return status.map { String(localized: "\(title) is \($0.title)") } ?? String(localized: "Changed \(title)")
        case (.movedToTrash?, _):
            return String(localized: "Moved \(title) to the Trash")
        default:
            return String(localized: "Changed \(title)")
        }
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
                lastChange.withLock { last in
                    if (last?.number ?? 0) <= entry.number { last = entry }
                }
            } else if !saved.change.reverts.isEmpty {
                // Undone elsewhere, such as on the assistant's reply.
                lastChange.withLock { last in
                    if let entry = last?.change.entry, saved.change.reverts.contains(entry) { last = nil }
                }
            }
            return saved
        }
        return client
    }
}
