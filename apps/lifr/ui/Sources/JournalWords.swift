import ChatUI
import Foundation
import LifrCore

/// Changes to the records in words, for the Activity log and for what the
/// assistant asks to change: records by name, values as people read them.
package struct JournalWords {
    /// The records names are taken from.
    package var workspace: Workspace
    /// Who an actor is, for a field that names one.
    package var author: (Actor) -> String

    package init(_ workspace: Workspace, author: @escaping (Actor) -> String) {
        self.workspace = workspace
        self.author = author
    }

    /// A field of a record, before and after, in words.
    package typealias FieldWords = ChatUI.ChangedField

    /// Every field the steps change, record by record, but `hiding`. Leaves
    /// out the order, and the task a comment or reminder is on, which the
    /// record's name already says.
    package func fields(of steps: [JournalStep], hiding: Set<Field> = []) -> [FieldWords] {
        let hidden = Self.hidden.union(hiding)
        return steps.flatMap { step in
            let record = name(step)
            return step.fields.filter { !hidden.contains($0.field) }.map { change in
                FieldWords(
                    record: record, field: change.field.title, before: change.before.map(text),
                    after: change.after.map(text))
            }
        }
    }

    private static let hidden: Set<Field> = [.sortOrder, .task]

    /// What the steps did in a line, such as "Changed task “Run” and 2
    /// more", or that they revert `reverts` changes.
    package func summary(of steps: [JournalStep], reverts: Int = 0) -> String {
        if reverts > 0 {
            return reverts == 1
                ? String(localized: "Reverted a change") : String(localized: "Reverted \(reverts) changes")
        }
        guard let first = steps.first else { return String(localized: "No change") }
        var text = "\(verb(first)) \(first.record.kind.noun)"
        if first.record.kind != .settings { text += " “\(title(first))”" }
        return steps.count == 1 ? text : text + " and \(steps.count - 1) more"
    }

    private func verb(_ step: JournalStep) -> String {
        switch (step.before, step.after) {
        case (nil, _): return String(localized: "Added")
        case (_, nil): return String(localized: "Removed")
        case (.task(let old)?, .task(let new)?):
            if old.deletedAt == nil, new.deletedAt != nil { return String(localized: "Trashed") }
            if old.deletedAt != nil, new.deletedAt == nil { return String(localized: "Restored") }
            return String(localized: "Changed")
        case (.comment(let old)?, .comment(let new)?) where old.deletedAt == nil && new.deletedAt != nil:
            return String(localized: "Deleted")
        default: return String(localized: "Changed")
        }
    }

    private func title(_ step: JournalStep) -> String {
        switch step.after ?? step.before {
        case .avenue(let avenue): avenue.name
        case .project(let project): project.name
        case .task(let task): task.title
        case .label(let label): label.name
        case .comment(let comment): workspace.tasks[comment.taskID]?.title ?? "a task"
        case .reminder(let reminder): workspace.tasks[reminder.taskID]?.title ?? "a task"
        case .settings, nil: String(localized: "Settings")
        }
    }

    private func name(_ step: JournalStep) -> String {
        step.record.kind == .settings ? String(localized: "Settings") : title(step)
    }

    // MARK: - Archived

    /// `fields(of:hiding:)` for an archived entry's steps.
    package func fields(of steps: [ArchivedStep], hiding: Set<Field> = []) -> [FieldWords] {
        let hidden = Self.hidden.union(hiding)
        return steps.flatMap { step in
            let record = name(step)
            return step.fields.filter { !hidden.contains($0.field) }.map { change in
                FieldWords(
                    record: record, field: change.field.title, before: change.before.map(text),
                    after: change.after.map(text))
            }
        }
    }

    /// `summary(of:reverts:)` for an archived entry's steps.
    package func summary(of steps: [ArchivedStep], reverts: Int = 0) -> String {
        if reverts > 0 {
            return reverts == 1
                ? String(localized: "Reverted a change") : String(localized: "Reverted \(reverts) changes")
        }
        guard let first = steps.first else { return String(localized: "No change") }
        var text = "\(verb(first)) \(first.record.kind.noun)"
        if first.record.kind != .settings { text += " “\(name(first))”" }
        return steps.count == 1 ? text : text + " and \(steps.count - 1) more"
    }

    private func verb(_ step: ArchivedStep) -> String {
        switch step.change {
        case .added: return String(localized: "Added")
        case .removed: return String(localized: "Removed")
        case .changed:
            guard let deleted = step.fields.first(where: { $0.field == .deletedAt }) else {
                return String(localized: "Changed")
            }
            switch (deleted.before, deleted.after, step.record.kind) {
            case (nil, _?, .comment): return String(localized: "Deleted")
            case (nil, _?, _): return String(localized: "Trashed")
            case (_?, nil, _): return String(localized: "Restored")
            default: return String(localized: "Changed")
            }
        }
    }

    /// The name the step gave the record, or else its name now.
    private func name(_ step: ArchivedStep) -> String {
        let id = step.record.id
        switch step.record.kind {
        case .settings: return String(localized: "Settings")
        case .comment, .reminder:
            let task = step.fields.first { $0.field == .task }.flatMap { $0.after ?? $0.before }
            guard case .record(_, let taskID)? = task, let title = workspace.tasks[taskID]?.title else {
                return "a task"
            }
            return title
        case .avenue, .project, .task, .label:
            let named = step.fields.first { $0.field == .title || $0.field == .name }
            guard case .text(let text)? = named.flatMap({ $0.after ?? $0.before }) else {
                return name(of: step.record.kind, id)
            }
            return text
        }
    }

    /// A value as people read it: records by name, times as dates.
    private func text(_ value: FieldValue) -> String {
        switch value {
        case .text(let text): text
        case .number(let number): String(number)
        case .day(let day): day.description
        case .time(let time): time.description
        case .moment(let date): date.formatted(date: .abbreviated, time: .shortened)
        case .taskStatus(let status): status.title
        case .projectStatus(let status): status.rawValue.capitalized
        case .priority(let priority): priority.title ?? String(localized: "None")
        case .access(let access): access.title
        case .record(let kind, let id): name(of: kind, id)
        case .records(let kind, let ids): ids.map { name(of: kind, $0) }.sorted().joined(separator: ", ")
        case .actor(let actor): author(actor)
        case .trigger(.at(let date)): date.formatted(date: .abbreviated, time: .shortened)
        case .trigger(.beforeDue(let seconds)):
            Duration.seconds(seconds).formatted(.units(allowed: [.days, .hours, .minutes], width: .wide))
                + " before due"
        }
    }

    private func name(of kind: RecordKind, _ id: UUID) -> String {
        let name: String? =
            switch kind {
            case .avenue: workspace.avenues[id]?.name
            case .project: workspace.projects[id]?.name
            case .task: workspace.tasks[id]?.title
            case .label: workspace.labels[id]?.name
            case .comment, .reminder, .settings: nil
            }
        return name ?? "something removed"
    }
}

extension RecordKind {
    /// The record's kind in a sentence.
    fileprivate var noun: String {
        switch self {
        case .avenue: String(localized: "Avenue")
        case .project: String(localized: "Project")
        case .task: "task"
        case .label: "label"
        case .comment: "comment"
        case .reminder: "reminder"
        case .settings: "settings"
        }
    }
}
