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
        /// Each session's messages on their way to the daemon, oldest
        /// first: the first is being sent, the rest wait for it, so two
        /// sent quickly reach the session in the order they were sent.
        var outbox: [Session.ID: [SessionFeature.Outgoing]] = [:]
        /// Whether the daemon was looked for, and started again if gone,
        /// since the sidebar last read: once for each run of failed reads.
        var daemonRetried = false
        /// Whether the daemon was started again in this run of failed
        /// reads and hasn't been looked for since: the next read that fails
        /// looks once more, so that one found answering, and gone later, is
        /// started again.
        var daemonStartedAgain = false
        package var sidebar = ProjectsFeature.State()
        /// The open session; nil until one is picked.
        @Presents package var session: SessionFeature.State?
        /// A side chat of the open session, shown beside it until it's
        /// closed or another session opens; it stays in the sidebar.
        @Presents package var side: SessionFeature.State?
        /// The column the person is in, whose session the Session menu and
        /// the chat's shortcuts act on: the side chat's only while one is
        /// open.
        package private(set) var focusedColumn = SessionColumn.main
        /// The open board; nil unless one is picked.
        @Presents package var board: BoardFeature.State?
        /// The hosts; nil unless they're open.
        @Presents package var hosts: HostsFeature.State?
        /// Searching every session; nil unless it's open.
        @Presents package var search: SearchFeature.State?
        /// The installer, in place of the window's content, for a copy
        /// opened from outside Applications; nil once the person chose
        /// to run this copy, and for every other build. The daemon starts
        /// only once it's gone.
        @Presents package var installer: InstallerFeature.State?
        /// A link opened from outside, shown once the sidebar has read
        /// what it names, which may be new.
        package var opening: HumataLink?
        /// Settings > Environments.
        package var settings = EnvironmentsFeature.State()
        package var approvals = ApprovalsFeature.State()
        package var storage = StorageFeature.State()
        package var github = GitHubSettingsFeature.State()
        package var agentAccess = AgentAccessFeature.State()
        package var repositories = SavedRepositoriesFeature.State()
        package var skills = SkillsFeature.State()
        package var plugins = PluginsFeature.State()
        package var models = ModelDefaultsFeature.State()
        /// How big the text of the chat and what's read beside it is.
        package var textSize = TextSizeFeature.State()
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

        /// The session the Session menu acts on: the side chat while the
        /// person is in its column, the open session otherwise.
        package var commandSession: SessionFeature.State? {
            focusedColumn == .side ? side ?? session : session
        }

        /// Puts the person in `column`, the main one when no side chat is
        /// open, so only its session takes the chat's shortcuts. Sets only
        /// what changed, so neither chat is drawn again for nothing.
        mutating func focus(_ column: SessionColumn) {
            let column = side == nil ? .main : column
            if focusedColumn != column { focusedColumn = column }
            if let open = session, open.takesShortcuts != (column == .main) {
                session?.takesShortcuts = column == .main
            }
            if let beside = side, beside.takesShortcuts != (column == .side) {
                side?.takesShortcuts = column == .side
            }
        }

        /// Closes the side chat, keeping what it showed and what was being
        /// written to it for when it's opened from the sidebar, unless
        /// it's gone for good; the open session takes the shortcuts again.
        mutating func closeSide(keeping: Bool = true) {
            // The person was writing in the column that goes, so the open
            // session's box takes the caret.
            if side != nil, focusedColumn == .side { session?.boxFocus += 1 }
            putAwaySide(keeping: keeping)
            focus(.main)
        }

        /// Takes the side chat out of its column. Kept, what it showed,
        /// what was being written to it and its history wait for it to
        /// open again, as any session's do; otherwise nothing of it stays.
        private mutating func putAwaySide(keeping: Bool) {
            guard let side else { return }
            if keeping {
                left[side.session.id] = side.left
                histories.keep(side.history, of: side.session.id)
            } else {
                histories.forget(side.session.id)
            }
            self.side = nil
        }

        /// Closes the side chat and the open session if they run on `host`,
        /// which Humata forgot with its sessions, and keeps nothing of any
        /// session on it: none of them opens again.
        mutating func closeSessions(on host: RemoteHost.ID) {
            if let beside = side, beside.session.host == host {
                left[beside.session.id] = nil
                closeSide(keeping: false)
            }
            if let open = session, open.session.host == host {
                let id = open.session.id
                // A side chat of it on another host is kept as any closed
                // one is.
                closeSide()
                session = nil
                left[id] = nil
                histories.forget(id)
                if sidebar.selection == .session(id) { sidebar.selection = nil }
            }
            for listed in sidebar.sessions where listed.host == host {
                left[listed.id] = nil
                histories.forget(listed.id)
            }
        }

        /// Shows `session` beside the open one, in the place of a side chat
        /// already there, which is kept as any closed one is. The person
        /// stays in the column they were in.
        mutating func showBeside(_ session: Session) {
            guard self.session != nil else { return }
            if sidebar.sessions[id: session.id] == nil { sidebar.sessions.append(session) }
            let column = focusedColumn
            putAwaySide(keeping: true)
            side = opened(session, beside: true)
            // The shortcuts stay where the person is until they go into
            // the new chat, which the window says when they do.
            focus(column)
        }

        /// `session` as it opens: with what its agent asked and waits on,
        /// from the inbox, so its strip is there from the start.
        func opened(_ session: Session, highlighted: String? = nil, beside: Bool = false) -> SessionFeature.State {
            var open = SessionFeature.State(session: session, highlighted: highlighted)
            if beside { open.beside = true }
            open.request = request(of: session.id)
            return open
        }

        /// What `session` asked and waits on, from the sidebar's inbox,
        /// which leaves out the requests answered here.
        func request(of session: Session.ID) -> AgentRequest? {
            sidebar.inbox.lazy.compactMap(\.request).first { $0.sessionID == session }
        }

        /// Marks the columns showing request `id` as being answered or not.
        /// One being answered keeps its strip, busy, until the answer lands.
        /// One no longer answered forgets which answer it was, so the next
        /// isn't shown under the last one's name.
        mutating func answering(_ id: AgentRequest.ID, _ answering: Bool) {
            if session?.request?.id == id {
                if session?.answeringRequest != answering { session?.answeringRequest = answering }
                if !answering, session?.requestAnswer != nil { session?.requestAnswer = nil }
            }
            if side?.request?.id == id {
                if side?.answeringRequest != answering { side?.answeringRequest = answering }
                if !answering, side?.requestAnswer != nil { side?.requestAnswer = nil }
            }
        }

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
            if session?.session.id != next?.session.id { closeSide() }
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
            // One opened in its own place, beside a side chat the person
            // is in, leaves the shortcuts with the side chat.
            focus(focusedColumn)
        }
    }

    /// The two chats the window can show side by side.
    package enum SessionColumn: Hashable, Sendable {
        /// The open session.
        case main
        /// The side chat beside it.
        case side
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
        /// The daemon running is a newer Humata's, which this copy can't
        /// use and leaves running.
        case newerRunning
        /// Why it couldn't be started or replaced, and what to do about it.
        case failed(String, needsLoginItems: Bool)
    }

    package enum Action {
        /// The window appeared, or the person tried again: starts the daemon
        /// and checks its build.
        case task
        case daemonChecked(isCurrent: Bool, sessions: [Session])
        /// The daemon answering is a newer Humata's: it isn't replaced.
        case newerDaemonFound
        case daemonFailed(String, needsLoginItems: Bool)
        /// Replaces the daemon now, ending the turns in progress.
        case restartTapped
        case restarted
        /// Nothing answers where the daemon was: it's started again.
        case daemonGone
        /// The daemon answers though a read of it failed, so the next read
        /// that fails looks for it again.
        case daemonAnswered
        /// The daemon answering isn't one Humata started, so it wasn't
        /// stopped, and this is why. `askedInHosts` when the person asked
        /// for the restart there, where it's then said.
        case restartRefused(String, askedInHosts: Bool)
        case openLoginItemsTapped
        case sidebar(ProjectsFeature.Action)
        case session(PresentationAction<SessionFeature.Action>)
        /// The side chat beside the open session; dismissing it closes it.
        case side(PresentationAction<SessionFeature.Action>)
        /// The person went into a column, or out of both, which is nil
        /// and leaves the commands with the one they were last in.
        case columnFocused(SessionColumn?)
        /// Something picked in the Session menu, for the session of the
        /// column the person is in.
        case sessionCommand(SessionFeature.Action)
        /// A message from a session's box reached the daemon.
        case sendFinished(Session.ID)
        /// It didn't, and this is why: the message goes back in its
        /// session's box, whether or not that session is still open.
        case sendFailed(SessionFeature.Outgoing, String)
        case sendFailure(PresentationAction<SendFailure>)
        case board(PresentationAction<BoardFeature.Action>)
        case hosts(PresentationAction<HostsFeature.Action>)
        case search(PresentationAction<SearchFeature.Action>)
        case installer(PresentationAction<InstallerFeature.Action>)
        /// A `humata://` URL was opened, as by `humata mcp`'s show tool.
        case opened(URL)
        case settings(EnvironmentsFeature.Action)
        case approvals(ApprovalsFeature.Action)
        case storage(StorageFeature.Action)
        case github(GitHubSettingsFeature.Action)
        case agentAccess(AgentAccessFeature.Action)
        case repositories(SavedRepositoriesFeature.Action)
        case skills(SkillsFeature.Action)
        case plugins(PluginsFeature.Action)
        case models(ModelDefaultsFeature.Action)
        /// Bigger Text, Smaller Text and Actual Size in the View menu.
        case textSize(TextSizeFeature.Action)

        package enum SendFailure: Equatable, Sendable {
            /// Open the session whose message wasn't sent.
            case openSession(Session.ID)
        }
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.daemonService) var service
    @Dependency(\.modelServer) var modelServer

    package init() {}

    /// Takes the session's sent message off its outbox and sends the one
    /// waiting behind it, if any.
    private func sendNext(in session: Session.ID, _ state: inout State) -> Effect<Action> {
        guard var waiting = state.outbox[session], !waiting.isEmpty else { return .none }
        waiting.removeFirst()
        state.outbox[session] = waiting.isEmpty ? nil : waiting
        guard let next = waiting.first else { return .none }
        return deliver(next)
    }

    private func deliver(_ outgoing: SessionFeature.Outgoing) -> Effect<Action> {
        .run { send in
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
    }

    package var body: some ReducerOf<Self> {
        Scope(state: \.sidebar, action: \.sidebar) { ProjectsFeature() }
        Scope(state: \.settings, action: \.settings) { EnvironmentsFeature() }
        Scope(state: \.approvals, action: \.approvals) { ApprovalsFeature() }
        Scope(state: \.storage, action: \.storage) { StorageFeature() }
        Scope(state: \.github, action: \.github) { GitHubSettingsFeature() }
        Scope(state: \.agentAccess, action: \.agentAccess) { AgentAccessFeature() }
        Scope(state: \.repositories, action: \.repositories) { SavedRepositoriesFeature() }
        Scope(state: \.skills, action: \.skills) { SkillsFeature() }
        Scope(state: \.plugins, action: \.plugins) { PluginsFeature() }
        Scope(state: \.models, action: \.models) { ModelDefaultsFeature() }
        Scope(state: \.textSize, action: \.textSize) { TextSizeFeature() }
        Reduce { state, action in
            // The outbox's order, kept before the sends are answered for.
            switch action {
            case .session(.presented(.delegate(.send(let outgoing)))),
                .side(.presented(.delegate(.send(let outgoing)))):
                state.outbox[outgoing.session, default: []].append(outgoing)
                return .none
            case .sendFinished(let session):
                return sendNext(in: session, &state)
            case .sendFailed(let outgoing, _):
                return sendNext(in: outgoing.session, &state)
            default:
                return .none
            }
        }
        Reduce(Self.followDaemonAndListing)
        Reduce<State, Action> { state, action in
            switch action {
            case .task:
                // Reopening the window checks again without hiding the
                // sidebar behind the spinner.
                switch state.daemon {
                case .running, .updatePending: break
                default: state.daemon = .starting
                }
                return check()

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
                // Hosts shows what the new daemon read of this Mac's shell.
                let hosts: Effect<Action> = state.hosts == nil ? .none : .send(.hosts(.presented(.daemonRestarted)))
                // Its watch ended with the old daemon; watch the new one.
                guard state.session != nil else {
                    return .merge(.send(.sidebar(.refresh)), hosts, serveModels())
                }
                state.session?.alert = nil
                state.side?.alert = nil
                let side: Effect<Action> = state.side == nil ? .none : .send(.side(.presented(.task)))
                return .merge(
                    .send(.sidebar(.refresh)), .send(.session(.presented(.task))), side, hosts, serveModels())

            case .sidebar(.loadFailed):
                // A read can fail while the daemon answers; only one that
                // is gone is started again, once until the sidebar reads.
                // One that goes while an update waits is started again too:
                // the banner's restart would otherwise never come.
                guard state.daemon == .running || state.daemon == .updatePending else { return .none }
                guard !state.daemonRetried else {
                    // Started again, and a read still fails. If it answers,
                    // the read failed for another reason and the next one
                    // that fails looks for it again; if it doesn't, starting
                    // it didn't keep it, and it isn't started over and over.
                    guard state.daemonStartedAgain else { return .none }
                    state.daemonStartedAgain = false
                    return .run { send in
                        if (try? await service.build()) != nil { await send(.daemonAnswered) }
                    }
                }
                state.daemonRetried = true
                return .run { send in
                    do {
                        _ = try await service.build()
                        await send(.daemonAnswered)
                    } catch {
                        await send(.daemonGone)
                    }
                }

            case .daemonAnswered:
                // It was there this time; it may not be at the next failed
                // read. One found gone leaves this set, so a run of failed
                // reads starts it once.
                state.daemonRetried = false
                state.daemonStartedAgain = false
                return .none

            case .restartRefused(let message, let askedInHosts):
                guard askedInHosts, state.hosts != nil else {
                    state.daemon = .failed(message, needsLoginItems: false)
                    return .none
                }
                // Nothing was stopped: the daemon runs as it did, and Hosts,
                // where the restart was asked for, says why.
                state.daemon = .running
                return .send(.hosts(.presented(.restartRefused(message))))

            case .daemonGone:
                guard state.daemon == .running || state.daemon == .updatePending else { return .none }
                state.daemon = .restarting
                state.daemonStartedAgain = true
                return .run { send in
                    try await service.start()
                    await send(.restarted)
                } catch: { error, send in
                    await send(Self.failed(error))
                }

            case .openLoginItemsTapped:
                return .run { _ in service.openLoginItems() }

            case .sidebar(.loaded(_, let sessions, _, _)):
                // Every two seconds: set only what changed, so the open
                // session isn't drawn again for nothing.
                if state.daemonRetried { state.daemonRetried = false }
                if state.daemonStartedAgain { state.daemonStartedAgain = false }
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
                    // From the sidebar's inbox, which this follows and which
                    // leaves out what was answered here. A request being
                    // answered keeps its strip until the answer lands.
                    if !open.answeringRequest {
                        let request = state.request(of: open.session.id)
                        if request != open.request { state.session?.request = request }
                    }
                }
                if let beside = state.side {
                    let listed = sessions.first { $0.id == beside.session.id }
                    if let listed, listed.archived, !beside.session.archived {
                        // Archived elsewhere, it takes no more questions.
                        state.closeSide()
                    } else {
                        if let listed {
                            if listed.hostOffline != beside.session.hostOffline {
                                state.side?.session.hostOffline = listed.hostOffline
                            }
                            if listed.host != beside.session.host { state.side?.session.host = listed.host }
                        }
                        if !beside.answeringRequest {
                            let request = state.request(of: beside.session.id)
                            if request != beside.request { state.side?.request = request }
                        }
                    }
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
                    if session.archived { state.closeSide() } else { state.side?.session = session }
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
                // A side chat of the one deleted is closed as any other is.
                if state.side?.session.id == id { state.closeSide(keeping: false) }
                if state.session?.session.id == id {
                    state.closeSide()
                    state.session = nil
                }
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
                state.show(state.opened(session))
                return .none

            case .sidebar(.delegate(.openChanges(let session))):
                state.putAway()
                if state.session?.session.id != session.id { state.show(state.opened(session)) }
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
                state.show(state.opened(session, highlighted: event))
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

            case .settings(.removed(let host)):
                // Its sessions went with it, the open one too, whose watch
                // would otherwise retry without end; Hosts lists it no more.
                state.closeSessions(on: host)
                guard state.hosts != nil else { return .send(.sidebar(.refresh)) }
                return .merge(.send(.sidebar(.refresh)), .send(.hosts(.presented(.task))))

            case .hosts(.presented(.delegate(.restartDaemon))):
                // Asked for and confirmed in Hosts.
                guard state.daemon != .restarting else { return .none }
                return restart(&state, askedInHosts: true)

            case .session(.presented(.delegate(.openSide(let session)))),
                .side(.presented(.delegate(.openSide(let session)))):
                // Beside the open session, which stays as it is; a side
                // chat of the side chat takes its place.
                guard state.session != nil else { return .none }
                state.showBeside(session)
                return .send(.sidebar(.refresh))

            case .side(.presented(.delegate(.openSession(let session)))) where session.readOnly:
                // What a side chat makes reads only, as it does, so it
                // opens in the side chat's place and the open session
                // stays as it is.
                guard state.session != nil else { return .none }
                state.showBeside(session)
                return .send(.sidebar(.refresh))

            case .side(.dismiss):
                // Ahead of the side chat's own reducer, which then finds it
                // closed: what it showed is kept as any session's is.
                state.closeSide()
                return .none

            case .columnFocused(let column):
                if let column { state.focus(column) }
                return .none

            case .sessionCommand(let action):
                switch state.focusedColumn {
                case .side where state.side != nil: return .send(.side(.presented(action)))
                case .main, .side: break
                }
                guard state.session != nil else { return .none }
                return .send(.session(.presented(action)))

            case .session(.presented(.delegate(.openSession(let session)))),
                .side(.presented(.delegate(.openSession(let session)))),
                .board(.presented(.delegate(.openSession(let session)))):
                if state.sidebar.sessions[id: session.id] == nil { state.sidebar.sessions.append(session) }
                state.sidebar.selection = .session(session.id)
                state.putAway()
                state.show(state.opened(session))
                return .send(.sidebar(.refresh))

            case .session(.presented(.delegate(.answerRequest(let id, let answer)))),
                .side(.presented(.delegate(.answerRequest(let id, let answer)))):
                return .send(.sidebar(.requestAnswered(id, answer)))

            case .sidebar(.requestAnswered(let id, _)),
                .sidebar(.destination(.presented(.alert(.closeItem(let id, _, _))))):
                // After the sidebar, which holds the request as answered
                // once the answer is on its way: one it asks about first,
                // as closing an item with sessions, isn't yet.
                state.answering(id, state.sidebar.answered.contains(id))
                return .none

            case .sidebar(.requestAnswerSent(let id)):
                // The answer landed: the strip goes, where it was kept up.
                state.answering(id, false)
                if state.session?.request?.id == id { state.session?.request = nil }
                if state.side?.request?.id == id { state.side?.request = nil }
                return .none

            case .sidebar(.requestNotAnswered(let id)):
                // The request is still the agent's to wait on, so the
                // strip can be answered again.
                state.answering(id, false)
                return .none

            case .session(.presented(.delegate(.send(let outgoing)))),
                .side(.presented(.delegate(.send(let outgoing)))):
                // Sent from here and not from the session, so that looking
                // at another session doesn't stop it: the daemon may already
                // have the request, and a failure must still be answered for.
                // One sent while an earlier one is on its way waits for it.
                guard state.outbox[outgoing.session]?.count == 1 else { return .none }
                return deliver(outgoing)

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
                // Closed since. Deleted, there's no box to put the message
                // back in; otherwise it goes back even if nothing was kept
                // of the session when it closed.
                var left: SessionFeature.State.Left
                if let kept = state.left[outgoing.session] {
                    left = kept
                } else if let listed = state.sidebar.sessions[id: outgoing.session] {
                    left = SessionFeature.State(session: listed).left
                } else {
                    return .none
                }
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

            case .installer:
                // The installer quits this copy itself, or is put away.
                return .none

            case .sidebar, .session, .side, .sendFailure, .board, .hosts, .search, .settings, .approvals, .storage,
                .github, .agentAccess, .repositories, .skills, .plugins, .models, .textSize, .newerDaemonFound:
                return .none
            }
        }
        .ifLet(\.$session, action: \.session) { SessionFeature() }
        .ifLet(\.$side, action: \.side) { SessionFeature() }
        .ifLet(\.$sendFailure, action: \.sendFailure)
        .ifLet(\.$board, action: \.board) { BoardFeature() }
        .ifLet(\.$hosts, action: \.hosts) { HostsFeature() }
        .ifLet(\.$search, action: \.search) { SearchFeature() }
        .ifLet(\.$installer, action: \.installer) { InstallerFeature() }
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

    /// Starts the daemon and checks its build. A newer Humata's daemon
    /// answering is left as it is and not started over, since this copy's
    /// login item would take its place.
    private func check() -> Effect<Action> {
        .run { send in
            if (try? await service.build()) == .newer {
                await send(.newerDaemonFound)
                return
            }
            try await service.start()
            switch try await service.build() {
            case .current: await send(.daemonChecked(isCurrent: true, sessions: []))
            case .older: await send(.daemonChecked(isCurrent: false, sessions: try await daemon.sessions(nil)))
            case .newer: await send(.newerDaemonFound)
            }
        } catch: { error, send in
            await send(Self.failed(error))
        }
    }

    /// What's kept apart from the main reducer: a newer daemon found, and
    /// the commits the open session's last turn read, as the listing has
    /// them. Its access is SessionFeature's, so it's left alone.
    private static func followDaemonAndListing(_ state: inout State, _ action: Action) -> Effect<Action> {
        switch action {
        case .newerDaemonFound:
            state.daemon = .newerRunning
        case .sidebar(.loaded(_, let sessions, _, _)):
            guard let open = state.session, let listed = sessions.first(where: { $0.id == open.session.id }),
                listed.codeCommits != open.session.codeCommits
            else { break }
            state.session?.session.codeCommits = listed.codeCommits
        default:
            break
        }
        return .none
    }

    private func restart(_ state: inout State, askedInHosts: Bool = false) -> Effect<Action> {
        state.daemon = .restarting
        return .run { send in
            try await service.restart()
            await send(.restarted)
        } catch: { error, send in
            // A daemon Humata didn't start is refused before anything is
            // stopped, so it still runs.
            if case DaemonServiceError.notManaged = error {
                await send(.restartRefused(error.localizedDescription, askedInHosts: askedInHosts))
            } else {
                await send(Self.failed(error))
            }
        }
    }

    /// Whether any session on this Mac is in a turn, which replacing the
    /// daemon would end. Hosts' sessions run in their own daemons and outlive
    /// this one, and an offline host's state is only what was last heard.
    static func anyBusy(_ sessions: [Session]) -> Bool {
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
