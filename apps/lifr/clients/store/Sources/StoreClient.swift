import ComposableArchitecture
import Foundation
import LifrCore

/// A Core operation on the records. It returns what it changed, or throws
/// a `LifrError` and changes nothing.
package typealias StoreOperation = @Sendable (inout Workspace, Context) throws(LifrError) -> Change

/// Reads and changes Lifr's records. Each change runs Core's rules, so the
/// errors are `LifrError`s whose descriptions can be shown as is.
@DependencyClient
package struct StoreClient: Sendable {
    /// Every record.
    package var load: @Sendable () async throws -> Workspace
    /// Every record, numbered as the latest save or update they include,
    /// for a screen that reloads after changes made outside it, such as the
    /// assistant's. See `SaveOrder`.
    package var snapshot: @Sendable () async throws -> Snapshot
    /// Runs `operation` on the records and saves what it changed, all or
    /// nothing. The named operations below are built on this.
    package var save: @Sendable (_ operation: StoreOperation) async throws -> Saved
    /// The records each time something other than this client changed them,
    /// such as a sync import, with what reconciling them fixed. Ends when the
    /// calling task is cancelled.
    package var updates: @Sendable () async -> AsyncStream<Update> = { AsyncStream { $0.finish() } }
    /// Writes what sync brought in from other devices without a journal
    /// entry of its own, reconciles the records, and sends the result to
    /// `updates`.
    package var importSynced: @Sendable (_ changes: SyncedChanges) async throws -> Update
    /// Packs the journal entries made before `cutoff` into monthly archives
    /// of the fields they changed, which take far less room. They can't be
    /// reverted after, and `load` leaves them out.
    package var archiveJournal: @Sendable (_ cutoff: Date) async throws -> Void = { _ in }
    /// Every archived journal entry, oldest first.
    package var archivedJournal: @Sendable () async throws -> [ArchivedEntry] = { [] }
}

/// Records, journal entries and history events another device sent.
package struct SyncedChanges: Hashable, Sendable {
    /// Each record as it now is, nil when it was removed.
    package var records: [RecordID: RecordValue?]
    package var journal: [JournalEntry]
    package var history: [HistoryEvent]

    package init(records: [RecordID: RecordValue?] = [:], journal: [JournalEntry] = [], history: [HistoryEvent] = []) {
        self.records = records
        self.journal = journal
        self.history = history
    }

    package var isEmpty: Bool { records.isEmpty && journal.isEmpty && history.isEmpty }
}

/// The records after a change, and what changed.
package struct Saved: Hashable, Sendable {
    package var workspace: Workspace
    package var change: Change
    /// Where the save comes among the store's saves and updates: a later
    /// one has a larger number. Saves made at once can reply in any order,
    /// so this tells which records are the latest. See `SaveOrder`.
    package var number: Int

    package init(workspace: Workspace, change: Change, number: Int = 0) {
        self.workspace = workspace
        self.change = change
        self.number = number
    }

    package var snapshot: Snapshot { Snapshot(workspace, number: number) }
}

/// The records at a place in the store's order: after a save or an update,
/// or as read with `snapshot`. Screens pass these to each other, so each
/// can tell whether records are later than the ones it shows.
package struct Snapshot: Hashable, Sendable {
    package var workspace: Workspace
    /// Where the records come among the store's saves and updates: later
    /// records have a larger number. Records read with `load` come before
    /// any, as 0.
    package var number: Int

    package init(_ workspace: Workspace, number: Int) {
        self.workspace = workspace
        self.number = number
    }
}

/// The latest records a screen has shown, by their place in the store's
/// order, so a save's reply or an update arriving after later records
/// doesn't put back what was there before them.
package struct SaveOrder: Hashable, Sendable {
    /// The number of the latest records shown.
    package private(set) var latest = 0

    package init() {}

    /// Whether records numbered `number` are as late as any shown so far,
    /// noting them if so.
    package mutating func isLatest(_ number: Int) -> Bool {
        guard number >= latest else { return false }
        latest = number
        return true
    }

    package mutating func isLatest(_ saved: Saved) -> Bool { isLatest(saved.number) }

    package mutating func isLatest(_ snapshot: Snapshot) -> Bool { isLatest(snapshot.number) }
}

