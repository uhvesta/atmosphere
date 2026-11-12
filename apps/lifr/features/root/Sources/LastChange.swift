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
    /// new comment, Settings, what an Avenue shares with apps or adding the
    /// Inbox, and for
    /// Undo itself, which isn't a change to undo in turn.
    package init?(_ saved: Saved, id: UUID) {
        guard saved.change.reverts.isEmpty else { return nil }
        self.init(saved, id: id, message: nil)
    }

    /// What taking back `saved`, an Undo or a Redo, would put back again,
    /// in the words of the change it took back.
    package init?(reversing saved: Saved, id: UUID, message: String) {
        self.init(saved, id: id, message: message)
    }

    private init?(_ saved: Saved, id: UUID, message: String?) {
        let change = saved.change
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
            let words =
                place.before == nil && !change.previous.isEmpty
                ? Self.message(for: change, in: workspace) : Self.message(for: place, in: workspace)
            self.message = message ?? words
            return
        }
        entry = nil
        after = Dictionary(
            uniqueKeysWithValues: change.previous.keys.map {
                (RecordID(.task, $0), workspace.tasks[$0].map(RecordValue.task))
            })
        self.message = message ?? Self.message(for: change, in: workspace)
    }

    /// Whether every record is still as this change left it in `workspace`.
    package func isCurrent(in workspace: Workspace) -> Bool {
        after.allSatisfy { record, value in workspace.value(of: record) == value }
    }

    /// Whether `step` is one Undo offers: not only what an Avenue shares
    /// with apps, which is a setting, changed where it's shown.
    /// Adding the Inbox isn't undoable: Lifr adds it, and it has to stay.
    private static func isUndoable(_ step: JournalStep) -> Bool {
        guard step.before != nil || step.record != RecordID(.avenue, Avenue.inboxID) else { return false }
        return step.fields.contains { $0.field != .agentAccess }
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
        // Several picked tasks changed at once.
        // Sub tasks that went with their parent don't count.
        var changed = Set(change.events.filter { $0.reason == nil }.map(\.taskID))
        // A picked task already in the status it was given is only touched,
        // with no event, but it's in that status as asked.
        if event?.field == .status, let status = event?.newValue {
            let followed = Set(change.events.filter { $0.reason != nil }.map(\.taskID))
            changed.formUnion(
                change.previous.keys.filter { id in
                    !followed.contains(id) && workspace.tasks[id]?.status.rawValue == status
                })
        }
        let picked = changed.filter { id in
            let task = workspace.tasks[id] ?? change.previous[id] ?? nil
            return task?.parentID.map { !changed.contains($0) } ?? true
        }
        if picked.count > 1, !change.events.contains(where: { $0.kind == .created && $0.reason == nil }) {
            return message(forTasks: picked.count, event: event)
        }
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
            guard let status else { return String(localized: "Changed \(title)") }
            // A repeating task closed: when the next one is.
            let next = change.events
                .first { $0.kind == .created && $0.reason == .nextOccurrence }
                .flatMap { workspace.tasks[$0.taskID] }
            if let day = next?.dueDate ?? next?.startDate {
                return String(localized: "\(title) is \(status.title). Next one due \(day.readable())")
            }
            return String(localized: "\(title) is \(status.title)")
        case (.fieldChanged?, .avenue?), (.fieldChanged?, .project?):
            // Where it went: its Project, or its Avenue when it left one.
            let place = task.flatMap { task in
                task.projectID.flatMap { workspace.projects[$0]?.name } ?? workspace.avenues[task.avenueID]?.name
            }
            guard let place else { return String(localized: "Changed \(title)") }
            // In another Avenue it got a new code, which people quote.
            if let task, change.events.contains(where: { $0.taskID == task.id && $0.field == .avenue }) {
                return String(localized: "Moved \(title) to \(place) as \(workspace.displayCode(of: task))")
            }
            return String(localized: "Moved \(title) to \(place)")
        case (.movedToTrash?, _):
            return String(localized: "Moved \(title) to the Trash")
        default:
            return String(localized: "Changed \(title)")
        }
    }

    /// What one change to `count` tasks did, in words.
    private static func message(forTasks count: Int, event: HistoryEvent?) -> String {
        switch (event?.kind, event?.field) {
        case (.fieldChanged?, .status?):
            let status = event?.newValue.flatMap(TaskStatus.init(rawValue:))
            return status.map { String(localized: "\(count) tasks are \($0.title)") }
                ?? String(localized: "Changed \(count) tasks")
        case (.fieldChanged?, .avenue?), (.fieldChanged?, .project?):
            return String(localized: "Moved \(count) tasks")
        case (.movedToTrash?, _):
            return String(localized: "Moved \(count) tasks to the Trash")
        default:
            return String(localized: "Changed \(count) tasks")
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

extension Change.Added {
    /// Whether it's a task, which `previous` brings back, rather than a
    /// record only Undo takes away.
    var isTask: Bool {
        if case .task = self { true } else { false }
    }
}

/// Where a due date shortcut puts the open task.
package enum DueShortcut: Hashable, Sendable {
    case today, tomorrow, none

    func date(now: Date, in timeZone: TimeZone) -> CalendarDate? {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        switch self {
        case .today: return CalendarDate(now, in: timeZone)
        case .tomorrow: return calendar.date(byAdding: .day, value: 1, to: now).map { CalendarDate($0, in: timeZone) }
        case .none: return nil
        }
    }
}
