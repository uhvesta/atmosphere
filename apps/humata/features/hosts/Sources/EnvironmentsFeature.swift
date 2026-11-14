import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Settings > Environments: the machines sessions can run on, as the person
/// names them in ~/.ssh/config. Adding one installs humatad there and checks
/// it; one that runs an older build than this app's offers Update; the
/// selected one's agents' variables and setup script are edited beside it.
@Reducer
package struct EnvironmentsFeature {
    @ObservableState
    package struct State: Equatable {
        /// The hosts ~/.ssh/config names.
        package var sshHosts: [String] = []
        /// The hosts Humata has, from the config or typed.
        package var hosts: IdentifiedArrayOf<RemoteHost> = []
        package var health: [RemoteHost.ID: HostsFeature.Health] = [:]
        /// What `ssh` connects to, while it's being added.
        package var adding: Set<String> = []
        package var selection: RemoteHost.ID?
        package var editor: HostEditorFeature.State?
        /// The variables and setup script changed and not saved on hosts
        /// looked away from, to show again when they're selected.
        var unsaved: [RemoteHost.ID: HostEditorFeature.State] = [:]
        /// Why the hosts couldn't be read, and what to do about it.
        package var failure: String?
        @Presents package var destination: Destination.State?
        /// The humatad version this app installs; a host running another
        /// is offered Update. Nil when it isn't known, so none is.
        package var bundledVersion: String?

        package init() {}

        /// Whether the host runs another build of humatad than this app's.
        package func needsUpdate(_ id: RemoteHost.ID) -> Bool {
            guard let bundledVersion, case .known(.reachable(let status))? = health[id] else { return false }
            return status.programVersion != bundledVersion
        }

        /// Every environment: the config's hosts in its order, each with the
        /// host Humata has for it, then hosts added by a typed destination.
        package var environments: [Environment] {
            let inConfig = sshHosts.map { name in
                Environment(destination: name, host: hosts.first { $0.sshDestination == name })
            }
            let typed = hosts.filter { host in !sshHosts.contains { $0 == host.sshDestination } }
                .map { Environment(destination: $0.sshDestination ?? $0.name, host: $0) }
            return inConfig + typed
        }
    }

    /// A machine in Settings, added or not.
    package struct Environment: Equatable, Identifiable {
        /// What `ssh` connects to.
        package var destination: String
        /// The host Humata has for it; nil until it's added.
        package var host: RemoteHost?

        package var id: String { destination }
    }

    @Reducer
    package enum Destination {
        case addOther(AddHostFeature)
        case alert(AlertState<Never>)
    }

    package enum Action {
        /// The pane appeared: reads the config's hosts and Humata's, and
        /// checks each of Humata's.
        case task
        case loaded(sshHosts: [String], hosts: [RemoteHost])
        case loadFailed(String)
        /// Adds the config's host by its name, installs humatad there and
        /// checks it.
        case addTapped(destination: String)
        case added(RemoteHost)
        case addFailed(destination: String, message: String)
        /// Installs this app's build of humatad on the host again.
        case updateTapped(RemoteHost.ID)
        case checkTapped(RemoteHost.ID)
        case checked(RemoteHost.ID, HostHealth)
        /// How the host is once its install ended.
        case installed(RemoteHost.ID, HostHealth)
        case selected(RemoteHost.ID?)
        case addOtherTapped
        case editor(HostEditorFeature.Action)
        case destination(PresentationAction<Destination.Action>)
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                return .run { send in
                    // Without a config, hosts are typed in with Add Other.
                    let sshHosts = (try? await daemon.sshHosts()) ?? []
                    await send(.loaded(sshHosts: sshHosts, hosts: try await daemon.hosts()))
                } catch: { error, send in
                    await send(.loadFailed(error.localizedDescription))
                }

            case .loaded(let sshHosts, let hosts):
                state.failure = nil
                state.sshHosts = sshHosts
                state.hosts = IdentifiedArray(hosts, uniquingIDsWith: { _, last in last })
                return .merge(hosts.map { check($0.id, in: &state) })

            case .loadFailed(let message):
                state.failure = message
                return .none

            case .addTapped(let destination):
                guard !state.adding.contains(destination),
                    !state.hosts.contains(where: { $0.sshDestination == destination })
                else { return .none }
                state.adding.insert(destination)
                return .run { send in
                    await send(.added(try await daemon.addHost(destination, destination)))
                } catch: { error, send in
                    await send(.addFailed(destination: destination, message: error.localizedDescription))
                }

            case .added(let host):
                if let destination = host.sshDestination { state.adding.remove(destination) }
                state.hosts[id: host.id] = host
                // A host being changed stays in view until that's saved.
                if state.editor?.update == nil { select(host.id, in: &state) }
                return install(host.id, in: &state)

            case .addFailed(let destination, let message):
                state.adding.remove(destination)
                state.destination = .alert(
                    AlertState {
                        TextState("Couldn't add \(destination)")
                    } message: {
                        TextState(message)
                    })
                return .none

            case .updateTapped(let id):
                return install(id, in: &state)

            case .checkTapped(let id):
                return check(id, in: &state)

            case .checked(let id, let health):
                // An install under way says how the host is when it ends.
                guard state.health[id] != .installing else { return .none }
                state.health[id] = .known(health)
                return .none

            case .installed(let id, let health):
                state.health[id] = .known(health)
                return .none

            case .selected(let id):
                select(id, in: &state)
                return .none

            case .addOtherTapped:
                state.destination = .addOther(AddHostFeature.State())
                return .none

            case .destination(.presented(.addOther(.delegate(.added(let host))))):
                state.destination = nil
                return .send(.added(host))

            case .editor(.delegate(.saved(let host))):
                state.hosts[id: host.id] = host
                state.unsaved[host.id] = nil
                // Saving sets it up again, which the check reports on.
                return check(host.id, in: &state)

            case .editor(.delegate(.saveFailed(let id, let message))):
                state.destination = .alert(AlertState.saveFailed(id, message, kept: state.unsaved[id] != nil))
                return .none

            case .editor, .destination:
                return .none
            }
        }
        .ifLet(\.editor, action: \.editor) { HostEditorFeature() }
        .ifLet(\.$destination, action: \.destination)
    }

    /// Shows `id`'s variables and setup script, with what was changed on it
    /// before and not saved. What was changed on the host shown until now
    /// is kept the same way.
    private func select(_ id: RemoteHost.ID?, in state: inout State) {
        if let unsaved = state.editor?.unsaved { state.unsaved[unsaved.host.id] = unsaved }
        state.selection = id
        if let id, let host = state.hosts[id: id] {
            state.editor = state.unsaved.removeValue(forKey: id) ?? HostEditorFeature.State(host: host)
        } else {
            state.editor = nil
        }
    }

    /// Installs this app's build on the host, unless that's under way.
    private func install(_ id: RemoteHost.ID, in state: inout State) -> Effect<Action> {
        guard state.health[id] != .installing else { return .none }
        state.health[id] = .installing
        return .run { send in
            _ = try await daemon.installHost(id)
            await send(.installed(id, try await daemon.checkHost(id)))
        } catch: { error, send in
            await send(.installed(id, .unreachable(error.localizedDescription)))
        }
    }

    /// Checks the host's health. One being installed on is left to its
    /// install.
    private func check(_ id: RemoteHost.ID, in state: inout State) -> Effect<Action> {
        guard state.health[id] != .installing else { return .none }
        state.health[id] = .checking
        return .run { send in
            await send(.checked(id, try await daemon.checkHost(id)))
        } catch: { error, send in
            await send(.checked(id, .unreachable(error.localizedDescription)))
        }
    }
}

extension EnvironmentsFeature.Destination.State: Equatable {}
