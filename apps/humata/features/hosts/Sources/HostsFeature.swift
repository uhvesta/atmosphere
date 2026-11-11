import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// The machines sessions run on over SSH: each one's health, checked when
/// the list shows and on request; adding one from ~/.ssh/config; installing
/// Humata's program on one; its agents' variables and setup script; and
/// removing one.
@Reducer
package struct HostsFeature {
    @ObservableState
    package struct State: Equatable {
        package var hosts: IdentifiedArrayOf<RemoteHost> = []
        package var health: [RemoteHost.ID: Health] = [:]
        package var selection: RemoteHost.ID?
        /// The selected host's variables and setup script.
        package var editor: HostEditorFeature.State?
        /// Why the hosts couldn't be read, and what to do about it.
        package var failure: String?
        @Presents package var destination: Destination.State?

        package init() {}

        package var selected: RemoteHost? { selection.flatMap { hosts[id: $0] } }
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
        }
    }

    package enum Action {
        /// The list appeared: reads the hosts and checks each.
        case task
        case loaded([RemoteHost])
        case loadFailed(String)
        case selected(RemoteHost.ID?)
        case addTapped
        case checkTapped(RemoteHost.ID)
        case checked(RemoteHost.ID, HostHealth)
        case installTapped(RemoteHost.ID)
        case removeTapped(RemoteHost.ID)
        case removed(RemoteHost.ID)
        case failed(title: String, message: String)
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
                    await send(.loaded(try await daemon.hosts()))
                } catch: { error, send in
                    await send(.loadFailed(error.localizedDescription))
                }

            case .loaded(let hosts):
                state.failure = nil
                state.hosts = IdentifiedArray(hosts, uniquingIDsWith: { _, last in last })
                if state.selected == nil { select(state.hosts.first?.id, in: &state) }
                return .merge(hosts.map { check($0.id, in: &state) })

            case .loadFailed(let message):
                state.failure = message
                return .none

            case .selected(let id):
                select(id, in: &state)
                return .none

            case .addTapped:
                state.destination = .add(AddHostFeature.State())
                return .none

            case .checkTapped(let id):
                return check(id, in: &state)

            case .checked(let id, let health):
                state.health[id] = .known(health)
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
                if state.selection == id { select(state.hosts.first?.id, in: &state) }
                return .none

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
                select(host.id, in: &state)
                return check(host.id, in: &state)

            case .editor(.delegate(.saved(let host))):
                state.hosts[id: host.id] = host
                // Saving sets it up again, which the check reports on.
                return check(host.id, in: &state)

            case .editor, .destination:
                return .none
            }
        }
        .ifLet(\.editor, action: \.editor) { HostEditorFeature() }
        .ifLet(\.$destination, action: \.destination)
    }

    private func select(_ id: RemoteHost.ID?, in state: inout State) {
        state.selection = id
        state.editor = state.selected.map(HostEditorFeature.State.init)
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
                    TextState("Couldn't save \(state.host.name)")
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
