import Foundation
import HumataDaemonClient

/// A skill or plugin command the session's agent has, or a command of
/// Humata's own, offered when a message starts with a /.
package struct SlashChoice: Hashable, Identifiable, Sendable {
    /// A command Humata carries out itself instead of sending it.
    package enum BuiltIn: String, Hashable, Sendable {
        /// Picks a message or a compaction to cut the chat back to.
        case rewind
        /// Has the agent sum up the conversation to make room.
        case compact
        /// Starts an empty chat as the session's child, in its copy of
        /// the code or, with "copy" after it, in a copy of its own.
        case newHere = "new-here"
        /// Forks the chat so far with its own copy of the code, or with
        /// "chat" after it, in the session's copy.
        case fork
        /// Opens a read-only side chat beside this one, with the chat so
        /// far, and sends it what's typed after the name.
        case side
    }

    /// What follows the /, such as "release".
    package var id: String
    package var summary: String
    /// The command of Humata's own it is; nil for the agent's.
    package var builtIn: BuiltIn?

    package init(_ name: String, summary: String = "", builtIn: BuiltIn? = nil) {
        id = name
        self.summary = summary
        self.builtIn = builtIn
    }

    /// Humata's own commands for `session`, none when it's archived:
    /// rewinding, a new chat here and forking, and what its host says it
    /// can do in `commands`, nil when it hasn't said: compacting, and a
    /// read-only side chat.
    package static func builtIns(for session: Session, commands: ChatCommands?) -> [SlashChoice] {
        guard !session.archived else { return [] }
        var choices = [
            SlashChoice(
                BuiltIn.rewind.rawValue, summary: "Go back to before a message or a compaction", builtIn: .rewind)
        ]
        if let commands, commands.compact {
            let summary =
                commands.compactTakesFocus
                ? "Sum up the conversation to make room; add what to keep in mind"
                : "Sum up the conversation to make room"
            choices.append(SlashChoice(BuiltIn.compact.rawValue, summary: summary, builtIn: .compact))
        }
        choices.append(
            SlashChoice(
                BuiltIn.newHere.rawValue,
                summary: "Start an empty chat under this one, in the same copy of the code; add “copy” for its own",
                builtIn: .newHere))
        choices.append(
            SlashChoice(
                BuiltIn.fork.rawValue,
                summary: "Fork the chat so far with its own copy of the code; add “chat” to share this copy",
                builtIn: .fork))
        // Only where the host holds it to reading.
        if let commands, commands.side {
            choices.append(
                SlashChoice(
                    BuiltIn.side.rawValue,
                    summary: "Ask on the side, in a read-only chat beside this one; add the question",
                    builtIn: .side))
        }
        return choices
    }

    /// The command of Humata's own `draft` asks for, with what's typed
    /// after its name: "/compact keep the API notes" is compacting with
    /// "keep the API notes". Nil for any other draft.
    package static func command(in draft: String) -> (command: BuiltIn, text: String)? {
        let draft = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard draft.hasPrefix("/") else { return nil }
        let name = draft.dropFirst().prefix { !$0.isWhitespace }
        guard let command = BuiltIn(rawValue: String(name)) else { return nil }
        let text = draft.dropFirst(name.count + 1).trimmingCharacters(in: .whitespacesAndNewlines)
        return (command, text)
    }

    /// What `session`'s agent runs when sent as "/name", out of every
    /// host's skills and plugins: its host's, those its backend reads, its
    /// project's own for a model the daemon runs, and the plugins on for
    /// its project or, where the project doesn't say, its CLI. One
    /// of each name, the one picked where two files differ. Humata's own
    /// commands come first, each in place of a skill of the same name.
    package static func offered(
        to session: Session, skills: SkillList, plugins: PluginList, project: Project?, commands: ChatCommands?
    ) -> [SlashChoice] {
        let host = session.host ?? ""
        let origins: Set<FoundSkill.Origin> =
            switch session.backend {
            // The project's own are Humata's, which only the daemon can
            // write out for "/name": it does for the models it runs, while
            // the CLIs run only the skills they find themselves.
            case .claude: [.claudeUser, .claudeRepository]
            case .copilot: [.copilotUser, .githubRepository, .claudeRepository]
            case .local, .scripted: [.claudeRepository, .githubRepository, .project]
            }
        let winners = Set(skills.clashes.compactMap(\.winner))
        let clashing = Set(skills.clashes.filter { $0.winner != nil }.map(\.name))
        var choices: [String: SlashChoice] = [:]
        for skill in skills.skills where skill.host == host && origins.contains(skill.origin) {
            switch skill.origin {
            case .claudeRepository, .githubRepository, .project:
                guard let project = session.projectID, skill.projectID == project else { continue }
            case .claudeUser, .copilotUser:
                break
            }
            if clashing.contains(skill.name), !winners.contains(skill.hash) { continue }
            if choices[skill.name] == nil || winners.contains(skill.hash) {
                choices[skill.name] = SlashChoice(skill.name, summary: skill.description)
            }
        }
        let agent: Plugin.Agent? =
            switch session.backend {
            case .claude: .claude
            case .copilot: .copilot
            case .local, .scripted: nil
            }
        for plugin in plugins.plugins where plugin.host == host && plugin.agent == agent {
            guard project?.plugin(plugin.agent, plugin.pluginID) ?? plugin.enabled else { continue }
            for name in plugin.skills + plugin.commands where choices[name] == nil {
                choices[name] = SlashChoice(name, summary: "From the \(plugin.name) plugin")
            }
        }
        let own = Self.builtIns(for: session, commands: commands)
        for choice in own { choices[choice.id] = nil }
        return own + choices.values.sorted { $0.id < $1.id }
    }

    /// The choices for `query`, typed after the /, best first: names that
    /// start with it, then those with a word that does, then those that
    /// hold it, then those with its letters in order. Case is ignored.
    package static func matching(_ query: String, in choices: [SlashChoice], limit: Int = 50) -> [SlashChoice] {
        let query = query.lowercased()
        guard !query.isEmpty else { return Array(choices.prefix(limit)) }
        var scored: [(rank: Int, choice: SlashChoice)] = []
        for choice in choices {
            if let rank = rank(of: choice.id.lowercased(), for: query) { scored.append((rank, choice)) }
        }
        scored.sort { one, other in
            if one.rank != other.rank { return one.rank < other.rank }
            // Humata's own ahead of the agent's, so the order is the same
            // each time.
            if (one.choice.builtIn == nil) != (other.choice.builtIn == nil) { return one.choice.builtIn != nil }
            if one.choice.id.count != other.choice.id.count { return one.choice.id.count < other.choice.id.count }
            return one.choice.id < other.choice.id
        }
        return scored.prefix(limit).map(\.choice)
    }

    /// How well `name` answers `query`, both in lower case; lower is
    /// better, nil not at all.
    fileprivate static func rank(of name: String, for query: String) -> Int? {
        if name.hasPrefix(query) { return 0 }
        let words = name.split { $0 == "-" || $0 == "_" || $0 == ":" || $0 == "." || $0.isWhitespace }
        if words.contains(where: { $0.hasPrefix(query) }) { return 1 }
        if name.contains(query) { return 2 }
        var rest = query[...]
        for character in name where rest.first == character { rest = rest.dropFirst() }
        return rest.isEmpty ? 3 : nil
    }

    /// The query in `draft` when it is a / and a name being typed, nothing
    /// after it yet; nil otherwise.
    package static func query(in draft: String) -> String? {
        guard draft.hasPrefix("/"), !draft.contains(where: \.isWhitespace) else { return nil }
        return String(draft.dropFirst())
    }
}

