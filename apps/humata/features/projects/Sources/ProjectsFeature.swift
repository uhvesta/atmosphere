import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// The sidebar: the inbox, then every project with its board and its
/// sessions as a tree of forks, kept up to date; the sheets to add a project
/// and to start a session in one; and renaming, archiving, comparing and
/// marking the keeper.
@Reducer
package struct ProjectsFeature {
    @ObservableState
    package struct State: Equatable {
        package var projects: IdentifiedArrayOf<Project> = []
        package var sessions: IdentifiedArrayOf<Session> = []
        /// Every session waiting on you.
        package var inbox: [InboxEntry] = []
        /// Admin actions other clients asked for, oldest first.
        package var approvals: [AdminApproval] = []
        /// What the person has typed to each agent's request, kept until it's
        /// sent.
        package var drafts: [AgentRequest.ID: String] = [:]
        /// Requests answered here, which a read begun before the answer
        /// went would bring back. Few, so kept while Humata runs. The
        /// window reads it to show an open session's request as answered.
        package internal(set) var answered: Set<AgentRequest.ID> = []
        /// Permission requests answered here, by request id, kept out of
        /// reads begun before the answer for the same reason.
        var answeredPermissions: Set<String> = []
        /// The count last put on the Dock; an answer given here lowers the
        /// inbox before the read that follows, so the read compares with
        /// this, not with the inbox.
        package var badged: Int?
        /// Admin approvals decided here, kept out the same way.
        var decidedApprovals: Set<AdminApproval.ID> = []
        /// Sessions whose last message is being sent again from the inbox.
        package var retrying: Set<Session.ID> = []
        /// Sessions being moved, archived or deleted, which show a spinner
        /// and take no second go.
        package var busy: Set<Session.ID> = []
        /// Whether the list was read yet: until then nothing is new, so
        /// nothing is notified.
        package var loadedOnce = false
        /// What's open beside the sidebar.
        package var selection: SidebarItem?
        /// Why the list couldn't be read, and what to do about it; nil once
        /// it was.
        package var failure: String?
        /// The names to look for; empty to show every project.
        package var search = ""
        /// When the list was last read, for how long ago each session was
        /// active.
        package var now: Date?
        /// Where each session sorts: when it was first seen, or last changed
        /// its state or finished a turn. A reply streaming in doesn't move it,
        /// so the row under the pointer stays put.
        package var order: [Session.ID: Date] = [:]
        @Presents package var destination: Destination.State?
        /// An alert about something that finished while a sheet was open,
        /// shown once the sheet goes.
        var heldAlert: AlertState<Destination.Alert>?

        package init() {}

        /// Shows `alert` now, or once the sheet that's open goes, so a late
        /// result doesn't take away what the person is filling in.
        mutating func present(_ alert: AlertState<Destination.Alert>) {
            switch destination {
            case nil, .alert: destination = .alert(alert)
            default: heldAlert = alert
            }
        }

        /// What ⇧⌘A and ⇧⌘D answer in the inbox: the first admin action, or
        /// with none waiting the first blocked row that can be answered from
        /// where it is. It names the request, so the next request of the same
        /// session is another target.
        package var inboxShortcut: InboxShortcut? {
            if let approval = approvals.first { return .approval(approval.id) }
            for entry in inbox where entry.reason.blocks {
                if let request = entry.request {
                    // A move, or what an agent asks to do to another
                    // session, is allowed from where it is.
                    if request.kind != .question { return .request(entry: entry.id, request.id) }
                } else if entry.reason == .approval, !entry.session.hostOffline,
                    let permission = entry.session.pending.first
                {
                    return .permission(entry: entry.id, requestID: permission.requestID)
                }
            }
            return nil
        }

        /// The sessions in the inbox that can't go on without the person,
        /// as the Dock counts them.
        package var blockedCount: Int { inbox.count(where: \.reason.blocks) + approvals.count }

        /// The projects, the most recently active first. One with nothing in
        /// it yet is new, so it comes first.
        package var sortedProjects: [Project] {
            let active = Dictionary(grouping: sessions.filter { $0.projectID != nil }) { $0.projectID ?? "" }
                .mapValues { $0.map(activity).max() ?? .distantFuture }
            return newestFirst(Array(projects)) { active[$0.id] ?? .distantFuture }
        }

        /// What matches the search: the projects and the sessions, archived
        /// or not, whose names contain it; nil when nothing is searched for.
        package var matches: (projects: [Project], sessions: [Session])? {
            let text = search.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { return nil }
            return (
                sortedProjects.filter { $0.name.localizedStandardContains(text) },
                sorted(sessions.filter { $0.title.localizedStandardContains(text) })
            )
        }

        /// Where `session` sorts: its kept order, or when anything last
        /// happened in it. One where nothing has yet was just started.
        func activity(_ session: Session) -> Date {
            order[session.id] ?? session.lastActive ?? .distantFuture
        }

        /// Where a tree sorts: the latest of it and its forks.
        func activity(_ node: SessionNode) -> Date {
            ([activity(node.session)] + (node.children ?? []).map(activity)).max() ?? .distantFuture
        }

        /// `sessions`, the most recently active first, in the order given
        /// when as recent.
        package func sorted(_ sessions: [Session]) -> [Session] {
            newestFirst(sessions, by: activity)
        }

        /// The order of `sessions` as just read: a session new to the list,
        /// or whose state or turns changed, sorts by when it was last active;
        /// the rest keep where they were.
        static func order(
            of sessions: [Session], before: IdentifiedArrayOf<Session>, kept: [Session.ID: Date]
        ) -> [Session.ID: Date] {
            Dictionary(
                sessions.map { session in
                    let latest = session.lastActive ?? .distantFuture
                    guard let old = before[id: session.id], old.state == session.state, old.turns == session.turns
                    else { return (session.id, latest) }
                    return (session.id, kept[session.id] ?? latest)
                },
                uniquingKeysWith: { _, last in last })
        }

        package func sessions(in project: Project.ID) -> [Session] {
            sessions.filter { $0.projectID == project }
        }

        /// Chats: sessions in no project, started here or from the CLI.
        package var unfiled: [Session] {
            sessions.filter { $0.projectID == nil }
        }

        /// The project's sessions that aren't archived, each under the one
        /// it was forked from or whose agent started it; nil for sessions
        /// in no project. One whose parent is archived or elsewhere is at
        /// the top.
        package func tree(in project: Project.ID?) -> [SessionNode] {
            let shown = sessions.filter { $0.projectID == project && !$0.archived }
            let ids = Set(shown.map(\.id))
            let children = Dictionary(grouping: shown.filter { $0.nestsUnder.map(ids.contains) == true }) {
                $0.nestsUnder ?? ""
            }
            func node(_ session: Session) -> SessionNode {
                let under = sortedNodes((children[session.id] ?? []).map(node))
                return SessionNode(session: session, children: under.isEmpty ? nil : under)
            }
            return sortedNodes(shown.filter { $0.nestsUnder.map(ids.contains) != true }.map(node))
        }

        /// `nodes`, the one whose tree was most recently active first.
        func sortedNodes(_ nodes: [SessionNode]) -> [SessionNode] {
            newestFirst(nodes, by: activity)
        }

        /// Archived sessions in every project.
        package var archived: [Session] {
            sessions.filter(\.archived)
        }

        /// The sessions `session` can be compared with: the others in its
        /// project.
        package func comparable(to session: Session) -> [Session] {
            guard let project = session.projectID else { return [] }
            return sessions.filter { $0.projectID == project && $0.id != session.id }
        }
    }

    @Reducer
    package enum Destination {
        case newProject(NewProjectFeature)
        case newSession(NewSessionFeature)
        case importSession(ImportSessionFeature)
        case rename(RenameSessionFeature)
        case move(MoveSessionFeature)
        case compare(CompareFeature)
        case projectSettings(ProjectSettingsFeature)
        case alert(AlertState<Alert>)

        package enum Alert: Equatable {
            case deleteProject(Project.ID)
            case deleteSession(Session.ID)
            /// Moves the session to the host, or to this Mac for "".
            case moveSession(Session.ID, host: String)
            /// Answers a request to close an item, stopping and archiving its
            /// sessions after if `archive`.
            case closeItem(AgentRequest.ID, RequestAnswer, archive: Bool)
        }
    }

    package enum Action {
        /// The sidebar appeared; reads the list now and again every few
        /// seconds, until it goes.
        case task
        /// Reads the list once.
        case refresh
        case loaded(projects: [Project], sessions: [Session], inbox: [InboxEntry], approvals: [AdminApproval] = [])
        case loadFailed(String)
        case addProjectTapped
        case newSessionTapped(projectID: Project.ID)
        /// A session that plans in the project, proposing tasks and specs,
        /// with no copy of the code.
        case planTapped(projectID: Project.ID)
        case projectSettingsTapped(Project.ID)
        /// A session in no project: a chat, in a folder or its own.
        case newChatTapped
        case importTapped
        case searchChanged(String)
        /// Asks before deleting the project, or says what to do first.
        case deleteProjectTapped(Project.ID)
        case projectDeleted(Project.ID)
        case deleteFailed(String)
        case selected(SidebarItem?)
        case renameTapped(Session.ID)
        case moveTapped(Session.ID)
        case archiveTapped(Session.ID, archived: Bool)
        /// Stops the session's turn, then archives it.
        case stopAndArchiveTapped(Session.ID)
        /// Asks before deleting an archived session for good, or says what
        /// to do first.
        case deleteSessionTapped(Session.ID)
        case sessionDeleted(Session.ID)
        case sessionDeleteFailed(Session.ID, String)
        /// Asks where to move a session: this Mac or one of the hosts.
        case moveToHostTapped(Session.ID)
        case moveHostsLoaded(Session.ID, hosts: [String])
        case sessionMoved(Session)
        case sessionMoveFailed(Session.ID, String)
        case keeperTapped(Session.ID, keeper: Bool)
        case compareTapped(Session.ID, with: Session.ID)
        case updated(Session)
        case updateFailed(Session.ID, String)
        /// The person allowed or denied a request from the inbox.
        case inboxAnswered(Session.ID, request: String, approved: Bool)
        /// The person let the session do more without asking, from the
        /// inbox, which releases the requests the new mode allows.
        case inboxModeWidened(Session.ID, ApprovalMode)
        /// Sends a failed session's last message again, from the inbox.
        case inboxRetryTapped(Session.ID)
        /// Opens the next session blocked on the person after the one open,
        /// or the inbox if none is.
        case nextWaitingTapped
        /// The person answered an agent's question or move from the inbox.
        case requestAnswered(AgentRequest.ID, RequestAnswer)
        /// The answer reached the agent, so its draft can go.
        case requestAnswerSent(AgentRequest.ID)
        case draftChanged(AgentRequest.ID, String)
        /// The person read a session's reply in the inbox without opening it.
        case markReadTapped(Session.ID)
        /// The person opened what a session changed, to look before moving
        /// its item.
        case changesTapped(Session.ID)
        case answerFailed(InboxCall, String)
        /// Sending a session's last message again from the inbox went.
        case inboxRetried(Session.ID)
        /// The answer to request `id` didn't go, so it's shown again.
        case requestNotAnswered(AgentRequest.ID)
        /// The person approved or denied admin actions other clients asked
        /// for; approving can let those of `grant`'s scopes run without
        /// asking for its seconds.
        case approvalsDecided([AdminApproval.ID], approve: Bool, grant: ApprovalGrantAsked? = nil)
        /// The person clicked the notification for a session.
        case notificationClicked(Session.ID)
        case destination(PresentationAction<Destination.Action>)
        case delegate(Delegate)

        /// What the inbox was doing when a call failed, so the alert says
        /// so and what was hidden comes back.
        package enum InboxCall: Equatable, Sendable {
            case answeringPermission(String)
            case decidingApprovals([AdminApproval.ID])
            case wideningMode
            case retrying(Session.ID)
            case answeringRequest
            /// The request was answered, but the session named wasn't
            /// stopped and archived after.
            case closingAfterAnswer(String)
        }

        @CasePathable
        package enum Delegate: Equatable {
            case openSession(Session)
            /// Opens the session at its changes.
            case openChanges(Session)
            case openBoard(Project)
            case openInbox
            case openHosts
            case openSearch
        }
    }

    /// How often the list is read again, for sessions' states.
    static let refreshInterval = Duration.seconds(2)
    /// How many reads in a row with nothing new go untaken before one is
    /// taken in all the same: a change made here that the daemon didn't
    /// keep is put right by then.
    static let unchangedReads = 4

    /// What a read changes in the sidebar, to tell a read with nothing new
    /// from one to take in: the lists as the sidebar keeps them, and the
    /// minute its times are shown against.
    private enum Shown: Equatable {
        case lists([Project], [Session], [InboxEntry], [AdminApproval], minute: Date)
        case failure(String)
        case other

        init(_ read: Action, at now: Date) {
            switch read {
            case .loaded(let projects, let sessions, let inbox, let approvals):
                // As `.loaded` keeps them: times to the minute, and not what
                // a session last said, which only the inbox's entries show.
                let sessions = sessions.map { session -> Session in
                    var session = session
                    session.lastActive = session.lastActive?.toTheMinute
                    session.lastMessage = nil
                    return session
                }
                let inbox = inbox.map { entry -> InboxEntry in
                    var entry = entry
                    entry.session.lastActive = entry.session.lastActive?.toTheMinute
                    return entry
                }
                self = .lists(projects, sessions, inbox, approvals, minute: now.toTheMinute)
            case .loadFailed(let message):
                self = .failure(message)
            default:
                self = .other
            }
        }
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.appActivity) var activity
    @Dependency(\.continuousClock) var clock
    @Dependency(\.notifications) var notifications
    @Dependency(\.date) var date

    private enum CancelID { case refresh, clicks }

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                return .merge(
                    .run { send in
                        // Most reads show what the last did. They're told
                        // apart here, off the main thread, and only one
                        // with something new is taken in.
                        var shown: Shown?
                        var unchanged = 0
                        while !Task.isCancelled {
                            let read = await load()
                            let shows = Shown(read, at: date.now)
                            if shows != shown || unchanged >= Self.unchangedReads {
                                shown = shows
                                unchanged = 0
                                await send(read)
                            } else {
                                unchanged += 1
                            }
                            try await clock.sleep(for: Self.refreshInterval)
                        }
                    }
                    .cancellable(id: CancelID.refresh, cancelInFlight: true),
                    .run { send in
                        for await session in await notifications.clicks() {
                            await send(.notificationClicked(session))
                        }
                    }
                    .cancellable(id: CancelID.clicks, cancelInFlight: true),
                    .run { _ in _ = await notifications.authorize() }
                )

            case .refresh:
                return .run { send in await send(await load()) }

            case .loaded(let projects, let sessions, let inbox, let approvals):
                if state.failure != nil { state.failure = nil }
                // Answered here, a request stays hidden, even from a read
                // that began before the answer and arrives after a newer one.
                let answered = state.answered
                let permissions = state.answeredPermissions
                let inbox = inbox.compactMap { entry -> InboxEntry? in
                    var entry = entry
                    // How long it has waited shows to the minute, and a
                    // session at work again is active at every read: with
                    // the seconds dropped the read leaves the inbox equal,
                    // and nothing showing it is drawn again.
                    entry.session.lastActive = entry.session.lastActive?.toTheMinute
                    if let request = entry.request { return answered.contains(request.id) ? nil : entry }
                    guard !permissions.isEmpty,
                        entry.session.pending.contains(where: { permissions.contains($0.requestID) })
                    else { return entry }
                    entry.session.pending.removeAll { permissions.contains($0.requestID) }
                    return entry.reason == .approval && entry.session.pending.isEmpty ? nil : entry
                }
                let decided = state.decidedApprovals
                let approvals = decided.isEmpty ? approvals : approvals.filter { !decided.contains($0.id) }
                // The open session needs no notice only while it's on screen.
                let open = activity.isActive() ? state.selection : nil
                let notices = state.loadedOnce ? Self.notices(new: inbox, old: state.inbox, open: open) : []
                // The Dock counts only the sessions that can't go on without
                // the person.
                let blocked = inbox.count(where: \.reason.blocks) + approvals.count
                let badge = blocked != state.badged ? blocked : nil
                if badge != nil { state.badged = blocked }
                if !state.loadedOnce {
                    state.loadedOnce = true
                    // Humata opens on what waits on the person, if anything
                    // does and nothing else was picked yet.
                    if blocked > 0, state.selection == nil { state.selection = .inbox }
                }
                // Times show to the minute; a new `now` each read would draw
                // every row again.
                let now = date.now.toTheMinute
                if state.now != now { state.now = now }
                // Read every two seconds and mostly the same: set only what
                // changed, so the sidebar isn't drawn again for nothing.
                if state.inbox != inbox { state.inbox = inbox }
                if state.approvals != approvals { state.approvals = approvals }
                let projects = IdentifiedArray(uniqueElements: projects)
                if state.projects != projects { state.projects = projects }
                let order = State.order(of: sessions, before: state.sessions, kept: state.order)
                if state.order != order { state.order = order }
                // A working session is active again at every read. The rows
                // show when to the minute and sort by `order`, so the
                // seconds are dropped and the read leaves them equal.
                // What a session last said changes with every reply, and
                // only the inbox shows it, from its own entries: here it's
                // left out, so a reply doesn't draw the sidebar again.
                let listed = sessions.map { session -> Session in
                    var session = session
                    session.lastActive = session.lastActive?.toTheMinute
                    session.lastMessage = nil
                    return session
                }
                let sessions = IdentifiedArray(listed, uniquingIDsWith: { _, last in last })
                if state.sessions != sessions { state.sessions = sessions }
                guard !notices.isEmpty || badge != nil else { return .none }
                return .run { [activity] _ in
                    // The Dock says how many sessions wait on the person.
                    if let badge { activity.setBadge(badge) }
                    for notice in notices { await notifications.post(notice) }
                }

            case .notificationClicked(let id):
                return .send(.selected(.session(id)))

            case .loadFailed(let message):
                state.failure = message
                return .none

            case .addProjectTapped:
                state.destination = .newProject(NewProjectFeature.State())
                return .none

            case .newSessionTapped(let id):
                guard let project = state.projects[id: id] else { return .none }
                state.destination = .newSession(NewSessionFeature.State(project: project))
                return .none

            case .planTapped(let id):
                guard let project = state.projects[id: id] else { return .none }
                state.destination = .newSession(NewSessionFeature.State(project: project, planning: true))
                return .none

            case .projectSettingsTapped(let id):
                guard let project = state.projects[id: id] else { return .none }
                state.destination = .projectSettings(ProjectSettingsFeature.State(project: project))
                return .none

            case .destination(.presented(.projectSettings(.delegate(.saved(let project))))):
                state.projects[id: project.id] = project
                state.destination = nil
                return .none

            case .newChatTapped:
                state.destination = .newSession(NewSessionFeature.State(project: nil))
                return .none

            case .importTapped:
                state.destination = .importSession(ImportSessionFeature.State(projects: state.sortedProjects))
                return .none

            case .searchChanged(let text):
                state.search = text
                return .none

            case .deleteProjectTapped(let id):
                guard let project = state.projects[id: id] else { return .none }
                let working = state.sessions(in: id).filter { !$0.archived }.count
                guard working == 0 else {
                    state.destination = .alert(
                        AlertState {
                            TextState("Archive \(project.name)'s Sessions First")
                        } message: {
                            TextState(
                                working == 1
                                    ? "A session still works in \(project.name). Archive it, then delete the project."
                                    : "\(working) sessions still work in \(project.name). Archive them, then delete "
                                        + "the project.")
                        })
                    return .none
                }
                state.destination = .alert(
                    AlertState {
                        TextState("Delete \(project.name)?")
                    } actions: {
                        ButtonState(role: .destructive, action: .deleteProject(id)) {
                            TextState("Delete Project")
                        }
                        ButtonState(role: .cancel) {
                            TextState("Cancel")
                        }
                    } message: {
                        TextState(
                            "Its board and specs are deleted, and can't be brought back. Its archived sessions are "
                                + "kept, under Archived, and its repositories are left as they are.")
                    })
                return .none

            case .destination(.presented(.alert(.deleteProject(let id)))):
                return .run { send in
                    try await daemon.deleteProject(id)
                    await send(.projectDeleted(id))
                    await send(await load())
                } catch: { error, send in
                    await send(.deleteFailed(error.localizedDescription))
                }

            case .projectDeleted(let id):
                state.projects.remove(id: id)
                for session in state.sessions where session.projectID == id {
                    state.sessions[id: session.id]?.projectID = nil
                    state.sessions[id: session.id]?.itemID = nil
                }
                if state.selection == .board(id) { state.selection = nil }
                return .none

            case .deleteFailed(let message):
                state.present(
                    AlertState {
                        TextState("Couldn't delete the project")
                    } message: {
                        TextState(message)
                    })
                return .none

            case .selected(let item):
                state.selection = item
                switch item {
                case .inbox:
                    return .send(.delegate(.openInbox))
                case .hosts:
                    return .send(.delegate(.openHosts))
                case .search:
                    return .send(.delegate(.openSearch))
                case .board(let id):
                    guard let project = state.projects[id: id] else { return .none }
                    return .send(.delegate(.openBoard(project)))
                case .session(let id):
                    guard let session = state.sessions[id: id] else { return .none }
                    return .send(.delegate(.openSession(session)))
                case nil:
                    return .none
                }

            case .renameTapped(let id):
                guard let session = state.sessions[id: id] else { return .none }
                state.destination = .rename(RenameSessionFeature.State(session: session))
                return .none

            case .moveTapped(let id):
                guard let session = state.sessions[id: id] else { return .none }
                state.destination = .move(MoveSessionFeature.State(session: session))
                return .none

            case .archiveTapped(let id, let archived):
                // A running turn holds an archive: Stop and Archive ends it first.
                if archived, let session = state.sessions[id: id], state.hold(on: session) == .inTurn {
                    return .none
                }
                guard state.busy.insert(id).inserted else { return .none }
                return update(id, SessionUpdate(archived: archived))

            case .stopAndArchiveTapped(let id):
                guard state.busy.insert(id).inserted else { return .none }
                return .run { send in
                    try await daemon.stopAll(id)
                    await send(.updated(try await daemon.update(id, SessionUpdate(archived: true))))
                } catch: { error, send in
                    await send(.updateFailed(id, error.localizedDescription))
                }

            case .deleteSessionTapped(let id):
                guard let session = state.sessions[id: id] else { return .none }
                if let fork = state.sessions.first(where: { $0.parentID == id }) {
                    state.destination = .alert(
                        AlertState {
                            TextState("Delete \(fork.title) First")
                        } message: {
                            TextState(
                                "\(fork.title) was forked from \(session.title). Archive and delete it, then "
                                    + "\(session.title).")
                        })
                    return .none
                }
                state.destination = .alert(
                    AlertState {
                        TextState("Delete \(session.title)?")
                    } actions: {
                        ButtonState(role: .destructive, action: .deleteSession(id)) {
                            TextState("Delete Session")
                        }
                        ButtonState(role: .cancel) {
                            TextState("Cancel")
                        }
                    } message: {
                        TextState(
                            "Its history, its copy of the code and its branch are deleted, and can't be brought "
                                + "back. Its task stays on the board.")
                    })
                return .none

            case .destination(.presented(.alert(.deleteSession(let id)))):
                guard state.busy.insert(id).inserted else { return .none }
                return .run { send in
                    try await daemon.delete(id)
                    await send(.sessionDeleted(id))
                } catch: { error, send in
                    await send(.sessionDeleteFailed(id, error.localizedDescription))
                }

            case .sessionDeleted(let id):
                state.busy.remove(id)
                state.sessions.remove(id: id)
                if state.selection == .session(id) { state.selection = nil }
                return .none

            case .sessionDeleteFailed(let id, let message):
                state.busy.remove(id)
                state.present(
                    AlertState {
                        TextState("Couldn't delete the session")
                    } message: {
                        TextState(message)
                    })
                return .none

            case .moveToHostTapped(let id):
                // It moves with its files, so not while they're changing.
                if let session = state.sessions[id: id], state.hold(on: session) != nil { return .none }
                // Reading the hosts asks each one, which can take a while:
                // the session shows it's busy and takes no second go.
                guard state.busy.insert(id).inserted else { return .none }
                return .run { send in
                    let hosts = try await daemon.hosts().map(\.name).filter { !$0.isEmpty }
                    await send(.moveHostsLoaded(id, hosts: hosts))
                } catch: { error, send in
                    await send(.sessionMoveFailed(id, error.localizedDescription))
                }

            case .moveHostsLoaded(let id, let hosts):
                state.busy.remove(id)
                guard let session = state.sessions[id: id] else { return .none }
                let here = session.host ?? ""
                let targets = ([""] + hosts).filter { $0 != here }
                guard !targets.isEmpty else {
                    state.present(
                        AlertState {
                            TextState("No Host to Move To")
                        } message: {
                            TextState("Add a host under Hosts, then move \(session.title) to it.")
                        })
                    return .none
                }
                state.present(
                    AlertState {
                        TextState("Move \(session.title) to…")
                    } actions: {
                        for target in targets {
                            ButtonState(action: .moveSession(id, host: target)) {
                                TextState(target.isEmpty ? "This Mac" : target)
                            }
                        }
                        ButtonState(role: .cancel) {
                            TextState("Cancel")
                        }
                    } message: {
                        TextState(
                            "It goes with every turn and its files, and its copy where it is now is deleted. "
                                + "Its agent's backend must be set up where it goes.")
                    })
                return .none

            case .destination(.presented(.alert(.moveSession(let id, let host)))):
                guard state.busy.insert(id).inserted else { return .none }
                return .run { send in
                    let moved = try await daemon.restoreSession(id, host.isEmpty ? nil : host)
                    await send(.sessionMoved(moved))
                    await send(await load())
                } catch: { error, send in
                    await send(.sessionMoveFailed(id, error.localizedDescription))
                }

            case .sessionMoved(let session):
                state.busy.remove(session.id)
                state.sessions[id: session.id] = session
                return .none

            case .sessionMoveFailed(let id, let message):
                state.busy.remove(id)
                state.present(
                    AlertState {
                        TextState("Couldn't move the session")
                    } message: {
                        TextState(message)
                    })
                return .none

            case .keeperTapped(let id, let keeper):
                return update(id, SessionUpdate(keeper: keeper))

            case .compareTapped(let id, let other):
                guard let left = state.sessions[id: id], let right = state.sessions[id: other] else { return .none }
                state.destination = .compare(CompareFeature.State(left: left, right: right))
                return .none

            case .updated(let session),
                .destination(.presented(.rename(.delegate(.renamed(let session))))),
                .destination(.presented(.move(.delegate(.moved(let session))))),
                .destination(.presented(.compare(.delegate(.updated(let session))))):
                state.busy.remove(session.id)
                state.sessions[id: session.id] = session
                switch state.destination {
                case .rename, .move: state.destination = nil
                default: break
                }
                return .none

            case .inboxAnswered(let id, let request, let approved):
                // Gone from the row at once, and from reads begun before
                // the answer; the refresh says what's next.
                guard state.answeredPermissions.insert(request).inserted else { return .none }
                if let index = state.inbox.firstIndex(where: { $0.session.id == id && $0.request == nil }) {
                    state.inbox[index].session.pending.removeAll { $0.requestID == request }
                    if state.inbox[index].reason == .approval, state.inbox[index].session.pending.isEmpty {
                        state.inbox.remove(at: index)
                    }
                }
                return .run { send in
                    try await daemon.approve(id, request, approved)
                    await send(await load())
                } catch: { error, send in
                    await send(.answerFailed(.answeringPermission(request), error.localizedDescription))
                    await send(await load())
                }

            case .approvalsDecided(let ids, let approve, let grant):
                // Gone from the inbox at once, and from reads begun before;
                // the refresh says what ran.
                let ids = ids.filter { state.decidedApprovals.insert($0).inserted }
                guard !ids.isEmpty else { return .none }
                state.approvals.removeAll { ids.contains($0.id) }
                return .run { send in
                    _ = try await daemon.decideApprovals(
                        ids, approve, grant?.scopes ?? [], grant.map { UInt32($0.duration.components.seconds) } ?? 0)
                    await send(await load())
                } catch: { error, send in
                    await send(.answerFailed(.decidingApprovals(ids), error.localizedDescription))
                    await send(await load())
                }

            case .inboxModeWidened(let id, let mode):
                return .run { send in
                    try await daemon.setApprovalMode(id, mode)
                    await send(await load())
                } catch: { error, send in
                    await send(.answerFailed(.wideningMode, error.localizedDescription))
                    await send(await load())
                }

            case .inboxRetryTapped(let id):
                guard state.retrying.insert(id).inserted else { return .none }
                return .run { send in
                    let history = try await daemon.history(id)
                    let last = history.reversed().lazy.compactMap { event -> (text: String, images: [String])? in
                        guard case .userMessage(let text, let images) = event.kind else { return nil }
                        return (text, images)
                    }.first
                    guard let last else {
                        await send(
                            .answerFailed(
                                .retrying(id),
                                "This session has no message of yours to send again. "
                                    + "Open the session and write to it."))
                        return
                    }
                    if last.images.isEmpty {
                        try await daemon.send(id, last.text, .enqueue)
                    } else {
                        // Its images go with it again; one that can't be
                        // read fails the retry, as in the chat.
                        var attachments: [ImageAttachment] = []
                        for name in last.images { attachments.append(try await daemon.readImage(id, name)) }
                        try await daemon.sendImages(id, last.text, attachments, .enqueue)
                    }
                    await send(.inboxRetried(id))
                    await send(await load())
                } catch: { error, send in
                    await send(.answerFailed(.retrying(id), error.localizedDescription))
                    await send(await load())
                }

            case .inboxRetried(let id):
                state.retrying.remove(id)
                return .none

            case .nextWaitingTapped:
                var blocked: [Session.ID] = []
                for entry in state.inbox where entry.reason.blocks && !blocked.contains(entry.session.id) {
                    blocked.append(entry.session.id)
                }
                guard !blocked.isEmpty else { return .send(.selected(.inbox)) }
                var next = blocked[0]
                if case .session(let open)? = state.selection, let index = blocked.firstIndex(of: open) {
                    next = blocked[(index + 1) % blocked.count]
                }
                return .send(.selected(.session(next)))

            case .requestAnswered(let id, let answer):
                // Answered already, from the session's strip or here: a
                // second answer would act twice, as allowing a start does.
                guard !state.answered.contains(id) else { return .none }
                let live = state.closing(id, with: answer)
                if !live.isEmpty {
                    state.destination = .alert(
                        AlertState {
                            TextState("Stop and archive its sessions?")
                        } actions: {
                            ButtonState(action: .closeItem(id, answer, archive: true)) {
                                TextState(
                                    live.count == 1
                                        ? "Stop and Archive 1 Session" : "Stop and Archive \(live.count) Sessions")
                            }
                            ButtonState(action: .closeItem(id, answer, archive: false)) {
                                TextState("Keep Them")
                            }
                            ButtonState(role: .cancel) {
                                TextState("Cancel")
                            }
                        } message: {
                            TextState(
                                "The item closes. Its sessions keep running and asking for approval until "
                                    + "they're archived; an archived session can be unarchived later.")
                        })
                    return .none
                }
                return answerRequest(&state, id, answer, archiving: [])

            case .destination(.presented(.alert(.closeItem(let id, let answer, let archive)))):
                guard !state.answered.contains(id) else { return .none }
                let live = archive ? state.closing(id, with: answer) : []
                return answerRequest(&state, id, answer, archiving: live)

            case .requestNotAnswered(let id):
                state.answered.remove(id)
                return .none

            case .requestAnswerSent(let id):
                state.drafts[id] = nil
                return .none

            case .draftChanged(let id, let text):
                state.drafts[id] = text.isEmpty ? nil : text
                return .none

            case .markReadTapped(let id):
                guard let entry = state.inbox.first(where: { $0.session.id == id && $0.reason == .reply })
                else { return .none }
                state.inbox.removeAll { $0.id == entry.id }
                let turns = entry.session.turns
                return .run { send in
                    try await daemon.markSeen(id, turns)
                    await send(await load())
                } catch: { _, send in
                    // It stays in the inbox, as the daemon still has it.
                    await send(await load())
                }

            case .changesTapped(let id):
                guard let session = state.sessions[id: id] else { return .none }
                state.selection = .session(id)
                return .send(.delegate(.openChanges(session)))

            case .answerFailed(let call, let message):
                let title: String
                switch call {
                case .answeringPermission(let request):
                    state.answeredPermissions.remove(request)
                    title = "Couldn't answer the permission request"
                case .decidingApprovals(let ids):
                    state.decidedApprovals.subtract(ids)
                    title = "Couldn't decide on the admin actions"
                case .wideningMode:
                    title = "Couldn't change what the agent may do"
                case .retrying(let id):
                    state.retrying.remove(id)
                    title = "Couldn't send the message again"
                case .answeringRequest:
                    title = "Couldn't answer the request"
                case .closingAfterAnswer(let name):
                    title = "Couldn't stop and archive \(name)"
                }
                state.present(
                    AlertState {
                        TextState(title)
                    } message: {
                        TextState(message)
                    })
                return .none

            case .updateFailed(let id, let message):
                state.busy.remove(id)
                state.present(
                    AlertState {
                        TextState("Couldn't change the session")
                    } message: {
                        TextState(message)
                    })
                return .none

            case .destination(.presented(.newProject(.delegate(.created(let project))))):
                state.projects.append(project)
                state.destination = .newSession(NewSessionFeature.State(project: project))
                return .none

            case .destination(.presented(.newSession(.delegate(.started(let session))))),
                .destination(.presented(.importSession(.delegate(.imported(let session))))):
                state.sessions.append(session)
                state.destination = nil
                state.selection = .session(session.id)
                return .send(.delegate(.openSession(session)))

            case .destination, .delegate:
                return .none
            }
        }
        .ifLet(\.$destination, action: \.destination)
        // A held alert shows once the sheet it waited behind is gone.
        Reduce { state, _ in
            guard state.destination == nil, let alert = state.heldAlert else { return .none }
            state.heldAlert = nil
            state.destination = .alert(alert)
            return .none
        }
    }

    private func update(_ id: Session.ID, _ update: SessionUpdate) -> Effect<Action> {
        .run { send in
            await send(.updated(try await daemon.update(id, update)))
        } catch: { error, send in
            await send(.updateFailed(id, error.localizedDescription))
        }
    }

    /// What to tell the person about the sessions that came into the inbox,
    /// or are there for another reason now, but the one they have open.
    static func notices(new: [InboxEntry], old: [InboxEntry], open: SidebarItem?) -> [Notice] {
        let before = Dictionary(old.map { ($0.id, $0.reason) }, uniquingKeysWith: { first, _ in first })
        return new.filter { before[$0.id] != $0.reason && open != .session($0.session.id) }.map(Notice.init)
    }

    /// Answers request `id`, then stops and archives `sessions`.
    private func answerRequest(
        _ state: inout State, _ id: AgentRequest.ID, _ answer: RequestAnswer, archiving sessions: [Session]
    ) -> Effect<Action> {
        state.inbox.removeAll { $0.request?.id == id }
        state.answered.insert(id)
        return .run { send in
            do {
                try await daemon.answerRequest(id, answer)
            } catch {
                await send(.requestNotAnswered(id))
                throw error
            }
            await send(.requestAnswerSent(id))
            // The answer went, so a session that can't be stopped or
            // archived is its own failure, and the others still close.
            for session in sessions {
                do {
                    if session.state == .working || session.state == .waitingForApproval {
                        try await daemon.stopAll(session.id)
                    }
                    _ = try await daemon.update(session.id, SessionUpdate(archived: true))
                } catch {
                    await send(
                        .answerFailed(
                            .closingAfterAnswer(session.name),
                            "The request was answered and the task moved, but this session wasn't stopped "
                                + "and archived: \(error.localizedDescription) Archive it from the sidebar."))
                }
            }
            await send(await load())
        } catch: { error, send in
            await send(.answerFailed(.answeringRequest, error.localizedDescription))
            await send(await load())
        }
    }

    private func load() async -> Action {
        do {
            async let projects = daemon.projects()
            async let sessions = daemon.sessions(nil)
            async let inbox = daemon.inbox()
            let read = try await inbox
            return .loaded(
                projects: try await projects, sessions: try await sessions, inbox: read.entries,
                approvals: read.approvals)
        } catch {
            return .loadFailed(error.localizedDescription)
        }
    }
}

