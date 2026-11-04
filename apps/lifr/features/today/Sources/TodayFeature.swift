import ComposableArchitecture
import Foundation
import LifrCore
import LifrProcrastinated
import LifrTaskList
import LifrUI

extension TaskQuery {
    /// Open tasks that are overdue or due today, across every Avenue.
    package static let today = TaskQuery(
        key: "today",
        empty: .init(
            "Nothing due today", systemImage: "sun.max",
            description: "Tasks due today, and overdue ones, show up here.")
    ) { workspace, now, timeZone in
        let today = CalendarDate(now, in: timeZone)
        let time = TimeOfDay(now, in: timeZone)
        let open = workspace.listedTasks.filter(\.isOpen)
        let overdue = open.filter { $0.isOverdue(today: today, time: time) }
        let dueToday = open.filter { $0.dueDate == today && !overdue.contains($0) }
        return [
            TaskSection(id: "overdue", title: "Overdue", rows: workspace.rows(overdue, now: now, timeZone: timeZone)),
            TaskSection(id: "today", title: "Today", rows: workspace.rows(dueToday, now: now, timeZone: timeZone)),
        ]
    }
}

/// Today: open tasks that are overdue or due today, with a way into the
/// Procrastinated review.
@Reducer
package struct TodayFeature {
    @ObservableState
    package struct State: Equatable {
        package var list: TaskListFeature.State
        /// How many tasks are stalled, shown on the way into Procrastinated.
        package var procrastinatedCount = 0
        @Presents package var procrastinated: ProcrastinatedFeature.State?

        package init() {
            list = TaskListFeature.State(query: .today)
        }

        package init(workspace: Workspace, now: Date, timeZone: TimeZone) {
            list = TaskListFeature.State(query: .today, workspace: workspace, now: now, timeZone: timeZone)
            procrastinatedCount = TaskQuery.procrastinatedCount(in: workspace, now: now)
        }
    }

    package enum Action {
        case list(TaskListFeature.Action)
        case procrastinatedTapped
        case procrastinated(PresentationAction<ProcrastinatedFeature.Action>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            /// Today or the review changed the records.
            case saved(Workspace)
        }
    }

    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        Scope(state: \.list, action: \.list) { TaskListFeature() }
        Reduce { state, action in
            switch action {
            case .list(.show(let workspace)):
                state.procrastinated?.list.show(workspace, now: now, timeZone: timeZone)
                state.procrastinatedCount = TaskQuery.procrastinatedCount(in: workspace, now: now)
                return .none

            case .list(.delegate(.saved(let workspace))):
                state.procrastinatedCount = TaskQuery.procrastinatedCount(in: workspace, now: now)
                return .send(.delegate(.saved(workspace)))

            case .list(.loaded(.success(let workspace))):
                state.procrastinatedCount = TaskQuery.procrastinatedCount(in: workspace, now: now)
                return .none

            case .procrastinatedTapped:
                guard let workspace = state.list.workspace else { return .none }
                state.procrastinated = ProcrastinatedFeature.State(workspace: workspace, now: now, timeZone: timeZone)
                return .none

            case .procrastinated(.presented(.list(.delegate(.saved(let workspace))))):
                state.list.show(workspace, now: now, timeZone: timeZone)
                state.procrastinatedCount = TaskQuery.procrastinatedCount(in: workspace, now: now)
                return .send(.delegate(.saved(workspace)))

            case .list, .procrastinated, .delegate:
                return .none
            }
        }
        .ifLet(\.$procrastinated, action: \.procrastinated) { ProcrastinatedFeature() }
    }
}
