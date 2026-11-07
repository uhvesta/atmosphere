import ComposableArchitecture
import Foundation

/// A tool the assistant can call while it answers.
package enum AssistantTool: String, Hashable, Codable, Sendable, CaseIterable {
    case findTasks, getTask, overview, recentActivity, updateTasks, updateProjects, addComment, revert

    /// Whether the tool saves changes, which needs `AssistantRequest.writes`.
    package var writes: Bool {
        switch self {
        case .findTasks, .getTask, .overview, .recentActivity: false
        case .updateTasks, .updateProjects, .addComment, .revert: true
        }
    }
}

/// What the assistant does in a conversation: instructions, the tools it
/// may call, and whether it drafts plans. Lifr has a few built in; the
/// person can write their own, whose IDs start with `user:`.
package struct Skill: Identifiable, Hashable, Codable, Sendable {
    /// "query" for a built-in skill, "user:weekly-review" for the person's.
    package var id: String
    package var summary: String
    /// What the assistant is told to do for a message that uses the skill.
    package var instructions: String
    package var tools: Set<AssistantTool>
    package var draftsPlans: Bool
    /// Off skills can't be picked.
    package var isOn: Bool
    /// Whether Lifr may pick the skill for a message by itself, when
    /// Settings › Assistant › Pick a skill automatically is on.
    package var autoPicks: Bool
    /// When the skill fits a message, for picking it automatically: "the
    /// person lists things they have to do".
    package var useWhen: String

    package init(
        id: String, summary: String, instructions: String, tools: Set<AssistantTool>, draftsPlans: Bool,
        isOn: Bool = true, autoPicks: Bool = false, useWhen: String = ""
    ) {
        self.id = id
        self.summary = summary
        self.instructions = instructions
        self.tools = tools
        self.draftsPlans = draftsPlans
        self.isOn = isOn
        self.autoPicks = autoPicks
        self.useWhen = useWhen
    }

    private enum CodingKeys: String, CodingKey {
        case id, summary, instructions, tools, draftsPlans, isOn, autoPicks, useWhen
    }

    /// Skills saved before automatic picking read as the built-in's default,
    /// or not picked automatically for the person's own.
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let id = try container.decode(String.self, forKey: .id)
        let builtIn = Self.builtIn.first { $0.id == id }
        self.init(
            id: id, summary: try container.decode(String.self, forKey: .summary),
            instructions: try container.decode(String.self, forKey: .instructions),
            tools: try container.decode(Set<AssistantTool>.self, forKey: .tools),
            draftsPlans: try container.decode(Bool.self, forKey: .draftsPlans),
            isOn: try container.decode(Bool.self, forKey: .isOn),
            autoPicks: try container.decodeIfPresent(Bool.self, forKey: .autoPicks) ?? builtIn?.autoPicks ?? false,
            useWhen: try container.decodeIfPresent(String.self, forKey: .useWhen) ?? builtIn?.useWhen ?? "")
    }

    package static let userPrefix = "user:"

    /// A skill of the person's, with `name` made into an ID such as
    /// "user:weekly-review". Nil when the name has no letters or digits.
    package init?(
        userNamed name: String, summary: String, instructions: String, tools: Set<AssistantTool>,
        draftsPlans: Bool, autoPicks: Bool = false, useWhen: String = ""
    ) {
        let words = name.lowercased().split { !$0.isLetter && !$0.isNumber }
        guard !words.isEmpty else { return nil }
        self.init(
            id: Self.userPrefix + words.joined(separator: "-"), summary: summary, instructions: instructions,
            tools: tools, draftsPlans: draftsPlans, autoPicks: autoPicks, useWhen: useWhen)
    }

    package var isUser: Bool { id.hasPrefix(Self.userPrefix) }

    /// What the pill says: "Query", or "user:weekly-review".
    package var title: String { isUser ? id : id.prefix(1).uppercased() + id.dropFirst() }

    // MARK: - Built in

    package static let query = Skill(
        id: "query", summary: String(localized: "Answer questions about your tasks."),
        instructions: """
            Answer the person's questions about their tasks from what the tools return. Don't propose a plan.
            """,
        tools: [.findTasks, .getTask, .overview, .recentActivity], draftsPlans: false, autoPicks: true,
        useWhen: "the person asks about their tasks, Projects or Avenues: what's due, what's left, what's in one")

    package static let add = Skill(
        id: "add", summary: String(localized: "Plan new tasks, Projects and Avenues."),
        instructions: """
            Help the person plan what to add. Ask what's missing, then put the whole plan in "plan".
            """,
        tools: [.findTasks, .getTask], draftsPlans: true, autoPicks: true,
        useWhen: "the person names one or a few new things to add, or answers about a plan being drafted")

    package static let change = Skill(
        id: "change", summary: String(localized: "Rename, move, reschedule or finish tasks and Projects."),
        instructions: """
            Change the tasks or Projects the person means: put each change in "changes", or in \
            "projectChanges" for a Project, and say in a sentence what you'd change.
            """,
        tools: [.findTasks, .getTask, .updateTasks, .updateProjects], draftsPlans: false, autoPicks: true,
        useWhen: """
            the person wants tasks or Projects they already have renamed, moved, rescheduled or reprioritised, \
            or marked done, such as "I did…" or "mark it done"
            """)

    package static let breakdown = Skill(
        id: "breakdown", summary: String(localized: "Turn a brain dump into a plan."),
        instructions: """
            The person is dumping what's on their mind. First understand it: ask short questions, one or two at \
            a time, about what's unclear: what each thing is, which Avenue or Project it belongs in, what has to \
            come first, and any deadlines. Leave "plan" out while you're still asking. Once you understand, plan \
            it in "plan": tasks in the order to do them, with steps as Sub tasks, and a Project where several \
            tasks serve one goal.
            """,
        tools: [.findTasks], draftsPlans: true, autoPicks: true,
        useWhen: "the person dumps many things they have to do, or a big goal that needs breaking into steps")

    package static let tidy = Skill(
        id: "tidy", summary: String(localized: "Clean up titles, priorities, dates and labels."),
        instructions: """
            Help tidy existing tasks: clearer titles, missing priorities and due dates, consistent labels. \
            Say what you'd change first, and put it in "changes" once the person agrees.
            """,
        tools: [.findTasks, .getTask, .overview, .updateTasks, .updateProjects], draftsPlans: false,
        useWhen: "the person asks to clean up or tidy their tasks")

    package static let review = Skill(
        id: "review", summary: String(localized: "Look back at what's done, overdue and stalled."),
        instructions: """
            Review the person's week with them: what got done, what's overdue, and what has stalled. Suggest \
            what to drop or move, and add a comment to a task when they ask.
            """,
        tools: [.findTasks, .getTask, .overview, .recentActivity, .addComment], draftsPlans: false,
        useWhen: "the person asks to look back over their week")

    package static let undo = Skill(
        id: "undo", summary: String(localized: "Put back recent changes."),
        instructions: """
            Help the person put back changes. Find them with recentActivity, say which you'd revert, and \
            revert them once the person agrees.
            """,
        tools: [.recentActivity, .revert], draftsPlans: false,
        useWhen: "the person wants a recent change put back")

    package static let builtIn: [Skill] = [.query, .add, .change, .breakdown, .tidy, .review, .undo]

    /// Whether every answer must hold a plan: the Add skill.
    package var requiresPlan: Bool { id == Self.add.id }
    /// Whether answers must hold changes to tasks or Projects: the Change skill.
    package var requiresChange: Bool { id == Self.change.id }
    /// Whether the skill plans Projects itself, so the person needn't ask
    /// for one by name: the Break down skill.
    package var suggestsProjects: Bool { id == Self.breakdown.id }

    // MARK: - Search

    /// The skills that match `query`, best first: the title starting with
    /// it, then its letters in order in the title, then every word in the
    /// summary. An empty query matches every skill, in order.
    package static func search(_ query: String, in skills: [Skill]) -> [Skill] {
        let query = query.lowercased().trimmingCharacters(in: .whitespaces)
        guard !query.isEmpty else { return skills }
        return skills.enumerated()
            .compactMap { index, skill in skill.score(query).map { (skill, $0, index) } }
            .sorted { ($1.1, $0.2) < ($0.1, $1.2) }
            .map(\.0)
    }

    private func score(_ query: String) -> Int? {
        let title = title.lowercased()
        if title.hasPrefix(query) || id.hasPrefix(Self.userPrefix + query) { return 3 }
        if title.isSubsequence(query) { return 2 }
        let summary = summary.lowercased()
        if query.split(separator: " ").allSatisfy({ summary.contains($0) }) { return 1 }
        return nil
    }
}

