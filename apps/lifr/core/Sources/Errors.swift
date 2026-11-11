import Foundation

/// The kinds of record a lookup can fail to find.
package enum Entity: String, Hashable, Codable, Sendable {
    case avenue, project, task, label, comment, reminder
    case savedFilter = "saved filter"
    /// A journal entry.
    case change
}

/// Why Core refused a change. Every client shows `description` as is, so each
/// message says what failed and how to fix it.
package enum LifrError: Error, Hashable, Sendable, CustomStringConvertible {
    case invalidTitle(length: Int)
    /// An Avenue's or Project's name.
    case invalidName(length: Int)
    case invalidAvenueKey(String)
    case avenueKeyTaken(AvenueKey)
    case invalidTaskCode(String)
    case invalidDate(String)
    case invalidTime(String)
    case dueTimeWithoutDueDate
    /// Only top-level tasks repeat; a Sub task comes back with its parent.
    case subtaskCannotRepeat(UUID)
    /// The rule never falls on a day, such as weekly on no weekdays.
    case invalidRecurrence
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
    /// Archiving would hide these; retry with `openTasks: .closeAsWontDo`.
    case avenueHasOpenTasks(avenue: UUID, open: [UUID])
    case inboxCannotBeArchived
    /// Another label has this name, ignoring case.
    case labelNameTaken(String)
    /// A saved filter with no words and no filters would show nothing.
    case emptyFilter
    case emptyComment
    case notCommentAuthor(comment: UUID)
    case inTrash(UUID)
    case reminderNeedsDueDate(UUID)
    /// History can't put this kind of change back, such as a move.
    case cannotRevert(event: UUID)
    /// The field changed again after the event, so reverting would lose that.
    case changedSince(event: UUID)
    /// A plan couldn't be added; `at` says which part, such as
    /// "task 2 “Buy clothes” › Sub task 1 “Jacket”". Nothing was added.
    indirect case plan(at: String, PlanProblem)

    package var description: String {
        switch self {
        case .invalidTitle(let length):
            "A title must be 1 to 200 characters; this one has \(length). Shorten it, or move the rest into the "
                + "details."
        case .invalidName(let length):
            "A name must be 1 to 100 characters; this one has \(length). Shorten it, or put the rest in the "
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
        case .subtaskCannotRepeat:
            "A Sub task can't repeat on its own. Make its parent repeat: each new occurrence brings its Sub tasks back."
        case .invalidRecurrence:
            "That repeat never falls on a day. Pick at least one weekday, a day of the month from 1 to 31, and an "
                + "interval of 1 or more."
        case .notFound(.change, let id):
            "No change that can still be reverted matches \(id). Changes older than 90 days stay in Activity but "
                + "can't be reverted; a newer one may not have synced to this device yet."
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
            "This task has \(counted(open.count, "open Sub task", "open Sub tasks")). Close them, or mark the "
                + "task Won't Do to drop them too."
        case .wontDoNeedsCascade(_, let open):
            "Marking this task Won't Do also closes its \(counted(open.count, "open Sub task", "open Sub tasks")) as "
                + "Won't Do. Confirm to close them all (cascade: true), or close the Sub tasks first."
        case .projectHasOpenTasks(_, let open):
            "This Project still has \(counted(open.count, "open task", "open tasks")). Close them before "
                + "completing it, or archive the Project instead."
        case .avenueHasOpenTasks(_, let open):
            "This Avenue still has \(counted(open.count, "open task", "open tasks")), and archiving hides them. "
                + "Move them to another Avenue first, or confirm to close them as Won't Do."
        case .labelNameTaken(let name):
            "Another label is already called “\(name)”. Pick another name, or merge this label into that one."
        case .emptyFilter:
            "A saved filter needs some words or at least one filter. Set one in Search, then save it."
        case .inboxCannotBeArchived:
            "The Inbox can't be archived, since it's where tasks go when no Avenue is picked. Move its tasks "
                + "to other Avenues, or close them, to empty it."
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
        case .plan(let at, let problem):
            "Nothing was added. In \(at): \(problem.description)"
        }
    }
}

extension PlanProblem: CustomStringConvertible {
    package var description: String {
        switch self {
        case .empty:
            "it adds nothing. Add at least one Avenue, label, Project or task."
        case .unknownAvenue(let key, let suggestion):
            "there's no Avenue \(key). "
                + (suggestion.map { "Did you mean \($0)? " } ?? "")
                + "Use an existing Avenue's key, or add the Avenue to the plan."
        case .unknownProject(let name, let avenue, let suggestion):
            "\(avenue) has no Project “\(name)”. "
                + (suggestion.map { "Did you mean “\($0)”? " } ?? "")
                + "Use an existing Project's name, or add the Project to the plan with its tasks inside it."
        case .projectExists(let name, let avenue):
            "\(avenue) already has a Project “\(name)”. To add tasks to it, list them as tasks placed in "
                + "\(avenue)/\(name), or give the new Project another name."
        case .noPlace:
            "the task doesn't say where it goes. Give it an Avenue key or KEY/Project name, or the code of the "
                + "task it's a Sub task of."
        case .placeGivenTwice:
            "the task says where it goes more than once. Tasks inside a Project and Sub tasks go where they're "
                + "listed, so remove their own Avenue or parent; a task on its own takes one or the other."
        case .subtaskTooDeep:
            "Sub tasks can't have their own Sub tasks. Make them separate Sub tasks of the same task, or a "
                + "checklist in the description."
        case .invalidDate(let text):
            "“\(text)” isn't a date Lifr reads. \(Plan.dateHelp)"
        case .invalidReminder(let text):
            "“\(text)” isn't a reminder Lifr reads. \(Plan.reminderHelp)"
        case .closedWithOpenSubtasks:
            "the task is Done or Won't Do but has open Sub tasks. Close the Sub tasks too, or leave the task open."
        case .rule(let error):
            error.description
        }
    }
}

/// `count` with the noun that fits it: "1 open task", "2 open tasks".
package func counted(_ count: Int, _ one: String, _ many: String) -> String {
    "\(count) \(count == 1 ? one : many)"
}
