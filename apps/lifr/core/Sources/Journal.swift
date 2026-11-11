import Foundation
import JournalKit

/// One operation's changes to the records, with who made them.
package typealias JournalEntry = JournalKit.JournalEntry<RecordValue, Actor>
/// One record as it was before and after an operation.
package typealias JournalStep = JournalKit.JournalStep<RecordValue>
/// A field of a record before and after.
package typealias FieldChange = JournalKit.FieldChange<RecordValue>
/// Which record: its kind and ID.
package typealias RecordID = JournalKit.RecordID<RecordKind>
/// A part of an entry that reverting left as it is, and why.
package typealias SkippedRevert = JournalKit.SkippedRevert<RecordValue>
/// A journal entry kept once it's too old to undo: the fields it changed.
package typealias ArchivedEntry = JournalKit.ArchivedEntry<RecordValue, Actor>
/// What one step of an archived entry changed.
package typealias ArchivedStep = JournalKit.ArchivedStep<RecordValue>
/// A month of archived journal entries.
package typealias JournalMonth = JournalKit.JournalMonth

extension JournalEntry {
    /// How long a change can be undone. Older entries are archived as the
    /// fields they changed, and kept.
    package static let undoWindow: TimeInterval = 90 * 24 * 60 * 60
}

extension Workspace {
    /// Takes the journal entries made before `cutoff` out, and returns them
    /// as the fields they changed.
    package mutating func archiveJournal(before cutoff: Date) -> [ArchivedEntry] {
        let (kept, archived) = journal.archiving(before: cutoff)
        journal = kept
        return archived.values.flatMap { $0 }
    }
}

package enum RecordKind: String, Hashable, Codable, Sendable, CaseIterable {
    case avenue, project, task, label, comment, reminder, settings, savedFilter
}

extension JournalKit.RecordID where Kind == RecordKind {
    /// Settings are one record with no ID of their own.
    package static var settings: Self { Self(.settings, UUID(uuid: (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))) }
}

/// Any record, for the journal.
package enum RecordValue: Hashable, Codable, Sendable {
    case avenue(Avenue)
    case project(Project)
    case task(LifrTask)
    case label(Label)
    case comment(Comment)
    case reminder(Reminder)
    case settings(Settings)
    case savedFilter(SavedFilter)

    package var id: RecordID {
        switch self {
        case .avenue(let record): RecordID(.avenue, record.id)
        case .project(let record): RecordID(.project, record.id)
        case .task(let record): RecordID(.task, record.id)
        case .label(let record): RecordID(.label, record.id)
        case .comment(let record): RecordID(.comment, record.id)
        case .reminder(let record): RecordID(.reminder, record.id)
        case .settings: .settings
        case .savedFilter(let record): RecordID(.savedFilter, record.id)
        }
    }
}

// MARK: - Recording

extension Workspace {
    /// Runs `operation` and appends one journal entry for every record it
    /// changed, by `change.actor` when it sets one and otherwise by the
    /// context's actor. Throws, changing nothing, when `operation` does.
    @discardableResult
    package mutating func perform<Failure: Error>(
        in context: Context, _ operation: (inout Workspace, Context) throws(Failure) -> Change
    ) throws(Failure) -> Change {
        let before = self
        var copy = self
        var change = try operation(&copy, context)
        let steps = copy.steps(since: before)
        if !steps.isEmpty {
            let entry = JournalEntry(
                id: context.newID(), actor: change.actor ?? context.actor, at: context.now,
                conversation: change.conversation, reverts: change.reverts, steps: steps)
            copy.journal.append(entry)
            change.entry = entry.id
        }
        self = copy
        return change
    }

