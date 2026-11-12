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
        /// What the person has typed to each agent's request, kept until it's
        /// sent.
        package var drafts: [AgentRequest.ID: String] = [:]
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

        package init() {}

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
        /// it was forked from; nil for sessions in no project. A fork whose
        /// parent is archived or elsewhere is at the top.
        package func tree(in project: Project.ID?) -> [SessionNode] {
            let shown = sessions.filter { $0.projectID == project && !$0.archived }
            let ids = Set(shown.map(\.id))
            let children = Dictionary(grouping: shown.filter { $0.parentID.map(ids.contains) == true }) {
                $0.parentID ?? ""
            }
            func node(_ session: Session) -> SessionNode {
                let under = sortedNodes((children[session.id] ?? []).map(node))
                return SessionNode(session: session, children: under.isEmpty ? nil : under)
            }
            return sortedNodes(shown.filter { $0.parentID.map(ids.contains) != true }.map(node))
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
        case loaded(projects: [Project], sessions: [Session], inbox: [InboxEntry])
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
        /// Asks before deleting an archived session for good, or says what
        /// to do first.
        case deleteSessionTapped(Session.ID)
        case sessionDeleted(Session.ID)
        case sessionDeleteFailed(String)
        /// Asks where to move a session: this Mac or one of the hosts.
        case moveToHostTapped(Session.ID)
        case moveHostsLoaded(Session.ID, hosts: [String])
        case sessionMoved(Session)
        case sessionMoveFailed(String)
        case keeperTapped(Session.ID, keeper: Bool)
        case compareTapped(Session.ID, with: Session.ID)
        case updated(Session)
        case updateFailed(String)
        /// The person allowed or denied a request from the inbox.
        case inboxAnswered(Session.ID, request: String, approved: Bool)
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
        case answerFailed(String)
        /// The person clicked the notification for a session.
        case notificationClicked(Session.ID)
        case destination(PresentationAction<Destination.Action>)
        case delegate(Delegate)

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
                        while !Task.isCancelled {
                            await send(await load())
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

            case .loaded(let projects, let sessions, let inbox):
                state.failure = nil
                // The open session needs no notice only while it's on screen.
                let open = activity.isActive() ? state.selection : nil
                let notices = state.loadedOnce ? Self.notices(new: inbox, old: state.inbox, open: open) : []
                // The Dock counts only the sessions that can't go on without
                // the person.
                let blocked = inbox.count(where: \.reason.blocks)
                let badge = !state.loadedOnce || blocked != state.inbox.count(where: \.reason.blocks) ? blocked : nil
                state.loadedOnce = true
                state.now = date.now
                state.inbox = inbox
                state.projects = IdentifiedArray(uniqueElements: projects)
                state.order = State.order(of: sessions, before: state.sessions, kept: state.order)
                state.sessions = IdentifiedArray(sessions, uniquingIDsWith: { _, last in last })
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
                state.destination = .alert(
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
                return update(id, SessionUpdate(archived: archived))

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
                return .run { send in
                    try await daemon.delete(id)
                    await send(.sessionDeleted(id))
                } catch: { error, send in
                    await send(.sessionDeleteFailed(error.localizedDescription))
                }

            case .sessionDeleted(let id):
                state.sessions.remove(id: id)
                if state.selection == .session(id) { state.selection = nil }
                return .none

            case .sessionDeleteFailed(let message):
                state.destination = .alert(
                    AlertState {
                        TextState("Couldn't delete the session")
                    } message: {
                        TextState(message)
                    })
                return .none

            case .moveToHostTapped(let id):
                return .run { send in
                    let hosts = try await daemon.hosts().map(\.name).filter { !$0.isEmpty }
                    await send(.moveHostsLoaded(id, hosts: hosts))
                } catch: { error, send in
                    await send(.sessionMoveFailed(error.localizedDescription))
                }

            case .moveHostsLoaded(let id, let hosts):
                guard let session = state.sessions[id: id] else { return .none }
                let here = session.host ?? ""
                let targets = ([""] + hosts).filter { $0 != here }
                guard !targets.isEmpty else {
                    state.destination = .alert(
                        AlertState {
                            TextState("No Host to Move To")
                        } message: {
                            TextState("Add a host under Hosts, then move \(session.title) to it.")
                        })
                    return .none
                }
                state.destination = .alert(
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
                return .run { send in
                    let moved = try await daemon.restoreSession(id, host.isEmpty ? nil : host)
                    await send(.sessionMoved(moved))
                    await send(await load())
                } catch: { error, send in
                    await send(.sessionMoveFailed(error.localizedDescription))
                }

            case .sessionMoved(let session):
                state.sessions[id: session.id] = session
                return .none

            case .sessionMoveFailed(let message):
                state.destination = .alert(
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
                state.sessions[id: session.id] = session
                switch state.destination {
                case .rename, .move: state.destination = nil
                default: break
                }
                return .none

            case .inboxAnswered(let id, let request, let approved):
                // Gone from the row at once; the refresh says what's next.
                if let index = state.inbox.firstIndex(where: { $0.session.id == id && $0.request == nil }) {
                    state.inbox[index].session.pending.removeAll { $0.requestID == request }
                }
                return .run { send in
                    try await daemon.approve(id, request, approved)
                    await send(await load())
                } catch: { error, send in
                    await send(.answerFailed(error.localizedDescription))
                    await send(await load())
                }

            case .requestAnswered(let id, let answer):
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
                let live = archive ? state.closing(id, with: answer) : []
                return answerRequest(&state, id, answer, archiving: live)

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

            case .answerFailed(let message):
                state.destination = .alert(
                    AlertState {
                        TextState("Couldn't answer the request")
                    } message: {
                        TextState(message)
                    })
                return .none

            case .updateFailed(let message):
                state.destination = .alert(
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
    }

    private func update(_ id: Session.ID, _ update: SessionUpdate) -> Effect<Action> {
        .run { send in
            await send(.updated(try await daemon.update(id, update)))
        } catch: { error, send in
            await send(.updateFailed(error.localizedDescription))
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
        return .run { send in
            try await daemon.answerRequest(id, answer)
            await send(.requestAnswerSent(id))
            for session in sessions where session.state == .working || session.state == .waitingForApproval {
                try await daemon.abort(session.id)
            }
            for session in sessions {
                _ = try await daemon.update(session.id, SessionUpdate(archived: true))
            }
            await send(await load())
        } catch: { error, send in
            await send(.answerFailed(error.localizedDescription))
            await send(await load())
        }
    }

    private func load() async -> Action {
        do {
            async let projects = daemon.projects()
            async let sessions = daemon.sessions(nil)
            async let inbox = daemon.inbox()
            return .loaded(projects: try await projects, sessions: try await sessions, inbox: try await inbox)
        } catch {
            return .loadFailed(error.localizedDescription)
        }
    }
}

extension ProjectsFeature.Destination.State: Equatable {}

extension ProjectsFeature.State {
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
        Reduce { state, action in
            switch action {
            case .binding, .alert, .delegate:
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
        package var approvalMode = ApprovalMode.askEveryTime
        /// An OpenAI-compatible server, such as Ollama's, for Copilot or a
        /// local model to use instead of a paid one; empty for none, or the
        /// local model's default.
        package var modelServer = ""
        /// The hosts it could run on besides this Mac.
        package var hosts: [RemoteHost] = []
        /// Where it runs; nil for this Mac.
        package var host: RemoteHost.ID?
        package var starting = false
        /// What the backend offers on the host, to pick the model from.
        package var models: [ModelChoice] = []
        /// Why they couldn't be listed; a typed model name still works.
        package var modelsUnavailable: String?
        @Presents package var alert: AlertState<Never>?
        /// What the last session started with, for the next to start from.
        @Shared(.appStorage("newSessionBackend")) var lastBackend = Backend.copilot
        @Shared(.appStorage("newSessionModel")) var lastModel = ""
        @Shared(.appStorage("newSessionApprovalMode")) var lastApprovalMode = ApprovalMode.askEveryTime
        @Shared(.appStorage("newSessionHost")) var lastHost: String?
        @Shared(.appStorage("newSessionModelServer")) var lastModelServer = ""

        /// A local model server has no default model, so one must be named.
        package var needsModel: Bool { backend == .local && model.trimmed.isEmpty }
        package var canStart: Bool { !starting && !needsModel }

        /// What starting the session asks of the daemon.
        package var request: StartRequest {
            let model = model.trimmed
            return StartRequest(
                backend: backend, workingDirectory: project == nil ? folder : nil,
                name: name.trimmed, host: host,
                model: model.isEmpty ? nil : model, backendConfig: backendConfig,
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
            backend = lastBackend
            model = lastModel
            approvalMode = lastApprovalMode
            host = lastHost
            modelServer = lastModelServer
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The sheet appeared: reads the hosts to pick from.
        case task
        case hostsLoaded([RemoteHost])
        case modelsLoaded(Result<[ModelChoice], any Error>)
        case startTapped
        case failed(String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case started(Session)
        }
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock

    private enum CancelID { case models }

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding(\.backend):
                // One agent's model means nothing to another.
                state.model = ""
                return loadModels(&state)

            case .binding(\.host):
                return loadModels(&state)

            case .binding(\.modelServer):
                // Typed a letter at a time; asks once it settles.
                return loadModels(&state, after: .milliseconds(500))

            case .binding, .alert, .delegate:
                return .none

            case .task:
                return .merge(
                    .run { send in
                        // Without them, the session runs on this Mac.
                        await send(.hostsLoaded((try? await daemon.hosts()) ?? []))
                    },
                    loadModels(&state))

            case .hostsLoaded(let hosts):
                state.hosts = hosts
                if let host = state.host, !hosts.contains(where: { $0.id == host }) {
                    state.host = nil
                    return loadModels(&state)
                }
                return .none

            case .modelsLoaded(.success(let models)):
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

            case .modelsLoaded(.failure(let error)):
                state.models = []
                state.modelsUnavailable = error.localizedDescription
                return .none

            case .startTapped:
                guard state.canStart else { return .none }
                state.starting = true
                state.$lastBackend.withLock { $0 = state.backend }
                state.$lastModel.withLock { $0 = state.model }
                state.$lastApprovalMode.withLock { $0 = state.approvalMode }
                state.$lastHost.withLock { $0 = state.host }
                state.$lastModelServer.withLock { $0 = state.modelServer }
                let request = state.request
                return .run { send in
                    await send(.delegate(.started(try await daemon.start(request))))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

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

    /// Lists what the backend offers where the session would run, dropping
    /// what another backend or host offered.
    private func loadModels(_ state: inout State, after delay: Duration? = nil) -> Effect<Action> {
        state.models = []
        state.modelsUnavailable = nil
        guard state.backend != .scripted else { return .cancel(id: CancelID.models) }
        let (host, backend, config) = (state.host, state.backend, state.backendConfig)
        return .run { send in
            if let delay { try await clock.sleep(for: delay) }
            await send(.modelsLoaded(Result { try await daemon.models(host, backend, config) }))
        }
        .cancellable(id: CancelID.models, cancelInFlight: true)
    }
}

extension String {
    var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
}
