import Foundation
import LifrAPI
import LifrCore

/// How `lifr` prints a response for people to read. `--json` prints the
/// response as the API sends it instead.
enum Output {
    static func text(for method: APIMethod, _ result: JSONValue, timeZone: TimeZone) throws -> String {
        switch method {
        case .hello:
            return "Connected."
        case .unlock:
            let unlock = try result.decode(Unlock.Result.self)
            return "Unlocked this terminal. It locks again after 15 minutes without a command (now "
                + "\(time(unlock.expiresAt, timeZone))).\n"
        case .listAvenues:
            let avenues = try result.decode([AvenueSummary].self)
            guard let width = avenues.map(\.key.count).max() else { return "No Avenues.\n" }
            return lines(
                avenues.map { avenue in
                    var line = pad(avenue.key, width) + "  " + avenue.name
                    switch avenue.agentAccess {
                    case .full: break
                    case .readOnly: line += "  (read only for agents)"
                    case .hidden: line += "  (hidden from agents)"
                    }
                    return line
                })
        case .listProjects:
            let projects = try result.decode([ProjectSummary].self)
            guard !projects.isEmpty else { return "No Projects.\n" }
            return lines(
                projects.map { project in
                    var line = "\(project.avenue)  \(project.name)"
                    if project.status != .active { line += "  (\(project.status.rawValue))" }
                    if let due = project.dueDate { line += "  due \(due)" }
                    return line + "  \(project.id.uuidString)"
                })
        case .searchTasks:
            let tasks = try result.decode([TaskSummary].self)
            guard !tasks.isEmpty else { return "No tasks match.\n" }
            return table(tasks)
        case .getTask:
            return detail(try result.decode(TaskDetail.self), timeZone)
        case .createTask, .updateTask, .setStatus, .moveTask:
            let change = try result.decode(ChangeResult.self)
            let verb =
                switch method {
                case .createTask: "Created"
                case .moveTask: "Moved"
                default: "Changed"
                }
            var text = "\(verb) \(change.task.code): \(change.task.title), \(title(change.task.status)).\n"
            if !change.alsoChanged.isEmpty {
                text += "Also changed:\n" + table(change.alsoChanged)
            }
            if let parent = change.reopenedParent {
                text += "Reopened \(parent), because it has an open Sub task now.\n"
            }
            return text
        case .addComment:
            return "Commented on \(try result.decode(AddComment.Result.self).task).\n"
        case .addPlan:
            let added = try result.decode(AddPlan.Result.self)
            var text = ""
            if !added.avenues.isEmpty { text += "New Avenues: \(added.avenues.joined(separator: ", ")).\n" }
            if !added.labels.isEmpty { text += "New labels: \(added.labels.joined(separator: ", ")).\n" }
            for project in added.projects { text += "New Project in \(project.avenue): \(project.name).\n" }
            if !added.tasks.isEmpty {
                text += "Added \(counted(added.tasks.count, "task", "tasks")):\n" + table(added.tasks)
            }
            return text
        case .trashTask:
            let trash = try result.decode(TrashTask.Result.self)
            return "Moved to the trash: \(trash.trashed.joined(separator: ", ")). Restore them in the Lifr app.\n"
        case .export:
            return try json(result)
        }
    }

    /// `value` as indented JSON with sorted keys, ending in a newline.
    static func json(_ value: some Encodable) throws -> String {
        let encoder = Wire.encoder
        encoder.outputFormatting.insert(.prettyPrinted)
        // JSONEncoder writes UTF-8, so the conversion never fails.
        return (String(bytes: try encoder.encode(value), encoding: .utf8) ?? "") + "\n"
    }

    static func title(_ status: TaskStatus) -> String {
        switch status {
        case .todo: "To Do"
        case .inProgress: "In Progress"
        case .procrastinated: "Procrastinated"
        case .done: "Done"
        case .wontDo: "Won't Do"
        }
    }

