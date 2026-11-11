import Foundation
import LifrCore

/// Planning with an assistant outside Lifr: a prompt to paste into any chat,
/// and the plan read back out of its reply.
package enum PlanPrompt {
    /// A prompt asking for a plan for `goal`, in the TOML `PlanFile` reads.
    /// It names the Avenues an assistant may add to, their active Projects
    /// and every label, so the reply reuses them. No task is in it.
    package static func make(goal: String, workspace: Workspace, today: CalendarDate, timeZone: TimeZone) -> String {
        let avenues = workspace.avenues.values
            .filter { $0.archivedAt == nil && $0.agentAccess == .full }
            .sorted { $0.key < $1.key }
        let ids = Set(avenues.map(\.id))
        let projects = workspace.projects.values
            .filter { ids.contains($0.avenueID) && $0.status == .active }
        let labels = workspace.labels.values.map(\.name).sorted()
        var context = ["Avenues (key: name), with their Projects:"]
        for avenue in avenues {
            context.append("- \(avenue.key): \(avenue.name)")
            let names = projects.filter { $0.avenueID == avenue.id }.map(\.name).sorted()
            context += names.map { "  - \($0)" }
        }
        if avenues.isEmpty { context.append("- none yet: add an [[avenue]] for the tasks") }
        context.append("Labels: " + (labels.isEmpty ? "none yet" : labels.joined(separator: ", ")))
        let goal = goal.trimmingCharacters(in: .whitespacesAndNewlines)
        return """
            You're helping plan tasks for Lifr, a to-do app. Reply with one ```toml block and nothing else, \
            following the format below. Put tasks in the Avenues and Projects listed further down, using their \
            keys and names as written, and reuse the labels listed. Add an Avenue, Project or label only when \
            none of them fits.

            The format:

            \(PlanFile.guide)

            An example:

            ```toml
            \(PlanFile.write(example(key: avenues.first?.key.rawValue)))```

            \(context.joined(separator: "\n"))

            Today is \(today), in the time zone \(timeZone.identifier).

            What to plan: \(goal.isEmpty ? "Ask me what to plan, and reply with the plan once I answer." : goal)
            """
    }

    /// A prompt asking the same chat to write `reply` again, fixed.
    package static func fixUp(_ reply: String, error: PlanFileError) -> String {
        """
        Lifr couldn't read the plan in your last reply. \(error.description)

        Write the whole plan again with that fixed, as one ```toml block and nothing else, following this format:

        \(PlanFile.guide)

        The plan Lifr couldn't read:

        \(plan(in: reply).trimmingCharacters(in: .newlines))
        """
    }

    /// The plan in an assistant's reply: inside its first ```toml or ```
    /// fence that has tables, or the reply itself from its first table or
    /// `PlanFile.firstLine` on, so words around it are left out.
    package static func read(_ reply: String) throws(PlanFileError) -> Plan {
        try PlanFile.read(plan(in: reply))
    }

    /// An example of every table, for the Avenue `key`, or a new one when
    /// there's none to add to.
    static func example(key: String?) -> Plan {
        let avenues = key == nil ? [Plan.AvenueDraft(key: "HOME", name: "Home", color: "green", symbol: "house")] : []
        let key = key ?? "HOME"
        return Plan(
            avenues: avenues,
            labels: [.init(name: "errands", color: "orange")],
            projects: [
                .init(
                    name: "Move flat", avenue: key, due: "+1m",
                    tasks: [
                        .init(
                            title: "Pack the kitchen", priority: .medium, due: "friday",
                            subtasks: [.init(title: "Buy boxes", labels: ["errands"])])
                    ])
            ],
            tasks: [.init(title: "Book the van", due: "+2w", dueTime: "09:00", place: key)])
    }

    private static func plan(in reply: String) -> String {
        let lines = reply.components(separatedBy: .newlines)
        var fence: [String]?
        for line in lines {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if var open = fence {
                if trimmed.hasPrefix("```") {
                    if open.contains(where: isTable) { return open.joined(separator: "\n") + "\n" }
                    fence = nil
                } else {
                    open.append(line)
                    fence = open
                }
            } else if trimmed.hasPrefix("```") {
                fence = []
            }
        }
        // A fence the reply never closed, such as one cut off.
        if let open = fence, open.contains(where: isTable) { return open.joined(separator: "\n") + "\n" }
        let start = lines.firstIndex { $0.hasPrefix(PlanFile.firstLine) || isTable($0) } ?? 0
        return lines[start...].joined(separator: "\n")
    }

    private static func isTable(_ line: String) -> Bool {
        line.trimmingCharacters(in: .whitespaces).hasPrefix("[[")
    }
}
