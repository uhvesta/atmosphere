import Foundation

/// The tools an app's assistant can call, and the skills it comes with.
public protocol SkillTool: Hashable, Codable, Sendable, CaseIterable {
    /// Whether the tool changes the app's records. What it would change
    /// comes back as a `Proposal` for the person to decide on.
    var writes: Bool { get }
    /// The skills the app comes with, in the order they're listed.
    static var builtInSkills: [Skill<Self>] { get }
}

/// What the assistant does with a message: instructions, the tools it may
/// call, and whether it drafts plans. An app has a few built in; the
/// person can write their own, whose IDs start with `user:`.
public struct Skill<Tool: SkillTool>: Identifiable, Hashable, Codable, Sendable {
    /// "query" for a built-in skill, "user:weekly-review" for the person's.
    public var id: String
    public var summary: String
    /// What the assistant is told to do for a message that uses the skill.
    public var instructions: String
    public var tools: Set<Tool>
    public var draftsPlans: Bool
    /// Off skills can't be picked.
    public var isOn: Bool
    /// Whether the app may pick the skill for a message by itself.
    public var autoPicks: Bool
    /// When the skill fits a message, for picking it automatically: "the
    /// person lists things they have to do".
    public var useWhen: String

    public init(
        id: String, summary: String, instructions: String, tools: Set<Tool>, draftsPlans: Bool,
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
    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let id = try container.decode(String.self, forKey: .id)
        let builtIn = Tool.builtInSkills.first { $0.id == id }
        self.init(
            id: id, summary: try container.decode(String.self, forKey: .summary),
            instructions: try container.decode(String.self, forKey: .instructions),
            tools: try container.decode(Set<Tool>.self, forKey: .tools),
            draftsPlans: try container.decode(Bool.self, forKey: .draftsPlans),
            isOn: try container.decode(Bool.self, forKey: .isOn),
            autoPicks: try container.decodeIfPresent(Bool.self, forKey: .autoPicks) ?? builtIn?.autoPicks ?? false,
            useWhen: try container.decodeIfPresent(String.self, forKey: .useWhen) ?? builtIn?.useWhen ?? "")
    }

    public static var userPrefix: String { "user:" }

    /// A skill of the person's, with `name` made into an ID such as
    /// "user:weekly-review". Nil when the name has no letters or digits.
    public init?(
        userNamed name: String, summary: String, instructions: String, tools: Set<Tool>,
        draftsPlans: Bool, autoPicks: Bool = false, useWhen: String = ""
    ) {
        let words = name.lowercased().split { !$0.isLetter && !$0.isNumber }
        guard !words.isEmpty else { return nil }
        self.init(
            id: Self.userPrefix + words.joined(separator: "-"), summary: summary, instructions: instructions,
            tools: tools, draftsPlans: draftsPlans, autoPicks: autoPicks, useWhen: useWhen)
    }

    public var isUser: Bool { id.hasPrefix(Self.userPrefix) }

    /// What a pill says: "Query", or "user:weekly-review".
    public var title: String { isUser ? id : id.prefix(1).uppercased() + id.dropFirst() }

    /// Whether any of its tools save changes.
    public var writes: Bool { tools.contains(where: \.writes) }

    /// The built-in skills with their stored on or off, then the stored
    /// skills of the person's by ID. Stored copies of built-ins only count
    /// for `isOn` and `autoPicks`, so their instructions always come from
    /// the app as it is now.
    public static func merged(stored: [Self]) -> [Self] {
        let byID = Dictionary(stored.map { ($0.id, $0) }) { $1 }
        let builtIn = Tool.builtInSkills.map { skill in
            var skill = skill
            if let stored = byID[skill.id] {
                skill.isOn = stored.isOn
                skill.autoPicks = stored.autoPicks
            }
            return skill
        }
        return builtIn + stored.filter(\.isUser).sorted { $0.id < $1.id }
    }

    // MARK: - Search

    /// The skills that match `query`, best first: the title starting with
    /// it, then its letters in order in the title, then every word in the
    /// summary. An empty query matches every skill, in order.
    public static func search(_ query: String, in skills: [Self]) -> [Self] {
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

    // MARK: - Picking

    /// The skills that may be picked for a message by themselves: on, and
    /// marked to pick automatically.
    public static func pickable(_ skills: [Self]) -> [Self] {
        skills.filter { $0.isOn && $0.autoPicks }
    }

    /// The skills that may be picked for a message by themselves in one
    /// chat: none when picking is off there. `chat` is the chat's own
    /// choice, and nil follows `app`, the app-wide one.
    public static func pickable(_ skills: [Self], chat: Bool?, app: Bool) -> [Self] {
        (chat ?? app) ? pickable(skills) : []
    }

    /// The skill a model named, ignoring case, if it's one of `skills`.
    public static func named(_ id: String, in skills: [Self]) -> Self? {
        let id = id.trimmingCharacters(in: .whitespaces)
        return skills.first { $0.id.caseInsensitiveCompare(id) == .orderedSame }
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

/// A skill the assistant wrote from the person's description, for an
/// editor to fill in.
public struct SkillDraft<Tool: SkillTool>: Hashable, Sendable {
    public var name: String
    public var summary: String
    public var instructions: String
    public var tools: Set<Tool>
    public var draftsPlans: Bool

    public init(name: String, summary: String, instructions: String, tools: Set<Tool>, draftsPlans: Bool) {
        self.name = name
        self.summary = summary
        self.instructions = instructions
        self.tools = tools
        self.draftsPlans = draftsPlans
    }
}
