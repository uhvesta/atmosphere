import ComposableArchitecture
import Foundation
import LifrCore
import LifrProcrastinated
import LifrStoreClient
import LifrTaskList
import LifrUI

extension TaskQuery {
    /// Open tasks that are overdue or due today, across every Avenue.
    package static let today = TaskQuery(
        key: "today",
        empty: .init(
            "Nothing due today", systemImage: "sun.max",
            description: String(localized: "Tasks due today, and overdue ones, show up here.")),
        finished: .init(
            "All done for today", systemImage: "checkmark.seal",
            description: String(localized: "Everything due today is closed.")),
        adds: NewTaskDefaults(isDueToday: true)
    ) { workspace, now, timeZone in
        let today = CalendarDate(now, in: timeZone)
        let time = TimeOfDay(now, in: timeZone)
        let open = workspace.listedTasks.filter(\.isOpen)
        // The longest overdue first: by day, then time, a task due at no
        // time of day coming first on its day.
        let overdue = open.filter { $0.isOverdue(today: today, time: time) }
            .sorted { first, second in
                guard first.dueDate == second.dueDate else {
                    return (first.dueDate ?? today) < (second.dueDate ?? today)
                }
                switch (first.dueTime, second.dueTime) {
                case (let first?, let second?): return first < second
                case (nil, _?): return true
                default: return false
                }
            }
        let overdueIDs = Set(overdue.map(\.id))
        let dueToday = open.filter { $0.dueDate == today && !overdueIDs.contains($0.id) }
        return [
            TaskSection(
                id: "overdue", title: String(localized: "Overdue"),
                rows: workspace.rows(overdue, now: now, timeZone: timeZone)),
            TaskSection(
                id: "today", title: String(localized: "Today"),
                rows: workspace.rows(dueToday, now: now, timeZone: timeZone)),
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
        /// Whether an empty Today can offer the open tasks with no due date
        /// to plan the day from.
        package var canPlanFromAnytime = false
        @Presents package var procrastinated: ProcrastinatedFeature.State?

        package init() {
            list = TaskListFeature.State(query: .today)
            list.offersSwipeTip = true
        }

        package init(workspace: Workspace, now: Date, timeZone: TimeZone) {
            list = TaskListFeature.State(query: .today, workspace: workspace, now: now, timeZone: timeZone)
            list.offersSwipeTip = true
            procrastinatedCount = TaskQuery.procrastinatedCount(in: workspace, now: now)
        }
    }

    package enum Action {
        case list(TaskListFeature.Action)
        case procrastinatedTapped
        case planFromAnytimeTapped
        case procrastinated(PresentationAction<ProcrastinatedFeature.Action>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            /// Today or the review changed the records.
            case saved(Snapshot)
            /// Shows Anytime, to pick tasks for today.
            case planFromAnytime
        }
    }

    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        Scope(state: \.list, action: \.list) { TaskListFeature() }
        Reduce { state, action in
            switch action {
            case .list(.show(let snapshot)):
                state.procrastinated?.list.show(snapshot, now: now, timeZone: timeZone)
                countProcrastinated(&state)
                return .none

            case .list(.delegate(.saved(let snapshot))):
                // Root shows the rest of Lifr this save, not Today, which
                // shows it already.
                state.procrastinated?.list.show(snapshot, now: now, timeZone: timeZone)
                countProcrastinated(&state)
                return .send(.delegate(.saved(snapshot)))

            case .list(.loaded(.success)):
                countProcrastinated(&state)
                return .none

            case .procrastinatedTapped:
                guard let workspace = state.list.workspace else { return .none }
                state.procrastinated = ProcrastinatedFeature.State(workspace: workspace, now: now, timeZone: timeZone)
                return .none

            case .planFromAnytimeTapped:
                return .send(.delegate(.planFromAnytime))

            case .procrastinated(.presented(.list(.delegate(.saved(let snapshot))))):
                state.list.show(snapshot, now: now, timeZone: timeZone)
                countProcrastinated(&state)
                return .send(.delegate(.saved(snapshot)))

            case .list, .procrastinated, .delegate:
                return .none
            }
        }
        .ifLet(\.$procrastinated, action: \.procrastinated) { ProcrastinatedFeature() }
    }

    /// Counts procrastinated tasks in the records the list shows: the
    /// latest, as the list keeps only those.
    private func countProcrastinated(_ state: inout State) {
        guard let workspace = state.list.workspace else { return }
        state.procrastinatedCount = TaskQuery.procrastinatedCount(in: workspace, now: now)
        // Only an empty Today offers it, so a full one skips the search.
        let today = CalendarDate(now, in: timeZone)
        state.canPlanFromAnytime =
            state.list.isEmpty
            && workspace.listedTasks.contains {
                $0.isOpen && !$0.isSubtask && $0.dueDate == nil && ($0.startDate.map { $0 <= today } ?? true)
                    && workspace.avenues[$0.avenueID]?.archivedAt == nil
            }
    }
}
