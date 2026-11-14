import ComposableArchitecture
import Foundation
import HumataDaemonClient
import SwiftUI

/// Settings › Plugins: every Claude Code and Copilot plugin installed on the
/// Mac and each host, what each adds and whether its CLI has it on; and, for
/// a project, which to turn on or off for its sessions. Installing stays in
/// each CLI. Looking on the hosts can take a while, so it shows a spinner
/// meanwhile.
@Reducer
package struct PluginsFeature {
    /// A project's choice for one plugin.
    package enum Choice: Hashable, Sendable {
        /// However its CLI has it.
        case cli
        case on
        case off

        init(_ enabled: Bool?) {
            switch enabled {
            case .some(true): self = .on
            case .some(false): self = .off
            case .none: self = .cli
            }
        }

        var enabled: Bool? {
            switch self {
            case .cli: nil
            case .on: true
            case .off: false
            }
        }
    }

    @ObservableState
    package struct State: Equatable {
        /// Nil until the Mac and hosts were looked through: while they're
        /// looked through the first time, and after a look that failed
        /// with none before it, which isn't a look that found none.
        package var list: PluginList?
        package var projects: [Project] = []
        /// The project whose choices are shown; nil shows only the CLIs'.
        package var projectID: String?
        /// The plugins whose choice is being saved, by agent and name.
        package var saving: Set<String> = []
        /// Why they couldn't be listed or the choice saved, and what to do.
        package var failure: String?
        /// Whether the Mac and hosts are being looked through.
        package var loading = false

        package init(list: PluginList? = nil, projectID: String? = nil) {
            self.list = list
            self.projectID = projectID
        }

        package var project: Project? { projects.first { $0.id == projectID } }

        /// The project's choice for a plugin.
        package func choice(_ plugin: Plugin) -> Choice {
            Choice(project?.plugin(plugin.agent, plugin.pluginID))
        }

        static func key(_ plugin: Plugin) -> String { "\(plugin.agent.rawValue)|\(plugin.pluginID)" }
    }

    package enum Action {
        case task
        case refreshTapped
        case loaded(PluginList, [Project])
        case failed(String)
        case projectPicked(String?)
        case choiceChanged(Plugin, Choice)
        case choiceSaved(String, Project)
        case choiceFailed(String, String)
    }

    @Dependency(\.daemon) var daemon

    private enum CancelID { case load }

    package init() {}

    /// Lists them again; a failure on screen stays until the next.
    private func load() -> Effect<Action> {
        .run { send in
            async let projects = daemon.projects()
            let list = try await daemon.plugins()
            await send(.loaded(list, try await projects))
        } catch: { error, send in
            await send(.failed(error.localizedDescription))
        }
        .cancellable(id: CancelID.load, cancelInFlight: true)
    }

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task, .refreshTapped:
                state.failure = nil
                state.loading = true
                return load()

            case .loaded(let list, let projects):
                state.loading = false
                state.list = list
                state.projects = projects.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
                if state.project == nil { state.projectID = state.projects.first?.id }
                return .none

            case .failed(let message):
                state.loading = false
                state.failure = message
                return .none

            case .projectPicked(let id):
                state.projectID = id
                return .none

            case .choiceChanged(let plugin, let choice):
                guard let project = state.projectID,
                    let index = state.projects.firstIndex(where: { $0.id == project })
                else { return .none }
                let key = State.key(plugin)
                state.projects[index].plugins.removeAll { $0.agent == plugin.agent && $0.id == plugin.pluginID }
                if let enabled = choice.enabled {
                    state.projects[index].plugins.append(
                        PluginChoice(agent: plugin.agent, id: plugin.pluginID, enabled: enabled))
                }
                state.saving.insert(key)
                state.failure = nil
                return .run { send in
                    let saved = try await daemon.setProjectPlugin(
                        project, plugin.agent, plugin.pluginID, choice.enabled)
                    await send(.choiceSaved(key, saved))
                } catch: { error, send in
                    await send(.choiceFailed(key, error.localizedDescription))
                }

            case .choiceSaved(let key, let project):
                state.saving.remove(key)
                if let index = state.projects.firstIndex(where: { $0.id == project.id }) {
                    state.projects[index] = project
                }
                return .none

            case .choiceFailed(let key, let message):
                state.saving.remove(key)
                state.failure = message
                state.loading = true
                return load()
            }
        }
    }
}

