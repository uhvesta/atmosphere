import ComposableArchitecture
import Foundation

/// A tool the assistant can call while it answers.
package enum AssistantTool: String, Hashable, Codable, Sendable, CaseIterable {
    case findTasks, getTask, overview, recentActivity, updateTasks, addComment, revert

    /// Whether the tool saves changes, which needs `AssistantRequest.writes`.
    package var writes: Bool {
        switch self {
        case .findTasks, .getTask, .overview, .recentActivity: false
        case .updateTasks, .addComment, .revert: true
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

    package init(
        id: String, summary: String, instructions: String, tools: Set<AssistantTool>, draftsPlans: Bool,
        isOn: Bool = true
    ) {
        self.id = id
        self.summary = summary
        self.instructions = instructions
        self.tools = tools
        self.draftsPlans = draftsPlans
        self.isOn = isOn
    }

    package static let userPrefix = "user:"

    /// A skill of the person's, with `name` made into an ID such as
    /// "user:weekly-review". Nil when the name has no letters or digits.
    package init?(
        userNamed name: String, summary: String, instructions: String, tools: Set<AssistantTool>,
        draftsPlans: Bool
    ) {
        let words = name.lowercased().split { !$0.isLetter && !$0.isNumber }
        guard !words.isEmpty else { return nil }
        self.init(
            id: Self.userPrefix + words.joined(separator: "-"), summary: summary, instructions: instructions,
            tools: tools, draftsPlans: draftsPlans)
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
        tools: [.findTasks, .getTask, .overview, .recentActivity], draftsPlans: false)

    package static let add = Skill(
        id: "add", summary: String(localized: "Plan new tasks, Projects and Avenues."),
        instructions: """
            Help the person plan what to add. Ask what's missing, then put the whole plan in "plan".
            """,
        tools: [.findTasks, .getTask], draftsPlans: true)

    package static let tidy = Skill(
        id: "tidy", summary: String(localized: "Clean up titles, priorities, dates and labels."),
        instructions: """
            Help tidy existing tasks: clearer titles, missing priorities and due dates, consistent labels. \
            Say what you'd change first, and change it with updateTasks once the person agrees.
            """,
        tools: [.findTasks, .getTask, .overview, .updateTasks], draftsPlans: false)

    package static let review = Skill(
        id: "review", summary: String(localized: "Look back at what's done, overdue and stalled."),
        instructions: """
            Review the person's week with them: what got done, what's overdue, and what has stalled. Suggest \
            what to drop or move, and add a comment to a task when they ask.
            """,
        tools: [.findTasks, .getTask, .overview, .recentActivity, .addComment], draftsPlans: false)

    package static let undo = Skill(
        id: "undo", summary: String(localized: "Put back recent changes."),
        instructions: """
            Help the person put back changes. Find them with recentActivity, say which you'd revert, and \
            revert them once the person agrees.
            """,
        tools: [.recentActivity, .revert], draftsPlans: false)

    package static let builtIn: [Skill] = [.query, .add, .tidy, .review, .undo]

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
    /// for `isOn`, so their instructions always come from this version.
    package static func merged(stored: [Skill]) -> [Skill] {
        let byID = Dictionary(stored.map { ($0.id, $0) }) { $1 }
        let builtIn = Self.builtIn.map { skill in
            var skill = skill
            skill.isOn = byID[skill.id]?.isOn ?? true
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
