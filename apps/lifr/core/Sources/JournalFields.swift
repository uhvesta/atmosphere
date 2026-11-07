import Foundation
import JournalKit

/// A field the journal compares and reverts. The times every change bumps
/// (`updatedAt`, `lastActivityAt`) aren't fields: they never count as
/// changed. Neither are IDs and creation times, which never change.
package enum Field: String, Hashable, Codable, Sendable, CaseIterable {
    case key, name, title, details, body, color, symbol, code
    case status, priority, agentAccess
    case startDate, dueDate, dueTime
    case avenue, project, parent, task, labels
    case author, trigger, procrastinatedAfterDays
    case sortOrder, archivedAt, completedAt, closedAt, editedAt, deletedAt

    /// The field's name for people.
    package var title: String {
        switch self {
        case .key: "Key"
        case .name: "Name"
        case .title: "Title"
        case .details: "Details"
        case .body: "Comment"
        case .color: "Color"
        case .symbol: "Symbol"
        case .code: "Code"
        case .status: "Status"
        case .priority: "Priority"
        case .agentAccess: "Agent access"
        case .startDate: "Start date"
        case .dueDate: "Due date"
        case .dueTime: "Due time"
        case .avenue: "Avenue"
        case .project: "Project"
        case .parent: "Parent"
        case .task: "Task"
        case .labels: "Labels"
        case .author: "Author"
        case .trigger: "Reminder"
        case .procrastinatedAfterDays: "Procrastinated after"
        case .sortOrder: "Order"
        case .archivedAt: "Archived"
        case .completedAt: "Completed"
        case .closedAt: "Closed"
        case .editedAt: "Edited"
        case .deletedAt: "Deleted"
        }
    }

    /// Bookkeeping people don't need to see in a change.
    package var isInternal: Bool { self == .sortOrder }
}

/// A field's value, typed, so it can be shown and compared without parsing.
package enum FieldValue: Hashable, Codable, Sendable {
    case text(String)
    case number(Int)
    case day(CalendarDate)
    case time(TimeOfDay)
    case moment(Date)
    case taskStatus(TaskStatus)
    case projectStatus(ProjectStatus)
    case priority(Priority)
    case access(AgentAccess)
    /// Another record, such as a task's Avenue.
    case record(RecordKind, UUID)
    /// Other records, sorted, such as a task's labels.
    case records(RecordKind, [UUID])
    case actor(Actor)
    case trigger(Reminder.Trigger)
}

/// One field of a record: how to read it as a value and how to copy it
/// from another version of the record.
package struct RecordField<Record>: Sendable {
    package let field: Field
    let value: @Sendable (Record) -> FieldValue?
    let copy: @Sendable (_ from: Record, _ to: inout Record) -> Void

    init<Value: Equatable & Sendable>(
        _ field: Field, _ keyPath: WritableKeyPath<Record, Value> & Sendable,
        _ show: @escaping @Sendable (Value) -> FieldValue?
    ) {
        self.field = field
        value = { show($0[keyPath: keyPath]) }
        copy = { from, to in to[keyPath: keyPath] = from[keyPath: keyPath] }
    }

    init<Value: Equatable & Sendable>(
        _ field: Field, _ keyPath: WritableKeyPath<Record, Value?> & Sendable,
        _ show: @escaping @Sendable (Value) -> FieldValue
    ) {
        self.init(field, keyPath) { (value: Value?) in value.map(show) }
    }
}

extension Avenue {
    static let fields: [RecordField<Avenue>] = [
        .init(.key, \.key) { .text($0.rawValue) },
        .init(.name, \.name) { .text($0) },
        .init(.color, \.color) { .text($0) },
        .init(.symbol, \.symbol) { .text($0) },
        .init(.sortOrder, \.sortOrder) { .text($0) },
        .init(.agentAccess, \.agentAccess) { .access($0) },
        .init(.archivedAt, \.archivedAt) { .moment($0) },
    ]
}

extension Project {
    static let fields: [RecordField<Project>] = [
        .init(.avenue, \.avenueID) { .record(.avenue, $0) },
        .init(.name, \.name) { .text($0) },
        .init(.details, \.details) { .text($0) },
        .init(.status, \.status) { .projectStatus($0) },
        .init(.dueDate, \.dueDate) { .day($0) },
        .init(.sortOrder, \.sortOrder) { .text($0) },
        .init(.completedAt, \.completedAt) { .moment($0) },
    ]
}