extension ProjectsFeature.Destination.State: Equatable {}

/// What keeps a session from being archived or moved to a host now.
enum SessionHold: Equatable, Sendable {
    /// It's being moved, archived or deleted already.
    case busy
    /// Its turn is running.
    case inTurn
}

/// What the menu bar needs of a session to offer its commands: Archive or
/// Unarchive by `archived`, and nothing that archives or moves it while
/// it's `held`.
package struct SessionMenuState: Equatable, Sendable {
    package var archived: Bool
    /// Whether something keeps it from being archived or moved now: its
    /// turn runs, or it's being moved, archived or deleted.
    package var held: Bool
    /// Whether it's being moved, archived or deleted already, so it takes
    /// no rename or delete either.
    package var busy: Bool
}

extension ProjectsFeature.State {
    /// The project the menu bar's commands act on: the selected board's,
    /// else the selected session's; nil when neither is in a project the
    /// sidebar lists.
    package var commandProject: Project.ID? {
        let id: Project.ID?
        switch selection {
        case .board(let project): id = project
        case .session(let session): id = sessions[id: session]?.projectID
        case .inbox, .hosts, .search, nil: id = nil
        }
        return id.flatMap { projects[id: $0]?.id }
    }

    /// What the menu bar needs of session `id`; nil when the sidebar
    /// doesn't list it.
    package func menuState(of id: Session.ID) -> SessionMenuState? {
        guard let session = sessions[id: id] else { return nil }
        let hold = hold(on: session)
        return SessionMenuState(archived: session.archived, held: hold != nil, busy: hold == .busy)
    }

