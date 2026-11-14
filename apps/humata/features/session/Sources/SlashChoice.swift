import Foundation
import HumataDaemonClient

/// A skill or plugin command the session's agent has, offered when a
/// message starts with a /.
package struct SlashChoice: Hashable, Identifiable, Sendable {
    /// What follows the /, such as "release".
    package var id: String
    package var summary: String

    package init(_ name: String, summary: String = "") {
        id = name
        self.summary = summary
    }

    /// What `session`'s agent has, out of every host's skills and plugins:
    /// its host's, those its backend reads, its project's, and the plugins
    /// on for its project or, where the project doesn't say, its CLI. One
    /// of each name, the one picked where two files differ.
    package static func offered(
        to session: Session, skills: SkillList, plugins: PluginList, project: Project?
    ) -> [SlashChoice] {
        let host = session.host ?? ""
        let origins: Set<FoundSkill.Origin> =
            switch session.backend {
            case .claude: [.claudeUser, .claudeRepository, .project]
            case .copilot: [.copilotUser, .githubRepository, .claudeRepository, .project]
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
        return choices.values.sorted { $0.id < $1.id }
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
        scored.sort { $0.rank != $1.rank ? $0.rank < $1.rank : $0.choice.id.count < $1.choice.id.count }
        return scored.prefix(limit).map(\.choice)
    }

    private static func rank(of name: String, for query: String) -> Int? {
        if name.hasPrefix(query) { return 0 }
        let words = name.split { $0 == "-" || $0 == "_" || $0 == ":" || $0 == "." || $0 == " " }
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
