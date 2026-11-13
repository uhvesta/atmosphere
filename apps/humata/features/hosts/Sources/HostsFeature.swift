import ComposableArchitecture
import Foundation
import HumataBrowser
import HumataDaemonClient
import HumataTerminal

/// The machines sessions run on over SSH: each one's health, checked when
/// the list shows and on request; adding one from ~/.ssh/config; installing
/// Humata's program on one; its agents' variables and setup script; the
/// sessions on it that couldn't be opened; the Mac's copies of its sessions,
/// to restore those it lost or move them elsewhere; its terminals; its files;
/// removing one; and for this Mac, linking its commands where a terminal
/// finds them.
@Reducer
package struct HostsFeature {
    @ObservableState
    package struct State: Equatable {
        package var hosts: IdentifiedArrayOf<RemoteHost> = []
        /// This Mac, the coordinator: its id, "", selects it.
        package var coordinator = RemoteHost.coordinator()
        package var health: [RemoteHost.ID: Health] = [:]
        package var selection: RemoteHost.ID?
        /// The selected host's variables and setup script.
        package var editor: HostEditorFeature.State?
        /// The selected host's sessions whose files couldn't be read.
        package var skipped: IdentifiedArrayOf<SkippedSession> = []
        /// The Mac's copies of the selected host's sessions.
        package var recoveries: IdentifiedArrayOf<Recovery> = []
        /// The sessions being restored or moved.
        package var restoring: Set<Recovery.ID> = []
        /// Why the hosts couldn't be read, and what to do about it.
        package var failure: String?
        /// What the selected host shows: its settings, terminals, files or ports.
        package var pane = Pane.settings
        /// The selected host's terminals, whether or not a session opened them.
        package var terminals: TerminalsFeature.State?
        /// The selected host's files, from its home.
        package var files: FileBrowserFeature.State?
        /// The selected host's forwarded ports.
        package var ports: PortsFeature.State?
        /// How this Mac's commands are linked, while it's selected.
        package var commandLinks: CommandLinks?
        package var linking = false
        @Presents package var destination: Destination.State?

        package init() {}

        package var selected: RemoteHost? {
            selection.flatMap { $0 == coordinator.id ? coordinator : hosts[id: $0] }
        }
    }

    package enum Pane: Hashable, Sendable {
        case settings
        case terminals
        case files
        case ports
    }

    /// What's known of a host's health.
    package enum Health: Equatable, Sendable {
        case checking
        case installing
        case known(HostHealth)
    }

    @Reducer
    package enum Destination {
        case add(AddHostFeature)
        case alert(AlertState<Alert>)

        package enum Alert: Equatable {
            case confirmRemove(RemoteHost.ID)
            case confirmRemoveSkipped(SkippedSession.ID)
        }
    }

    package enum Action {
        /// The list appeared: reads the hosts and checks each.
        case task
        case loaded([RemoteHost])
        case coordinatorLoaded(RemoteHost)
        case loadFailed(String)
        case selected(RemoteHost.ID?)
        case addTapped
        case checkTapped(RemoteHost.ID)
        case checked(RemoteHost.ID, HostHealth)
        case installTapped(RemoteHost.ID)
        case removeTapped(RemoteHost.ID)
        case removed(RemoteHost.ID)
        case skippedLoaded(RemoteHost.ID, [SkippedSession])
        case removeSkippedTapped(SkippedSession.ID)
        case skippedRemoved(RemoteHost.ID, SkippedSession.ID)
        case recoveriesLoaded(RemoteHost.ID, [Recovery])
        /// Restores the session on the selected host, if that lost it.
        case restoreTapped(Recovery.ID)
        /// Restores every session the selected host lost.
        case restoreAllTapped
        /// Moves the session to `host`, or this Mac if nil, from its copy.
        case moveTapped(Recovery.ID, to: RemoteHost.ID?)
        case restored(RemoteHost.ID, [Recovery.ID])
        case restoreFailed(RemoteHost.ID, [Recovery.ID], title: String, message: String)
        case failed(title: String, message: String)
        case editor(HostEditorFeature.Action)
        case paneSelected(Pane)
        case commandLinksLoaded(CommandLinks)
        case linkCommandsTapped
        case linkFailed(String)
        case terminals(TerminalsFeature.Action)
        case files(FileBrowserFeature.Action)
        case ports(PortsFeature.Action)
        case destination(PresentationAction<Destination.Action>)
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                return .run { send in
                    await send(.loaded(try await daemon.hosts()))
                    await send(.coordinatorLoaded(try await daemon.coordinator()))
                } catch: { error, send in
                    await send(.loadFailed(error.localizedDescription))
                }

            case .loaded(let hosts):
                state.failure = nil
                state.hosts = IdentifiedArray(hosts, uniquingIDsWith: { _, last in last })
                let selecting =
                    state.selected == nil ? select(state.hosts.first?.id ?? state.coordinator.id, in: &state) : .none
                return .merge([selecting] + hosts.map { check($0.id, in: &state) })

            case .coordinatorLoaded(let coordinator):
                state.coordinator = coordinator
                if state.selection == coordinator.id, state.editor?.update == nil {
                    state.editor = HostEditorFeature.State(host: coordinator)
                }
                return .none

            case .loadFailed(let message):
                state.failure = message
                return .none

            case .selected(let id):
                return select(id, in: &state)

            case .addTapped:
                state.destination = .add(AddHostFeature.State())
                return .none

            case .checkTapped(let id):
                return check(id, in: &state)

            case .checked(let id, let health):
                state.health[id] = .known(health)
                // Reachable now, it can say what it skipped.
                guard id == state.selection, case .reachable = health else { return .none }
                return loadSkipped(id)

            case .recoveriesLoaded(let id, let recoveries):
                guard id == state.selection else { return .none }
                state.recoveries = IdentifiedArray(
                    recoveries.filter { $0.host == id }, uniquingIDsWith: { _, last in last })
                return .none

            case .restoreTapped(let id):
                guard let host = state.selection, !state.restoring.contains(id) else { return .none }
                state.restoring.insert(id)
                return .run { send in
                    _ = try await daemon.restoreSession(id, host)
                    await send(.restored(host, [id]))
                } catch: { error, send in
                    await send(
                        .restoreFailed(
                            host, [id], title: "Couldn't restore the session", message: error.localizedDescription))
                }

            case .restoreAllTapped:
                guard let host = state.selection else { return .none }
                let lost = state.recoveries.filter { !$0.onHost && !state.restoring.contains($0.id) }.map(\.id)
                guard !lost.isEmpty else { return .none }
                state.restoring.formUnion(lost)
                return .run { send in
                    let restore = try await daemon.restoreHost(host)
                    if restore.failed.isEmpty {
                        await send(.restored(host, lost))
                    } else {
                        let message = restore.failed.map { "\($0.id): \($0.message)" }.joined(separator: "\n\n")
                        let title =
                            restore.failed.count == 1
                            ? "Couldn't restore 1 session" : "Couldn't restore \(restore.failed.count) sessions"
                        await send(.restoreFailed(host, lost, title: title, message: message))
                    }
                } catch: { error, send in
                    await send(
                        .restoreFailed(
                            host, lost, title: "Couldn't restore the sessions", message: error.localizedDescription))
                }

            case .moveTapped(let id, let target):
                guard let host = state.selection, !state.restoring.contains(id) else { return .none }
                state.restoring.insert(id)
                return .run { send in
                    _ = try await daemon.restoreSession(id, target)
                    await send(.restored(host, [id]))
                } catch: { error, send in
                    await send(
                        .restoreFailed(
                            host, [id], title: "Couldn't move the session", message: error.localizedDescription))
                }

            case .restored(let host, let ids):
                state.restoring.subtract(ids)
                guard host == state.selection else { return .none }
                return loadRecoveries(host)

            case .restoreFailed(let host, let ids, let title, let message):
                state.restoring.subtract(ids)
                state.destination = .alert(
                    AlertState {
                        TextState(title)
                    } message: {
                        TextState(message)
                    })
                guard host == state.selection else { return .none }
                return loadRecoveries(host)

            case .skippedLoaded(let id, let skipped):
                guard id == state.selection else { return .none }
                state.skipped = IdentifiedArray(skipped, uniquingIDsWith: { _, last in last })
                return .none

            case .removeSkippedTapped(let id):
                guard let session = state.skipped[id: id] else { return .none }
                state.destination = .alert(
                    AlertState {
                        TextState("Remove the session in \(session.folder)?")
                    } actions: {
                        ButtonState(role: .destructive, action: .confirmRemoveSkipped(id)) { TextState("Remove") }
                        ButtonState(role: .cancel) { TextState("Cancel") }
                    } message: {
                        TextState(
                            "Its folder and history are deleted, and can't be brought back. To keep the session "
                                + "instead, fix what's damaged and restart Humata.")
                    })
                return .none

            case .destination(.presented(.alert(.confirmRemoveSkipped(let id)))):
                guard let host = state.selection, let session = state.skipped[id: id] else { return .none }
                return .run { send in
                    try await daemon.removeSkipped(session, host.isEmpty ? nil : host)
                    await send(.skippedRemoved(host, id))
                } catch: { error, send in
                    await send(.failed(title: "Couldn't remove the session", message: error.localizedDescription))
                }

            case .skippedRemoved(let host, let id):
                if host == state.selection { state.skipped.remove(id: id) }
                return .none

            case .installTapped(let id):
                state.health[id] = .installing
                return .run { send in
                    _ = try await daemon.installHost(id)
                    await send(.checked(id, try await daemon.checkHost(id)))
                } catch: { error, send in
                    await send(.checked(id, .unreachable(error.localizedDescription)))
                }

            case .removeTapped(let id):
                state.destination = .alert(
                    AlertState {
                        TextState("Remove \(id)?")
                    } actions: {
                        ButtonState(role: .destructive, action: .confirmRemove(id)) { TextState("Remove") }
                        ButtonState(role: .cancel) { TextState("Cancel") }
                    } message: {
                        TextState(
                            "Humata forgets the host and its sessions. Their code and Humata's program stay on the "
                                + "host; remove its Humata folder there to free the space.")
                    })
                return .none

            case .destination(.presented(.alert(.confirmRemove(let id)))):
                return .run { send in
                    try await daemon.removeHost(id)
                    await send(.removed(id))
                } catch: { error, send in
                    await send(.failed(title: "Couldn't remove \(id)", message: error.localizedDescription))
                }

            case .removed(let id):
                state.hosts.remove(id: id)
                state.health[id] = nil
                guard state.selection == id else { return .none }
                return select(state.hosts.first?.id ?? state.coordinator.id, in: &state)

            case .failed(let title, let message):
                state.destination = .alert(
                    AlertState {
                        TextState(title)
                    } message: {
                        TextState(message)
                    })
                return .none

            case .destination(.presented(.add(.delegate(.added(let host))))):
                state.destination = nil
                state.hosts.append(host)
                return .merge(select(host.id, in: &state), check(host.id, in: &state))

            case .editor(.delegate(.saved(let host))) where host.isCoordinator:
                state.coordinator = host
                return .none

            case .editor(.delegate(.saved(let host))):
                state.hosts[id: host.id] = host
                // Saving sets it up again, which the check reports on.
                return check(host.id, in: &state)

            case .paneSelected(let pane):
                state.pane = pane
                return .none

            case .commandLinksLoaded(let links):
                state.linking = false
                state.commandLinks = links
                return .none

            case .linkCommandsTapped:
                guard !state.linking else { return .none }
                state.linking = true
                return .run { send in
                    await send(.commandLinksLoaded(try await daemon.linkCommands(false)))
                } catch: { error, send in
                    await send(.linkFailed(error.localizedDescription))
                }

            case .linkFailed(let message):
                state.linking = false
                state.destination = .alert(
                    AlertState {
                        TextState("Couldn't link the commands")
                    } message: {
                        TextState(message)
                    })
                return .none

            case .editor, .destination, .terminals, .files, .ports:
                return .none
            }
        }
        .ifLet(\.editor, action: \.editor) { HostEditorFeature() }
        .ifLet(\.terminals, action: \.terminals) { TerminalsFeature() }
        .ifLet(\.files, action: \.files) { FileBrowserFeature() }
        .ifLet(\.ports, action: \.ports) { PortsFeature() }
        .ifLet(\.$destination, action: \.destination)
    }

    private func select(_ id: RemoteHost.ID?, in state: inout State) -> Effect<Action> {
        state.selection = id
        state.editor = state.selected.map(HostEditorFeature.State.init)
        state.terminals = id.map {
            TerminalsFeature.State(place: HostPlace(host: $0.isEmpty ? nil : $0), opensFirst: false)
        }
        state.files = id.map { FileBrowserFeature.State(place: HostPlace(host: $0.isEmpty ? nil : $0)) }
        state.ports = id.map { PortsFeature.State(host: $0.isEmpty ? nil : $0) }
        state.skipped = []
        state.recoveries = []
        state.commandLinks = nil
        guard let id else { return .none }
        guard !id.isEmpty else { return .merge(loadSkipped(id), loadCommandLinks()) }
        return .merge(loadSkipped(id), loadRecoveries(id))
    }

    /// Copies are kept on this Mac, so they list even while the host is
    /// unreachable.
    private func loadRecoveries(_ id: RemoteHost.ID) -> Effect<Action> {
        .run { send in
            await send(.recoveriesLoaded(id, (try? await daemon.recoveries()) ?? []))
        }
        .cancellable(id: RecoveriesLoad.host(id), cancelInFlight: true)
    }

    /// Without an answer, nothing about them shows.
    private func loadCommandLinks() -> Effect<Action> {
        .run { send in
            if let links = try? await daemon.linkCommands(true) {
                await send(.commandLinksLoaded(links))
            }
        }
    }

    /// One that can't be reached has none to list until it can be.
    private func loadSkipped(_ id: RemoteHost.ID) -> Effect<Action> {
        .run { send in
            await send(.skippedLoaded(id, (try? await daemon.skipped(id.isEmpty ? nil : id)) ?? []))
        }
        .cancellable(id: SkippedLoad.host(id), cancelInFlight: true)
    }

    private func check(_ id: RemoteHost.ID, in state: inout State) -> Effect<Action> {
        state.health[id] = .checking
        return .run { send in
            await send(.checked(id, try await daemon.checkHost(id)))
        } catch: { error, send in
            await send(.checked(id, .unreachable(error.localizedDescription)))
        }
    }
}