    private static func table(_ tasks: [TaskSummary]) -> String {
        let codeWidth = tasks.map(\.code.count).max() ?? 0
        let statusWidth = tasks.map { title($0.status).count }.max() ?? 0
        return lines(
            tasks.map { task in
                var line = pad(task.code, codeWidth) + "  " + pad(title(task.status), statusWidth) + "  " + task.title
                if task.priority != .none { line += "  !\(task.priority.rawValue)" }
                if let due = task.dueDate {
                    line += "  due \(due)" + (task.dueTime.map { " \($0)" } ?? "")
                }
                if task.openSubtasks > 0 {
                    line += "  (\(task.openSubtasks) open Sub task\(task.openSubtasks == 1 ? "" : "s"))"
                }
                return line
            })
    }

    private static func detail(_ detail: TaskDetail, _ timeZone: TimeZone) -> String {
        let task = detail.task
        var text = "\(task.code)  \(task.title)\n"
        var facts = [title(task.status), task.avenue]
        if task.priority != .none { facts.append("priority \(task.priority.rawValue)") }
        if let parent = task.parent { facts.append("Sub task of \(parent)") }
        if let start = task.startDate { facts.append("starts \(start)") }
        if let due = task.dueDate { facts.append("due \(due)" + (task.dueTime.map { " \($0)" } ?? "")) }
        if !task.labels.isEmpty { facts.append(task.labels.map { "#\($0)" }.joined(separator: " ")) }
        text += facts.joined(separator: " · ") + "\n"
        if !detail.details.isEmpty { text += "\n" + detail.details + "\n" }
        if !detail.subtasks.isEmpty { text += "\nSub tasks:\n" + table(detail.subtasks) }
        if !detail.timeline.isEmpty {
            text += "\nTimeline:\n"
            text += lines(
                detail.timeline.map { entry in
                    let who = entry.actor.client ?? entry.actor.device
                    let what: String =
                        switch entry.kind {
                        case .comment: entry.body ?? ""
                        case .history: history(entry, task.avenue)
                        }
                    return "\(day(entry.at, timeZone)) \(time(entry.at, timeZone))  \(who): \(what)"
                })
        }
        return text
    }

    private static func history(_ entry: TimelineEntry, _ avenue: String) -> String {
        func value(_ raw: String?) -> String {
            guard let raw else { return "none" }
            return entry.field == .status ? TaskStatus(rawValue: raw).map(title) ?? raw : raw
        }
        let change: String =
            switch (entry.event, entry.field) {
            case (.created?, _): "Created"
            case (.fieldChanged?, let field?): "\(field.rawValue): \(value(entry.oldValue)) → \(value(entry.newValue))"
            case (.subtaskAdded?, _): "Added Sub task \(avenue)-\(entry.newValue ?? "")"
            case (.subtaskRemoved?, _): "Removed Sub task \(avenue)-\(entry.newValue ?? "")"
            case (.commentAdded?, _): "Commented"
            case (.commentEdited?, _): "Edited a comment"
            case (.commentDeleted?, _): "Deleted a comment"
            case (.movedToTrash?, _): "Moved to the trash"
            case (.restored?, _): "Restored from the trash"
            case (.fieldChanged?, nil), (nil, _): "Changed"
            }
        return entry.reason.map { "\(change) (\($0.rawValue))" } ?? change
    }

    private static func lines(_ lines: [String]) -> String { lines.map { $0 + "\n" }.joined() }

    private static func pad(_ text: String, _ width: Int) -> String {
        text + String(repeating: " ", count: max(0, width - text.count))
    }

    private static func day(_ date: Date, _ timeZone: TimeZone) -> String {
        date.formatted(Date.ISO8601FormatStyle(timeZone: timeZone).year().month().day())
    }

    private static func time(_ date: Date, _ timeZone: TimeZone) -> String {
        date.formatted(Date.ISO8601FormatStyle(timeZone: timeZone).time(includingFractionalSeconds: false))
            .prefix(5).description
    }
}
