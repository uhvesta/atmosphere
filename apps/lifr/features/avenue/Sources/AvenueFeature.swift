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
        /// How the list is grouped, remembered per Avenue or Project on this
        /// device.
        @Shared package var grouping: Grouping
        /// Whether closed tasks show, on this device for every Avenue.
        @Shared(.showsCompleted) package var showsCompleted
        package var list: TaskListFeature.State
        package var newProjectName: String?
        /// A Project name whose save was refused, to start from next time.
        package var refusedProjectName: String?
        /// The Project's fields, when the scope is a Project.
        package var project: ProjectDetailFeature.State?
        @Presents package var editAvenue: EditAvenueFeature.State?

        package init(
            scope: AvenueScope, workspace: Workspace, grouping: Grouping? = nil, now: Date, timeZone: TimeZone
        ) {
            self.scope = scope
            let id =
                switch scope {
                case .avenue(let id): "avenue-\(id.uuidString)"
                case .project(let id): "project-\(id.uuidString)"
                }
            let shared = Shared(wrappedValue: Grouping.status, .appStorage("lifrGrouping-\(id)"))
            if let grouping { shared.withLock { $0 = grouping } }
            if case .project = scope, shared.wrappedValue == .project { shared.withLock { $0 = .status } }
            _grouping = shared
            title = Self.title(of: scope, in: workspace)
            @Shared(.showsCompleted) var showsCompleted
            list = TaskListFeature.State(
                query: .scope(scope, by: shared.wrappedValue, showsCompleted: showsCompleted), workspace: workspace,
                now: now, timeZone: timeZone)
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

        /// Lists the tasks again, after the grouping or Show Closed
        /// changed.
        mutating func requery(now: Date, timeZone: TimeZone) {
            list.query = .scope(scope, by: grouping, showsCompleted: showsCompleted)
            if let snapshot = list.snapshot {
                list.show(snapshot, now: now, timeZone: timeZone)
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
        /// Show Closed, on or off.
        case showsCompletedToggled(Bool)
        case newProjectTapped
        /// The iPhone's Add Task: Quick Add, in this Avenue or Project.
        case addTaskTapped
        case newProjectCancelled
        case newProjectSubmitted
        /// The new Project's name was refused; it's kept for the next try.
        case newProjectRefused(String)
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
                state.$grouping.withLock { $0 = grouping }
                state.requery(now: now, timeZone: timeZone)
                return .none

            case .showsCompletedToggled(let shows):
                state.$showsCompleted.withLock { $0 = shows }
                state.requery(now: now, timeZone: timeZone)
                return .none

            case .newProjectTapped:
                state.newProjectName = state.refusedProjectName ?? ""
                state.refusedProjectName = nil
                return .none

            case .newProjectRefused(let name):
                state.refusedProjectName = name
                return .none

            case .newProjectCancelled:
                state.newProjectName = nil
                return .none

            case .newProjectSubmitted:
                let name = (state.newProjectName ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                state.newProjectName = nil
                guard !name.isEmpty, case .avenue(let avenueID) = state.scope else { return .none }
                return .run { [store] send in
                    let result = await Result { try await store.createProject(name, in: avenueID) }
                    if case .failure = result { await send(.newProjectRefused(name)) }
                    await send(.list(.saved(result)))
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

            case .binding, .list, .project, .editAvenue, .delegate, .addTaskTapped:
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
        // One pass over the tasks, not one per Avenue and Project.
        var byAvenue: [UUID: Int] = [:]
        var byProject: [UUID: Int] = [:]
        for task in listedTasks where task.isOpen && !task.isSubtask {
            byAvenue[task.avenueID, default: 0] += 1
            if let project = task.projectID { byProject[project, default: 0] += 1 }
        }
        return avenues.values
            .filter { $0.archivedAt == nil }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
            .map { avenue in
                AvenueRow(
                    id: avenue.id, key: avenue.key.rawValue, name: avenue.name, color: avenue.color,
                    symbol: avenue.symbol, openCount: byAvenue[avenue.id] ?? 0,
                    projects: projects.values
                        .filter { $0.avenueID == avenue.id && $0.status == .active }
                        .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                        .map { project in
                            AvenueRow.ProjectRow(
                                id: project.id, name: project.name,
                                openCount: byProject[project.id] ?? 0)
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
        /// Why the Avenues couldn't be loaded, until they load.
        package var loadError: String?

        /// The Inbox, which shows apart, above the Avenues.
        package var inbox: AvenueRow? { avenues.first { $0.id == Avenue.inboxID } }
        /// Every Avenue but the Inbox, in order: the ones that move, since
        /// the Inbox stays first.
        package var movable: [AvenueRow] { avenues.filter { $0.id != Avenue.inboxID } }

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
        /// An Avenue dragged onto another, to take its place.
        case avenueDropped(UUID, onto: UUID)
        /// A Project dragged onto another in its Avenue, to take its place.
        case projectDropped(UUID, onto: UUID)
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
                state.loadError = nil
                state.show(Snapshot(workspace, number: 0), now: now, timeZone: timeZone)
                return .none

            case .show(let snapshot):
                state.show(snapshot, now: now, timeZone: timeZone)
                return .none

            case .loaded(.failure(let error)):
                state.loadError = String(describing: error)
                return .none

            case .opened(let scope):
                guard let workspace = state.workspace else { return .none }
                state.avenue = AvenueFeature.State(scope: scope, workspace: workspace, now: now, timeZone: timeZone)
                return .none

            case .newAvenueTapped:
                state.newAvenue = NewAvenueFeature.State()
                return .none

            case .moveAvenue(let id, let up):
                guard let (lower, upper) = Self.neighbours(of: id, in: state.movable.map(\.id), up: up)
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

            case .avenueDropped(let id, let target):
                guard let (lower, upper) = Self.neighbours(of: id, onto: target, in: state.movable.map(\.id))
                else { return .none }
                return .run { [store] send in
                    await send(.moved(Result { try await store.reorder(avenue: id, after: lower, before: upper) }))
                }

            case .projectDropped(let id, let target):
                // Only within its Avenue: in another, its tasks would get new codes.
                guard let row = state.avenues.first(where: { $0.projects.contains { $0.id == id } }),
                    let (lower, upper) = Self.neighbours(of: id, onto: target, in: row.projects.map(\.id))
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

    /// The rows to put `id` between so it takes `target`'s place in `ids`:
    /// after it when moving down, before it when moving up. `nil` when
    /// either isn't there or they're the same.
    private static func neighbours(of id: UUID, onto target: UUID, in ids: [UUID]) -> (UUID?, UUID?)? {
        guard id != target, let index = ids.firstIndex(of: id), let to = ids.firstIndex(of: target) else {
            return nil
        }
        var rest = ids
        rest.remove(at: index)
        rest.insert(id, at: to)
        let lower = to > 0 ? rest[to - 1] : nil
        let upper = to + 1 < rest.count ? rest[to + 1] : nil
        return (lower, upper)
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

extension SharedKey where Self == AppStorageKey<Bool>.Default {
    /// Whether Avenue and Project lists show closed tasks. This device
    /// only; on unless the person turns it off.
    package static var showsCompleted: Self {
        Self[.appStorage("lifrShowsCompleted"), default: true]
    }
}