package struct PluginsView: View {
    @Bindable var store: StoreOf<PluginsFeature>

    package init(store: StoreOf<PluginsFeature>) {
        self.store = store
    }

    package var body: some View {
        Group {
            if let list = store.list {
                content(list)
            } else if let failure = store.failure, !store.loading {
                // Never listed: the failure and a way to look again, not
                // a list that says there are none.
                VStack(spacing: 12) {
                    Label(failure, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                        .textSelection(.enabled)
                    Text("The plugins couldn’t be listed. Check that Humata’s daemon is running, then try again.")
                        .foregroundStyle(.secondary)
                    Button("Try Again") { store.send(.refreshTapped) }
                }
                .multilineTextAlignment(.center)
                .padding()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                VStack(spacing: 8) {
                    ProgressView()
                    Text("Looking for plugins on this Mac and each host…")
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .toolbar {
            // The first look shows its own spinner in place of the list.
            if store.loading, store.list != nil {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Looking again…")
                        .foregroundStyle(.secondary)
                }
            }
            Button("Look Again", systemImage: "arrow.clockwise") { store.send(.refreshTapped) }
                .disabled(store.loading || store.list == nil)
        }
        .task { await store.send(.task).finish() }
    }

    private func content(_ list: PluginList) -> some View {
        Form {
            if let failure = store.failure {
                Section {
                    Label(failure, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                }
            }
            if !store.projects.isEmpty, !list.plugins.isEmpty {
                Section {
                    Picker("Project", selection: $store.projectID.sending(\.projectPicked)) {
                        ForEach(store.projects) { project in
                            Text(project.name).tag(String?.some(project.id))
                        }
                    }
                } footer: {
                    Text(
                        "Turn a plugin on or off for this project's sessions, on every host, over how its CLI has it. "
                            + "A change reaches a session when it next opens. Copilot reads the choice from the "
                            + "session's copies of the repositories, so it doesn't reach a chat in a folder."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
            ForEach(groups(list), id: \.title) { group in
                Section(group.title) {
                    ForEach(group.plugins) { plugin in
                        row(plugin)
                    }
                }
            }
            if list.plugins.isEmpty {
                Section {
                    Text(
                        "No plugins yet. Install one with `claude plugin install` or `copilot plugin install`, "
                            + "then look again."
                    )
                    .foregroundStyle(.secondary)
                }
            }
            if !list.unreachable.isEmpty {
                Section {
                    Text(
                        "Couldn't reach \(list.unreachable.formatted(.list(type: .and))), so its plugins aren't "
                            + "listed. Check the host in Environments, then look again."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
        }
        .formStyle(.grouped)
    }

    private func row(_ plugin: Plugin) -> some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(plugin.name)
                    if !plugin.version.isEmpty {
                        Text(plugin.version).font(.caption).foregroundStyle(.tertiary)
                    }
                }
                if !plugin.description.isEmpty {
                    Text(plugin.description)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                Text(caption(plugin))
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
            Spacer()
            if store.saving.contains(PluginsFeature.State.key(plugin)) {
                ProgressView().controlSize(.small)
            }
            if store.project != nil {
                Picker(
                    plugin.name,
                    selection: Binding(
                        get: { store.state.choice(plugin) },
                        set: { store.send(.choiceChanged(plugin, $0)) })
                ) {
                    Text(plugin.enabled ? "As the CLI Has It (On)" : "As the CLI Has It (Off)")
                        .tag(PluginsFeature.Choice.cli)
                    Text("On").tag(PluginsFeature.Choice.on)
                    Text("Off").tag(PluginsFeature.Choice.off)
                }
                .labelsHidden()
                .fixedSize()
            } else {
                Text(plugin.enabled ? "On" : "Off")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func caption(_ plugin: Plugin) -> String {
        let adds = plugin.adds
        return adds.isEmpty ? plugin.pluginID : "\(plugin.pluginID) · adds \(adds)"
    }

    /// The plugins by host, then by agent.
    private func groups(_ list: PluginList) -> [(title: String, plugins: [Plugin])] {
        var groups: [(title: String, plugins: [Plugin])] = []
        for host in Set(list.plugins.map(\.host)).sorted() {
            for agent in Plugin.Agent.allCases {
                let theirs = list.plugins
                    .filter { $0.host == host && $0.agent == agent }
                    .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
                guard !theirs.isEmpty else { continue }
                groups.append(("\(agent.title) \(host.isEmpty ? "on This Mac" : "on \(host)")", theirs))
            }
        }
        return groups
    }
}
