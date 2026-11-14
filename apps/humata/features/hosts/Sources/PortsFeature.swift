import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// A host's forwarded ports: a dev server's port there, listened for on
/// 127.0.0.1 here and opened in the browser.
@Reducer
package struct PortsFeature {
    /// Identified by its host, so another host's ports shown in its place
    /// stop what this one was doing instead of finishing it into them.
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var id: String? { host }
        /// Nil for this Mac.
        package var host: String?
        /// This host's forwards, oldest first.
        package var forwards: IdentifiedArrayOf<Forward> = []
        /// The port typed in, to forward next.
        package var port = ""
        package var forwarding = false
        /// What failed last, and what to do about it.
        package var failure: String?

        package init(host: String?) {
            self.host = host
        }

        /// The typed port, if it's one that can be forwarded.
        package var portNumber: Int? {
            Int(port.trimmingCharacters(in: .whitespaces)).flatMap { (1...65_535).contains($0) ? $0 : nil }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case loaded([Forward])
        case forwardTapped
        case forwarded(Forward)
        case stopTapped(Forward.ID)
        case stopped(Forward.ID)
        case failed(String)
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding:
                return .none
            case .task:
                return .run { [host = state.host] send in
                    await send(.loaded(try await daemon.forwards().filter { $0.host == host }))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }
            case .loaded(let forwards):
                state.forwards = IdentifiedArray(uniqueElements: forwards)
                return .none
            case .forwardTapped:
                guard let port = state.portNumber, !state.forwarding else { return .none }
                state.forwarding = true
                state.failure = nil
                return .run { [host = state.host] send in
                    await send(.forwarded(try await daemon.forwardPort(host, port, nil)))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }
            case .forwarded(let forward):
                state.forwarding = false
                state.port = ""
                state.forwards[id: forward.id] = forward
                return .none
            case .stopTapped(let id):
                state.failure = nil
                return .run { send in
                    try await daemon.stopForward(id)
                    await send(.stopped(id))
                } catch: { error, send in
                    if case DaemonError.refused(.unknownForward, _) = error {
                        await send(.stopped(id))
                    } else {
                        await send(.failed(error.localizedDescription))
                    }
                }
            case .stopped(let id):
                state.forwards.remove(id: id)
                return .none
            case .failed(let message):
                state.forwarding = false
                state.failure = message
                return .none
            }
        }
    }
}