extension LifrTask {
    static let fields: [RecordField<LifrTask>] = [
        .init(.code, \.code) { .text($0.rawValue) },
        .init(.title, \.title) { .text($0) },
        .init(.details, \.details) { .text($0) },
        .init(.status, \.status) { .taskStatus($0) },
        .init(.priority, \.priority) { .priority($0) },
        .init(.startDate, \.startDate) { .day($0) },
        .init(.dueDate, \.dueDate) { .day($0) },
        .init(.dueTime, \.dueTime) { .time($0) },
        .init(.avenue, \.avenueID) { .record(.avenue, $0) },
        .init(.project, \.projectID) { .record(.project, $0) },
        .init(.parent, \.parentID) { .record(.task, $0) },
        .init(.labels, \.labelIDs) { .records(.label, $0.sorted { $0.uuidString < $1.uuidString }) },
        .init(.sortOrder, \.sortOrder) { .text($0) },
        .init(.closedAt, \.closedAt) { .moment($0) },
        .init(.deletedAt, \.deletedAt) { .moment($0) },
    ]
}

extension Label {
    static let fields: [RecordField<Label>] = [
        .init(.name, \.name) { .text($0) },
        .init(.color, \.color) { .text($0) },
    ]
}

extension Comment {
    static let fields: [RecordField<Comment>] = [
        .init(.task, \.taskID) { .record(.task, $0) },
        .init(.body, \.body) { .text($0) },
        .init(.author, \.author) { .actor($0) },
        .init(.editedAt, \.editedAt) { .moment($0) },
        .init(.deletedAt, \.deletedAt) { .moment($0) },
    ]
}

extension Reminder {
    static let fields: [RecordField<Reminder>] = [
        .init(.task, \.taskID) { .record(.task, $0) },
        .init(.trigger, \.trigger) { .trigger($0) },
    ]
}

extension Settings {
    static let fields: [RecordField<Settings>] = [
        .init(.procrastinatedAfterDays, \.procrastinatedAfterDays) { .number($0) }
    ]
}

extension RecordValue: Journaled {
    /// The record's fields that have a value.
    package var fieldValues: [Field: FieldValue] {
        func values<Record>(_ fields: [RecordField<Record>], _ record: Record) -> [Field: FieldValue] {
            Dictionary(
                uniqueKeysWithValues: fields.compactMap { field in field.value(record).map { (field.field, $0) } })
        }
        return switch self {
        case .avenue(let record): values(Avenue.fields, record)
        case .project(let record): values(Project.fields, record)
        case .task(let record): values(LifrTask.fields, record)
        case .label(let record): values(Label.fields, record)
        case .comment(let record): values(Comment.fields, record)
        case .reminder(let record): values(Reminder.fields, record)
        case .settings(let record): values(Settings.fields, record)
        }
    }

    /// The fields of `kind`'s records, in the order people see them.
    package static func fields(of kind: RecordKind) -> [Field] {
        switch kind {
        case .avenue: Avenue.fields.map(\.field)
        case .project: Project.fields.map(\.field)
        case .task: LifrTask.fields.map(\.field)
        case .label: Label.fields.map(\.field)
        case .comment: Comment.fields.map(\.field)
        case .reminder: Reminder.fields.map(\.field)
        case .settings: Settings.fields.map(\.field)
        }
    }

    /// This record with `field` copied from `other`, a version of the
    /// same record.
    package func copying(_ field: Field, from other: RecordValue) -> RecordValue {
        func copy<Record>(_ fields: [RecordField<Record>], _ from: Record, _ to: Record) -> Record {
            var to = to
            fields.first { $0.field == field }?.copy(from, &to)
            return to
        }
        return switch (self, other) {
        case (.avenue(let to), .avenue(let from)): .avenue(copy(Avenue.fields, from, to))
        case (.project(let to), .project(let from)): .project(copy(Project.fields, from, to))
        case (.task(let to), .task(let from)): .task(copy(LifrTask.fields, from, to))
        case (.label(let to), .label(let from)): .label(copy(Label.fields, from, to))
        case (.comment(let to), .comment(let from)): .comment(copy(Comment.fields, from, to))
        case (.reminder(let to), .reminder(let from)): .reminder(copy(Reminder.fields, from, to))
        case (.settings(let to), .settings(let from)): .settings(copy(Settings.fields, from, to))
        default: self
        }
    }
}
