import Foundation
import LifrCore

/// Planning with an assistant outside Lifr: a prompt to paste into any chat,
/// and the plan read back out of its reply.
package enum PlanPrompt {
    /// Which of the person's tasks a prompt lists, so the reply doesn't
    /// repeat them. Copying titles out of Lifr is the person's choice.
    package enum Scope: Hashable, Sendable {
        /// None: Avenue, Project and label names only.
        case names
        /// The open tasks in every Avenue the assistant may add to.
        case everything
        /// The open tasks in one of them.
        case avenue(UUID)
        /// The open tasks in one of their Projects.
        case project(UUID)
    }

    /// What a prompt says of each task `Scope` lists.
    package struct Listing: Hashable, Sendable {
        /// Lists closed tasks too, with their status, so the reply knows
        /// what's been done.
        package var closed = false
        /// Adds each task's description.
        package var descriptions = false

        package init(closed: Bool = false, descriptions: Bool = false) {
            self.closed = closed
            self.descriptions = descriptions
        }
    }

    /// The most tasks a prompt lists, so a big workspace stays pasteable.
    static let taskLimit = 200

    /// A prompt asking for a plan for `goal`, in the TOML `PlanFile` reads.
    /// It names the Avenues an assistant may add to, their active Projects
    /// and every label, so the reply reuses them, and the open tasks in
    /// `scope`, as `listing` says.
    package static func make(
        goal: String, scope: Scope = .names, listing: Listing = Listing(), instructions: String = "",
        workspace: Workspace, today: CalendarDate, timeZone: TimeZone
    ) -> String {
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
        context += tasks(in: scope, listing: listing, avenues: avenues, workspace: workspace)
        let goal = goal.trimmingCharacters(in: .whitespacesAndNewlines)
        let instructions = instructions.trimmingCharacters(in: .whitespacesAndNewlines)
        let extra = instructions.isEmpty ? "" : "Also follow these instructions: \(instructions)\n\n"
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

            \(extra)What to plan: \(goal.isEmpty ? "Ask me what to plan, and reply with the plan once I answer." : goal)
            """
    }

    /// The lines listing the tasks in `scope`, Avenue by Avenue, or none
    /// for `.names`.
    private static func tasks(
        in scope: Scope, listing: Listing, avenues: [Avenue], workspace: Workspace
    ) -> [String] {
        let listed: [Avenue]
        var project: UUID?
        switch scope {
        case .names: return []
        case .everything: listed = avenues
        case .avenue(let id): listed = avenues.filter { $0.id == id }
        case .project(let id):
            let avenue = workspace.projects[id]?.avenueID
            listed = avenues.filter { $0.id == avenue }
            project = id
        }
        let ids = Set(listed.map(\.id))
        let found = workspace.tasks.values
            .filter { task in
                ids.contains(task.avenueID) && task.deletedAt == nil && (listing.closed || task.isOpen)
                    && (project == nil || task.projectID == project)
            }
            .sorted { $0.title < $1.title }
        var lines = [
            "",
            listing.closed
                ? "Tasks already in Lifr, which the plan shouldn't add again:"
                : "Open tasks already in Lifr, which the plan shouldn't add again:",
        ]
        if found.isEmpty { lines.append("- none") }
        var count = 0
        for avenue in listed {
            for task in found where task.avenueID == avenue.id {
                count += 1
                guard count <= taskLimit else { continue }
                let status = task.isOpen && task.status == .todo ? "" : " (\(PlanFile.statusName(task.status)))"
                lines.append("- \(avenue.key): \(task.title)\(status)")
                let details = task.details.split(whereSeparator: \.isNewline).joined(separator: " ")
                    .trimmingCharacters(in: .whitespaces)
                if listing.descriptions, !details.isEmpty { lines.append("  \(details.prefix(500))") }
            }
        }
        if count > taskLimit { lines.append("- and \(count - taskLimit) more") }
        return lines
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
