import Foundation
import LifrCore
import LifrUI

/// One line of a task's activity: a comment or a history event.
package struct TimelineEntry: Identifiable, Hashable, Sendable {
    package var id: UUID
    /// Such as "You on iPhone" or "Claude Code (agent)".
    package var author: String
    package var date: Date
    package var text: String
    package var isComment: Bool
    /// Whether Undo can put back what this change did.
    package var canUndo = false
}

extension Actor {
    /// Who made a change, as the timeline shows it.
    package var displayName: String {
        switch kind {
        case .user: "You on \(device)"
        case .agent: "\(client ?? "An agent") (agent)"
        case .system: "Lifr on \(device)"
        }
    }
}

extension Workspace {
    /// The task's comments and history, newest first.
    package func timeline(of id: UUID) -> [TimelineEntry] {
        let comments = comments.values
            .filter { $0.taskID == id && $0.deletedAt == nil }
            .map {
                TimelineEntry(
                    id: $0.id, author: $0.author.displayName, date: $0.createdAt, text: $0.body, isComment: true)
            }
        let events =
            history
            .filter { $0.taskID == id && ![.commentAdded, .commentEdited, .commentDeleted].contains($0.kind) }
            .map {
                TimelineEntry(
                    id: $0.id, author: $0.actor.displayName, date: $0.createdAt, text: describe($0), isComment: false,
                    canUndo: canRevert($0))
            }
        return (comments + events).sorted { ($0.date, $0.id.uuidString) > ($1.date, $1.id.uuidString) }
    }

    /// "Read the draft (WORK-0007)" for a Sub task's code, or the code alone
    /// if that task is gone.
    private func subtask(_ code: String?) -> String {
        guard let code else { return "" }
        guard let task = tasks.values.first(where: { $0.code.rawValue == code }) else { return code }
        return "\"\(task.title)\" (\(displayCode(of: task)))"
    }

    private func describe(_ event: HistoryEvent) -> String {
        func status(_ raw: String?) -> String { raw.flatMap(TaskStatus.init(rawValue:))?.title ?? "none" }
        let change: String =
            switch (event.kind, event.field) {
            case (.created, _): "Created"
            case (.fieldChanged, .status?): "\(status(event.oldValue)) -> \(status(event.newValue))"
            case (.fieldChanged, let field?):
                "\(field.rawValue): \(event.oldValue ?? "none") -> \(event.newValue ?? "none")"
            case (.subtaskAdded, _): "Added Sub task \(subtask(event.newValue))"
            case (.subtaskRemoved, _): "Removed Sub task \(subtask(event.newValue))"
            case (.movedToTrash, _): "Moved to the trash"
            case (.restored, _): "Restored from the trash"
            case (.fieldChanged, nil), (.commentAdded, _), (.commentEdited, _), (.commentDeleted, _):
                event.kind.rawValue
            }
        return event.reason.map { "\(change) (\($0.explanation))" } ?? change
    }
}

extension HistoryEvent.Reason {
    var explanation: String {
        switch self {
        case .closedWithParent: "closed with its parent"
        case .subtaskOpenedUnderClosedParent: "a Sub task is open"
        case .movedWithParent: "moved with its parent"
        case .undo: "undone"
        case .reconciledClosedParent: "reopened after sync: a Sub task is open"
        case .reconciledNestedSubtask: "moved after sync: Sub tasks can't nest"
        case .reconciledDuplicateCode: "new code after sync: another task had it"
        case .reconciledParentPlacement: "moved after sync to its parent's Avenue"
        }
    }
}
