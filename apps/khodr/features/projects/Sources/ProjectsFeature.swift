import ComposableArchitecture
import Foundation
import KhodrDaemonClient

/// The sidebar: every project with its sessions, kept up to date, and the
/// sheets to add a project and to start a session in one.
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
    }

    @Reducer
    package enum Destination {
        case newProject(NewProjectFeature)
        case newSession(NewSessionFeature)
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

/// A new project: its name, its repository's folder on this Mac and
/// instructions for its sessions.
@Reducer
package struct NewProjectFeature {
    @ObservableState
    package struct State: Equatable {
        package var name = ""
        /// The repository's folder, as an absolute path.
        package var repository = ""
        package var instructions = ""
        package var creating = false
        @Presents package var alert: AlertState<Never>?

        package init() {}

        package var canCreate: Bool {
            !creating && !name.trimmed.isEmpty && repository.trimmed.hasPrefix("/")
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// A folder was picked; it names the project if it has no name yet.
        case folderPicked(URL)
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
                state.repository = path
                if state.name.trimmed.isEmpty { state.name = url.lastPathComponent }
                return .none

            case .createTapped:
                guard state.canCreate else { return .none }
                state.creating = true
                let (name, repository, instructions) = (
                    state.name.trimmed, state.repository.trimmed, state.instructions
                )
                return .run { send in
                    let project = try await daemon.createProject(name, repository, instructions)
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
