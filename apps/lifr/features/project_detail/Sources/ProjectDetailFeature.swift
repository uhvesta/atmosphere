import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskDetail

/// One Project's own fields, shown above its tasks: name, details, due date
/// and Avenue, and completing, archiving or restoring it. Archiving a
/// Project with open tasks asks first whether to move them out to its
/// Avenue or close them.
@Reducer
package struct ProjectDetailFeature {
    @ObservableState
    package struct State: Equatable {
        package let id: UUID
        /// The latest save shown, so an earlier one's late reply is ignored.
        package var saves = SaveOrder()
        /// The name as typed; saved on submit.
        package var name: String
        package var savedName: String
        /// The details as typed; saved on submit.
        package var details: String
        package var savedDetails: String
        package var dueDate: CalendarDate?
        package var status: ProjectStatus
        package var avenueID: UUID
        package var avenueName: String
        /// Where the Project can move: open Avenues, and its own.
        package var avenueChoices: [AvenueChoice]
        /// Top-level tasks, not counting trashed ones.
        package var taskCount: Int
        package var openCount: Int
        /// An alert, not a confirmation dialog: on iOS a dialog is a popover
        /// that hides Cancel.
        @Presents package var archive: AlertState<Workspace.OpenTasks>?
        @Presents package var alert: AlertState<Never>?

        /// `nil` when the Project isn't in `workspace`.
        package init?(id: UUID, workspace: Workspace) {
            guard let project = workspace.projects[id] else { return nil }
            self.id = id
            name = project.name
            savedName = project.name
            details = project.details
            savedDetails = project.details
            dueDate = project.dueDate
            status = project.status
            avenueID = project.avenueID
            avenueName = workspace.avenues[project.avenueID]?.name ?? ""
            avenueChoices = workspace.avenues.values
                .filter { $0.archivedAt == nil || $0.id == project.avenueID }
                .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                .map { AvenueChoice(id: $0.id, name: $0.name) }
            let tasks = workspace.tasks.values.filter { $0.projectID == id && $0.deletedAt == nil && !$0.isSubtask }
            taskCount = tasks.count
            openCount = tasks.count(where: \.isOpen)
        }

        /// A Project completes only once every task in it is closed.
        package var canComplete: Bool { status == .active && openCount == 0 }

        /// Shows `workspace`, keeping what the person is typing. False when
        /// the Project is gone.
        package mutating func refresh(from workspace: Workspace) -> Bool {
            guard var refreshed = State(id: id, workspace: workspace) else { return false }
            if name != savedName { refreshed.name = name }
            if details != savedDetails { refreshed.details = details }
            refreshed.archive = archive
            refreshed.alert = alert
            refreshed.saves = saves
            self = refreshed
            return true
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case nameSubmitted
        case detailsSubmitted
        case dueDatePicked(CalendarDate?)
        case avenuePicked(UUID)
        case completeTapped
        case archiveTapped
        case restoreTapped
        case archive(PresentationAction<Workspace.OpenTasks>)
        case saved(Result<Saved, any Error>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
        }
    }

    @Dependency(\.store) private var store

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .nameSubmitted:
                let name = state.name.trimmingCharacters(in: .whitespacesAndNewlines)
                guard name != state.savedName else { return .none }
                guard !name.isEmpty else {
                    state.name = state.savedName
                    return .none
                }
                return save { [id = state.id] in try await $0.edit(project: id, Workspace.ProjectEdit(name: name)) }

            case .detailsSubmitted:
                guard state.details != state.savedDetails else { return .none }
                return save { [id = state.id, details = state.details] in
                    try await $0.edit(project: id, Workspace.ProjectEdit(details: details))
                }

            case .dueDatePicked(let date):
                guard date != state.dueDate else { return .none }
                return save { [id = state.id] in
                    try await $0.edit(project: id, Workspace.ProjectEdit(dueDate: .some(date)))
                }

            case .avenuePicked(let avenueID):
                guard avenueID != state.avenueID else { return .none }
                return save { [id = state.id] in try await $0.move(project: id, toAvenue: avenueID) }

            case .completeTapped:
                return save { [id = state.id] in try await $0.setStatus(.completed, ofProject: id) }

            case .archiveTapped:
                guard state.openCount > 0 else {
                    return save { [id = state.id] in try await $0.archive(project: id, openTasks: .moveToAvenue) }
                }
                state.archive = .archive(openCount: state.openCount, avenue: state.avenueName)
                return .none

            case .archive(.presented(let openTasks)):
                return save { [id = state.id] in try await $0.archive(project: id, openTasks: openTasks) }

            case .restoreTapped:
                return save { [id = state.id] in try await $0.setStatus(.active, ofProject: id) }

            case .saved(.success(let saved)):
                // Saved, so what was typed is what's stored now.
                state.name = state.name.trimmingCharacters(in: .whitespacesAndNewlines)
                guard state.saves.isLatest(saved) else { return .none }
                _ = state.refresh(from: saved.workspace)
                return .send(.delegate(.saved(saved.workspace)))

            case .saved(.failure(let error)):
                state.name = state.savedName
                state.alert = .failed("Couldn't change the Project", error)
                return .none

            case .binding, .archive, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$archive, action: \.archive)
        .ifLet(\.$alert, action: \.alert)
    }

    private func save(_ operation: @escaping @Sendable (StoreClient) async throws -> Saved) -> Effect<Action> {
        .run { [store] send in
            do {
                await send(.saved(.success(try await operation(store))))
            } catch {
                await send(.saved(.failure(error)))
            }
        }
    }
}

/// An Avenue a Project can move to.
package struct AvenueChoice: Identifiable, Hashable, Sendable {
    package let id: UUID
    package let name: String
}

extension AlertState where Action == Workspace.OpenTasks {
    /// Asks what to do with a Project's open tasks before archiving it.
    static func archive(openCount: Int, avenue: String) -> Self {
        let open = counted(openCount, "open task", "open tasks")
        return AlertState {
            TextState("Archive this Project?")
        } actions: {
            ButtonState(action: .moveToAvenue) { TextState("Move Them to \(avenue)") }
            ButtonState(role: .destructive, action: .closeAsWontDo) { TextState("Close Them as Won't Do") }
            ButtonState(role: .cancel) { TextState("Cancel") }
        } message: {
            TextState(
                "It has \(open). Archiving takes the Project out of the sidebar; you can restore it from Archived.")
        }
    }
}
