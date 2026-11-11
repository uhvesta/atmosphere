import ComposableArchitecture
import KhodrBoard
import KhodrDaemonClient
import KhodrProjects
import KhodrSession

/// Khodr's window: the sidebar of projects and sessions, and beside it the
/// inbox, a project's board or the open session's chat. It starts the daemon, and replaces one left
/// running by an older build once no turn is in progress.
@Reducer
package struct AppFeature {
    @ObservableState
    package struct State: Equatable {
        package var daemon = DaemonStatus.starting
        package var sidebar = ProjectsFeature.State()
        /// The open session; nil until one is picked.
        @Presents package var session: SessionFeature.State?
        /// The open board; nil unless one is picked.
        @Presents package var board: BoardFeature.State?

        package init() {}
    }

    package enum DaemonStatus: Equatable {
        case starting
        case running
        /// The daemon running is another build than the app's. It's replaced
        /// once no session is working or waiting, since that ends their turns.
        case updatePending
        case restarting
        /// Why it couldn't be started or replaced, and what to do about it.
        case failed(String, needsLoginItems: Bool)
    }

    package enum Action {
        /// The window appeared, or the person tried again: starts the daemon
        /// and checks its build.
        case task
        case daemonChecked(isCurrent: Bool, sessions: [Session])
        case daemonFailed(String, needsLoginItems: Bool)
        /// Replaces the daemon now, ending the turns in progress.
        case restartTapped
        case restarted
        case openLoginItemsTapped
        case sidebar(ProjectsFeature.Action)
        case session(PresentationAction<SessionFeature.Action>)
        case board(PresentationAction<BoardFeature.Action>)
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.daemonService) var service

    package init() {}

    package var body: some ReducerOf<Self> {
        Scope(state: \.sidebar, action: \.sidebar) { ProjectsFeature() }
        Reduce { state, action in
            switch action {
            case .task:
                state.daemon = .starting
                return .run { send in
                    try await service.start()
                    let current = try await service.isCurrent()
                    await send(
                        .daemonChecked(isCurrent: current, sessions: current ? [] : try await daemon.sessions(nil)))
                } catch: { error, send in
                    await send(Self.failed(error))
                }

            case .daemonChecked(isCurrent: true, _):
                state.daemon = .running
                return .send(.sidebar(.refresh))

            case .daemonChecked(isCurrent: false, let sessions):
                guard !Self.anyBusy(sessions) else {
                    state.daemon = .updatePending
                    return .none
                }
                return restart(&state)

            case .daemonFailed(let message, let needsLoginItems):
                state.daemon = .failed(message, needsLoginItems: needsLoginItems)
                return .none

            case .restartTapped:
                return restart(&state)

            case .restarted:
                state.daemon = .running
                // Its watch ended with the old daemon; watch the new one.
                guard state.session != nil else { return .send(.sidebar(.refresh)) }
                state.session?.alert = nil
                return .merge(.send(.sidebar(.refresh)), .send(.session(.presented(.task))))

            case .openLoginItemsTapped:
                return .run { _ in service.openLoginItems() }

            case .sidebar(.loaded(_, let sessions, _)):
                guard state.daemon == .updatePending, !Self.anyBusy(sessions) else { return .none }
                return restart(&state)

            case .sidebar(.updated(let session)),
                .sidebar(.destination(.presented(.rename(.delegate(.renamed(let session)))))),
                .sidebar(.destination(.presented(.compare(.delegate(.updated(let session)))))):
                if state.session?.session.id == session.id { state.session?.session = session }
                return .none

            case .sidebar(.delegate(.openSession(let session))):
                state.board = nil
                guard state.session?.session.id != session.id else { return .none }
                state.session = SessionFeature.State(session: session)
                return .none

            case .sidebar(.delegate(.openBoard(let project))):
                state.session = nil
                guard state.board?.project.id != project.id else { return .none }
                state.board = BoardFeature.State(project: project)
                return .none

            case .sidebar(.delegate(.openInbox)):
                state.session = nil
                state.board = nil
                return .none

            case .session(.presented(.delegate(.openSession(let session)))),
                .board(.presented(.delegate(.openSession(let session)))):
                if state.sidebar.sessions[id: session.id] == nil { state.sidebar.sessions.append(session) }
                state.sidebar.selection = .session(session.id)
                state.board = nil
                state.session = SessionFeature.State(session: session)
                return .send(.sidebar(.refresh))

            case .sidebar, .session, .board:
                return .none
            }
        }
        .ifLet(\.$session, action: \.session) { SessionFeature() }
        .ifLet(\.$board, action: \.board) { BoardFeature() }
    }

    private func restart(_ state: inout State) -> Effect<Action> {
        state.daemon = .restarting
        return .run { send in
            try await service.restart()
            await send(.restarted)
        } catch: { error, send in
            await send(Self.failed(error))
        }
    }

    /// Whether any session is in a turn, which replacing the daemon would end.
    private static func anyBusy(_ sessions: [Session]) -> Bool {
        sessions.contains { $0.state == .working || $0.state == .waitingForApproval }
    }

    private static func failed(_ error: any Error) -> Action {
        if case DaemonServiceError.needsApproval = error {
            return .daemonFailed(error.localizedDescription, needsLoginItems: true)
        }
        return .daemonFailed(error.localizedDescription, needsLoginItems: false)
    }
}
