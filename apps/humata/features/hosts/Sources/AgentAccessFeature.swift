import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Settings › Agent Access: what agents (`humata mcp`, and sessions' own
/// agents' humata tools) and the `humata` command line may do. Each has a
/// main switch, a switch for each group of actions and one for each
/// action; the daemon holds them, and refuses what is off. Each change is
/// saved at once; one that isn't goes back.
@Reducer
package struct AgentAccessFeature {
    @ObservableState
    package struct State: Equatable {
        /// Nil until first read.
        package var access: AgentAccess?
        /// A change is being saved.
        package var saving = false
        /// Why it couldn't be read or saved, and what to do about it.
        package var failure: String?

        package init(access: AgentAccess? = nil) {
            self.access = access
        }
    }

    package enum Action {
        case task
        case loaded(AgentAccess)
        case loadFailed(String)
        case saved(AgentAccess)
        /// Not saved; the switches go back to `was`.
        case saveFailed(was: AgentAccess, String)
        /// A client's main switch.
        case mainSwitchToggled(AccessClient, Bool)
        case groupToggled(AccessClient, AccessGroup, Bool)
        /// One action's own switch, by its id.
        case actionToggled(AccessClient, id: String, Bool)
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                state.failure = nil
                return .run { send in
                    await send(.loaded(try await daemon.agentAccess()))
                } catch: { error, send in
                    await send(.loadFailed(error.localizedDescription))
                }

            case .loaded(let access):
                // A change being saved is newer than what was just read.
                guard !state.saving else { return .none }
                state.access = access
                return .none

            case .loadFailed(let message):
                state.failure = String(
                    localized: """
                        Couldn't read what agents may do: \(message) Check that Humata's daemon is running, \
                        then try again.
                        """)
                return .none

            case .saved(let access):
                state.saving = false
                state.failure = nil
                state.access = access
                return .none

            case .saveFailed(let was, let message):
                state.saving = false
                state.access = was
                state.failure = String(
                    localized: """
                        Couldn't save the change, so the switch is back as it was: \(message) Check that \
                        Humata's daemon is running, then switch it again.
                        """)
                return .none

            case .mainSwitchToggled(let client, let enabled):
                guard var access = state.access else { return .none }
                access[client].enabled = enabled
                return save(&state, access)

            case .groupToggled(let client, let group, let enabled):
                guard var access = state.access else { return .none }
                access[client][group] = enabled
                return save(&state, access)

            case .actionToggled(let client, let id, let enabled):
                guard var access = state.access,
                    let index = access[client].actions.firstIndex(where: { $0.id == id })
                else { return .none }
                access[client].actions[index].enabled = enabled
                return save(&state, access)
            }
        }
    }

    private func save(_ state: inout State, _ access: AgentAccess) -> Effect<Action> {
        guard !state.saving, let was = state.access, access != was else { return .none }
        state.saving = true
        state.failure = nil
        // Shown at once; put back if it isn't saved.
        state.access = access
        return .run { send in
            await send(.saved(try await daemon.setAgentAccess(access)))
        } catch: { error, send in
            await send(.saveFailed(was: was, error.localizedDescription))
        }
    }
}