    /// Every record that differs from `before`, in a fixed order.
    package func steps(since before: Workspace) -> [JournalStep] {
        func diff<Record: Equatable>(
            _ old: [UUID: Record], _ new: [UUID: Record], _ value: (Record) -> RecordValue
        ) -> [JournalStep] {
            JournalStep.steps(from: old, to: new, as: value)
        }
        var steps = diff(before.avenues, avenues, RecordValue.avenue)
        steps += diff(before.labels, labels, RecordValue.label)
        steps += diff(before.projects, projects, RecordValue.project)
        steps += diff(before.tasks, tasks, RecordValue.task)
        steps += diff(before.comments, comments, RecordValue.comment)
        steps += diff(before.reminders, reminders, RecordValue.reminder)
        steps += diff(before.savedFilters, savedFilters, RecordValue.savedFilter)
        if before.settings != settings {
            steps.append(JournalStep(before: .settings(before.settings), after: .settings(settings)))
        }
        return steps
    }

    package func value(of record: RecordID) -> RecordValue? {
        switch record.kind {
        case .avenue: avenues[record.id].map(RecordValue.avenue)
        case .project: projects[record.id].map(RecordValue.project)
        case .task: tasks[record.id].map(RecordValue.task)
        case .label: labels[record.id].map(RecordValue.label)
        case .comment: comments[record.id].map(RecordValue.comment)
        case .reminder: reminders[record.id].map(RecordValue.reminder)
        case .settings: .settings(settings)
        case .savedFilter: savedFilters[record.id].map(RecordValue.savedFilter)
        }
    }

    mutating func set(_ record: RecordID, to value: RecordValue?) {
        switch (record.kind, value) {
        case (.avenue, .avenue(let avenue)?): avenues[record.id] = avenue
        case (.avenue, nil): avenues[record.id] = nil
        case (.project, .project(let project)?): projects[record.id] = project
        case (.project, nil): projects[record.id] = nil
        case (.task, .task(let task)?): tasks[record.id] = task
        case (.task, nil): tasks[record.id] = nil
        case (.label, .label(let label)?): labels[record.id] = label
        case (.label, nil): labels[record.id] = nil
        case (.comment, .comment(let comment)?): comments[record.id] = comment
        case (.comment, nil): comments[record.id] = nil
        case (.reminder, .reminder(let reminder)?): reminders[record.id] = reminder
        case (.reminder, nil): reminders[record.id] = nil
        case (.settings, .settings(let value)?): settings = value
        case (.savedFilter, .savedFilter(let saved)?): savedFilters[record.id] = saved
        case (.savedFilter, nil): savedFilters[record.id] = nil
        default: break
        }
    }
}

// MARK: - Reverting

extension Workspace {
    /// Puts back what the entries changed, newest first, field by field. A
    /// field changed again since is left as it is and listed in
    /// `change.skipped`. Records the entries added are trashed (tasks and
    /// comments) or removed when nothing uses them; the journal keeps them,
    /// so reverting this revert brings them back. Run it with `perform` so
    /// the revert is an entry of its own.
    package mutating func revert(_ entryIDs: [UUID], in context: Context) throws(LifrError) -> Change {
        let byID = Dictionary(journal.map { ($0.id, $0) }) { first, _ in first }
        for id in entryIDs where byID[id] == nil { throw .notFound(.change, id.uuidString) }
        let entries = entryIDs.compactMap { byID[$0] }.sorted(by: JournalEntry.newestFirst)
        var change = Change()
        change.reverts = entries.map(\.id)
        let before = self
        for entry in entries {
            for step in entry.steps.reversed() {
                revert(step, in: context, &change)
            }
        }
        for (id, old) in before.tasks where tasks[id] != old {
            change.previous[id] = .some(old)
            recordReverted(from: old, to: tasks[id], in: context, &change)
        }
        _ = reconcile(in: context)
        return change
    }

    /// The entries undone: reverted by an entry that isn't undone itself.
    package var undoneEntries: Set<UUID> { journal.undone() }

    /// The entries made since `date`, newest first, for "revert everything
    /// since".
    package func entries(since date: Date) -> [JournalEntry] { journal.made(since: date) }

    /// The entries made in an assistant conversation, newest first.
    package func entries(inConversation conversation: UUID) -> [JournalEntry] {
        journal.made(inConversation: conversation)
    }

