import ComposableArchitecture
import KhodrDaemonClient
import KhodrProjects
import KhodrSession

/// Khodr's window: the sidebar of projects and sessions, and the open
/// session's chat beside it.
@Reducer
package struct AppFeature {
    @ObservableState
    package struct State: Equatable {
        package var sidebar = ProjectsFeature.State()
        /// The open session; nil until one is picked.
        @Presents package var session: SessionFeature.State?

        package init() {}
    }

    package enum Action {
        case sidebar(ProjectsFeature.Action)
        case session(PresentationAction<SessionFeature.Action>)
    }

    package init() {}

    package var body: some ReducerOf<Self> {
        Scope(state: \.sidebar, action: \.sidebar) { ProjectsFeature() }
        Reduce { state, action in
            switch action {
            case .sidebar(.delegate(.openSession(let session))):
                guard state.session?.session.id != session.id else { return .none }
                state.session = SessionFeature.State(session: session)
                return .none

            case .session(.presented(.delegate(.openSession(let fork)))):
                state.sidebar.sessions.append(fork)
                state.sidebar.selection = fork.id
                state.session = SessionFeature.State(session: fork)
                return .send(.sidebar(.refresh))

            case .sidebar, .session:
                return .none
            }
        }
        .ifLet(\.$session, action: \.session) { SessionFeature() }
    }
}