extension String {
    /// Whether `other`'s characters all appear in this, in order.
    fileprivate func isSubsequence(_ other: String) -> Bool {
        var remaining = other[...]
        for character in self where remaining.first == character {
            remaining = remaining.dropFirst()
            if remaining.isEmpty { return true }
        }
        return remaining.isEmpty
    }
}

/// A skill the assistant wrote from the person's description, for the
/// editor to fill in.
package struct SkillDraft: Hashable, Sendable {
    package var name: String
    package var summary: String
    package var instructions: String
    package var tools: Set<AssistantTool>
    package var draftsPlans: Bool

    package init(
        name: String, summary: String, instructions: String, tools: Set<AssistantTool>, draftsPlans: Bool
    ) {
        self.name = name
        self.summary = summary
        self.instructions = instructions
        self.tools = tools
        self.draftsPlans = draftsPlans
    }

    package static let example = SkillDraft(
        name: "Weekly review", summary: "Fridays: what got done and what's next.",
        instructions: "Go through what got done this week and what's overdue, then suggest the three tasks to do next.",
        tools: [.findTasks, .overview, .recentActivity], draftsPlans: false)
}

/// Keeps the person's skills, and which skills are on, with conversations:
/// on this device, or in iCloud when conversations sync.
@DependencyClient
package struct SkillsClient: Sendable {
    /// The built-in skills, then the person's, each as on or off as it was
    /// left.
    package var load: @Sendable () async throws -> [Skill]
    /// Adds or replaces a skill of the person's, or turns a built-in one on
    /// or off. A built-in's instructions can't change.
    package var save: @Sendable (_ skill: Skill) async throws -> Void
    /// Removes skills of the person's. Messages that used one still show
    /// its ID.
    package var delete: @Sendable (_ ids: [String]) async throws -> Void
}

