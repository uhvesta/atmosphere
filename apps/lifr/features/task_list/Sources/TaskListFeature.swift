import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskDetail
import LifrUI

/// A list of tasks in sections, shared by Today, Upcoming, Procrastinated,
/// the Avenue view, search results and the Mac's board. Rows open the task,
/// toggle Done, and change status by swipe; refused changes ask first, and
/// a reopened parent offers Undo.
@Reducer
package struct TaskListFeature {
    @ObservableState
    package struct State: Equatable {
        package var query: TaskQuery
        /// `nil` until the records load.
        package var workspace: Workspace?
        package var sections: [TaskSection] = []
        package var banner: Banner?
        @Presents package var detail: TaskDetailFeature.State?
        @Presents package var reschedule: RescheduleFeature.State?
        @Presents package var prompt: ConfirmationDialogState<StatusPrompt>?
        @Presents package var alert: AlertState<Never>?

        package init(query: TaskQuery) {
            self.query = query
        }

        package init(query: TaskQuery, workspace: Workspace, now: Date, timeZone: TimeZone) {
            self.query = query
            show(workspace, now: now, timeZone: timeZone)
        }

        package var isEmpty: Bool { sections.allSatisfy(\.rows.isEmpty) }

        package var rows: [TaskRowData] { sections.flatMap(\.rows) }

        /// Shows `workspace` through the query, and the open task with it.
        package mutating func show(_ workspace: Workspace, now: Date, timeZone: TimeZone) {
            self.workspace = workspace
            sections = query.sections(workspace, now, timeZone).filter { !$0.rows.isEmpty }
            if var detail, detail.refresh(from: workspace, now: now, timeZone: timeZone) {
                self.detail = detail
            }
        }
    }

    package enum Action {
        /// The view appeared: load the records if they aren't yet.
        case task
        case loaded(Result<Workspace, any Error>)
        /// The records changed elsewhere, such as in another tab or a sync.
        case show(Workspace)
        case rowTapped(UUID)
        case toggled(UUID)
        case rowAction(UUID, RowAction)
        case saved(Result<Saved, any Error>)
        case bannerUndoTapped
        case bannerDismissed
        case detail(PresentationAction<TaskDetailFeature.Action>)
        case reschedule(PresentationAction<RescheduleFeature.Action>)
        case prompt(PresentationAction<StatusPrompt>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            /// This list changed the records; other lists should show them.
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
                return .run { [store] send in
                    await send(.loaded(Result { try await store.load() }))
                }

            case .loaded(.success(let workspace)), .show(let workspace):
                state.show(workspace, now: now, timeZone: timeZone)
                return .none

            case .loaded(.failure(let error)):
                state.alert = .failed("Couldn't load your tasks", error)
                return .none

            case .rowTapped(let id):
                guard let workspace = state.workspace else { return .none }
                state.detail = TaskDetailFeature.State(id: id, workspace: workspace, now: now, timeZone: timeZone)
                return .none

            case .toggled(let id):
                guard let row = state.rows.first(where: { $0.id == id }) else { return .none }
                return setStatus(row.status.isClosed ? .todo : .done, id)

            case .rowAction(let id, .status(let status)):
                return setStatus(status, id)

            case .rowAction(let id, .reschedule):
                guard let row = state.rows.first(where: { $0.id == id }) else { return .none }
                state.reschedule = RescheduleFeature.State(
                    id: id, title: row.title, date: now.addingTimeInterval(24 * 60 * 60))
                return .none

            case .reschedule(.presented(.delegate(.picked(let id, let date)))):
                return save { try await $0.edit(id, Workspace.TaskEdit(dueDate: .some(date))) }

            case .prompt(.presented(.showSubtasks(let id))):
                return .send(.rowTapped(id))

            case .prompt(.presented(let answer)):
                guard let retry = answer.retry else { return .none }
                return setStatus(retry.status, retry.task, cascade: true)

            case .bannerUndoTapped:
                guard let change = state.banner?.undo else { return .none }
                state.banner = nil
                return save { try await $0.undo(change) }

            case .bannerDismissed:
                state.banner = nil
                return .none

            case .saved(.success(let saved)):
                state.show(saved.workspace, now: now, timeZone: timeZone)
                if let parentID = saved.change.reopenedParent, let parent = saved.workspace.tasks[parentID] {
                    state.banner = .reopened(parent, by: saved.change)
                }
                return .send(.delegate(.saved(saved.workspace)))

            case .saved(.failure(let error)):
                let workspace = state.workspace
                if let prompt = ConfirmationDialogState.forRefusal(
                    error, canShow: true, title: { workspace?.tasks[$0]?.title })
                {
                    state.prompt = prompt
                } else {
                    state.alert = .failed("Couldn't change the task", error)
                }
                return .none

            case .detail(.presented(.delegate(.saved(let workspace)))):
                state.show(workspace, now: now, timeZone: timeZone)
                return .send(.delegate(.saved(workspace)))

            case .detail, .reschedule, .prompt, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$detail, action: \.detail) { TaskDetailFeature() }
        .ifLet(\.$reschedule, action: \.reschedule) { RescheduleFeature() }
        .ifLet(\.$prompt, action: \.prompt)
        .ifLet(\.$alert, action: \.alert)
    }

    private func setStatus(_ status: TaskStatus, _ id: UUID, cascade: Bool = false) -> Effect<Action> {
        save { try await $0.setStatus(status, id, cascade) }
    }

    private func save(_ operation: @escaping @Sendable (StoreClient) async throws -> Saved) -> Effect<Action> {
        .run { [store] send in
            await send(.saved(Result { try await operation(store) }))
        }
    }
}
