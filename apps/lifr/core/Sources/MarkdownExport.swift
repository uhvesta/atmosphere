import Foundation

extension Workspace {
    /// An Avenue as Markdown, for reading: its Projects, then the tasks in
    /// each with their Sub tasks, open ones as `- [ ]` and closed ones as
    /// `- [x]`, with code, status and due date. The trash is left out.
    package func markdown(of avenueID: UUID) -> String? {
        guard let avenue = avenues[avenueID] else { return nil }
        let live = tasks.values.filter { $0.avenueID == avenueID && $0.deletedAt == nil }
        func ordered(_ tasks: some Sequence<LifrTask>) -> [LifrTask] {
            tasks.sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
        }
        let subtasks = Dictionary(grouping: live.filter { $0.parentID != nil }) { $0.parentID }
        var lines = ["# \(avenue.name)"]
        func write(_ tasks: [LifrTask]) {
            for task in tasks {
                lines.append(line(for: task, indent: ""))
                for subtask in ordered(subtasks[task.id] ?? []) {
                    lines.append(line(for: subtask, indent: "  "))
                }
            }
        }
        let topLevel = live.filter { $0.parentID == nil }
        let loose = ordered(topLevel.filter { $0.projectID == nil })
        if !loose.isEmpty {
            lines.append("")
            write(loose)
        }
        let projects = self.projects.values.filter { $0.avenueID == avenueID }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
        for project in projects {
            lines += ["", "## \(project.name)"]
            if !project.details.isEmpty { lines += ["", project.details] }
            let inProject = ordered(topLevel.filter { $0.projectID == project.id })
            if !inProject.isEmpty {
                lines.append("")
                write(inProject)
            }
        }
        return lines.joined(separator: "\n") + "\n"
    }

    private func line(for task: LifrTask, indent: String) -> String {
        var line = "\(indent)- [\(task.isOpen ? " " : "x")] \(displayCode(of: task)) \(task.title)"
        var notes: [String] = []
        if task.status == .inProgress || task.status == .procrastinated || task.status == .wontDo {
            notes.append(Self.statusWord(task.status))
        }
        if let due = task.dueDate { notes.append("due \(due)") }
        if !notes.isEmpty { line += " (\(notes.joined(separator: ", ")))" }
        if !task.details.isEmpty {
            for detail in task.details.split(separator: "\n", omittingEmptySubsequences: false) {
                line += "\n\(indent)  \(detail)"
            }
        }
        return line
    }

    private static func statusWord(_ status: TaskStatus) -> String {
        switch status {
        case .todo: "to do"
        case .inProgress: "in progress"
        case .procrastinated: "procrastinated"
        case .done: "done"
        case .wontDo: "won't do"
        }
    }

    /// A file name for an Avenue's Markdown: its name, without characters
    /// file systems refuse.
    package func markdownFileName(of avenueID: UUID) -> String? {
        guard let avenue = avenues[avenueID] else { return nil }
        let name = avenue.name.components(separatedBy: CharacterSet(charactersIn: "/\\:?*\"<>|")).joined(separator: "-")
        return "\(name.isEmpty ? avenue.key.rawValue : name).md"
    }
}