extension SkillsClient {
    /// Keeps skills in memory, for previews and tests.
    package static func inMemory(_ skills: [Skill] = []) -> Self {
        let stored = LockIsolated(Dictionary(skills.map { ($0.id, $0) }) { $1 })
        return SkillsClient(
            load: { Skill.merged(stored: Array(stored.value.values)) },
            save: { skill in stored.withValue { $0[skill.id] = skill } },
            delete: { ids in stored.withValue { for id in ids { $0[id] = nil } } })
    }
}

extension Skill {
    /// The built-in skills with their stored on or off, then the stored
    /// skills of the person's by ID. Stored copies of built-ins only count
    /// for `isOn` and `autoPicks`, so their instructions always come from
    /// this version.
    package static func merged(stored: [Skill]) -> [Skill] {
        let byID = Dictionary(stored.map { ($0.id, $0) }) { $1 }
        let builtIn = Self.builtIn.map { skill in
            var skill = skill
            if let stored = byID[skill.id] {
                skill.isOn = stored.isOn
                skill.autoPicks = stored.autoPicks
            }
            return skill
        }
        return builtIn + stored.filter(\.isUser).sorted { $0.id < $1.id }
    }
}

extension SkillsClient: TestDependencyKey {
    package static let testValue = SkillsClient()
    package static let previewValue = SkillsClient.inMemory()
}

extension DependencyValues {
    package var skills: SkillsClient {
        get { self[SkillsClient.self] }
        set { self[SkillsClient.self] = newValue }
    }
}
