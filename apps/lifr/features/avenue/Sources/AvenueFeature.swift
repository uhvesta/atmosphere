import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskDetail
import LifrTaskList
import LifrUI

/// One Avenue or Project: its tasks grouped by status or Project. An
/// Avenue view can add Projects.
@Reducer
package struct AvenueFeature {
    @ObservableState
    package struct State: Equatable {
        package var scope: AvenueScope
        package var title: String
        package var grouping: Grouping
        package var list: TaskListFeature.State
        package var newProjectName: String?

        package init(
            scope: AvenueScope, workspace: Workspace, grouping: Grouping = .status, now: Date, timeZone: TimeZone
        ) {
            self.scope = scope
            self.grouping = grouping
            title = Self.title(of: scope, in: workspace)
            list = TaskListFeature.State(
                query: .scope(scope, by: grouping), workspace: workspace, now: now, timeZone: timeZone)
        }

        static func title(of scope: AvenueScope, in workspace: Workspace) -> String {
            switch scope {
            case .avenue(let id): workspace.avenues[id]?.name ?? ""
            case .project(let id): workspace.projects[id]?.name ?? ""
            }
        }

        /// Grouping by Project only makes sense for a whole Avenue.
        package var canGroupByProject: Bool {
            if case .avenue = scope { true } else { false }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case list(TaskListFeature.Action)
        case groupingPicked(Grouping)
        case newProjectTapped
        case newProjectCancelled
        case newProjectSubmitted
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Scope(state: \.list, action: \.list) { TaskListFeature() }
        Reduce { state, action in
            switch action {
            case .groupingPicked(let grouping):
                state.grouping = grouping
                state.list.query = .scope(state.scope, by: grouping)
                if let workspace = state.list.workspace {
                    state.list.show(workspace, now: now, timeZone: timeZone)
                }
                return .none

            case .newProjectTapped:
                state.newProjectName = ""
                return .none

            case .newProjectCancelled:
                state.newProjectName = nil
                return .none

            case .newProjectSubmitted:
                let name = (state.newProjectName ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                state.newProjectName = nil
                guard !name.isEmpty, case .avenue(let avenueID) = state.scope else { return .none }
                return .run { [store] send in
                    await send(.list(.saved(Result { try await store.createProject(name, in: avenueID) })))
                }

            case .list(.show(let workspace)), .list(.loaded(.success(let workspace))):
                state.title = State.title(of: state.scope, in: workspace)
                return .none

            case .list(.delegate(.saved(let workspace))):
                state.title = State.title(of: state.scope, in: workspace)
                return .send(.delegate(.saved(workspace)))

            case .binding, .list, .delegate:
                return .none
            }
        }
    }
}

/// One row on the Avenues screen.
package struct AvenueRow: Identifiable, Hashable, Sendable {
    package struct ProjectRow: Identifiable, Hashable, Sendable {
        package var id: UUID
        package var name: String
        package var openCount: Int
    }

    package var id: UUID
    package var key: String
    package var name: String
    package var color: String
    package var symbol: String
    package var openCount: Int
    /// Active Projects, in order.
    package var projects: [ProjectRow]
}

extension Workspace {
    /// Every Avenue that isn't archived, in order, with open task counts.
    package var avenueRows: [AvenueRow] {
        let open = listedTasks.filter { $0.isOpen && !$0.isSubtask }
        return avenues.values
            .filter { $0.archivedAt == nil }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
            .map { avenue in
                AvenueRow(
                    id: avenue.id, key: avenue.key.rawValue, name: avenue.name, color: avenue.color,
                    symbol: avenue.symbol, openCount: open.count { $0.avenueID == avenue.id },
                    projects: projects.values
                        .filter { $0.avenueID == avenue.id && $0.status == .active }
                        .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                        .map { project in
                            AvenueRow.ProjectRow(
                                id: project.id, name: project.name,
                                openCount: open.count { $0.projectID == project.id })
                        })
            }
    }
}

/// Every Avenue and its Projects. Opens one, or adds an Avenue.
@Reducer
package struct AvenuesFeature {
    @ObservableState
    package struct State: Equatable {
        /// `nil` until the records load.
        package var workspace: Workspace?
        package var avenues: [AvenueRow] = []
        @Presents package var avenue: AvenueFeature.State?
        @Presents package var newAvenue: NewAvenueFeature.State?
        @Presents package var alert: AlertState<Never>?

        package init() {}

        package init(workspace: Workspace) {
            self.workspace = workspace
            avenues = workspace.avenueRows
        }
    }

    package enum Action {
        case task
        case loaded(Result<Workspace, any Error>)
        case show(Workspace)
        case opened(AvenueScope)
        case newAvenueTapped
        case avenue(PresentationAction<AvenueFeature.Action>)
        case newAvenue(PresentationAction<NewAvenueFeature.Action>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                guard state.workspace == nil else { return .none }
                return .run { [store] send in await send(.loaded(Result { try await store.load() })) }

            case .loaded(.success(let workspace)), .show(let workspace):
                state.workspace = workspace
                state.avenues = workspace.avenueRows
                state.avenue?.list.show(workspace, now: now, timeZone: timeZone)
                return .none

            case .loaded(.failure(let error)):
                state.alert = .failed("Couldn't load your Avenues", error)
                return .none

            case .opened(let scope):
                guard let workspace = state.workspace else { return .none }
                state.avenue = AvenueFeature.State(scope: scope, workspace: workspace, now: now, timeZone: timeZone)
                return .none

            case .newAvenueTapped:
                state.newAvenue = NewAvenueFeature.State()
                return .none

            case .avenue(.presented(.delegate(.saved(let workspace)))),
                .newAvenue(.presented(.delegate(.saved(let workspace)))):
                state.workspace = workspace
                state.avenues = workspace.avenueRows
                return .send(.delegate(.saved(workspace)))

            case .avenue, .newAvenue, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$avenue, action: \.avenue) { AvenueFeature() }
        .ifLet(\.$newAvenue, action: \.newAvenue) { NewAvenueFeature() }
        .ifLet(\.$alert, action: \.alert)
    }
}

/// Adding an Avenue: a short key for task codes, and a name.
@Reducer
package struct NewAvenueFeature {
    @ObservableState
    package struct State: Equatable {
        package var key = ""
        package var name = ""
        /// Why the last try failed, such as a key already taken.
        package var error: String?

        package init() {}
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case addTapped
        case cancelTapped
        case added(Result<Saved, any Error>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .addTapped:
                let key = state.key.trimmingCharacters(in: .whitespaces).uppercased()
                let name = state.name.trimmingCharacters(in: .whitespacesAndNewlines)
                return .run { [store] send in
                    await send(
                        .added(Result { try await store.createAvenue(key: key, name: name.isEmpty ? key : name) }))
                }
            case .added(.success(let saved)):
                return .run { [dismiss] send in
                    await send(.delegate(.saved(saved.workspace)))
                    await dismiss()
                }
            case .added(.failure(let error)):
                state.error = (error as? LifrError)?.description ?? error.localizedDescription
                return .none
            case .cancelTapped:
                return .run { [dismiss] _ in await dismiss() }
            case .binding, .delegate:
                return .none
            }
        }
    }
}
