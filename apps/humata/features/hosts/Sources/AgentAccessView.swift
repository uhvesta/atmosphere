import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// Settings › Agent Access.
package struct AgentAccessView: View {
    let store: StoreOf<AgentAccessFeature>
    /// Whose switches are shown.
    @State private var client = AccessClient.agents

    package init(store: StoreOf<AgentAccessFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                Picker("Switches for", selection: $client) {
                    Text("Agents").tag(AccessClient.agents)
                    Text("Terminal").tag(AccessClient.terminal)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
            } footer: {
                Text(
                    """
                    These switches decide what the `humata` command can do: for agents, through `humata mcp` \
                    and the tools Humata gives its own sessions' agents; for Terminal, when you run `humata` \
                    yourself. Humata's daemon refuses what is off, and the app itself is never held back. \
                    This stops agents and scripts that go through `humata`. It isn't a lock against a \
                    program running as you, which could change Humata's files or claim to be the app.
                    """
                )
                .foregroundStyle(.secondary)
            }
            if let access = store.access {
                switches(access[client])
            } else if store.failure == nil {
                HStack(spacing: 8) {
                    ProgressView()
                        .controlSize(.small)
                    Text("Reading the settings…")
                        .foregroundStyle(.secondary)
                }
            }
            if let failure = store.failure {
                VStack(alignment: .leading, spacing: 6) {
                    Text(failure)
                        .foregroundStyle(.red)
                    if store.access == nil {
                        Button("Try Again") { store.send(.task) }
                    }
                }
            }
        }
        .formStyle(.grouped)
        .task { await store.send(.task).finish() }
    }

    @ViewBuilder
    private func switches(_ access: ClientAccess) -> some View {
        Section {
            Toggle(
                Self.mainSwitch(client),
                isOn: Binding(get: { access.enabled }, set: { store.send(.mainSwitchToggled(client, $0)) })
            )
        } header: {
            HStack(spacing: 8) {
                Text(Self.name(client))
                if store.saving {
                    ProgressView()
                        .controlSize(.small)
                }
            }
        } footer: {
            if !access.enabled {
                Text(Self.allOff(client))
                    .foregroundStyle(.secondary)
            }
        }
        .disabled(store.saving)
        ForEach(AccessGroup.allCases, id: \.self) { group in
            let actions = access.actions.filter { $0.group == group }
            if !actions.isEmpty {
                Section {
                    Toggle(
                        isOn: Binding(
                            get: { access[group] }, set: { store.send(.groupToggled(client, group, $0)) })
                    ) {
                        Text(Self.all(group))
                            .fontWeight(.medium)
                    }
                    .disabled(!access.enabled)
                    ForEach(actions) { action in
                        Toggle(
                            action.title,
                            isOn: Binding(
                                get: { action.enabled },
                                set: { store.send(.actionToggled(client, id: action.id, $0)) })
                        )
                        .disabled(!access.enabled || !access[group])
                    }
                } header: {
                    Text(Self.name(group))
                } footer: {
                    Text(Self.about(group))
                        .foregroundStyle(.secondary)
                }
                .disabled(store.saving)
            }
        }
    }

    private static func name(_ client: AccessClient) -> String {
        switch client {
        case .agents: String(localized: "Agents")
        case .terminal: String(localized: "Terminal")
        }
    }

    private static func mainSwitch(_ client: AccessClient) -> String {
        switch client {
        case .agents: String(localized: "Let agents use Humata")
        case .terminal: String(localized: "Let the humata command use Humata")
        }
    }

    private static func allOff(_ client: AccessClient) -> String {
        switch client {
        case .agents:
            String(localized: "Off: agents are refused everything below, whatever its switch says.")
        case .terminal:
            String(localized: "Off: the humata command is refused everything below, whatever its switch says.")
        }
    }

    private static func name(_ group: AccessGroup) -> String {
        switch group {
        case .read: String(localized: "Read")
        case .change: String(localized: "Change")
        case .destroy: String(localized: "Destroy")
        }
    }

    private static func all(_ group: AccessGroup) -> String {
        switch group {
        case .read: String(localized: "Allow reading")
        case .change: String(localized: "Allow changes")
        case .destroy: String(localized: "Allow destroying")
        }
    }

    private static func about(_ group: AccessGroup) -> String {
        switch group {
        case .read:
            String(localized: "Looking at sessions, boards and settings; nothing changes.")
        case .change:
            String(localized: "Starting and directing sessions and changing what Humata keeps, short of losing work.")
        case .destroy:
            String(localized: "Actions that remove something or throw work away. Off until you turn them on.")
        }
    }
}