    /// What keeps `session` from being archived or moved now; nil when
    /// nothing does.
    func hold(on session: Session) -> SessionHold? {
        if busy.contains(session.id) { return .busy }
        if session.state == .working || session.state == .waitingForApproval { return .inTurn }
        return nil
    }

    /// The unarchived sessions on the item that answering request `id` with
    /// `answer` closes; none if it closes none.
    fileprivate func closing(_ id: AgentRequest.ID, with answer: RequestAnswer) -> [Session] {
        guard let request = inbox.lazy.compactMap(\.request).first(where: { $0.id == id }),
            case .move(let item, let asked) = request.kind
        else { return [] }
        let status: ItemStatus? =
            switch answer {
            case .approve: asked
            case .move(let other): other
            case .reply, .dismiss: nil
            }
        guard let status, status.closed else { return [] }
        return sessions.filter { $0.itemID == item && !$0.archived }
    }
}

/// `items`, the latest by `time` first, in the order given when as recent.
private func newestFirst<Item>(_ items: [Item], by time: (Item) -> Date) -> [Item] {
    items.enumerated().sorted { left, right in
        let (earlier, later) = (time(left.element), time(right.element))
        return earlier == later ? left.offset < right.offset : earlier > later
    }
    .map(\.element)
}

/// A row of the sidebar to open.
package enum SidebarItem: Hashable, Sendable {
    case inbox
    case hosts
    /// Searching what every session said and did.
    case search
    case board(Project.ID)
    case session(Session.ID)
}

