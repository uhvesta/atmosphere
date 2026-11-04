import Foundation

/// One operation's changes to the records, with who made them. Written once,
/// never changed: reverting an entry appends another, so it can be reverted
/// too.
package struct JournalEntry: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    package var actor: Actor
    package var at: Date
    /// The assistant conversation the change was made in.
    package var conversation: UUID?
    /// The entries this one reverts, newest first.
    package var reverts: [UUID]
    package var steps: [JournalStep]

    package init(
        id: UUID, actor: Actor, at: Date, conversation: UUID? = nil, reverts: [UUID] = [], steps: [JournalStep]
    ) {
        self.id = id
        self.actor = actor
        self.at = at
        self.conversation = conversation
        self.reverts = reverts
        self.steps = steps
    }
}

/// One record as it was before and after an operation. `before` is nil for
/// a record the operation added, `after` for one it removed.
package struct JournalStep: Hashable, Codable, Sendable {
    package var before: RecordValue?
    package var after: RecordValue?

    package init(before: RecordValue?, after: RecordValue?) {
        precondition(before != nil || after != nil, "A step needs the record before or after")
        self.before = before
        self.after = after
    }

    package var record: RecordID { after?.id ?? before?.id ?? .settings }

    /// The fields that differ, by their names in Core's records, without the
    /// times every change bumps.
    package var fields: [FieldChange] {
        let old = before.map(Fields.init) ?? Fields()
        let new = after.map(Fields.init) ?? Fields()
        return Set(old.values.keys).union(new.values.keys).sorted().compactMap { name in
            guard !Fields.bumped.contains(name), !Fields.same(old[name], new[name]) else { return nil }
            return FieldChange(name: name, before: Fields.text(old[name]), after: Fields.text(new[name]))
        }
    }
}

/// A field of a record before and after, as text.
package struct FieldChange: Hashable, Sendable {
    package var name: String
    package var before: String?
    package var after: String?
    package init(name: String, before: String?, after: String?) {
        self.name = name
        self.before = before
        self.after = after
    }
}

package enum RecordKind: String, Hashable, Codable, Sendable, CaseIterable {
    case avenue, project, task, label, comment, reminder, settings
}

package struct RecordID: Hashable, Codable, Sendable {
    package var kind: RecordKind
    package var id: UUID

    package init(_ kind: RecordKind, _ id: UUID) {
        self.kind = kind
        self.id = id
    }

    /// Settings are one record with no ID of their own.
    package static let settings = RecordID(.settings, UUID(uuid: (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)))
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

    package var id: RecordID {
        switch self {
        case .avenue(let record): RecordID(.avenue, record.id)
        case .project(let record): RecordID(.project, record.id)
        case .task(let record): RecordID(.task, record.id)
        case .label(let record): RecordID(.label, record.id)
        case .comment(let record): RecordID(.comment, record.id)
        case .reminder(let record): RecordID(.reminder, record.id)
        case .settings: .settings
        }
    }
}

/// A part of an entry that reverting left as it is, and why.
package struct SkippedRevert: Hashable, Sendable {
    package enum Reason: Hashable, Sendable {
        /// The field was changed again since, so reverting would overwrite that.
        case changedSince
        /// The record is gone, such as a task purged from the trash.
        case gone
        /// The record would be removed, but other records still use it.
        case inUse
    }

    package var record: RecordID
    /// The field, when only one was skipped.
    package var field: String?
    package var reason: Reason
    package init(record: RecordID, field: String?, reason: Reason) {
        self.record = record
        self.field = field
        self.reason = reason
    }
}

// MARK: - Recording

extension Workspace {
    /// Runs `operation` and appends one journal entry for every record it
    /// changed, by `change.actor` when it sets one and otherwise by the
    /// context's actor. Throws, changing nothing, when `operation` does.
    package mutating func perform(
        in context: Context, _ operation: (inout Workspace, Context) throws(LifrError) -> Change
    ) throws(LifrError) -> Change {
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
    func steps(since before: Workspace) -> [JournalStep] {
        func diff<Record: Equatable>(
            _ old: [UUID: Record], _ new: [UUID: Record], _ value: (Record) -> RecordValue
        ) -> [JournalStep] {
            Set(old.keys).union(new.keys).sorted { $0.uuidString < $1.uuidString }.compactMap { id in
                guard old[id] != new[id] else { return nil }
                return JournalStep(before: old[id].map(value), after: new[id].map(value))
            }
        }
        var steps = diff(before.avenues, avenues, RecordValue.avenue)
        steps += diff(before.labels, labels, RecordValue.label)
        steps += diff(before.projects, projects, RecordValue.project)
        steps += diff(before.tasks, tasks, RecordValue.task)
        steps += diff(before.comments, comments, RecordValue.comment)
        steps += diff(before.reminders, reminders, RecordValue.reminder)
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
        }
    }

