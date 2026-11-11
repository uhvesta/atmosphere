import ComposableArchitecture
import Foundation
import LifrCore
import LifrFeedbackClient
import LifrStoreClient
import LifrTaskList

/// A month of tasks by the day they're due, and the selected day's tasks
/// as a list under it. A task dropped on a day is due that day.
@Reducer
package struct CalendarFeature {
    @ObservableState
    package struct State: Equatable {
        /// The first of the month shown, or `nil` until the records show.
        package var month: CalendarDate?
        package var selected: CalendarDate?
        package var weeks: [[CalendarDay]] = []
        /// The day weeks start on, 1 for Sunday through 7 for Saturday, as
        /// the view's calendar has it.
        package var firstWeekday = 1
        /// The selected day's tasks, with their detail, prompts and alerts.
        package var list = TaskListFeature.State(query: .calendarDay(nil))

        package init() {}
    }

    package enum Action {
        case list(TaskListFeature.Action)
        case daySelected(CalendarDate)
        /// The next month, or the previous one when negative.
        case monthStepped(Int)
        case todayTapped
        case firstWeekdayChanged(Int)
        case dropped(UUID, on: CalendarDate)
    }

    @Dependency(\.store) private var store
    @Dependency(\.feedback) private var feedback
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        Scope(state: \.list, action: \.list) { TaskListFeature() }
        Reduce { state, action in
            switch action {
            case .list(.show), .list(.loaded(.success)), .list(.saved(.success)):
                // Opens on today; afterwards the records change under the
                // day the person picked.
                if state.selected == nil {
                    select(CalendarDate(now, in: timeZone), in: &state)
                } else {
                    layOut(&state)
                }
                return .none

            case .daySelected(let day):
                select(day, in: &state)
                return .none

            case .monthStepped(let months):
                guard let month = state.month else { return .none }
                let shown = month.firstOfMonth(adding: months)
                let today = CalendarDate(now, in: timeZone)
                select(today.firstOfMonth == shown ? today : shown, in: &state)
                return .none

            case .todayTapped:
                select(CalendarDate(now, in: timeZone), in: &state)
                return .none

            case .firstWeekdayChanged(let weekday):
                guard (1...7).contains(weekday), weekday != state.firstWeekday else { return .none }
                state.firstWeekday = weekday
                layOut(&state)
                return .none

            case .dropped(let id, let day):
                guard let task = state.list.workspace?.tasks[id], task.dueDate != day else { return .none }
                var edit = Workspace.TaskEdit(dueDate: .some(day))
                // A task that runs over days moves as a whole.
                if let start = task.startDate {
                    if let due = task.dueDate {
                        edit.startDate = .some(start.adding(days: due.days(until: day)))
                    } else if start > day {
                        edit.startDate = .some(day)
                    }
                }
                return .run { [store, edit, feedback] send in
                    let saved = await Result { try await store.edit(id, edit) }
                    if case .success = saved { await feedback(.tap) }
                    await send(.list(.saved(saved)))
                }

            case .list:
                return .none
            }
        }
    }

    private func select(_ day: CalendarDate, in state: inout State) {
        state.selected = day
        state.month = day.firstOfMonth
        state.list.query = .calendarDay(day)
        if let snapshot = state.list.snapshot { state.list.show(snapshot, now: now, timeZone: timeZone) }
        layOut(&state)
    }

    private func layOut(_ state: inout State) {
        guard let month = state.month, let workspace = state.list.workspace else { return }
        state.weeks = workspace.calendarWeeks(
            month: month, firstWeekday: state.firstWeekday, now: now, timeZone: timeZone)
    }
}
