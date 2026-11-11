import ComposableArchitecture
import Foundation
import KhodrDaemonClient

/// One session: its events as they happen, a message to send, and forks.
@Reducer
package struct SessionFeature {
    @ObservableState
    package struct State: Equatable {
        package var session: Session
        /// Every event but the reply's parts as it streams, oldest first.
        package var events: IdentifiedArrayOf<SessionEvent> = []
        /// The reply so far, until its whole message arrives.
        package var streaming = ""
        package var draft = ""
        /// Whether the daemon stopped sending events; watching again resumes.
        package var disconnected = false
        @Presents package var alert: AlertState<Never>?

        package init(session: Session) {
            self.session = session
        }

        /// Whether a turn has started and not yet ended.
        package var isWorking: Bool {
            for event in events.reversed() {
                switch event.kind {
                case .turnStarted: return true
                case .turnEnded, .idle: return false
                default: continue
                }
            }
            return false
        }

        /// Requests for permission the agent is still waiting on.
        package var pendingPermissions: [SessionEvent] {
            let answered = Set(
                events.compactMap { event -> String? in
                    guard case .permissionCompleted(let request, _) = event.kind else { return nil }
                    return request
                })
            return events.filter { event in
                guard case .permissionRequested(let request, _, _) = event.kind else { return false }
                return !answered.contains(request)
            }
        }

        mutating func receive(_ event: SessionEvent) {
            switch event.kind {
            case .assistantDelta(let text):
                streaming += text
            case .assistantMessage, .turnEnded:
                streaming = ""
                events.updateOrAppend(event)
            default:
                events.updateOrAppend(event)
            }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The view appeared; watches the session until it goes.
        case task
        case received(SessionEvent)
        case watchEnded(failure: String?)
        case sendTapped(SendMode)
        case abortTapped
        case answerPermission(requestID: String, approved: Bool)
        /// Forks the session before `event`, or with all of it if nil.
        case forkTapped(before: String?)
        case forked(Session)
        case failed(String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            /// A fork was made; open it.
            case openSession(Session)
        }
    }

    @Dependency(\.daemon) var daemon

    private enum CancelID { case watch }

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .delegate, .alert:
                return .none

            case .task:
                state.disconnected = false
                let session = state.session.id
                return .run { send in
                    do {
                        for try await event in await daemon.watch(session) {
                            await send(.received(event))
                        }
                        await send(.watchEnded(failure: nil))
                    } catch is CancellationError {
                    } catch {
                        await send(.watchEnded(failure: error.localizedDescription))
                    }
                }
                .cancellable(id: CancelID.watch, cancelInFlight: true)

            case .received(let event):
                state.receive(event)
                return .none

            case .watchEnded(let failure):
                state.disconnected = true
                if let failure { state.alert = Self.alert(failure) }
                return .none

            case .sendTapped(let mode):
                let text = state.draft.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !text.isEmpty else { return .none }
                state.draft = ""
                let session = state.session.id
                return .run { _ in
                    try await daemon.send(session, text, mode)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .abortTapped:
                let session = state.session.id
                return .run { _ in
                    try await daemon.abort(session)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .answerPermission(let request, let approved):
                let session = state.session.id
                return .run { _ in
                    try await daemon.approve(session, request, approved)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .forkTapped(let before):
                let session = state.session
                return .run { send in
                    await send(.forked(try await daemon.fork(session.id, before, "\(session.name) fork")))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .forked(let fork):
                return .send(.delegate(.openSession(fork)))

            case .failed(let message):
                state.alert = Self.alert(message)
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    private static func alert(_ message: String) -> AlertState<Never> {
        AlertState {
            TextState("Something went wrong")
        } message: {
            TextState(message)
        }
    }
}