/// Somewhere a chat can be cut back to: before a message the person sent,
/// or before a compaction.
package struct RewindPoint: Identifiable, Hashable, Sendable {
    package enum Kind: Hashable, Sendable {
        /// The message's start: enough to tell it by and to search.
        case message(String)
        case compaction
    }

    /// The event rewound to.
    package var id: String
    package var kind: Kind
    package var time: Date

    /// Characters of a message kept: one can be megabytes of pasted text.
    private static let kept = 2000

    /// Where the chat of `events` can be cut back to, newest first: the
    /// session's own agent's, not a subagent's.
    package static func points(in events: some Sequence<SessionEvent>) -> [RewindPoint] {
        var points: [RewindPoint] = []
        for event in events where event.parentCallID == nil {
            switch event.kind {
            case .userMessage(let text, _):
                points.append(RewindPoint(id: event.id, kind: .message(String(text.prefix(kept))), time: event.time))
            case .compacted:
                points.append(RewindPoint(id: event.id, kind: .compaction, time: event.time))
            default:
                continue
            }
        }
        return points.reversed()
    }

    /// The points for `query`, as the / list ranks names: messages that
    /// start with it, then those with a word that does, then those that
    /// hold it; the newest first among equals. The newest for no query.
    package static func matching(_ query: String, in points: [RewindPoint], limit: Int = 100) -> [RewindPoint] {
        let query = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !query.isEmpty else { return Array(points.prefix(limit)) }
        var scored: [(rank: Int, index: Int)] = []
        for (index, point) in points.enumerated() {
            let text =
                switch point.kind {
                case .message(let text): text.lowercased()
                case .compaction: "compaction context compacted"
                }
            // Letters in order alone would find nearly every long message.
            if let rank = SlashChoice.rank(of: text, for: query), rank < 3 { scored.append((rank, index)) }
        }
        scored.sort { $0.rank != $1.rank ? $0.rank < $1.rank : $0.index < $1.index }
        return scored.prefix(limit).map { points[$0.index] }
    }
}
