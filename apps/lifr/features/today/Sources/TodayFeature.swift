import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskDetail
import LifrUI

/// Open tasks that are overdue or due today, across every Avenue.
@Reducer
package struct TodayFeature {
    @ObservableState
    package struct State: Equatable {
        /// `nil` until the records load.
        package var workspace: Workspace?
        package var overdue: [TaskRowData] = []
        package var dueToday: [TaskRowData] = []
        @Presents package var detail: TaskDetailFeature.State?
        @Presents package var alert: AlertState<Action.Alert>?

        package init() {}

        package init(workspace: Workspace, now: Date, timeZone: TimeZone) {
            show(workspace, now: now, timeZone: timeZone)
        }

        mutating func show(_ workspace: Workspace, now: Date, timeZone: TimeZone) {
            self.workspace = workspace
            let today = CalendarDate(now, in: timeZone)
            let time = TimeOfDay(now, in: timeZone)
            let open = workspace.tasks.values
                .filter { task in
                    task.isOpen && task.deletedAt == nil
                        && workspace.avenues[task.avenueID]?.archivedAt == nil
                }
                .sorted(by: LifrTask.listOrder)
            func rows(_ tasks: [LifrTask]) -> [TaskRowData] {
                tasks.map { TaskRowData($0, in: workspace, now: now, timeZone: timeZone) }
            }
            let overdue = open.filter { $0.isOverdue(today: today, time: time) }
            self.overdue = rows(overdue)
            dueToday = rows(open.filter { $0.dueDate == today && !overdue.contains($0) })
        }
    }

    package enum Action {
        case task
        case loaded(Result<Workspace, any Error>)
        case rowTapped(UUID)
        case toggled(UUID)
        case saved(Result<Saved, any Error>)
        case detail(PresentationAction<TaskDetailFeature.Action>)
        case alert(PresentationAction<Alert>)

        package enum Alert: Equatable {}
    }

    @Dependency(\.store) private var store
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                return .run { [store] send in
                    do {
                        await send(.loaded(.success(try await store.load())))
                    } catch {
                        await send(.loaded(.failure(error)))
                    }
                }

            case .loaded(.success(let workspace)):
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
                guard let row = (state.overdue + state.dueToday).first(where: { $0.id == id }) else { return .none }
                let status: TaskStatus = row.status.isClosed ? .todo : .done
                return .run { [store] send in
                    do {
                        await send(.saved(.success(try await store.setStatus(status, id, false))))
                    } catch {
                        await send(.saved(.failure(error)))
                    }
                }

            case .saved(.success(let saved)):
                state.show(saved.workspace, now: now, timeZone: timeZone)
                return .none

            case .saved(.failure(let error)):
                state.alert = .failed("Couldn't change the task", error)
                return .none

            case .detail(.presented(.delegate(.saved(let workspace)))):
                state.show(workspace, now: now, timeZone: timeZone)
                return .none

            case .detail, .alert:
                return .none
            }
        }
        .ifLet(\.$detail, action: \.detail) {
            TaskDetailFeature()
        }
        .ifLet(\.$alert, action: \.alert)
    }
}

extension AlertState where Action == TodayFeature.Action.Alert {
    static func failed(_ title: String, _ error: any Error) -> Self {
        AlertState {
            TextState(title)
        } actions: {
            ButtonState(role: .cancel) { TextState("OK") }
        } message: {
            TextState((error as? LifrError)?.description ?? error.localizedDescription)
        }
    }
}
