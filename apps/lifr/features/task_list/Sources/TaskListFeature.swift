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
        /// The latest save shown, so an earlier one's late reply is ignored.
        package var saves = SaveOrder()
        /// `nil` until the records load.
        package var workspace: Workspace?
        package var sections: [TaskSection] = []
        package var banner: Banner?
        @Presents package var detail: TaskDetailFeature.State?
        @Presents package var reschedule: RescheduleFeature.State?
        @Presents package var prompt: AlertState<StatusPrompt>?
        /// Every status, from a row's Status swipe.
        @Presents package var statusPicker: ConfirmationDialogState<StatusPick>?
        @Presents package var alert: AlertState<Never>?

        package init(query: TaskQuery) {
            self.query = query
        }

        package init(query: TaskQuery, workspace: Workspace, now: Date, timeZone: TimeZone) {
            self.query = query
            show(Snapshot(workspace, number: 0), now: now, timeZone: timeZone)
        }

        package var isEmpty: Bool { sections.allSatisfy(\.rows.isEmpty) }

        package var rows: [TaskRowData] { sections.flatMap(\.rows) }

        /// The records shown, with their place in the store's order.
        package var snapshot: Snapshot? { workspace.map { Snapshot($0, number: saves.latest) } }

        /// Shows `snapshot`'s records through the query, and the open task
        /// with them, unless later ones are shown already.
        package mutating func show(_ snapshot: Snapshot, now: Date, timeZone: TimeZone) {
            guard saves.isLatest(snapshot) else { return }
            workspace = snapshot.workspace
            sections = query.sections(snapshot.workspace, now, timeZone).filter { !$0.rows.isEmpty }
            // A task trashed, here or elsewhere, closes.
            if var detail {
                self.detail = detail.refresh(from: snapshot, now: now, timeZone: timeZone) ? detail : nil
            }
        }
    }

    package enum Action {
        /// The view appeared: load the records if they aren't yet.
        case task
        case loaded(Result<Workspace, any Error>)
        /// The records changed elsewhere, such as in another tab or a sync.
        case show(Snapshot)
        case rowTapped(UUID)
        case toggled(UUID)
        case rowAction(UUID, RowAction)
        case saved(Result<Saved, any Error>)
        case bannerUndoTapped
        case bannerDismissed
        case detail(PresentationAction<TaskDetailFeature.Action>)
        case reschedule(PresentationAction<RescheduleFeature.Action>)
        case prompt(PresentationAction<StatusPrompt>)
        case statusPicker(PresentationAction<StatusPick>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            /// This list changed the records; other lists should show them.
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
                return .run { [store] send in
                    await send(.loaded(Result { try await store.load() }))
                }

            case .loaded(.success(let workspace)):
                state.show(Snapshot(workspace, number: 0), now: now, timeZone: timeZone)
                return .none

            case .show(let snapshot):
                state.show(snapshot, now: now, timeZone: timeZone)
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

            case .rowAction(let id, .pickStatus):
                guard let row = state.rows.first(where: { $0.id == id }) else { return .none }
                state.statusPicker = .statuses(for: row)
                return .none

            case .statusPicker(.presented(let pick)):
                return setStatus(pick.status, pick.task)

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
                let isLatest = saved.number >= state.saves.latest
                state.show(saved.snapshot, now: now, timeZone: timeZone)
                if let parentID = saved.change.reopenedParent, let parent = saved.workspace.tasks[parentID] {
                    state.banner = .reopened(parent, by: saved.change)
                }
                return isLatest ? .send(.delegate(.saved(saved.snapshot))) : .none

            case .saved(.failure(let error)):
                let workspace = state.workspace
                if let prompt = AlertState.forRefusal(
                    error, canShow: true, title: { workspace?.tasks[$0]?.title })
                {
                    state.prompt = prompt
                } else {
                    state.alert = .failed("Couldn't change the task", error)
                }
                return .none

            case .detail(.presented(.delegate(.saved(let snapshot)))):
                state.show(snapshot, now: now, timeZone: timeZone)
                return .send(.delegate(.saved(snapshot)))

            case .detail, .reschedule, .prompt, .statusPicker, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$detail, action: \.detail) { TaskDetailFeature() }
        .ifLet(\.$reschedule, action: \.reschedule) { RescheduleFeature() }
        .ifLet(\.$prompt, action: \.prompt)
        .ifLet(\.$statusPicker, action: \.statusPicker)
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

/// A status picked for a task from a row's Status swipe.
package struct StatusPick: Equatable, Sendable {
    package var task: UUID
    package var status: TaskStatus

    package init(task: UUID, status: TaskStatus) {
        self.task = task
        self.status = status
    }
}

extension ConfirmationDialogState where Action == StatusPick {
    /// Every status but the row's own.
    static func statuses(for row: TaskRowData) -> Self {
        ConfirmationDialogState(titleVisibility: .visible) {
            TextState(row.title)
        } actions: {
            for status in TaskStatus.allCases where status != row.status {
                ButtonState(action: StatusPick(task: row.id, status: status)) { TextState(status.title) }
            }
            ButtonState(role: .cancel) { TextState("Cancel") }
        } message: {
            TextState("Currently \(row.status.title).")
        }
    }
}
