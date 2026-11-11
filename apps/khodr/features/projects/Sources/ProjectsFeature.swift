import ComposableArchitecture
import Foundation
import KhodrDaemonClient

/// The sidebar: every project with its sessions as a tree of forks, kept up
/// to date; the sheets to add a project and to start a session in one; and
/// renaming, archiving, comparing and marking the keeper.
@Reducer
package struct ProjectsFeature {
    @ObservableState
    package struct State: Equatable {
        package var projects: IdentifiedArrayOf<Project> = []
        package var sessions: IdentifiedArrayOf<Session> = []
        /// The open session's id.
        package var selection: String?
        /// Why the list couldn't be read, and what to do about it; nil once
        /// it was.
        package var failure: String?
        @Presents package var destination: Destination.State?

        package init() {}

        package func sessions(in project: Project.ID) -> [Session] {
            sessions.filter { $0.projectID == project }
        }

        /// Sessions started outside any project, such as from the CLI.
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
        case compare(CompareFeature)
        case alert(AlertState<Never>)
    }

    package enum Action {
        /// The sidebar appeared; reads the list now and again every few
        /// seconds, until it goes.
        case task
        /// Reads the list once.
        case refresh
        case loaded(projects: [Project], sessions: [Session])
        case loadFailed(String)
        case addProjectTapped
        case newSessionTapped(projectID: Project.ID)
        case sessionSelected(Session.ID?)
        case renameTapped(Session.ID)
        case archiveTapped(Session.ID, archived: Bool)
        case keeperTapped(Session.ID, keeper: Bool)
        case compareTapped(Session.ID, with: Session.ID)
        case updated(Session)
        case updateFailed(String)
        case destination(PresentationAction<Destination.Action>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case openSession(Session)
        }
    }

    /// How often the list is read again, for sessions' states.
    static let refreshInterval = Duration.seconds(2)

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock

    private enum CancelID { case refresh }

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                return .run { send in
                    while !Task.isCancelled {
                        await send(await load())
                        try await clock.sleep(for: Self.refreshInterval)
                    }
                }
                .cancellable(id: CancelID.refresh, cancelInFlight: true)

            case .refresh:
                return .run { send in await send(await load()) }

            case .loaded(let projects, let sessions):
                state.failure = nil
                state.projects = IdentifiedArray(uniqueElements: projects)
                state.sessions = IdentifiedArray(sessions, uniquingIDsWith: { _, last in last })
                return .none

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

            case .sessionSelected(let id):
                state.selection = id
                guard let id, let session = state.sessions[id: id] else { return .none }
                return .send(.delegate(.openSession(session)))

            case .renameTapped(let id):
                guard let session = state.sessions[id: id] else { return .none }
                state.destination = .rename(RenameSessionFeature.State(session: session))
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
                .destination(.presented(.compare(.delegate(.updated(let session))))):
                state.sessions[id: session.id] = session
                if case .rename = state.destination { state.destination = nil }
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
                state.selection = session.id
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

    private func load() async -> Action {
        do {
            async let projects = daemon.projects()
            async let sessions = daemon.sessions(nil)
            return .loaded(projects: try await projects, sessions: try await sessions)
        } catch {
            return .loadFailed(error.localizedDescription)
        }
    }
}

extension ProjectsFeature.Destination.State: Equatable {}

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

/// A new session in a project: its backend and model, and what it may do
/// without asking.
@Reducer
package struct NewSessionFeature {
    @ObservableState
    package struct State: Equatable {
        package var project: Project
        package var name = ""
        package var backend = Backend.copilot
        /// Empty for the backend's default.
        package var model = ""
        package var approvalMode = ApprovalMode.askEveryTime
        package var starting = false
        @Presents package var alert: AlertState<Never>?

        package init(project: Project) {
            self.project = project
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
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

            case .startTapped:
                guard !state.starting else { return .none }
                state.starting = true
                let model = state.model.trimmed
                let request = StartRequest(
                    backend: state.backend, name: state.name.trimmed, model: model.isEmpty ? nil : model,
                    approvalMode: state.approvalMode, projectID: state.project.id)
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
