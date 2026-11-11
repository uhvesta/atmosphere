import Foundation

/// One change made to several tasks at once.
package enum BulkEdit: Hashable, Sendable {
    case status(TaskStatus)
    case priority(Priority)
    /// A due date, or none.
    case dueDate(CalendarDate?)
    /// Adds the label to each task that doesn't have it.
    case addLabel(UUID)
    /// Moves each top-level task, with its Sub tasks.
    case move(avenue: UUID, project: UUID?)
    /// Moves each task, with its Sub tasks, to the trash.
    case trash
}

extension Workspace {
    /// Makes `edit` to every task in `ids` as one change, so one Undo puts
    /// them all back. All or nothing: when Core refuses one task, none
    /// change. A Sub task picked with its parent goes with the parent when
    /// moved or trashed. `cascade` closes open Sub tasks as Won't Do, as
    /// `setStatus` does once the person agrees.
    package mutating func apply(
        _ edit: BulkEdit, to ids: [UUID], cascade: Bool = false, in context: Context
    ) throws(LifrError) -> Change {
        let picked = Set(ids)
        var tasks: [LifrTask] = []
        for id in ids {
            tasks.append(try liveTask(id))
        }
        // Sub tasks first, so a parent closed with them sees them closed.
        let ordered = tasks.filter(\.isSubtask) + tasks.filter { !$0.isSubtask }
        var change = Change()
        for task in ordered {
            let followsParent = task.parentID.map(picked.contains) ?? false
            switch edit {
            case .status(let status):
                change.merge(try setStatus(status, of: task.id, cascade: cascade, in: context))
            case .priority(let priority):
                change.merge(try self.edit(task.id, TaskEdit(priority: priority), in: context))
            case .dueDate(let date):
                change.merge(try self.edit(task.id, TaskEdit(dueDate: .some(date)), in: context))
            case .addLabel(let label):
                guard labels[label] != nil else { throw .notFound(.label, label.uuidString) }
                guard !task.labelIDs.contains(label) else { continue }
                change.merge(try self.edit(task.id, TaskEdit(labelIDs: task.labelIDs.union([label])), in: context))
            case .move(let avenue, let project):
                guard !followsParent else { continue }
                change.merge(try move(task.id, toAvenue: avenue, project: project, in: context))
            case .trash:
                guard !followsParent else { continue }
                change.merge(try moveToTrash(task.id, in: context))
            }
        }
        return change
    }
}