/// What ⇧⌘A and ⇧⌘D answer in the inbox, by the request rather than its row.
package enum InboxShortcut: Hashable, Sendable {
    case approval(AdminApproval.ID)
    /// The first permission request of the session in row `entry`.
    case permission(entry: InboxEntry.ID, requestID: String)
    /// An agent's request to move its item, to start a session or to
    /// write to one, in row `entry`.
    case request(entry: InboxEntry.ID, AgentRequest.ID)
}

/// A session in the sidebar's tree, with the forks made from it.
package struct SessionNode: Identifiable, Hashable, Sendable {
    package var session: Session
    /// Nil when nothing was forked from it, so it shows no disclosure.
    package var children: [SessionNode]?

    package var id: Session.ID { session.id }
}

/// A new project: its name, its repositories' folders on this Mac and
/// instructions for its sessions.
@Reducer
package struct NewProjectFeature {
    @ObservableState
    package struct State: Equatable {
        package var name = ""
        /// The repositories' folders, as absolute paths, or URLs to clone.
        /// Each session works across all of them.
        package var repositories: IdentifiedArrayOf<RepositoryRow> = [RepositoryRow(id: 0)]
        var nextRow = 1
        package var instructions = ""
        package var creating = false
        /// The saved repositories and folders offered under the row typed in.
        package var search = RepositorySearchFeature.State()
        @Presents package var alert: AlertState<Never>?

        package init() {}

        /// The folders and URLs filled in so far.
        var paths: [String] {
            repositories.map(\.path.trimmed).filter { !$0.isEmpty }
        }

        package var canCreate: Bool {
            !creating && !name.trimmed.isEmpty && !paths.isEmpty
                && repositories.allSatisfy { $0.path.trimmed.isEmpty || $0.repository != nil }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// A folder was picked: it fills the first empty row, or adds one,
        /// and names the project if it has no name yet.
        case folderPicked(URL)
        case addRepositoryTapped
        case removeRepositoryTapped(RepositoryRow.ID)
        case createTapped
        case failed(String)
        case search(RepositorySearchFeature.Action)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case created(Project)
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Scope(state: \.search, action: \.search) { RepositorySearchFeature() }
        Reduce { state, action in
            switch action {
            case .search(.delegate(.picked(let row, let suggestion))):
                // The project keeps its own copy of what was picked.
                state.repositories[id: row]?.path = suggestion.location
                if state.name.trimmed.isEmpty { state.name = suggestion.name }
                return .none

            case .binding, .alert, .delegate, .search:
                return .none

            case .folderPicked(let url):
                var path = url.standardizedFileURL.path(percentEncoded: false)
                while path.count > 1, path.hasSuffix("/") { path.removeLast() }
                guard !state.paths.contains(path) else { return .none }
                if let empty = state.repositories.first(where: { $0.path.trimmed.isEmpty }) {
                    state.repositories[id: empty.id]?.path = path
                } else {
                    state.addRow(path)
                }
                if state.name.trimmed.isEmpty { state.name = url.lastPathComponent }
                return .none

            case .addRepositoryTapped:
                state.addRow("")
                return .none

            case .removeRepositoryTapped(let id):
                state.repositories.remove(id: id)
                if state.repositories.isEmpty { state.addRow("") }
                return .none

            case .createTapped:
                guard state.canCreate else { return .none }
                state.creating = true
                let (name, repositories, instructions) = (
                    state.name.trimmed, state.repositories.compactMap(\.repository), state.instructions
                )
                return .run { send in
                    let project = try await daemon.createProject(name, repositories, instructions)
                    await send(.delegate(.created(project)))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .failed(let message):
                state.creating = false
                state.alert = AlertState {
                    TextState("Couldn't add the project")
                } message: {
                    TextState(message)
                }
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }
}

/// A row of a project's repositories, as it's being written.
package struct RepositoryRow: Identifiable, Hashable, Sendable {
    package var id: Int
    /// Its folder's name in a session's copy; empty to name it after its
    /// folder or URL.
    package var name = ""
    /// The folder as typed or picked, or the URL to clone.
    package var path = ""

    package init(id: Int, name: String = "", path: String = "") {
        self.id = id
        self.name = name
        self.path = path
    }

    /// The row as one of the project's, if it's filled in with a URL or an
    /// absolute path.
    package var repository: Repository? {
        let (name, path) = (name.trimmed, path.trimmed)
        if Repository.isURL(path) { return Repository(name: name, url: path) }
        return path.hasPrefix("/") ? Repository(name: name, path: path) : nil
    }

    /// Whether something is typed that is neither a URL nor a path from /,
    /// which keeps the project from being saved.
    package var needsFullPath: Bool { !path.trimmed.isEmpty && repository == nil }
}

extension NewProjectFeature.State {
    fileprivate mutating func addRow(_ path: String) {
        repositories.append(RepositoryRow(id: nextRow, path: path))
        nextRow += 1
    }
}

/// A new session in a project, or on one of its tasks or subtasks, or a
/// chat in no project, in a folder picked or one of its own: its backend
/// and model, what it may do without asking, and which specs it reads.
@Reducer
package struct NewSessionFeature {
    @ObservableState
    package struct State: Equatable {
        /// Nil for a chat.
        package var project: Project?
        /// The folder a chat works in; nil for one of its own.
        package var folder: String?
        /// The task or subtask it works on; nil for the project itself.
        package var item: Item?
        /// Whether a session on a subtask also reads its task's spec.
        package var specs = SpecScope.withParents
        /// Plans in the project with no copy of the code; only for a
        /// session on the project itself.
        package var planning = false
        package var name = ""
        package var backend = Backend.copilot
        /// Empty for the backend's default.
        package var model = ""
        /// How hard the model thinks; empty for its default. For Copilot
        /// Fusion, its tier.
        package var effort = ""
        /// Copilot's experimental features, such as local models in Fusion.
        package var experimental = false
        /// Changed by the person, so the defaults, once read, leave the
        /// agent, model and effort as they are.
        package var picked = false
        package var approvalMode = ApprovalMode.askEveryTime
        /// An OpenAI-compatible server, such as Ollama's, for Copilot or a
        /// local model to use instead of a paid one; empty for none, or the
        /// local model's default.
        package var modelServer = ""
        /// The hosts it could run on besides this Mac.
        package var hosts: [RemoteHost] = []
        /// Where it runs; nil for this Mac.
        package var host: RemoteHost.ID?
        /// Whether the hosts were read, so a remembered one is known to be
        /// there still.
        package var hostsRead = false
        /// Why the hosts couldn't be read.
        package var hostsFailure: String?
        package var starting = false
        /// What the backend offers on the host, to pick the model from.
        package var models: [ModelChoice] = []
        /// Why they couldn't be listed; a typed model name still works.
        package var modelsUnavailable: String?
        /// Whether the list is being read, which on a host takes a while.
        package var modelsLoading = false
        /// Numbers each listing asked for, so only the latest one's list
        /// shows: effects can start out of the order they were asked in.
        var modelsRequest = 0
        @Presents package var alert: AlertState<Never>?
        /// Why the session waits on a pick between skills of one name, made
        /// in Settings › Skills.
        package var skillClash: String?
        /// What the last session started with, for the next to start from;
        /// the agent, model and effort come from the defaults instead.
        @Shared(.appStorage("newSessionApprovalMode")) var lastApprovalMode = ApprovalMode.askEveryTime
        @Shared(.appStorage("newSessionHost")) var lastHost: String?
        @Shared(.appStorage("newSessionModelServer")) var lastModelServer = ""

        /// A local model server has no default model, so one must be named.
        package var needsModel: Bool { backend == .local && model.trimmed.isEmpty }
        /// A session for a host waits until the hosts are read: started
        /// before, it could run somewhere the person didn't pick.
        package var canStart: Bool { !starting && !needsModel && (host == nil || hostsRead) }

        /// What starting waits on: fetching the project's repositories,
        /// which a planning session or a chat doesn't.
        package var startingStatus: String {
            guard let project, !planning else { return "Starting…" }
            return project.startingStatus
        }

        /// The model picked, if the backend lists it.
        package var pickedModel: ModelChoice? { models.first { $0.id == model.trimmed } }
        /// The model whose efforts the picker offers, typed or the default
        /// one too.
        package var effortModel: ModelChoice? {
            ModelChoice.effortModel(in: models, id: model.trimmed, backend: backend)
        }
        /// Copilot Fusion, which takes a tier for its effort.
        package var fusion: Bool { backend == .copilot && model.trimmed == ModelChoice.fusion }

        /// What starting the session asks of the daemon.
        package var request: StartRequest {
            let model = model.trimmed
            return StartRequest(
                backend: backend, workingDirectory: project == nil ? folder : nil,
                name: name.trimmed, host: host,
                model: model.isEmpty ? nil : model, effort: effort.isEmpty ? nil : effort,
                experimental: backend == .copilot && experimental, backendConfig: backendConfig,
                approvalMode: approvalMode, projectID: project?.id, itemID: item?.id,
                specs: item?.parentID == nil ? .item : specs,
                planning: planning && project != nil && item == nil)
        }

        /// Whether the backend can use a model server of the person's.
        package var takesModelServer: Bool { backend == .copilot || backend == .local }

        /// The backend's settings for the model server, as JSON.
        package var backendConfig: String? {
            let server = modelServer.trimmed
            guard takesModelServer, !server.isEmpty else { return nil }
            let config: [String: Any] =
                backend == .copilot ? ["provider": ["base_url": server]] : ["base_url": server]
            guard let data = try? JSONSerialization.data(withJSONObject: config, options: [.sortedKeys]) else {
                return nil
            }
            return String(data: data, encoding: .utf8)
        }

        package init(project: Project?, item: Item? = nil, planning: Bool = false) {
            self.project = project
            self.item = item
            self.planning = planning && project != nil && item == nil
            self.name = item?.title ?? (self.planning ? "Plan" : "")
            if let defaults = project?.defaults { apply(defaults) }
            approvalMode = lastApprovalMode
            host = lastHost
            modelServer = lastModelServer
        }

        /// Starts from `defaults`: every project's, with the project's over.
        /// Starts from every project's defaults with the project's over them,
        /// as New Session does once they're read.
        package mutating func applyDefaults(_ every: SessionDefaults) {
            apply(every.under(project?.defaults))
        }

        mutating func apply(_ defaults: SessionDefaults) {
            if let backend = defaults.backend { self.backend = backend }
            model = defaults.model ?? ""
            effort = defaults.effort ?? ""
            experimental = defaults.experimental
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The sheet appeared: reads the hosts to pick from.
        case task
        /// Every project's defaults, to start from with the project's over.
        case defaultsLoaded(SessionDefaults)
        case hostsLoaded([RemoteHost])
        /// The hosts couldn't be read, so the one remembered stays.
        case hostsFailed(String)
        /// The person asked for the hosts to be read again.
        case hostsRetried
        /// The person gave up on the host remembered, for this Mac.
        case thisMacPicked
        case modelsDue(request: Int)
        case modelsLoaded(request: Int, Result<[ModelChoice], any Error>)
        case startTapped
        case failed(String)
        /// Two skills it would get share a name, and none is picked.
        case skillsClash(String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case started(Session)
        }
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding(\.backend):
                // One agent's model means nothing to another.
                state.picked = true
                state.model = ""
                state.effort = ""
                if !ApprovalMode.offered(for: state.backend).contains(state.approvalMode) {
                    state.approvalMode = .askEveryTime
                }
                return loadModels(&state)

            case .binding(\.host):
                return loadModels(&state)

            case .binding(\.modelServer):
                // Typed a letter at a time; asks once it settles.
                return loadModels(&state, after: .milliseconds(500))

            case .binding(\.model):
                state.picked = true
                // Another model's effort may not be one this one takes.
                if !state.effort.isEmpty, state.effortModel?.efforts.contains(state.effort) != true {
                    state.effort = ""
                }
                return .none

            case .binding(\.effort):
                state.picked = true
                return .none

            case .binding, .alert, .delegate:
                return .none

            case .task:
                return .merge(
                    .run { send in
                        // Without them, the session starts from the project's.
                        if let defaults = try? await daemon.sessionDefaults() {
                            await send(.defaultsLoaded(defaults))
                        }
                    },
                    readHosts(),
                    loadModels(&state))

            case .defaultsLoaded(let defaults):
                guard !state.picked else { return .none }
                let backend = state.backend
                state.applyDefaults(defaults)
                return state.backend == backend ? .none : loadModels(&state)

            case .hostsLoaded(let hosts):
                state.hosts = hosts
                state.hostsRead = true
                state.hostsFailure = nil
                if let host = state.host, !hosts.contains(where: { $0.id == host }) {
                    state.host = nil
                    return loadModels(&state)
                }
                return .none

            case .hostsFailed(let message):
                state.hostsFailure = message
                return .none

            case .hostsRetried:
                state.hostsFailure = nil
                return readHosts()

            case .thisMacPicked:
                state.hostsFailure = nil
                guard state.host != nil else { return .none }
                state.host = nil
                return loadModels(&state)

            case .modelsDue(let request):
                guard request == state.modelsRequest else { return .none }
                return fetchModels(state)

            case .modelsLoaded(let request, _) where request != state.modelsRequest:
                // Asked for another agent or host than the one picked now.
                return .none

            case .modelsLoaded(_, .success(let models)):
                state.modelsLoading = false
                state.models = models
                state.modelsUnavailable = nil
                // Others take a typed name their list lacks; a local server
                // runs only what it has.
                if state.backend == .local, let first = models.first,
                    !models.contains(where: { $0.id == state.model.trimmed })
                {
                    state.model = first.id
                }
                return .none

            case .modelsLoaded(_, .failure(let error)):
                state.modelsLoading = false
                state.models = []
                state.modelsUnavailable = error.localizedDescription
                return .none

            case .startTapped:
                guard state.canStart else { return .none }
                state.starting = true
                state.$lastApprovalMode.withLock { $0 = state.approvalMode }
                state.$lastHost.withLock { $0 = state.host }
                state.$lastModelServer.withLock { $0 = state.modelServer }
                state.skillClash = nil
                let request = state.request
                return .run { send in
                    await send(.delegate(.started(try await daemon.start(request))))
                } catch: { error, send in
                    if case DaemonError.refused(.skillClash, let message) = error {
                        await send(.skillsClash(message))
                    } else {
                        await send(.failed(error.localizedDescription))
                    }
                }

            case .skillsClash(let message):
                state.starting = false
                state.skillClash = message
                return .none

            case .failed(let message):
                state.starting = false
                state.alert = AlertState {
                    TextState("Couldn't start the session")
                } message: {
                    TextState(message)
                }
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    private func readHosts() -> Effect<Action> {
        .run { send in
            await send(.hostsLoaded(try await daemon.hosts()))
        } catch: { error, send in
            await send(.hostsFailed(error.localizedDescription))
        }
    }

    /// Lists what the backend offers where the session would run, dropping
    /// what another backend or host offered.
    private func loadModels(_ state: inout State, after delay: Duration? = nil) -> Effect<Action> {
        state.models = []
        state.modelsUnavailable = nil
        state.modelsRequest += 1
        state.modelsLoading = state.backend != .scripted
        guard state.backend != .scripted else { return .none }
        guard let delay else { return fetchModels(state) }
        let request = state.modelsRequest
        return .run { send in
            try await clock.sleep(for: delay)
            await send(.modelsDue(request: request))
        }
    }

    private func fetchModels(_ state: State) -> Effect<Action> {
        let (request, host, backend, config) = (state.modelsRequest, state.host, state.backend, state.backendConfig)
        return .run { send in
            let models = await Result { try await daemon.models(host, backend, config) }
            await send(.modelsLoaded(request: request, models))
        }
    }
}

extension String {
    var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
}

/// Leave asked for with an approval: those of `scopes` run without asking,
/// for `duration`.
package struct ApprovalGrantAsked: Equatable, Sendable {
    package var scopes: [ApprovalScope]
    package var duration: Duration

    package init(scopes: [ApprovalScope], duration: Duration) {
        self.scopes = scopes
        self.duration = duration
    }
}

extension Date {
    /// This time with its seconds dropped.
    var toTheMinute: Date {
        Date(timeIntervalSinceReferenceDate: (timeIntervalSinceReferenceDate / 60).rounded(.down) * 60)
    }
}
