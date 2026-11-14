import Foundation

/// Whose switches: agents', or the command line's.
package enum AccessClient: Hashable, Sendable, CaseIterable {
    /// Agents: `humata mcp`, and the humata tools of sessions' own agents.
    case agents
    /// The person's own `humata` in a terminal.
    case terminal
}

/// The group an action's switch is in, by what the action does.
package enum AccessGroup: Hashable, Sendable, CaseIterable {
    case read
    case change
    case destroy
}

/// One action's switch.
package struct ActionAccess: Equatable, Identifiable, Sendable {
    /// What the daemon keeps the switch under.
    package var id: String
    package var title: String
    package var group: AccessGroup
    /// Its own switch; the action runs only while its group's switch and
    /// the main switch are on too.
    package var enabled: Bool

    package init(id: String, title: String, group: AccessGroup, enabled: Bool = true) {
        self.id = id
        self.title = title
        self.group = group
        self.enabled = enabled
    }
}

/// The switches of the agents, or of the command line.
package struct ClientAccess: Equatable, Sendable {
    /// The main switch.
    package var enabled: Bool
    package var read: Bool
    package var change: Bool
    package var destroy: Bool
    /// Every action, in the order to list them.
    package var actions: [ActionAccess]

    package init(
        enabled: Bool = true, read: Bool = true, change: Bool = true, destroy: Bool = false,
        actions: [ActionAccess] = []
    ) {
        self.enabled = enabled
        self.read = read
        self.change = change
        self.destroy = destroy
        self.actions = actions
    }

    /// A group's switch.
    package subscript(group: AccessGroup) -> Bool {
        get {
            switch group {
            case .read: read
            case .change: change
            case .destroy: destroy
            }
        }
        set {
            switch group {
            case .read: read = newValue
            case .change: change = newValue
            case .destroy: destroy = newValue
            }
        }
    }

    /// Whether the action runs: its own switch, its group's and the
    /// main switch are all on.
    package func allows(_ action: ActionAccess) -> Bool {
        enabled && self[action.group] && action.enabled
    }
}

/// What agents and the `humata` command line may do, from Settings › Agent
/// Access. The app is never gated. It stops agents and scripts that go
/// through `humata`, not a program running as the person.
package struct AgentAccess: Equatable, Sendable {
    package var agents: ClientAccess
    package var terminal: ClientAccess

    package init(agents: ClientAccess, terminal: ClientAccess) {
        self.agents = agents
        self.terminal = terminal
    }

    package subscript(client: AccessClient) -> ClientAccess {
        get {
            switch client {
            case .agents: agents
            case .terminal: terminal
            }
        }
        set {
            switch client {
            case .agents: agents = newValue
            case .terminal: terminal = newValue
            }
        }
    }

    /// What the daemon starts with, with a few actions of each group, for
    /// the in-memory daemon: reads and changes on, destroys off.
    package static let standard: AgentAccess = {
        let actions = [
            ActionAccess(id: "list_sessions", title: "List sessions", group: .read),
            ActionAccess(id: "read_history", title: "Read sessions' chats", group: .read),
            ActionAccess(id: "start_session", title: "Start sessions", group: .change),
            ActionAccess(id: "send_message", title: "Send messages", group: .change),
            ActionAccess(id: "archive_session", title: "Archive sessions", group: .destroy),
            ActionAccess(id: "delete_project", title: "Delete projects", group: .destroy),
        ]
        return AgentAccess(agents: ClientAccess(actions: actions), terminal: ClientAccess(actions: actions))
    }()
}