/// The records after another device or process changed them.
package struct Update: Hashable, Sendable {
    package var workspace: Workspace
    /// What reconciling the imported records fixed, for the banner that
    /// explains it.
    package var fixes: [Fix]
    /// Where the update comes among the store's saves and updates, as
    /// `Saved.number`.
    package var number: Int

    package init(workspace: Workspace, fixes: [Fix] = [], number: Int = 0) {
        self.workspace = workspace
        self.fixes = fixes
        self.number = number
    }

    package var snapshot: Snapshot { Snapshot(workspace, number: number) }
}

extension StoreClient {
    /// `Workspace.setStatus`. Pass `cascade: true` once the person has
    /// confirmed closing open Sub tasks as Won't Do.
    package func setStatus(_ status: TaskStatus, _ id: UUID, _ cascade: Bool = false) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.setStatus(status, of: id, cascade: cascade, in: context)
        }
    }

    /// Reverts a change made earlier, such as a parent reopened by its Sub task.
    package func undo(_ change: Change) async throws -> Saved {
        try await save { workspace, context in workspace.undo(change, in: context) }
    }

    /// `Workspace.revert`: puts back what a history event changed.
    package func revert(_ eventID: UUID) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in try workspace.revert(eventID, in: context) }
    }

    /// `Workspace.revert(_:in:)`: puts back what journal entries changed, as
    /// one new entry.
    package func revert(entries ids: [UUID]) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in try workspace.revert(ids, in: context) }
    }

    package func createTask(_ new: Workspace.NewTask) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in try workspace.createTask(new, in: context) }
    }

    package func edit(_ id: UUID, _ edit: Workspace.TaskEdit) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in try workspace.edit(id, edit, in: context) }
    }

    /// `Workspace.move`: a top-level task and its Sub tasks to another Avenue
    /// or Project.
    package func move(_ id: UUID, toAvenue avenueID: UUID, project projectID: UUID?) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.move(id, toAvenue: avenueID, project: projectID, in: context)
        }
    }

    /// Adds the label called `name` to a task, making the label first when
    /// there's none by that name in any case.
    package func addLabel(named name: String, to id: UUID) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            let task = try workspace.task(id)
            let existing = workspace.labels.values.first { $0.name.caseInsensitiveCompare(name) == .orderedSame }
            let label = existing ?? workspace.createLabel(name: name, in: context)
            return try workspace.edit(id, Workspace.TaskEdit(labelIDs: task.labelIDs.union([label.id])), in: context)
        }
    }

    package func addComment(_ body: String, to id: UUID) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            _ = try workspace.addComment(body, to: id, in: context)
            return Change()
        }
    }

    package func createAvenue(key: String, name: String) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.createAvenue(key: key, name: name, in: context)
            return Change()
        }
    }

    package func createProject(_ name: String, in avenueID: UUID) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.createProject(name: name, inAvenue: avenueID, in: context)
            return Change()
        }
    }

    package func edit(project id: UUID, _ edit: Workspace.ProjectEdit) async throws -> Saved {
        try await save { (workspace, _) throws(LifrError) in
            try workspace.edit(project: id, edit)
            return Change()
        }
    }

    /// `Workspace.move(project:toAvenue:)`: a Project and its tasks.
    package func move(project id: UUID, toAvenue avenueID: UUID) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.move(project: id, toAvenue: avenueID, in: context)
        }
    }

    /// Completes a Project, or makes an archived or completed one active
    /// again.
    package func setStatus(_ status: ProjectStatus, ofProject id: UUID) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.setStatus(status, ofProject: id, in: context)
            return Change()
        }
    }

    package func archive(project id: UUID, openTasks: Workspace.OpenTasks) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.archive(project: id, openTasks: openTasks, in: context)
        }
    }

    package func edit(avenue id: UUID, _ edit: Workspace.AvenueEdit) async throws -> Saved {
        try await save { (workspace, _) throws(LifrError) in
            try workspace.edit(avenue: id, edit)
            return Change()
        }
    }

    package func archive(avenue id: UUID, openTasks: Workspace.OpenTasks?) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.archive(avenue: id, openTasks: openTasks, in: context)
        }
    }

    package func restore(avenue id: UUID) async throws -> Saved {
        try await save { (workspace, _) throws(LifrError) in
            try workspace.restore(avenue: id)
            return Change()
        }
    }

    /// `Workspace.moveToTrash`: a task and its Sub tasks, for 30 days.
    package func moveToTrash(_ id: UUID) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in try workspace.moveToTrash(id, in: context) }
    }

    /// `Workspace.restore`: a task and the Sub tasks trashed with it, out
    /// of the trash.
    package func restore(_ id: UUID) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in try workspace.restore(id, in: context) }
    }

    /// `Workspace.emptyTrash`: deletes everything in the trash for good.
    package func emptyTrash() async throws -> Saved {
        try await save { workspace, _ in
            workspace.emptyTrash()
            return Change()
        }
    }

    /// `Workspace.reorder(task:after:before:)`: puts a task between two
    /// others in manual order.
    package func reorder(task id: UUID, after lower: UUID?, before upper: UUID?) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.reorder(task: id, after: lower, before: upper, in: context)
        }
    }

    /// `Workspace.reorder(avenue:after:before:)`.
    package func reorder(avenue id: UUID, after lower: UUID?, before upper: UUID?) async throws -> Saved {
        try await save { (workspace, _) throws(LifrError) in
            try workspace.reorder(avenue: id, after: lower, before: upper)
            return Change()
        }
    }

    /// `Workspace.reorder(project:after:before:)`.
    package func reorder(project id: UUID, after lower: UUID?, before upper: UUID?) async throws -> Saved {
        try await save { (workspace, _) throws(LifrError) in
            try workspace.reorder(project: id, after: lower, before: upper)
            return Change()
        }
    }

    /// `Workspace.ensureInbox`: adds the Inbox if there isn't one. Saves
    /// nothing when there is.
    package func ensureInbox() async throws {
        guard try await load().avenues[Avenue.inboxID] == nil else { return }
        _ = try await save { workspace, context in
            workspace.ensureInbox(in: context)
            return Change()
        }
    }

    /// `Workspace.purgeTrash`: deletes what has been in the trash longer
    /// than it's kept. Saves nothing when there's nothing to delete.
    package func purgeTrash(now: Date) async throws {
        let workspace = try await load()
        var purged = workspace
        purged.purgeTrash(now: now)
        guard purged.tasks.count != workspace.tasks.count else { return }
        _ = try await save { workspace, _ in
            workspace.purgeTrash(now: now)
            return Change()
        }
    }

    /// `Workspace.setParent`: makes a task a Sub task of `parentID`, or a
    /// top-level task when it's nil.
    package func setParent(_ parentID: UUID?, of id: UUID) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.setParent(parentID, of: id, in: context)
        }
    }

    package func addReminder(_ trigger: Reminder.Trigger, to id: UUID) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.addReminder(trigger, to: id, in: context)
            return Change()
        }
    }

    package func removeReminder(_ id: UUID) async throws -> Saved {
        try await save { (workspace, _) throws(LifrError) in
            try workspace.removeReminder(id)
            return Change()
        }
    }

    /// What agents may do in an Avenue: everything, read only, or not see it.
    package func setAgentAccess(_ access: AgentAccess, ofAvenue id: UUID) async throws -> Saved {
        try await save { (workspace, _) throws(LifrError) in
            try workspace.setAgentAccess(access, ofAvenue: id)
            return Change()
        }
    }

    package func setSettings(_ settings: Settings) async throws -> Saved {
        try await save { workspace, _ in
            workspace.settings = settings
            return Change()
        }
    }
}

extension StoreClient: TestDependencyKey {
    package static let testValue = StoreClient()
    package static let previewValue = StoreClient.inMemory(.sample(now: Date(), timeZone: .current))
}

extension DependencyValues {
    package var store: StoreClient {
        get { self[StoreClient.self] }
        set { self[StoreClient.self] = newValue }
    }
}
