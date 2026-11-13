import ComposableArchitecture
import Foundation
import HumataBoard
import HumataDaemonClient
import HumataHosts
import HumataProjects
import HumataSearch
import HumataSession

/// Humata's window: the sidebar of projects and sessions, and beside it the
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
        /// The hosts; nil unless they're open.
        @Presents package var hosts: HostsFeature.State?
        /// Searching every session; nil unless it's open.
        @Presents package var search: SearchFeature.State?
        /// A link opened from outside, shown once the sidebar has read
        /// what it names, which may be new.
        package var opening: HumataLink?
        /// Settings > Environments.
        package var settings = EnvironmentsFeature.State()
        package var approvals = ApprovalsFeature.State()
        package var storage = StorageFeature.State()
        package var skills = SkillsFeature.State()
        package var plugins = PluginsFeature.State()
        /// What each session looked away from was showing, put back when
        /// it opens again while Humata runs.
        var left: [Session.ID: SessionFeature.State.Left] = [:]

        package init() {}

        /// Opens `next` in place of the open session, keeping what the one
        /// closed was showing; nil closes it.
        mutating func show(_ next: SessionFeature.State?) {
            if let open = session, open.session.id != next?.session.id {
                left[open.session.id] = open.left
            }
            var next = next
            if let id = next?.session.id, let kept = left.removeValue(forKey: id) {
                // A search hit opens on the chat, at what was found.
                if next?.highlighted == nil { next?.left = kept } else { next?.docs.drafts = kept.drafts }
            }
            session = next
        }
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
        case hosts(PresentationAction<HostsFeature.Action>)
        case search(PresentationAction<SearchFeature.Action>)
        /// A `humata://` URL was opened, as by `humata mcp`'s show tool.
        case opened(URL)
        case settings(EnvironmentsFeature.Action)
        case approvals(ApprovalsFeature.Action)
        case storage(StorageFeature.Action)
        case skills(SkillsFeature.Action)
        case plugins(PluginsFeature.Action)
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.daemonService) var service
    @Dependency(\.modelServer) var modelServer

    package init() {}

    package var body: some ReducerOf<Self> {
        Scope(state: \.sidebar, action: \.sidebar) { ProjectsFeature() }
        Scope(state: \.settings, action: \.settings) { EnvironmentsFeature() }
        Scope(state: \.approvals, action: \.approvals) { ApprovalsFeature() }
        Scope(state: \.storage, action: \.storage) { StorageFeature() }
        Scope(state: \.skills, action: \.skills) { SkillsFeature() }
        Scope(state: \.plugins, action: \.plugins) { PluginsFeature() }
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
                return .merge(.send(.sidebar(.refresh)), serveModels())

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
                guard state.session != nil else { return .merge(.send(.sidebar(.refresh)), serveModels()) }
                state.session?.alert = nil
                return .merge(.send(.sidebar(.refresh)), .send(.session(.presented(.task))), serveModels())

            case .openLoginItemsTapped:
                return .run { _ in service.openLoginItems() }

            case .sidebar(.loaded(_, let sessions, let inbox, _)):
                // Every two seconds: set only what changed, so the open
                // session isn't drawn again for nothing.
                if let open = state.session {
                    if let listed = sessions.first(where: { $0.id == open.session.id }),
                        listed.hostOffline != open.session.hostOffline
                    {
                        state.session?.session.hostOffline = listed.hostOffline
                    }
                    let request = Self.request(of: open.session.id, in: inbox)
                    if request != open.request { state.session?.request = request }
                }
                let shown = show(&state)
                guard state.daemon == .updatePending, !Self.anyBusy(sessions) else { return shown }
                return .merge(shown, restart(&state))

            case .sidebar(.updated(let session)),
                .sidebar(.destination(.presented(.rename(.delegate(.renamed(let session)))))),
                .sidebar(.destination(.presented(.compare(.delegate(.updated(let session)))))):
                if state.session?.session.id == session.id { state.session?.session = session }
                return .none

            case .sidebar(.delegate(.openSession(let session))):
                state.board = nil
                state.hosts = nil
                state.search = nil
                guard state.session?.session.id != session.id else { return .none }
                var open = SessionFeature.State(session: session)
                open.request = Self.request(of: session.id, in: state.sidebar.inbox)
                state.show(open)
                return .none

            case .sidebar(.delegate(.openChanges(let session))):
                state.board = nil
                state.hosts = nil
                state.search = nil
                if state.session?.session.id != session.id { state.show(SessionFeature.State(session: session)) }
                return .send(.session(.presented(.tabPicked(.review))))

            case .sidebar(.delegate(.openBoard(let project))):
                state.show(nil)
                state.hosts = nil
                state.search = nil
                guard state.board?.project.id != project.id else { return .none }
                state.board = BoardFeature.State(project: project)
                return .none

            case .sidebar(.delegate(.openInbox)):
                state.show(nil)
                state.board = nil
                state.hosts = nil
                state.search = nil
                return .none

            case .sidebar(.delegate(.openSearch)):
                state.show(nil)
                state.board = nil
                state.hosts = nil
                if state.search == nil {
                    state.search = SearchFeature.State(projects: Array(state.sidebar.projects))
                }
                return .none

            case .search(.presented(.delegate(.openSession(let id, let event)))):
                guard let session = state.sidebar.sessions[id: id] else { return .none }
                state.sidebar.selection = .session(id)
                state.search = nil
                state.show(SessionFeature.State(session: session, highlighted: event))
                return .none

            case .search(.presented(.delegate(.openItem(let projectID, let itemID)))),
                .session(.presented(.delegate(.openItem(let projectID, let itemID)))):
                guard let project = state.sidebar.projects[id: projectID] else { return .none }
                state.sidebar.selection = .board(projectID)
                state.search = nil
                state.show(nil)
                state.board = BoardFeature.State(project: project, opening: itemID)
                return .none

            case .opened(let url):
                guard let link = HumataLink(url) else { return .none }
                state.opening = link
                return .send(.sidebar(.refresh))

            case .sidebar(.delegate(.openHosts)):
                state.show(nil)
                state.board = nil
                state.search = nil
                if state.hosts == nil { state.hosts = HostsFeature.State() }
                return .none

            case .hosts(.presented(.removed)):
                // Its sessions went with it.
                return .send(.sidebar(.refresh))

            case .session(.presented(.delegate(.openSession(let session)))),
                .board(.presented(.delegate(.openSession(let session)))):
                if state.sidebar.sessions[id: session.id] == nil { state.sidebar.sessions.append(session) }
                state.sidebar.selection = .session(session.id)
                state.board = nil
                state.hosts = nil
                state.search = nil
                state.show(SessionFeature.State(session: session))
                return .send(.sidebar(.refresh))

            case .session(.presented(.delegate(.answerRequest(let id, let answer)))):
                return .send(.sidebar(.requestAnswered(id, answer)))

            case .sidebar, .session, .board, .hosts, .search, .settings, .approvals, .storage, .skills, .plugins:
                return .none
            }
        }
        .ifLet(\.$session, action: \.session) { SessionFeature() }
        .ifLet(\.$board, action: \.board) { BoardFeature() }
        .ifLet(\.$hosts, action: \.hosts) { HostsFeature() }
        .ifLet(\.$search, action: \.search) { SearchFeature() }
    }

    /// What `session` asked and waits on, from the inbox.
    private static func request(of session: Session.ID, in inbox: [InboxEntry]) -> AgentRequest? {
        inbox.lazy.compactMap(\.request).first { $0.sessionID == session }
    }

    /// Shows the link opened from outside, once the sidebar has read it.
    private func show(_ state: inout State) -> Effect<Action> {
        guard let link = state.opening else { return .none }
        state.opening = nil
        switch link {
        case .session(let id):
            guard state.sidebar.sessions[id: id] != nil else { return .none }
            return .send(.sidebar(.selected(.session(id))))
        case .board(let id):
            guard state.sidebar.projects[id: id] != nil else { return .none }
            return .send(.sidebar(.selected(.board(id))))
        case .task(let projectID, let itemID):
            guard let project = state.sidebar.projects[id: projectID] else { return .none }
            state.sidebar.selection = .board(projectID)
            state.show(nil)
            state.hosts = nil
            state.search = nil
            state.board = BoardFeature.State(project: project, opening: itemID)
            return .none
        }
    }

    /// Starts the model server, if it isn't running, and tells the daemon
    /// where it is. If either fails, local-model sessions on it wait and
    /// say why, so there's nothing more to show here.
    private func serveModels() -> Effect<Action> {
        .run { _ in
            guard let port = await modelServer.start() else { return }
            try? await daemon.setModelServer(port)
        }
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

    /// Whether any session on this Mac is in a turn, which replacing the
    /// daemon would end. Hosts' sessions run in their own daemons and outlive
    /// this one, and an offline host's state is only what was last heard.
    private static func anyBusy(_ sessions: [Session]) -> Bool {
        sessions.contains { session in
            (session.host ?? "").isEmpty && !session.hostOffline
                && (session.state == .working || session.state == .waitingForApproval)
        }
    }

    private static func failed(_ error: any Error) -> Action {
        if case DaemonServiceError.needsApproval = error {
            return .daemonFailed(error.localizedDescription, needsLoginItems: true)
        }
        return .daemonFailed(error.localizedDescription, needsLoginItems: false)
    }
}
