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
        /// The variables and setup script changed and not saved on hosts
        /// looked away from, to show again when they're selected.
        var unsaved: [RemoteHost.ID: HostEditorFeature.State] = [:]
        /// The selected host's sessions whose files couldn't be read.
        package var skipped: IdentifiedArrayOf<SkippedSession> = []
        /// Why they couldn't be read, on This Mac or a host that answers;
        /// nil once they are. An unreachable host's page says that already.
        package var skippedFailure: String?
        /// The Mac's copies of the selected host's sessions.
        package var recoveries: IdentifiedArrayOf<Recovery> = []
        /// Why the copies couldn't be read; nil once they are.
        package var recoveriesFailure: String?
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

        /// Whether `id` is This Mac or a host whose last check reached it,
        /// so a read of it that fails is worth saying.
        fileprivate func answers(_ id: RemoteHost.ID) -> Bool {
            if id == coordinator.id { return true }
            if case .known(.reachable) = health[id] { return true }
            return false
        }

        /// The hosts to keep while something else shows in their place. What
        /// they were waiting on stops when they go, so nothing kept says it's
        /// still in progress: the answer that would clear it never comes.
        package func settled() -> Self {
            var kept = self
            kept.restoring = []
            kept.linking = false
            // Checked again when the list shows, as an install's answer is
            // the only one that ends "Installing…".
            kept.health = kept.health.filter { $0.value != .installing }
            kept.editor?.saving = false
            kept.ports?.forwarding = false
            if case .add(var add) = kept.destination {
                add.adding = false
                kept.destination = .add(add)
            }
            return kept
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
        /// Humata's program couldn't be put on the host, and this is why;
        /// not the same as a host a check couldn't reach.
        case installFailed(String)
        case known(HostHealth)
    }

    @Reducer
    package enum Destination {
        case add(AddHostFeature)
        case alert(AlertState<Alert>)

        package enum Alert: Equatable {
            case confirmRemove(RemoteHost.ID)
            case confirmRemoveSkipped(SkippedSession.ID)
            case confirmRestartDaemon
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
        /// How the host is once its install ended.
        case installed(RemoteHost.ID, HostHealth)
        /// Its install failed, and this is why.
        case installFailed(RemoteHost.ID, String)
        case removeTapped(RemoteHost.ID)
        case removed(RemoteHost.ID)
        case skippedLoaded(RemoteHost.ID, [SkippedSession])
        /// They couldn't be read, and this is why; the list stays as it was.
        case skippedFailed(RemoteHost.ID, String)
        /// Reads the selected host's skipped sessions again.
        case skippedRetryTapped
        case removeSkippedTapped(SkippedSession.ID)
        case skippedRemoved(RemoteHost.ID, SkippedSession.ID)
        case recoveriesLoaded(RemoteHost.ID, [Recovery])
        /// They couldn't be read, and this is why; the list stays as it was.
        case recoveriesFailed(RemoteHost.ID, String)
        /// Reads the selected host's copies again.
        case recoveriesRetryTapped
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
        /// Asks before restarting this Mac's daemon, which stops its turns.
        case restartDaemonTapped
        /// This Mac's daemon was started anew: reads its commands again,
        /// with what it read of the login shell this time.
        case daemonRestarted
        /// The restart asked for here wasn't made, and this is why; the
        /// daemon runs as it did.
        case restartRefused(String)
        case terminals(TerminalsFeature.Action)
        case files(FileBrowserFeature.Action)
        case ports(PortsFeature.Action)
        case destination(PresentationAction<Destination.Action>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            /// The person confirmed restarting this Mac's daemon.
            case restartDaemon
        }
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
                return check(id, in: &state, again: true)

            case .checked(let id, let health):
                // An install under way says how the host is when it ends.
                guard state.health[id] != .installing else { return .none }
                return known(health, of: id, in: &state)

            case .installed(let id, let health):
                return known(health, of: id, in: &state)

            case .installFailed(let id, let message):
                state.health[id] = .installFailed(message)
                // Its page says the install failed, which is why nothing of
                // it is read.
                if id == state.selection { state.skippedFailure = nil }
                return .none

            case .recoveriesLoaded(let id, let recoveries):
                guard id == state.selection else { return .none }
                state.recoveriesFailure = nil
                state.recoveries = IdentifiedArray(
                    recoveries.filter { $0.host == id }, uniquingIDsWith: { _, last in last })
                return .none

            case .recoveriesFailed(let id, let message):
                guard id == state.selection else { return .none }
                state.recoveriesFailure = message
                return .none

            case .recoveriesRetryTapped:
                guard let id = state.selection, !id.isEmpty else { return .none }
                return loadRecoveries(id)

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
                state.skippedFailure = nil
                state.skipped = IdentifiedArray(skipped, uniquingIDsWith: { _, last in last })
                return .none

            case .skippedFailed(let id, let message):
                // An unreachable host's page says why nothing of it is read.
                guard id == state.selection, state.answers(id) else { return .none }
                state.skippedFailure = message
                return .none

            case .skippedRetryTapped:
                guard let id = state.selection else { return .none }
                return loadSkipped(id)

            case .removeSkippedTapped(let id):
                guard let session = state.skipped[id: id] else { return .none }
                // A daemon reads its sessions when it starts, so one that's
                // fixed opens once its daemon is started anew.
                let wayBack =
                    state.selection == state.coordinator.id
                    ? "restart Humata's daemon with the button under the list."
                    : "stop humatad on the host (for example `pkill -x humatad`; turns in progress there stop) "
                        + "and press Check Again, which starts it anew."
                state.destination = .alert(
                    AlertState {
                        TextState("Remove the session in \(session.folder)?")
                    } actions: {
                        ButtonState(role: .destructive, action: .confirmRemoveSkipped(id)) { TextState("Remove") }
                        ButtonState(role: .cancel) { TextState("Cancel") }
                    } message: {
                        TextState(
                            "Its folder and history are deleted, and can't be brought back. To keep the session "
                                + "instead, fix what's damaged, then \(wayBack)")
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
                guard state.health[id] != .installing else { return .none }
                state.health[id] = .installing
                return .run { send in
                    do {
                        _ = try await daemon.installHost(id)
                    } catch {
                        await send(.installFailed(id, error.localizedDescription))
                        return
                    }
                    // Installed; a check that fails now is a host that
                    // doesn't answer.
                    do {
                        await send(.installed(id, try await daemon.checkHost(id)))
                    } catch {
                        await send(.installed(id, .unreachable(error.localizedDescription)))
                    }
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
                guard state.selection == id else {
                    state.unsaved[id] = nil
                    return .none
                }
                let selecting = select(state.hosts.first?.id ?? state.coordinator.id, in: &state)
                // Selecting another kept what was being changed on it.
                state.unsaved[id] = nil
                return selecting

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
                // Its health, and what to do if the install fails, are on
                // its settings, whichever pane the last host showed.
                state.pane = .settings
                let selecting = select(host.id, in: &state)
                // Humata's program is copied over SSH; one reached by its
                // own command has it already, so it's only checked.
                guard host.sshDestination != nil else {
                    return .merge(selecting, check(host.id, in: &state, again: true))
                }
                return .merge(selecting, .send(.installTapped(host.id)))

            case .editor(.delegate(.saved(let host))) where host.isCoordinator:
                state.coordinator = host
                state.unsaved[host.id] = nil
                return .none

            case .editor(.delegate(.saved(let host))):
                state.hosts[id: host.id] = host
                state.unsaved[host.id] = nil
                // Saving sets it up again, which the check reports on.
                return check(host.id, in: &state, again: true)

            case .editor(.delegate(.saveFailed(let id, let message))):
                state.destination = .alert(AlertState.saveFailed(id, message, kept: state.unsaved[id] != nil))
                return .none

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

            case .restartDaemonTapped:
                state.destination = .alert(
                    AlertState {
                        TextState("Restart Humata's daemon?")
                    } actions: {
                        ButtonState(action: .confirmRestartDaemon) { TextState("Restart") }
                        ButtonState(role: .cancel) { TextState("Cancel") }
                    } message: {
                        TextState(
                            "Turns in progress on this Mac stop; send to those sessions again once it's back. "
                                + "Sessions on hosts go on.")
                    })
                return .none

            case .destination(.presented(.alert(.confirmRestartDaemon))):
                return .send(.delegate(.restartDaemon))

            case .daemonRestarted:
                // A session it couldn't open before may open now, so those
                // are read again as well.
                guard let id = state.selection, id == state.coordinator.id else { return .none }
                return .merge(loadCommandLinks(), loadSkipped(id))

            case .restartRefused(let message):
                state.destination = .alert(
                    AlertState {
                        TextState("Humata's daemon wasn't restarted")
                    } message: {
                        TextState(message)
                    })
                return .none

            case .editor, .destination, .terminals, .files, .ports, .delegate:
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
        // What was changed on the host shown is kept until it's saved.
        if let unsaved = state.editor?.unsaved { state.unsaved[unsaved.host.id] = unsaved }
        state.selection = id
        if let host = state.selected {
            state.editor = state.unsaved.removeValue(forKey: host.id) ?? HostEditorFeature.State(host: host)
        } else {
            state.editor = nil
        }
        state.terminals = id.map {
            TerminalsFeature.State(place: HostPlace(host: $0.isEmpty ? nil : $0), opensFirst: false)
        }
        state.files = id.map { FileBrowserFeature.State(place: HostPlace(host: $0.isEmpty ? nil : $0)) }
        state.ports = id.map { PortsFeature.State(host: $0.isEmpty ? nil : $0) }
        state.skipped = []
        state.skippedFailure = nil
        state.recoveries = []
        state.recoveriesFailure = nil
        state.commandLinks = nil
        guard let id else { return .none }
        guard !id.isEmpty else { return .merge(loadSkipped(id), loadCommandLinks()) }
        return .merge(loadSkipped(id), loadRecoveries(id))
    }

    /// Copies are kept on this Mac, so they list even while the host is
    /// unreachable.
    private func loadRecoveries(_ id: RemoteHost.ID) -> Effect<Action> {
        .run { send in
            await send(.recoveriesLoaded(id, try await daemon.recoveries()))
        } catch: { error, send in
            guard !(error is CancellationError) else { return }
            await send(.recoveriesFailed(id, error.localizedDescription))
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
            await send(.skippedLoaded(id, try await daemon.skipped(id.isEmpty ? nil : id)))
        } catch: { error, send in
            guard !(error is CancellationError) else { return }
            await send(.skippedFailed(id, error.localizedDescription))
        }
        .cancellable(id: SkippedLoad.host(id), cancelInFlight: true)
    }

    /// Notes how the host is, and reads what it skipped once it can say.
    private func known(_ health: HostHealth, of id: RemoteHost.ID, in state: inout State) -> Effect<Action> {
        state.health[id] = .known(health)
        guard id == state.selection else { return .none }
        guard case .reachable = health else {
            // Its page says it can't be reached, which is why.
            state.skippedFailure = nil
            return .none
        }
        return loadSkipped(id)
    }

    /// Checks the host's health. Shown again, a host keeps what was known
    /// until the check answers; `again`, as the person asked or its setup
    /// changed, it shows it's checking. One being installed on is left to
    /// its install.
    private func check(_ id: RemoteHost.ID, in state: inout State, again: Bool = false) -> Effect<Action> {
        guard state.health[id] != .installing else { return .none }
        if again || state.health[id] == nil { state.health[id] = .checking }
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
        /// The model providers its sessions can pay for with a key of the
        /// person's own. Each names one of the variables, never the key.
        package var providers: IdentifiedArrayOf<HostProvider>
        package var saving = false
        @Presents package var alert: AlertState<Never>?

        package init(host: RemoteHost) {
            self.host = host
            self.variables = IdentifiedArray(
                uniqueElements: host.envNames.enumerated().map { Variable(id: $0, name: $1, saved: true) })
            self.nextRow = host.envNames.count
            self.setupScript = host.setupScript
            self.providers = IdentifiedArray(host.providers, uniquingIDsWith: { _, last in last })
        }

        /// What saving would change; nil if nothing.
        package var update: HostUpdate? {
            let kept = Set(variables.map(\.name.trimmed))
            let edited = providers.map(\.trimmed)
            let update = HostUpdate(
                setEnv: Dictionary(
                    variables.filter { !$0.name.trimmed.isEmpty && !$0.value.isEmpty }.map {
                        ($0.name.trimmed, $0.value)
                    },
                    uniquingKeysWith: { _, last in last }),
                unsetEnv: host.envNames.filter { !kept.contains($0) },
                setupScript: setupScript == host.setupScript ? nil : setupScript,
                providers: edited == host.providers ? nil : edited)
            return update == HostUpdate() ? nil : update
        }

        /// Names typed that can't be a variable's.
        package var badNames: [String] {
            variables.map(\.name.trimmed).filter { !$0.isEmpty && !$0.isVariableName }
        }

        /// What keeps `provider` from being saved, said so it can be put
        /// right; nil when nothing does.
        package func problem(with typed: HostProvider) -> String? {
            let provider = typed.trimmed
            if provider.name.isEmpty { return "Give it a name to pick it by." }
            if !(provider.baseURL.hasPrefix("https://") || provider.baseURL.hasPrefix("http://")) {
                return "Its base URL starts with https://, or http:// for a server on your own network."
            }
            if !provider.keyVariable.isKeyVariableName {
                return "Name the variable that holds its key, such as \(provider.kind.keyVariable): upper-case "
                    + "letters, digits and _, not starting with a digit, at most 64 characters. The key itself "
                    + "goes in that variable above."
            }
            return nil
        }

        /// Whether "Add <variable>" would add a row for `provider`'s key:
        /// false once a row of that name is there to type the key in.
        package func offersAddKeyVariable(for provider: HostProvider) -> Bool {
            let name = provider.keyVariable.trimmed
            return name.isKeyVariableName && !variables.contains { $0.name.trimmed == name }
        }

        /// Whether the variable `provider` takes its key from is set, or
        /// is about to be with what's typed; sessions are offered only a
        /// provider whose key is there.
        package func keyIsSet(for provider: HostProvider) -> Bool {
            let name = provider.keyVariable.trimmed
            return variables.contains { $0.name.trimmed == name && ($0.saved || !$0.value.isEmpty) }
        }

        package var canSave: Bool {
            !saving && update != nil && badNames.isEmpty && providers.allSatisfy { problem(with: $0) == nil }
        }

        /// What to keep of it while another host shows: its changes, without
        /// what it was waiting on. Nil when it changed nothing.
        var unsaved: State? {
            guard update != nil else { return nil }
            var kept = self
            kept.saving = false
            kept.alert = nil
            return kept
        }
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
        /// Adds a provider of `kind`, set up as that kind usually is.
        case addProviderTapped(HostProvider.Kind)
        case removeProviderTapped(HostProvider.ID)
        /// Adds a row for the variable the provider takes its key from, to
        /// type the key in.
        case addKeyVariableTapped(HostProvider.ID)
        case saveTapped
        case saved(RemoteHost)
        /// The host whose save failed, and why.
        case failed(RemoteHost.ID, String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case saved(RemoteHost)
            /// A save failed for a host that's no longer the one shown.
            case saveFailed(RemoteHost.ID, String)
        }
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.uuid) var uuid

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

            case .addProviderTapped(let kind):
                state.providers.append(HostProvider(id: uuid().uuidString, kind: kind))
                return .none

            case .removeProviderTapped(let id):
                state.providers.remove(id: id)
                return .none

            case .addKeyVariableTapped(let id):
                guard let provider = state.providers[id: id], state.offersAddKeyVariable(for: provider) else {
                    return .none
                }
                let name = provider.keyVariable.trimmed
                state.variables.append(Variable(id: state.nextRow, name: name))
                state.nextRow += 1
                return .none

            case .saveTapped:
                guard state.canSave, let update = state.update else { return .none }
                state.saving = true
                let name = state.host.name
                return .run { send in
                    await send(.saved(try await daemon.updateHost(name, update)))
                } catch: { error, send in
                    await send(.failed(name, error.localizedDescription))
                }

            case .saved(let host):
                // Another host shows by now: its editor stays as it is.
                if host.id == state.host.id { state = State(host: host) }
                return .send(.delegate(.saved(host)))

            case .failed(let id, let message):
                guard id == state.host.id else { return .send(.delegate(.saveFailed(id, message))) }
                state.saving = false
                state.alert = AlertState.saveFailed(id, message, kept: false)
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }
}

extension AlertState {
    /// Says that saving a host's variables and setup script failed. `kept`
    /// when the host isn't the one shown and what was changed on it waits
    /// there, so the alert says where to find it.
    static func saveFailed(_ host: RemoteHost.ID, _ message: String, kept: Bool) -> Self {
        let name = host.isEmpty ? "This Mac" : host
        let text = kept ? "\(message)\n\nWhat you changed is kept: select \(name) to save it again." : message
        return AlertState {
            TextState("Couldn't save \(name)")
        } message: {
            TextState(text)
        }
    }
}

extension String {
    fileprivate var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }

    /// Whether it can be a variable's name: letters, digits and _, not
    /// starting with a digit.
    fileprivate var isVariableName: Bool {
        first.map { $0.isLetter || $0 == "_" } == true
            && allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_") }
    }

    /// Whether it can name the variable a provider's key is in: upper-case
    /// letters, digits and _, not starting with a digit, at most 64
    /// characters. A key pasted here by mistake rarely passes.
    fileprivate var isKeyVariableName: Bool {
        count <= 64 && first.map { $0.isUppercase || $0 == "_" } == true
            && allSatisfy { $0.isASCII && ($0.isUppercase || $0.isNumber || $0 == "_") }
    }
}

extension HostProvider {
    /// As it's saved: what was typed, without spaces around it.
    fileprivate var trimmed: Self {
        var provider = self
        provider.name = name.trimmed
        provider.baseURL = baseURL.trimmed
        provider.keyVariable = keyVariable.trimmed
        return provider
    }
}
