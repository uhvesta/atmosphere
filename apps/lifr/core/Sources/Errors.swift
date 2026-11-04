import Foundation

/// The kinds of record a lookup can fail to find.
package enum Entity: String, Hashable, Codable, Sendable {
    case avenue, project, task, label, comment, reminder
}

/// Why Core refused a change. Every client shows `description` as is, so each
/// message says what failed and how to fix it.
package enum LifrError: Error, Hashable, Sendable, CustomStringConvertible {
    case invalidTitle(length: Int)
    case invalidAvenueKey(String)
    case avenueKeyTaken(AvenueKey)
    case invalidTaskCode(String)
    case invalidDate(String)
    case invalidTime(String)
    case dueTimeWithoutDueDate
    case notFound(Entity, String)
    case projectInDifferentAvenue(project: UUID, avenue: UUID)
    /// The would-be parent is itself a Sub task, and only one level is allowed.
    case parentIsSubtask(UUID)
    /// A Task with Sub tasks can't become a Sub task.
    case taskHasSubtasks(UUID)
    /// A Sub task always has its parent's Avenue and Project.
    case subtaskFollowsParent(UUID)
    case cannotParentToItself(UUID)
    case openSubtasks(task: UUID, open: [UUID])
    /// Won't Do would also close these Sub tasks; retry with `cascade: true`.
    case wontDoNeedsCascade(task: UUID, open: [UUID])
    case projectHasOpenTasks(project: UUID, open: [UUID])
    case emptyComment
    case notCommentAuthor(comment: UUID)
    case inTrash(UUID)
    case reminderNeedsDueDate(UUID)
    /// History can't put this kind of change back, such as a move.
    case cannotRevert(event: UUID)
    /// The field changed again after the event, so reverting would lose that.
    case changedSince(event: UUID)

    package var description: String {
        switch self {
        case .invalidTitle(let length):
            "A title must be 1 to 200 characters; this one has \(length). Shorten it, or move the rest into the "
                + "details."
        case .invalidAvenueKey(let key):
            "\"\(key)\" isn't a valid Avenue key. Use 2 to 10 capital letters, such as HEALTH."
        case .avenueKeyTaken(let key):
            "Another Avenue already uses the key \(key). Pick a different key, or rename the other Avenue first."
        case .invalidTaskCode(let code):
            "\"\(code)\" isn't a task code. Codes are 4 characters, with or without the Avenue key, such as "
                + "HEALTH-7K2Q or 7K2Q."
        case .invalidDate(let date):
            "\"\(date)\" isn't a calendar date. Use YYYY-MM-DD, such as 2026-11-05."
        case .invalidTime(let time):
            "\"\(time)\" isn't a time of day. Use 24-hour HH:MM, such as 09:30."
        case .dueTimeWithoutDueDate:
            "A due time needs a due date. Set the due date too, or clear the time."
        case .notFound(let entity, let id):
            "No \(entity.rawValue) matches \(id). It may have been deleted or not synced to this device yet."
        case .projectInDifferentAvenue:
            "That Project is in a different Avenue. Pick a Project in the task's Avenue, or move the task to the "
                + "Project's Avenue."
        case .parentIsSubtask:
            "That task is already a Sub task, and Sub tasks can't have their own. Pick a top-level task as the parent."
        case .taskHasSubtasks:
            "This task has Sub tasks, so it can't become a Sub task. Move or promote its Sub tasks first."
        case .subtaskFollowsParent:
            "A Sub task always has its parent's Avenue and Project. Move the parent instead, or promote the Sub task "
                + "first."
        case .cannotParentToItself:
            "A task can't be its own parent. Pick a different task."
        case .openSubtasks(_, let open):
            "This task has \(open.count) open Sub task(s). Close them, or mark the task Won't Do to drop them too."
        case .wontDoNeedsCascade(_, let open):
            "Marking this task Won't Do also closes its \(open.count) open Sub task(s) as Won't Do. Confirm to close "
                + "them all (cascade: true), or close the Sub tasks first."
        case .projectHasOpenTasks(_, let open):
            "This Project still has \(open.count) open task(s). Close them before completing it, or archive the "
                + "Project instead."
        case .emptyComment:
            "A comment can't be empty. Write something, or delete the comment."
        case .notCommentAuthor:
            "Only the comment's author can edit or delete it. Add a new comment instead."
        case .inTrash:
            "This task is in the trash. Restore it before changing it."
        case .reminderNeedsDueDate:
            "A reminder before the due date needs a due date. Set one first, or remind at a set time instead."
        case .cannotRevert:
            "Only changes to a task's own fields can be undone from its history. Move it, or change its code, "
                + "by hand instead."
        case .changedSince:
            "That field changed again after this. Undo the later change first, or set the field by hand."
        }
    }
}