// Keyed by host: a load for one host that starts late can't cancel the
// load for the host picked after it.
private enum SkippedLoad: Hashable { case host(RemoteHost.ID) }
private enum RecoveriesLoad: Hashable { case host(RemoteHost.ID) }

extension HostsFeature.Destination.State: Equatable {}

/// A new host, by its SSH destination: one from ~/.ssh/config or typed.
@Reducer
package struct AddHostFeature {
    @ObservableState
    package struct State: Equatable {
        /// What Humata calls it.
        package var name = ""
        /// What `ssh` connects to: a Host in ~/.ssh/config, or user@address.
        package var destination = ""
        package var sshHosts: [String] = []
        package var adding = false
        @Presents package var alert: AlertState<Never>?

        package init() {}

        package var canAdd: Bool {
            !adding && !name.trimmed.isEmpty && !destination.trimmed.isEmpty
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The sheet appeared: reads ~/.ssh/config's hosts to pick from.
        case task
        case sshHostsLoaded([String])
        /// Fills in the destination, and the name if it has none yet.
        case sshHostPicked(String)
        case addTapped
        case failed(String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case added(RemoteHost)
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .alert, .delegate:
                return .none

            case .task:
                return .run { send in
                    // Without them, the destination is typed.
                    await send(.sshHostsLoaded((try? await daemon.sshHosts()) ?? []))
                }

            case .sshHostsLoaded(let hosts):
                state.sshHosts = hosts
                return .none

            case .sshHostPicked(let host):
                if state.name.trimmed.isEmpty || state.name == state.destination { state.name = host }
                state.destination = host
                return .none

            case .addTapped:
                guard state.canAdd else { return .none }
                state.adding = true
                let (name, destination) = (state.name.trimmed, state.destination.trimmed)
                return .run { send in
                    await send(.delegate(.added(try await daemon.addHost(name, destination))))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .failed(let message):
                state.adding = false
                state.alert = AlertState {
                    TextState("Couldn't add the host")
                } message: {
                    TextState(message)
                }
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }
}

/// A host's agents' variables and setup script. Saved variables' values
/// never come back from the daemon: one shows empty, and stays as it is
/// unless a new value is typed.
@Reducer
package struct HostEditorFeature {
    @ObservableState
    package struct State: Equatable {
        package var host: RemoteHost
        package var variables: IdentifiedArrayOf<Variable>
        var nextRow: Int
        package var setupScript: String
        package var saving = false
        @Presents package var alert: AlertState<Never>?

        package init(host: RemoteHost) {
            self.host = host
            self.variables = IdentifiedArray(
                uniqueElements: host.envNames.enumerated().map { Variable(id: $0, name: $1, saved: true) })
            self.nextRow = host.envNames.count
            self.setupScript = host.setupScript
        }

        /// What saving would change; nil if nothing.
        package var update: HostUpdate? {
            let kept = Set(variables.map(\.name.trimmed))
            let update = HostUpdate(
                setEnv: Dictionary(
                    variables.filter { !$0.name.trimmed.isEmpty && !$0.value.isEmpty }.map {
                        ($0.name.trimmed, $0.value)
                    },
                    uniquingKeysWith: { _, last in last }),
                unsetEnv: host.envNames.filter { !kept.contains($0) },
                setupScript: setupScript == host.setupScript ? nil : setupScript)
            return update == HostUpdate() ? nil : update
        }

        /// Names typed that can't be a variable's.
        package var badNames: [String] {
            variables.map(\.name.trimmed).filter { name in
                !name.isEmpty
                    && !(name.first.map { $0.isLetter || $0 == "_" } == true
                        && name.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_") })
            }
        }

        package var canSave: Bool { !saving && update != nil && badNames.isEmpty }
    }

    /// A variable for the host's agents.
    package struct Variable: Identifiable, Hashable, Sendable {
        package var id: Int
        package var name = ""
        /// Empty for a saved one to keep its value.
        package var value = ""
        /// Whether the daemon has it already, so its name stays as it is.
        package var saved = false

        package init(id: Int, name: String = "", value: String = "", saved: Bool = false) {
            self.id = id
            self.name = name
            self.value = value
            self.saved = saved
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case addVariableTapped
        case removeVariableTapped(Variable.ID)
        case saveTapped
        case saved(RemoteHost)
        case failed(String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case saved(RemoteHost)
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .alert, .delegate:
                return .none

            case .addVariableTapped:
                state.variables.append(Variable(id: state.nextRow))
                state.nextRow += 1
                return .none

            case .removeVariableTapped(let id):
                state.variables.remove(id: id)
                return .none

            case .saveTapped:
                guard state.canSave, let update = state.update else { return .none }
                state.saving = true
                let name = state.host.name
                return .run { send in
                    await send(.saved(try await daemon.updateHost(name, update)))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .saved(let host):
                state = State(host: host)
                return .send(.delegate(.saved(host)))

            case .failed(let message):
                state.saving = false
                state.alert = AlertState {
                    TextState("Couldn't save \(state.host.isCoordinator ? "This Mac" : state.host.name)")
                } message: {
                    TextState(message)
                }
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }
}

extension String {
    fileprivate var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
}
