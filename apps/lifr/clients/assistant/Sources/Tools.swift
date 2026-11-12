import ComposableArchitecture
import Foundation
import LifrCore

/// What the assistant can look up and change, as short text for a model
/// with a small context: one line per task, at most `limit` of them. One
/// set of tools serves one answer, and sees its own changes.
package final class AssistantTools: Sendable {
    package static let limit = 15

    private let current: LockIsolated<Workspace>
    /// The records as the answer started, before what it proposed.
    let initial: Workspace
    /// What the answer asked to change, in order, with how to make each.
    let proposed = LockIsolated<[(action: AssistantAction, operation: AssistantOperation)]>([])
    private let budget = LockIsolated(1_200)
    package let now: Date
    package let timeZone: TimeZone
    /// Whether the tools may propose changes; without it they only read.
    let writes: AssistantWrites?

    package init(workspace: Workspace, now: Date, timeZone: TimeZone, writes: AssistantWrites? = nil) {
        // Never anything in an Avenue hidden from agents, even when a
        // proposal is made again on newer records.
        let seen = workspace.seenByAgents
        current = LockIsolated(seen)
        initial = seen
        self.now = now
        self.timeZone = timeZone
        self.writes = writes
    }

    /// The records, with the changes these tools proposed.
    package var workspace: Workspace {
        get { current.value }
        set { current.setValue(newValue) }
    }

    /// The most characters of each result `capped` returns.
    package var resultBudget: Int {
        get { budget.value }
        set { budget.setValue(newValue) }
    }

    /// `result` cut to `resultBudget` at the end of a line, saying the rest
    /// was left out, so one long answer can't fill the model's context.
    package func capped(_ result: String) -> String {
        let limit = resultBudget
        guard result.count > limit else { return result }
        let lines = result.split(separator: "\n", omittingEmptySubsequences: false)
        var kept: [Substring] = []
        var used = 0
        for line in lines {
            guard used + line.count + 1 <= limit - 40 else { break }
            kept.append(line)
            used += line.count + 1
        }
        if kept.isEmpty, let first = lines.first { kept = [first.prefix(limit - 40)] }
        return kept.joined(separator: "\n") + "\n(The rest is left out. Search with more words.)"
    }

    /// The status names the assistant reads and writes.
    package static let statusNames: [TaskStatus: String] = [
        .todo: "todo", .inProgress: "in_progress", .procrastinated: "procrastinated", .done: "done",
        .wontDo: "wont_do",
    ]

    var today: CalendarDate { CalendarDate(now, in: timeZone) }

    /// Tasks whose title or details have every word, in `avenue` (a KEY),
    /// with `status`, due by `dueBy`. Without a status, only open tasks.
    package func findTasks(words: String, avenue: String?, status: String?, dueBy: String?) -> String {
        let words = words.lowercased().split(whereSeparator: \.isWhitespace)
        let avenueID: UUID?
        if let avenue = avenue?.trimmingCharacters(in: .whitespaces), !avenue.isEmpty {
            guard let found = workspace.avenues.values.first(where: { $0.key.rawValue == avenue.uppercased() })
            else {
                return "There's no Avenue \(avenue.uppercased()). The Avenues are: "
                    + workspace.avenues.values.map(\.key.rawValue).sorted().joined(separator: ", ") + "."
            }
            avenueID = found.id
        } else {
            avenueID = nil
        }
        let wanted: TaskStatus?
        if let status, !status.isEmpty {
            guard let match = Self.statusNames.first(where: { $0.value == status })?.key else {
                return "There's no status \(status). Use one of: "
                    + Self.statusNames.values.sorted().joined(separator: ", ") + "."
            }
            wanted = match
        } else {
            wanted = nil
        }
        let due: CalendarDate?
        if let dueBy, !dueBy.isEmpty {
            guard let date = CalendarDate.resolving(dueBy, today: today) else {
                return "\(dueBy) isn't a date. Use YYYY-MM-DD, today, tomorrow, a weekday, +3d, +2w or +1m."
            }
            due = date
        } else {
            due = nil
        }
        let found = workspace.tasks.values.filter { task in
            guard task.deletedAt == nil, workspace.avenues[task.avenueID]?.archivedAt == nil else { return false }
            if let avenueID, task.avenueID != avenueID { return false }
            let effective = task.effectiveStatus(now: now, settings: workspace.settings)
            if let wanted {
                guard effective == wanted else { return false }
            } else if effective.isClosed {
                return false
            }
            if let due {
                guard let date = task.dueDate, date <= due else { return false }
            }
            let text = (task.title + "\n" + task.details).lowercased()
            return words.allSatisfy { text.contains($0) }
        }
        .sorted(by: LifrTask.listOrder)
        guard !found.isEmpty else { return "No tasks match." }
        return lines(found)
    }

    /// One task with its Sub tasks and comments, or why it can't be found.
    package func getTask(code: String) -> String {
        guard let task = try? workspace.task(code: code.uppercased()), task.deletedAt == nil else {
            return "There's no task \(code.uppercased()). Use findTasks to look up its code."
        }
        var text = line(task)
        if !task.details.isEmpty { text += "\nDetails: " + task.details }
        let subtasks = workspace.subtasks(of: task.id).filter { $0.deletedAt == nil }
        if !subtasks.isEmpty { text += "\nSub tasks:\n" + lines(subtasks) }
        let comments = workspace.comments.values
            .filter { $0.taskID == task.id && $0.deletedAt == nil }
            .sorted { $0.createdAt < $1.createdAt }
        if !comments.isEmpty {
            text += "\nComments:\n" + comments.suffix(5).map { "- " + $0.body }.joined(separator: "\n")
        }
        return text
    }

    /// Open tasks per Avenue, then what's overdue, due today, due this week
    /// and procrastinated.
    package func overview() -> String {
        let (workspace, today, now) = (workspace, today, now)
        let open = workspace.tasks.values.filter { task in
            task.deletedAt == nil && task.isOpen && workspace.avenues[task.avenueID]?.archivedAt == nil
        }
        let avenues = workspace.avenues.values.filter { $0.archivedAt == nil }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
        var text =
            "Today is \(today). Open tasks per Avenue: "
            + avenues.map { avenue in "\(avenue.key.rawValue) \(open.filter { $0.avenueID == avenue.id }.count)" }
            .joined(separator: ", ")
        let week = CalendarDate.resolving("+7d", today: today) ?? today
        let sections: [(String, (LifrTask) -> Bool)] = [
            ("Overdue", { $0.dueDate.map { $0 < today } ?? false }),
            ("Due today", { $0.dueDate == today }),
            ("Due in the next 7 days", { $0.dueDate.map { $0 > today && $0 <= week } ?? false }),
            ("Procrastinated", { $0.effectiveStatus(now: now, settings: workspace.settings) == .procrastinated }),
        ]
        for (title, matches) in sections {
            let tasks = open.filter(matches).sorted(by: LifrTask.listOrder)
            text += "\n\(title): " + (tasks.isEmpty ? "none" : "\n" + lines(tasks))
        }
        return text
    }

    func lines(_ tasks: [LifrTask]) -> String {
        var text = tasks.prefix(Self.limit).map(line).joined(separator: "\n")
        if tasks.count > Self.limit { text += "\n(\(tasks.count - Self.limit) more; search with more words)" }
        return text
    }

    private func line(_ task: LifrTask) -> String {
        var place = workspace.avenues[task.avenueID]?.key.rawValue ?? "?"
        if let project = task.projectID.flatMap({ workspace.projects[$0] }) { place += "/" + project.name }
        var parts = [
            workspace.displayCode(of: task), task.title,
            Self.statusNames[task.effectiveStatus(now: now, settings: workspace.settings)] ?? "", place,
        ]
        if task.priority != .none { parts.append("priority " + task.priority.rawValue) }
        if let due = task.dueDate { parts.append("due \(due)" + (task.dueTime.map { " \($0)" } ?? "")) }
        if let parent = task.parentID.flatMap({ workspace.tasks[$0] }) {
            parts.append("under " + workspace.displayCode(of: parent))
        }
        let labels = task.labelIDs.compactMap { workspace.labels[$0]?.name }.sorted()
        if !labels.isEmpty { parts.append("labels " + labels.joined(separator: ", ")) }
        let open = workspace.openSubtasks(of: task.id).count
        if open > 0 { parts.append("\(open) open Sub tasks") }
        return "- " + parts.joined(separator: " · ")
    }
}