    private mutating func revert(_ step: JournalStep, in context: Context, _ change: inout Change) {
        let record = step.record
        switch step.revert(current: value(of: record)) {
        case .nothing:
            return
        case .remove(let current):
            remove(current, in: context, &change)
        case .restore(let before):
            set(record, to: before)
        case .gone:
            change.skipped.append(SkippedRevert(record: record, field: nil, reason: .gone))
        case .update(let merged, let skipped):
            change.skipped += skipped.map { SkippedRevert(record: record, field: $0, reason: .changedSince) }
            if let merged { set(record, to: merged) }
        }
    }

    /// Takes out a record that an entry added: tasks and comments go to the
    /// trash, the rest are removed unless something uses them.
    private mutating func remove(_ value: RecordValue, in context: Context, _ change: inout Change) {
        switch value {
        case .task(var task):
            guard task.deletedAt == nil else { return }
            task.deletedAt = context.now
            tasks[task.id] = task
        case .comment(var comment):
            guard comment.deletedAt == nil else { return }
            comment.deletedAt = context.now
            comments[comment.id] = comment
        case .reminder(let reminder):
            reminders[reminder.id] = nil
        case .savedFilter(let saved):
            savedFilters[saved.id] = nil
        case .label(let label):
            if tasks.values.contains(where: { $0.labelIDs.contains(label.id) && $0.deletedAt == nil }) {
                change.skipped.append(SkippedRevert(record: value.id, field: nil, reason: .inUse))
            } else {
                for (id, task) in tasks where task.labelIDs.contains(label.id) {
                    tasks[id]?.labelIDs.remove(label.id)
                }
                labels[label.id] = nil
            }
        case .project(let project):
            if tasks.values.contains(where: { $0.projectID == project.id && $0.deletedAt == nil }) {
                change.skipped.append(SkippedRevert(record: value.id, field: nil, reason: .inUse))
            } else {
                for (id, task) in tasks where task.projectID == project.id { tasks[id]?.projectID = nil }
                projects[project.id] = nil
            }
        case .avenue(let avenue):
            if tasks.values.contains(where: { $0.avenueID == avenue.id })
                || projects.values.contains(where: { $0.avenueID == avenue.id })
            {
                change.skipped.append(SkippedRevert(record: value.id, field: nil, reason: .inUse))
            } else {
                avenues[avenue.id] = nil
            }
        case .settings:
            break
        }
    }

    /// Bumps a reverted task's activity and writes its history, so the
    /// task's timeline shows the revert as an undo.
    private mutating func recordReverted(
        from old: LifrTask, to new: LifrTask?, in context: Context, _ change: inout Change
    ) {
        guard var new else { return }
        new.updatedAt = context.now
        new.lastActivityAt = context.now
        tasks[new.id] = new
        if old.deletedAt == nil, new.deletedAt != nil {
            record(.movedToTrash, on: new.id, reason: .undo, by: context.actor, in: context, &change)
        } else if old.deletedAt != nil, new.deletedAt == nil {
            record(.restored, on: new.id, reason: .undo, by: context.actor, in: context, &change)
        }
        let fields: [(HistoryEvent.Field, String?, String?)] = [
            (.title, old.title, new.title),
            (.details, old.details, new.details),
            (.status, old.status.rawValue, new.status.rawValue),
            (.priority, old.priority.rawValue, new.priority.rawValue),
            (.startDate, old.startDate?.description, new.startDate?.description),
            (.dueDate, old.dueDate?.description, new.dueDate?.description),
            (.dueTime, old.dueTime?.description, new.dueTime?.description),
            (.labels, Self.describe(old.labelIDs), Self.describe(new.labelIDs)),
            (.avenue, avenues[old.avenueID]?.key.rawValue, avenues[new.avenueID]?.key.rawValue),
            (.project, old.projectID.flatMap { projects[$0]?.name }, new.projectID.flatMap { projects[$0]?.name }),
            (
                .parent, old.parentID.flatMap { tasks[$0]?.code.rawValue },
                new.parentID.flatMap { tasks[$0]?.code.rawValue }
            ),
        ]
        for (field, oldValue, newValue) in fields where oldValue != newValue {
            record(
                .fieldChanged, on: new.id, field: field, oldValue: oldValue, newValue: newValue, reason: .undo,
                by: context.actor, in: context, &change)
        }
    }
}
