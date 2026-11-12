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
        /// Whether the list was read yet: until then nothing is new, so
        /// nothing is notified.
        package var loadedOnce = false
        /// What's open beside the sidebar.
        package var selection: SidebarItem?
        /// Why the list couldn't be read, and what to do about it; nil once
        /// it was.
        package var failure: String?
        @Presents package var destination: Destination.State?

        package init() {}

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
                let under = (children[session.id] ?? []).map(node)
                return SessionNode(session: session, children: under.isEmpty ? nil : under)
            }
            return shown.filter { $0.parentID.map(ids.contains) != true }.map(node)
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
        case rename(RenameSessionFeature)
        case move(MoveSessionFeature)
        case compare(CompareFeature)
        case alert(AlertState<Never>)
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
        /// A session in no project: a chat, in a folder or its own.
        case newChatTapped
        case selected(SidebarItem?)
        case renameTapped(Session.ID)
        case moveTapped(Session.ID)
        case archiveTapped(Session.ID, archived: Bool)
        case keeperTapped(Session.ID, keeper: Bool)
        case compareTapped(Session.ID, with: Session.ID)
        case updated(Session)
        case updateFailed(String)
        /// The person allowed or denied a request from the inbox.
        case inboxAnswered(Session.ID, request: String, approved: Bool)
        case answerFailed(String)
        /// The person clicked the notification for a session.
        case notificationClicked(Session.ID)
        case destination(PresentationAction<Destination.Action>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case openSession(Session)
            case openBoard(Project)
            case openInbox
            case openHosts
        }
    }

    /// How often the list is read again, for sessions' states.
    static let refreshInterval = Duration.seconds(2)

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock
    @Dependency(\.notifications) var notifications

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
                let notices = state.loadedOnce ? Self.notices(new: inbox, old: state.inbox, open: state.selection) : []
                state.loadedOnce = true
                state.inbox = inbox
                state.projects = IdentifiedArray(uniqueElements: projects)
                state.sessions = IdentifiedArray(sessions, uniquingIDsWith: { _, last in last })
                guard !notices.isEmpty else { return .none }
                return .run { _ in
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

            case .newChatTapped:
                state.destination = .newSession(NewSessionFeature.State(project: nil))
                return .none

            case .selected(let item):
                state.selection = item
                switch item {
                case .inbox:
                    return .send(.delegate(.openInbox))
                case .hosts:
                    return .send(.delegate(.openHosts))
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
                if let index = state.inbox.firstIndex(where: { $0.id == id }) {
                    state.inbox[index].session.pending.removeAll { $0.requestID == request }
                }
                return .run { send in
                    try await daemon.approve(id, request, approved)
                    await send(await load())
                } catch: { error, send in
                    await send(.answerFailed(error.localizedDescription))
                    await send(await load())
                }

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

            case .destination(.presented(.newSession(.delegate(.started(let session))))):
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
        return new.filter { before[$0.id] != $0.reason && open != .session($0.id) }.map(Notice.init)
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

/// A row of the sidebar to open.
package enum SidebarItem: Hashable, Sendable {
    case inbox
    case hosts
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
        /// The repositories' folders, as absolute paths. Each session works
        /// across all of them.
        package var repositories: IdentifiedArrayOf<RepositoryRow> = [RepositoryRow(id: 0)]
        var nextRow = 1
        package var instructions = ""
        package var creating = false
        @Presents package var alert: AlertState<Never>?

        package init() {}

        /// The folders filled in so far.
        var paths: [String] {
            repositories.map(\.path.trimmed).filter { !$0.isEmpty }
        }

        package var canCreate: Bool {
            !creating && !name.trimmed.isEmpty && !paths.isEmpty && paths.allSatisfy { $0.hasPrefix("/") }
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
                    state.name.trimmed, state.paths.map { Repository(path: $0) }, state.instructions
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

/// A row of the new project's repositories.
package struct RepositoryRow: Identifiable, Hashable, Sendable {
    package var id: Int
    /// The folder as typed or picked.
    package var path = ""

    package init(id: Int, path: String = "") {
        self.id = id
        self.path = path
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
        @Presents package var alert: AlertState<Never>?
        /// What the last session started with, for the next to start from.
        @Shared(.appStorage("newSessionBackend")) var lastBackend = Backend.copilot
        @Shared(.appStorage("newSessionModel")) var lastModel = ""
        @Shared(.appStorage("newSessionApprovalMode")) var lastApprovalMode = ApprovalMode.askEveryTime
        @Shared(.appStorage("newSessionHost")) var lastHost: String?
        @Shared(.appStorage("newSessionModelServer")) var lastModelServer = ""

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

        package init(project: Project?, item: Item? = nil) {
            self.project = project
            self.item = item
            self.name = item?.title ?? ""
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

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .alert, .delegate:
                return .none

            case .task:
                return .run { send in
                    // Without them, the session runs on this Mac.
                    await send(.hostsLoaded((try? await daemon.hosts()) ?? []))
                }

            case .hostsLoaded(let hosts):
                state.hosts = hosts
                if let host = state.host, !hosts.contains(where: { $0.id == host }) { state.host = nil }
                return .none

            case .startTapped:
                guard !state.starting else { return .none }
                state.starting = true
                state.$lastBackend.withLock { $0 = state.backend }
                state.$lastModel.withLock { $0 = state.model }
                state.$lastApprovalMode.withLock { $0 = state.approvalMode }
                state.$lastHost.withLock { $0 = state.host }
                state.$lastModelServer.withLock { $0 = state.modelServer }
                let model = state.model.trimmed
                let request = StartRequest(
                    backend: state.backend, workingDirectory: state.project == nil ? state.folder : nil,
                    name: state.name.trimmed, host: state.host,
                    model: model.isEmpty ? nil : model, backendConfig: state.backendConfig,
                    approvalMode: state.approvalMode, projectID: state.project?.id, itemID: state.item?.id,
                    specs: state.item?.parentID == nil ? .item : state.specs)
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
}

extension String {
    fileprivate var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
}
