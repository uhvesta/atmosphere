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
        /// A side chat of the open session, shown beside it until it's
        /// closed or another session opens; it stays in the sidebar.
        @Presents package var side: SessionFeature.State?
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
        package var github = GitHubSettingsFeature.State()
        package var repositories = SavedRepositoriesFeature.State()
        package var skills = SkillsFeature.State()
        package var plugins = PluginsFeature.State()
        package var models = ModelDefaultsFeature.State()
        /// What each session looked away from was showing, put back when
        /// it opens again while Humata runs.
        var left: [Session.ID: SessionFeature.State.Left] = [:]
        /// The histories of the chats last looked away from, so going
        /// back to one shows it at once instead of reading it all again.
        var histories = Histories()
        /// Search, each board and the hosts as they were left, put back
        /// when they open again while Humata runs.
        var keptSearch: SearchFeature.State?
        var keptBoards: [Project.ID: BoardFeature.State] = [:]
        var keptHosts: HostsFeature.State?
        /// Says a message wasn't sent to a session no longer open, and
        /// that it's back in that session's box.
        @Presents package var sendFailure: AlertState<Action.SendFailure>?

        package init() {}

        /// Puts away the open board, hosts and search, to show again as they
        /// were, but for what they were waiting on: that stops with them.
        mutating func putAway() {
            if let board { keptBoards[board.project.id] = board.settled() }
            if let hosts { keptHosts = hosts.settled() }
            if let search { keptSearch = search.settled() }
            board = nil
            hosts = nil
            search = nil
        }

        /// Opens `project`'s board as it was left, to open `item` once it's
        /// read if one is given.
        mutating func openBoard(_ project: Project, at item: String? = nil) {
            putAway()
            var next = keptBoards.removeValue(forKey: project.id) ?? BoardFeature.State(project: project)
            next.project = project
            if let item { next.opening = item }
            board = next
        }

        /// Shows the chat if the open session no longer has the view it
        /// showed, as once it's archived or leaves its project.
        mutating func keepTab() {
            guard let open = session, !open.tabs.contains(open.tab) else { return }
            session?.tab = .chat
        }

        /// Opens `next` in place of the open session, keeping what the one
        /// closed was showing; nil closes it.
        mutating func show(_ next: SessionFeature.State?) {
            // A side chat is beside the session it was opened from only.
            if session?.session.id != next?.session.id { side = nil }
            if let open = session {
                if open.session.id != next?.session.id { left[open.session.id] = open.left }
                // Kept for the session opened again in its own place too,
                // as a search hit in the open chat does.
                histories.keep(open.history, of: open.session.id)
            }
            var next = next
            if let id = next?.session.id, var kept = left.removeValue(forKey: id) {
                // A search hit opens on the chat, at what was found, but
                // keeps what was being written.
                if next?.highlighted != nil, let tab = next?.tab { kept.tab = tab }
                next?.left = kept
            }
            if let id = next?.session.id, let history = histories.take(id) { next?.restore(history) }
            session = next
        }
    }

    /// The histories of the chats last looked away from, the one left
    /// longest ago dropped first: a few chats, and no more memory than
    /// `bytes` between them, since a history is held whole.
    struct Histories: Equatable {
        /// How many chats' histories are kept.
        private static let chats = 4
        /// How much they hold between them at most.
        private static let bytes = 48 * 1_048_576

        private var kept: [Session.ID: SessionFeature.State.History] = [:]
        /// The sessions kept, the one left longest ago first.
        private var order: [Session.ID] = []

        /// Keeps `history` as `session`'s, in place of what was kept for
        /// it; nil, or one too large to keep at all, keeps nothing.
        mutating func keep(_ history: SessionFeature.State.History?, of session: Session.ID) {
            forget(session)
            guard let history, history.bytes <= Self.bytes else { return }
            kept[session] = history
            order.append(session)
            while let oldest = order.first, order.count > Self.chats || held > Self.bytes {
                forget(oldest)
            }
        }

        /// Hands over what's kept for `session`, keeping it no longer.
        mutating func take(_ session: Session.ID) -> SessionFeature.State.History? {
            order.removeAll { $0 == session }
            return kept.removeValue(forKey: session)
        }

        mutating func forget(_ session: Session.ID) {
            guard kept.removeValue(forKey: session) != nil else { return }
            order.removeAll { $0 == session }
        }

        private var held: Int { kept.values.reduce(0) { $0 + $1.bytes } }
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
        /// The side chat beside the open session; dismissing it closes it.
        case side(PresentationAction<SessionFeature.Action>)
        /// A message from a session's box reached the daemon.
        case sendFinished(Session.ID)
        /// It didn't, and this is why: the message goes back in its
        /// session's box, whether or not that session is still open.
        case sendFailed(SessionFeature.Outgoing, String)
        case sendFailure(PresentationAction<SendFailure>)
        case board(PresentationAction<BoardFeature.Action>)
        case hosts(PresentationAction<HostsFeature.Action>)
        case search(PresentationAction<SearchFeature.Action>)
        /// A `humata://` URL was opened, as by `humata mcp`'s show tool.
        case opened(URL)
        case settings(EnvironmentsFeature.Action)
        case approvals(ApprovalsFeature.Action)
        case storage(StorageFeature.Action)
        case github(GitHubSettingsFeature.Action)
        case repositories(SavedRepositoriesFeature.Action)
        case skills(SkillsFeature.Action)
        case plugins(PluginsFeature.Action)
        case models(ModelDefaultsFeature.Action)

        package enum SendFailure: Equatable, Sendable {
            /// Open the session whose message wasn't sent.
            case openSession(Session.ID)
        }
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
        Scope(state: \.github, action: \.github) { GitHubSettingsFeature() }
        Scope(state: \.repositories, action: \.repositories) { SavedRepositoriesFeature() }
        Scope(state: \.skills, action: \.skills) { SkillsFeature() }
        Scope(state: \.plugins, action: \.plugins) { PluginsFeature() }
        Scope(state: \.models, action: \.models) { ModelDefaultsFeature() }
        Reduce { state, action in
            switch action {
            case .task:
                // Reopening the window checks again without hiding the
                // sidebar behind the spinner.
                switch state.daemon {
                case .running, .updatePending: break
                default: state.daemon = .starting
                }
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
                state.side?.alert = nil
                let side: Effect<Action> = state.side == nil ? .none : .send(.side(.presented(.task)))
                return .merge(.send(.sidebar(.refresh)), .send(.session(.presented(.task))), side, serveModels())

            case .openLoginItemsTapped:
                return .run { _ in service.openLoginItems() }

            case .sidebar(.loaded(_, let sessions, let inbox, _)):
                // Every two seconds: set only what changed, so the open
                // session isn't drawn again for nothing.
                if let open = state.session {
                    if let listed = sessions.first(where: { $0.id == open.session.id }) {
                        // What changes outside its chat: its host is lost or
                        // it's moved, put on another task or archived.
                        if listed.hostOffline != open.session.hostOffline {
                            state.session?.session.hostOffline = listed.hostOffline
                        }
                        if listed.host != open.session.host { state.session?.session.host = listed.host }
                        if listed.itemID != open.session.itemID { state.session?.session.itemID = listed.itemID }
                        if listed.itemTitle != open.session.itemTitle {
                            state.session?.session.itemTitle = listed.itemTitle
                        }
                        if listed.archived != open.session.archived {
                            state.session?.session.archived = listed.archived
                            state.keepTab()
                        }
                    }
                    let request = Self.request(of: open.session.id, in: inbox)
                    if request != open.request { state.session?.request = request }
                }
                let shown = show(&state)
                guard state.daemon == .updatePending, !Self.anyBusy(sessions) else { return shown }
                return .merge(shown, restart(&state))

            case .sidebar(.updated(let session)),
                .sidebar(.sessionMoved(let session)),
                .sidebar(.destination(.presented(.rename(.delegate(.renamed(let session)))))),
                .sidebar(.destination(.presented(.move(.delegate(.moved(let session)))))),
                .sidebar(.destination(.presented(.compare(.delegate(.updated(let session)))))):
                if state.session?.session.id == session.id {
                    state.session?.session = session
                    state.keepTab()
                }
                if state.side?.session.id == session.id {
                    // Archived, it takes no more questions.
                    if session.archived { state.side = nil } else { state.side?.session = session }
                }
                // A session moved to another host is read from there.
                switch action {
                case .sidebar(.sessionMoved), .sidebar(.destination(.presented(.move(.delegate(.moved))))):
                    state.histories.forget(session.id)
                default: break
                }
                return .none

            case .sidebar(.sessionDeleted(let id)):
                // Closed without keeping what it showed or what was being
                // written to it: it won't open again.
                state.left[id] = nil
                state.histories.forget(id)
                if state.session?.session.id == id {
                    state.session = nil
                    state.side = nil
                }
                if state.side?.session.id == id { state.side = nil }
                SessionFeature.State.forgetDraft(of: id)
                return .none

            case .sidebar(.projectDeleted(let id)):
                state.keptBoards[id] = nil
                if state.board?.project.id == id { state.board = nil }
                // Its sessions stay, on no project.
                if state.session?.session.projectID == id {
                    state.session?.session.projectID = nil
                    state.session?.session.itemID = nil
                    state.session?.session.itemTitle = nil
                    state.keepTab()
                }
                return .none

            case .sidebar(.delegate(.openSession(let session))):
                state.putAway()
                guard state.session?.session.id != session.id else { return .none }
                var open = SessionFeature.State(session: session)
                open.request = Self.request(of: session.id, in: state.sidebar.inbox)
                state.show(open)
                return .none

            case .sidebar(.delegate(.openChanges(let session))):
                state.putAway()
                if state.session?.session.id != session.id { state.show(SessionFeature.State(session: session)) }
                return .send(.session(.presented(.tabPicked(.review))))

            case .sidebar(.delegate(.openBoard(let project))):
                state.show(nil)
                guard state.board?.project.id != project.id else { return .none }
                state.openBoard(project)
                return .none

            case .sidebar(.delegate(.openInbox)):
                state.show(nil)
                state.putAway()
                return .none

            case .sidebar(.delegate(.openSearch)):
                state.show(nil)
                guard state.search == nil else { return .none }
                var search = state.keptSearch ?? SearchFeature.State()
                state.putAway()
                state.keptSearch = nil
                search.projects = Array(state.sidebar.projects)
                state.search = search
                // Text left before it was searched is searched now, as
                // nothing else would.
                guard !search.searched, !search.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                    return .none
                }
                return .send(.search(.presented(.submitted)))

            case .search(.presented(.delegate(.openSession(let id, let event)))):
                guard let session = state.sidebar.sessions[id: id] else { return .none }
                state.sidebar.selection = .session(id)
                state.putAway()
                state.show(SessionFeature.State(session: session, highlighted: event))
                return .none

            case .search(.presented(.delegate(.openItem(let projectID, let itemID)))),
                .session(.presented(.delegate(.openItem(let projectID, let itemID)))),
                .side(.presented(.delegate(.openItem(let projectID, let itemID)))):
                guard let project = state.sidebar.projects[id: projectID] else { return .none }
                state.sidebar.selection = .board(projectID)
                state.show(nil)
                state.openBoard(project, at: itemID)
                // A board kept isn't waiting to be read; read it to open the item.
                return .send(.board(.presented(.refresh)))

            case .opened(let url):
                guard let link = HumataLink(url) else { return .none }
                state.opening = link
                return .send(.sidebar(.refresh))

            case .sidebar(.delegate(.openHosts)):
                state.show(nil)
                guard state.hosts == nil else { return .none }
                let hosts = state.keptHosts ?? HostsFeature.State()
                state.putAway()
                state.keptHosts = nil
                state.hosts = hosts
                return .none

            case .hosts(.presented(.removed)):
                // Its sessions went with it.
                return .send(.sidebar(.refresh))

            case .session(.presented(.delegate(.openSide(let session)))),
                .side(.presented(.delegate(.openSide(let session)))):
                // Beside the open session, which stays as it is; a side
                // chat of the side chat takes its place.
                guard state.session != nil else { return .none }
                if state.sidebar.sessions[id: session.id] == nil { state.sidebar.sessions.append(session) }
                var side = SessionFeature.State(session: session)
                side.beside = true
                state.side = side
                return .send(.sidebar(.refresh))

            case .session(.presented(.delegate(.openSession(let session)))),
                .side(.presented(.delegate(.openSession(let session)))),
                .board(.presented(.delegate(.openSession(let session)))):
                if state.sidebar.sessions[id: session.id] == nil { state.sidebar.sessions.append(session) }
                state.sidebar.selection = .session(session.id)
                state.putAway()
                state.show(SessionFeature.State(session: session))
                return .send(.sidebar(.refresh))

            case .session(.presented(.delegate(.answerRequest(let id, let answer)))),
                .side(.presented(.delegate(.answerRequest(let id, let answer)))):
                return .send(.sidebar(.requestAnswered(id, answer)))

            case .session(.presented(.delegate(.send(let outgoing)))),
                .side(.presented(.delegate(.send(let outgoing)))):
                // Sent from here and not from the session, so that looking
                // at another session doesn't stop it: the daemon may already
                // have the request, and a failure must still be answered for.
                return .run { send in
                    let images = outgoing.images.compactMap(\.image)
                    if images.isEmpty {
                        try await daemon.send(outgoing.session, outgoing.text, outgoing.mode)
                    } else {
                        try await daemon.sendImages(outgoing.session, outgoing.text, images, outgoing.mode)
                    }
                    await send(.sendFinished(outgoing.session))
                } catch: { error, send in
                    await send(.sendFailed(outgoing, error.localizedDescription))
                }

            case .sendFinished(let session):
                if state.side?.session.id == session { return .send(.side(.presented(.sent))) }
                guard state.session?.session.id == session else { return .none }
                return .send(.session(.presented(.sent)))

            case .sendFailed(let outgoing, let message):
                if state.side?.session.id == outgoing.session {
                    return .send(
                        .side(
                            .presented(
                                .sendFailed(
                                    text: outgoing.typed, pastes: outgoing.pastes, images: outgoing.images,
                                    message: message))))
                }
                if state.session?.session.id == outgoing.session {
                    return .send(
                        .session(
                            .presented(
                                .sendFailed(
                                    text: outgoing.typed, pastes: outgoing.pastes, images: outgoing.images,
                                    message: message))))
                }
                // Deleted since, if nothing was kept of it: there's no box
                // to put the message back in.
                guard var left = state.left[outgoing.session] else { return .none }
                SessionFeature.State.keepUnsent(outgoing, in: &left)
                state.left[outgoing.session] = left
                let whose = outgoing.name.isEmpty ? "another session" : "“\(outgoing.name)”"
                state.sendFailure = AlertState {
                    TextState("Couldn’t send the message to \(whose)")
                } actions: {
                    ButtonState(action: .openSession(outgoing.session)) { TextState("Open the Session") }
                    ButtonState(role: .cancel) { TextState("OK") }
                } message: {
                    TextState("\(message)\n\nThe message is back in that session’s box. Open it to send it again.")
                }
                return .none

            case .sendFailure(.presented(.openSession(let id))):
                guard state.sidebar.sessions[id: id] != nil else { return .none }
                return .send(.sidebar(.selected(.session(id))))

            case .sidebar, .session, .side, .sendFailure, .board, .hosts, .search, .settings, .approvals, .storage,
                .github, .repositories, .skills, .plugins, .models:
                return .none
            }
        }
        .ifLet(\.$session, action: \.session) { SessionFeature() }
        .ifLet(\.$side, action: \.side) { SessionFeature() }
        .ifLet(\.$sendFailure, action: \.sendFailure)
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
            state.openBoard(project, at: itemID)
            return .send(.board(.presented(.refresh)))
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