    private mutating func set(_ record: RecordID, to value: RecordValue?) {
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
        let entries = entryIDs.compactMap { byID[$0] }.sorted { ($0.at, $0.id.uuidString) > ($1.at, $1.id.uuidString) }
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

    /// The entries made since `date`, newest first, for "revert everything
    /// since".
    package func entries(since date: Date) -> [JournalEntry] {
        journal.filter { $0.at >= date }.sorted { ($0.at, $0.id.uuidString) > ($1.at, $1.id.uuidString) }
    }

    /// The entries made in an assistant conversation, newest first.
    package func entries(inConversation conversation: UUID) -> [JournalEntry] {
        journal.filter { $0.conversation == conversation }
            .sorted { ($0.at, $0.id.uuidString) > ($1.at, $1.id.uuidString) }
    }

    private mutating func revert(_ step: JournalStep, in context: Context, _ change: inout Change) {
        let record = step.record
        let current = value(of: record)
        switch (step.before, step.after, current) {
        case (nil, _, nil):
            return  // Added, and already gone.
        case (nil, _, let current?):
            remove(current, in: context, &change)
        case (let before?, nil, nil):
            set(record, to: before)  // Removed by the entry: put it back.
        case (_?, _, nil):
            change.skipped.append(SkippedRevert(record: record, field: nil, reason: .gone))
        case (let before?, let after, let current?):
            let merged = Self.merge(
                current: current, from: after.map(Fields.init) ?? Fields(), to: Fields(before), record: record,
                &change)
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

    /// `current` with each field that went from `from` to `to` put back to
    /// `to`, unless it has changed since. `nil` when nothing changes.
    private static func merge(
        current: RecordValue, from: Fields, to: Fields, record: RecordID, _ change: inout Change
    ) -> RecordValue? {
        var fields = Fields(current)
        var changed = false
        for name in Set(from.values.keys).union(to.values.keys).sorted() where !Fields.bumped.contains(name) {
            guard !Fields.same(from[name], to[name]) else { continue }
            if Fields.same(fields[name], to[name]) { continue }
            guard Fields.same(fields[name], from[name]) else {
                change.skipped.append(SkippedRevert(record: record, field: name, reason: .changedSince))
                continue
            }
            fields.values[name] = to[name]
            changed = true
        }
        return changed ? fields.record(like: current) : nil
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

// MARK: - Fields

/// A record's fields as JSON values, for comparing and merging any kind of
/// record the same way.
private struct Fields {
    /// Every change sets these, so they never count as changed.
    static let bumped: Set<String> = ["updatedAt", "lastActivityAt"]

    var values: [String: Any] = [:]

    init() {}

    init(_ value: RecordValue) {
        values = (try? Self.object(value)) ?? [:]
    }

    subscript(name: String) -> Any? {
        values[name]
    }

    func record(like value: RecordValue) -> RecordValue? {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            let text = try decoder.singleValueContainer().decode(String.self)
            guard let seconds = Double(text) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: text))
            }
            return Date(timeIntervalSinceReferenceDate: seconds)
        }
        guard let data = try? JSONSerialization.data(withJSONObject: values) else { return nil }
        return switch value {
        case .avenue: (try? decoder.decode(Avenue.self, from: data)).map(RecordValue.avenue)
        case .project: (try? decoder.decode(Project.self, from: data)).map(RecordValue.project)
        case .task: (try? decoder.decode(LifrTask.self, from: data)).map(RecordValue.task)
        case .label: (try? decoder.decode(Label.self, from: data)).map(RecordValue.label)
        case .comment: (try? decoder.decode(Comment.self, from: data)).map(RecordValue.comment)
        case .reminder: (try? decoder.decode(Reminder.self, from: data)).map(RecordValue.reminder)
        case .settings: (try? decoder.decode(Settings.self, from: data)).map(RecordValue.settings)
        }
    }

    private static func object(_ value: RecordValue) throws -> [String: Any] {
        let encoder = JSONEncoder()
        // Text, so dates come back exactly.
        encoder.dateEncodingStrategy = .custom { date, encoder in
            var container = encoder.singleValueContainer()
            try container.encode(String(date.timeIntervalSinceReferenceDate))
        }
        let data =
            switch value {
            case .avenue(let record): try encoder.encode(record)
            case .project(let record): try encoder.encode(record)
            case .task(let record): try encoder.encode(record)
            case .label(let record): try encoder.encode(record)
            case .comment(let record): try encoder.encode(record)
            case .reminder(let record): try encoder.encode(record)
            case .settings(let record): try encoder.encode(record)
            }
        return try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
    }

    /// Equal JSON values. A missing field is null, and lists compare in any
    /// order, since some are sets.
    static func same(_ lhs: Any?, _ rhs: Any?) -> Bool {
        normalized(lhs) == normalized(rhs)
    }

    /// A field as text for people: lists joined with commas, other
    /// structures as JSON. `nil` for null.
    static func text(_ value: Any?) -> String? {
        switch value {
        case nil, is NSNull: nil
        case let string as String: string
        case let list as [Any]: list.compactMap { text($0) }.sorted().joined(separator: ", ")
        case let number as NSNumber: number.stringValue
        case let value?:
            (try? JSONSerialization.data(withJSONObject: value, options: .sortedKeys))
                .flatMap { String(data: $0, encoding: .utf8) }
        }
    }

    private static func normalized(_ value: Any?) -> String {
        switch value {
        case nil, is NSNull: "null"
        case let list as [Any]: "[" + list.map { normalized($0) }.sorted().joined(separator: ",") + "]"
        case let object as [String: Any]:
            "{" + object.keys.sorted().map { "\($0):\(normalized(object[$0]))" }.joined(separator: ",") + "}"
        case let string as String: "s" + string
        case let number as NSNumber: "n" + number.stringValue
        default: "?"
        }
    }
}
