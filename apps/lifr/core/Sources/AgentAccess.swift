import Foundation

extension Workspace {
    /// The records as agents see them: nothing in an Avenue hidden from
    /// agents, and no change or history that touched one.
    package var seenByAgents: Workspace {
        let hidden = Set(avenues.values.filter { $0.agentAccess == .hidden }.map(\.id))
        guard !hidden.isEmpty else { return self }
        var seen = self
        seen.avenues = avenues.filter { !hidden.contains($0.key) }
        seen.projects = projects.filter { !hidden.contains($0.value.avenueID) }
        seen.tasks = tasks.filter { !hidden.contains($0.value.avenueID) }
        seen.comments = comments.filter { seen.tasks[$0.value.taskID] != nil }
        seen.reminders = reminders.filter { seen.tasks[$0.value.taskID] != nil }
        seen.history = history.filter { seen.tasks[$0.taskID] != nil }
        seen.journal = journal.filter { entry in
            !entry.steps.contains { step in
                [step.before, step.after].contains { value in
                    value.flatMap(avenueID(of:)).map(hidden.contains) ?? false
                }
            }
        }
        seen.unreadable = []
        return seen
    }

    /// The first Avenue that agents may see but not change, among those the
    /// changes from `before` to these records touch: in it before or after.
    package func readOnlyForAgents(since before: Workspace) -> Avenue? {
        for step in steps(since: before) {
            for value in [step.before, step.after].compactMap({ $0 }) {
                guard let id = avenueID(of: value) ?? before.avenueID(of: value),
                    let avenue = avenues[id] ?? before.avenues[id], avenue.agentAccess != .full
                else { continue }
                return avenue
            }
        }
        return nil
    }

    /// The Avenue `value` is in, if it's in one.
    func avenueID(of value: RecordValue) -> UUID? {
        switch value {
        case .avenue(let avenue): avenue.id
        case .project(let project): project.avenueID
        case .task(let task): task.avenueID
        case .comment(let comment): tasks[comment.taskID]?.avenueID
        case .reminder(let reminder): tasks[reminder.taskID]?.avenueID
        case .label, .settings, .savedFilter: nil
        }
    }
}
