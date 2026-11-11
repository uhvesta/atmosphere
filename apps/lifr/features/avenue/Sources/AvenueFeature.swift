import ComposableArchitecture
import Foundation
import LifrCore
import LifrProjectDetail
import LifrStoreClient
import LifrTaskDetail
import LifrTaskList
import LifrUI

/// One Avenue or Project: its tasks grouped by status or Project. An
/// Avenue view can add Projects; a Project view starts with the Project's
/// own fields.
@Reducer
package struct AvenueFeature {
    @ObservableState
    package struct State: Equatable {
        package var scope: AvenueScope
        package var title: String
        package var grouping: Grouping
        package var list: TaskListFeature.State
        package var newProjectName: String?
        /// The Project's fields, when the scope is a Project.
        package var project: ProjectDetailFeature.State?
        @Presents package var editAvenue: EditAvenueFeature.State?

        package init(
            scope: AvenueScope, workspace: Workspace, grouping: Grouping = .status, now: Date, timeZone: TimeZone
        ) {
            self.scope = scope
            self.grouping = grouping
            title = Self.title(of: scope, in: workspace)
            list = TaskListFeature.State(
                query: .scope(scope, by: grouping), workspace: workspace, now: now, timeZone: timeZone)
            if case .project(let id) = scope {
                project = ProjectDetailFeature.State(id: id, workspace: workspace)
            }
        }

        /// Shows `snapshot`'s records in the list, the title and the
        /// Project's fields, such as after Undo or a sync, unless later ones
        /// are shown already.
        package mutating func show(_ snapshot: Snapshot, now: Date, timeZone: TimeZone) {
            list.show(snapshot, now: now, timeZone: timeZone)
            refresh()
        }

        /// Shows the list's records, the latest, in the title and the
        /// Project's fields.
        mutating func refresh() {
            guard let snapshot = list.snapshot else { return }
            title = Self.title(of: scope, in: snapshot.workspace)
            if project?.refresh(from: snapshot) == false { project = nil }
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

        /// The groupings offered: by Project only for a whole Avenue.
        package var groupings: [Grouping] {
            canGroupByProject ? Grouping.allCases : Grouping.allCases.filter { $0 != .project }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case list(TaskListFeature.Action)
        case groupingPicked(Grouping)
        case newProjectTapped
        case newProjectCancelled
        case newProjectSubmitted
        case project(ProjectDetailFeature.Action)
        case editAvenueTapped
        case editAvenue(PresentationAction<EditAvenueFeature.Action>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Snapshot)
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
                if let snapshot = state.list.snapshot {
                    state.list.show(snapshot, now: now, timeZone: timeZone)
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

            case .list(.show), .list(.loaded(.success)):
                state.refresh()
                return .none

            case .list(.delegate(.saved(let snapshot))):
                state.refresh()
                return .send(.delegate(.saved(snapshot)))

            case .editAvenueTapped:
                guard case .avenue(let id) = state.scope, let workspace = state.list.workspace else { return .none }
                state.editAvenue = EditAvenueFeature.State(id: id, workspace: workspace)
                return .none

            case .project(.delegate(.saved(let snapshot))), .editAvenue(.presented(.delegate(.saved(let snapshot)))):
                state.show(snapshot, now: now, timeZone: timeZone)
                return .send(.delegate(.saved(snapshot)))

            case .binding, .list, .project, .editAvenue, .delegate:
                return .none
            }
        }
        .ifLet(\.project, action: \.project) { ProjectDetailFeature() }
        .ifLet(\.$editAvenue, action: \.editAvenue) { EditAvenueFeature() }
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

/// An archived Avenue, or a Project that's completed or archived, which can
/// be opened to restore it.
package struct ArchivedRow: Identifiable, Hashable, Sendable {
    package var scope: AvenueScope
    package var name: String
    /// Such as "Health" for a Project, or "Avenue" for an Avenue.
    package var detail: String
    package var id: AvenueScope { scope }
}

extension Workspace {
    /// Archived Avenues, then completed and archived Projects of open
    /// Avenues, each by name.
    package var archivedRows: [ArchivedRow] {
        let byName = { (lhs: ArchivedRow, rhs: ArchivedRow) in
            lhs.name.localizedStandardCompare(rhs.name) == .orderedAscending
        }
        let archivedAvenues = avenues.values.filter { $0.archivedAt != nil }
            .map { ArchivedRow(scope: .avenue($0.id), name: $0.name, detail: String(localized: "Avenue")) }
            .sorted(by: byName)
        let closedProjects = projects.values
            .filter { $0.status != .active && avenues[$0.avenueID]?.archivedAt == nil }
            .map { project in
                ArchivedRow(
                    scope: .project(project.id), name: project.name, detail: avenues[project.avenueID]?.name ?? "")
            }
            .sorted(by: byName)
        return archivedAvenues + closedProjects
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
        package var archived: [ArchivedRow] = []
        @Presents package var avenue: AvenueFeature.State?
        @Presents package var newAvenue: NewAvenueFeature.State?
        @Presents package var alert: AlertState<Never>?

        package init() {}

        /// The latest records shown, so earlier ones arriving late are ignored.
        package var saves = SaveOrder()

        package init(workspace: Workspace) {
            self.workspace = workspace
            avenues = workspace.avenueRows
            archived = workspace.archivedRows
        }

        /// Shows `snapshot`'s records, and in the open Avenue, unless later
        /// ones are shown already.
        mutating func show(_ snapshot: Snapshot, now: Date, timeZone: TimeZone) {
            guard saves.isLatest(snapshot) else { return }
            workspace = snapshot.workspace
            avenues = snapshot.workspace.avenueRows
            archived = snapshot.workspace.archivedRows
            avenue?.show(snapshot, now: now, timeZone: timeZone)
        }
    }

    package enum Action {
        case task
        case loaded(Result<Workspace, any Error>)
        case show(Snapshot)
        case opened(AvenueScope)
        case newAvenueTapped
        /// Swaps an Avenue with the one above or below it.
        case moveAvenue(UUID, up: Bool)
        /// Swaps a Project with the one above or below it in its Avenue.
        case moveProject(UUID, up: Bool)
        case moved(Result<Saved, any Error>)
        case avenue(PresentationAction<AvenueFeature.Action>)
        case newAvenue(PresentationAction<NewAvenueFeature.Action>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Snapshot)
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

            case .loaded(.success(let workspace)):
                state.show(Snapshot(workspace, number: 0), now: now, timeZone: timeZone)
                return .none

            case .show(let snapshot):
                state.show(snapshot, now: now, timeZone: timeZone)
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

            case .moveAvenue(let id, let up):
                guard let (lower, upper) = Self.neighbours(of: id, in: state.avenues.map(\.id), up: up)
                else { return .none }
                return .run { [store] send in
                    await send(.moved(Result { try await store.reorder(avenue: id, after: lower, before: upper) }))
                }

            case .moveProject(let id, let up):
                guard let row = state.avenues.first(where: { $0.projects.contains { $0.id == id } }),
                    let (lower, upper) = Self.neighbours(of: id, in: row.projects.map(\.id), up: up)
                else { return .none }
                return .run { [store] send in
                    await send(.moved(Result { try await store.reorder(project: id, after: lower, before: upper) }))
                }

            case .moved(.success(let saved)):
                guard state.saves.isLatest(saved) else { return .none }
                state.show(saved.snapshot, now: now, timeZone: timeZone)
                return .send(.delegate(.saved(saved.snapshot)))

            case .moved(.failure(let error)):
                state.alert = .failed("Couldn't move it", error)
                return .none

            case .avenue(.presented(.delegate(.saved(let snapshot)))),
                .newAvenue(.presented(.delegate(.saved(let snapshot)))):
                state.show(snapshot, now: now, timeZone: timeZone)
                return .send(.delegate(.saved(snapshot)))

            case .avenue, .newAvenue, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$avenue, action: \.avenue) { AvenueFeature() }
        .ifLet(\.$newAvenue, action: \.newAvenue) { NewAvenueFeature() }
        .ifLet(\.$alert, action: \.alert)
    }

    /// The rows to put `id` between to move it one place up or down in
    /// `ids`, or `nil` when it's already at that end.
    private static func neighbours(of id: UUID, in ids: [UUID], up: Bool) -> (UUID?, UUID?)? {
        guard let index = ids.firstIndex(of: id) else { return nil }
        let target = up ? index - 1 : index + 1
        guard ids.indices.contains(target) else { return nil }
        var rest = ids
        rest.remove(at: index)
        rest.insert(id, at: target)
        let lower = target > 0 ? rest[target - 1] : nil
        let upper = target + 1 < rest.count ? rest[target + 1] : nil
        return (lower, upper)
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
            case saved(Snapshot)
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
                    await send(.delegate(.saved(saved.snapshot)))
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
