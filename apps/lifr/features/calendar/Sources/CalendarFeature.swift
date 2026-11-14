import ComposableArchitecture
import Foundation
import LifrCore
import LifrFeedbackClient
import LifrStoreClient
import LifrTaskList
import LifrUI

/// How much of the calendar shows at once.
package enum CalendarMode: String, CaseIterable, Sendable {
    case month, week
}

/// A month or a week of tasks by the day they're due, and the selected
/// day's tasks as a list under it. A task dropped on a day is due that day.
@Reducer
package struct CalendarFeature {
    @ObservableState
    package struct State: Equatable {
        /// The first of the month shown, or `nil` until the records show.
        package var month: CalendarDate?
        package var selected: CalendarDate?
        /// The month's weeks, in month mode.
        package var weeks: [[CalendarDay]] = []
        /// The selected day's week, in week mode.
        package var week: [CalendarDay] = []
        /// The day weeks start on, 1 for Sunday through 7 for Saturday, as
        /// the view's calendar has it.
        package var firstWeekday = 1
        /// The weekdays of the weekend, shaded, as the view's calendar has
        /// them.
        package var weekend = Calendar.saturdayAndSunday
        /// The selected day's tasks, with their detail, prompts and alerts.
        package var list = TaskListFeature.State(query: .calendarDay(nil))
        /// The months on one long page, to scroll through and pick a day
        /// from, while it's open.
        package var months: [CalendarMonth]?
        /// Month or week, remembered on this device.
        @Shared(.appStorage("lifrCalendarMode")) package var mode = CalendarMode.month
        /// Whether Done and Won't Do tasks show, remembered on this device.
        @Shared(.appStorage("lifrCalendarShowsCompleted")) package var showsCompleted = false
        /// Whether tasks show as bars from their start dates, remembered on
        /// this device.
        @Shared(.appStorage("lifrCalendarShowsStartDates")) package var showsStartDates = false
        /// The Avenues shown; empty shows them all.
        package var avenues: Set<UUID> = []
        /// Whether the open tasks with no due date show, to drag onto a day.
        package var showsTray = false
        /// Those tasks, in list order, while they show.
        package var tray: [CalendarChip] = []
        /// Whether this device saw the tip about dragging a task onto a
        /// day, closed or used.
        @Shared(.appStorage(GestureTip.calendarDrag.storageKey)) package var dragTipSeen = false

        /// Whether to show the drag tip now: once the day has tasks to drag.
        package var showsDragTip: Bool { !dragTipSeen && list.workspace != nil && !list.isEmpty }

        package var filter: CalendarFilter {
            CalendarFilter(avenues: avenues, showsCompleted: showsCompleted, showsStartDates: showsStartDates)
        }

        /// The Avenues the filter menu offers, in sidebar order.
        package var avenueChoices: [Avenue] {
            (list.workspace?.avenues.values.filter { $0.archivedAt == nil } ?? [])
                .sorted { ($0.sortOrder, $0.name) < ($1.sortOrder, $1.name) }
        }

        package init() {}
    }

    package enum Action {
        case list(TaskListFeature.Action)
        case daySelected(CalendarDate)
        /// The next month or week, or the previous one when negative.
        case stepped(Int)
        case todayTapped
        case modePicked(CalendarMode)
        /// The view's calendar: the day its weeks start on, 1 for Sunday
        /// through 7 for Saturday, and the weekdays of its weekend.
        case calendarChanged(firstWeekday: Int, weekend: Set<Int>)
        /// Tasks dropped on a day are due that day, with their start dates
        /// moved as far. `time` sets the time, `.some(nil)` clears it, and
        /// nil keeps it.
        case dropped([UUID], on: CalendarDate, time: TimeOfDay?? = nil)
        /// Moves a task's due date by `days`, as a drop would, from the
        /// keyboard or VoiceOver.
        case moved(UUID, days: Int)
        /// A bar's leading edge dropped on a day: the task starts that day,
        /// and stays due when it was.
        case startDropped(UUID, on: CalendarDate)
        /// Shows or hides an Avenue's tasks.
        case avenueToggled(UUID)
        case allAvenuesShown
        case showsCompletedToggled
        case showsStartDatesToggled
        /// Shows or hides the tasks with no due date.
        case trayToggled
        case trayDismissed
        case projectTapped(UUID)
        /// Opens the months on one long page.
        case monthsTapped
        /// A month's title on the long page: goes back to that month,
        /// on today when it's this month and its first day otherwise.
        case monthPicked(CalendarDate)
        case monthsDismissed
        /// The drag tip's close button.
        case dragTipClosed
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            /// A Project's flag: opens the Project.
            case openProject(UUID)
        }
    }

    /// How many months the long page shows either side of the one open.
    package static let monthsAround = 24

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
                state.months = nil
                return .none

            case .monthsTapped:
                guard let month = state.month, let workspace = state.list.workspace else { return .none }
                state.months = months(around: month, in: workspace, state)
                return .none

            case .monthPicked(let month):
                let today = CalendarDate(now, in: timeZone)
                select(today.firstOfMonth == month.firstOfMonth ? today : month.firstOfMonth, in: &state)
                state.months = nil
                return .none

            case .monthsDismissed:
                state.months = nil
                return .none

            case .stepped(let steps):
                guard let month = state.month, let selected = state.selected else { return .none }
                let today = CalendarDate(now, in: timeZone)
                switch state.mode {
                case .month:
                    let shown = month.firstOfMonth(adding: steps)
                    select(today.firstOfMonth == shown ? today : shown, in: &state)
                case .week:
                    select(selected.adding(days: steps * 7), in: &state)
                }
                return .none

            case .todayTapped:
                select(CalendarDate(now, in: timeZone), in: &state)
                return .none

            case .modePicked(let mode):
                guard mode != state.mode else { return .none }
                state.$mode.withLock { $0 = mode }
                layOut(&state)
                return .none

            case .calendarChanged(let weekday, let weekend):
                let weekend = weekend.filter { (1...7).contains($0) }
                guard (1...7).contains(weekday), weekday != state.firstWeekday || weekend != state.weekend
                else { return .none }
                state.firstWeekday = weekday
                state.weekend = weekend
                layOut(&state)
                return .none

            case .avenueToggled(let id):
                if state.avenues.contains(id) {
                    state.avenues.remove(id)
                } else {
                    state.avenues.insert(id)
                }
                refilter(&state)
                return .none

            case .allAvenuesShown:
                guard !state.avenues.isEmpty else { return .none }
                state.avenues = []
                refilter(&state)
                return .none

            case .showsCompletedToggled:
                state.$showsCompleted.withLock { $0.toggle() }
                refilter(&state)
                return .none

            case .showsStartDatesToggled:
                state.$showsStartDates.withLock { $0.toggle() }
                layOut(&state)
                return .none

            case .trayToggled:
                state.showsTray.toggle()
                layOut(&state)
                return .none

            case .trayDismissed:
                state.showsTray = false
                state.tray = []
                return .none

            case .projectTapped(let id):
                return .send(.delegate(.openProject(id)))

            case .dragTipClosed:
                state.$dragTipSeen.withLock { $0 = true }
                return .none

            case .dropped(let ids, let day, let time):
                state.$dragTipSeen.withLock { $0 = true }
                guard let workspace = state.list.workspace else { return .none }
                let edits = ids.compactMap { id in Self.edit(workspace.tasks[id], to: day, time: time) }
                guard !edits.isEmpty else { return .none }
                return save(edits)

            case .startDropped(let id, let day):
                state.$dragTipSeen.withLock { $0 = true }
                guard let task = state.list.workspace?.tasks[id], task.deletedAt == nil, task.startDate != day
                else { return .none }
                // Core refuses a start after the due date, and the list
                // explains it.
                return save([(id, Workspace.TaskEdit(startDate: .some(day)))])

            case .moved(let id, let days):
                guard let due = state.list.workspace?.tasks[id]?.dueDate, days != 0 else { return .none }
                return .send(.dropped([id], on: due.adding(days: days)))

            case .list, .delegate:
                return .none
            }
        }
    }

    /// Saves `edits` together, so one Undo puts every task back.
    private func save(_ edits: [(UUID, Workspace.TaskEdit)]) -> Effect<Action> {
        .run { [store, feedback] send in
            let saved = await Result { try await store.edit(edits) }
            if case .success = saved { await feedback(.tap) }
            await send(.list(.saved(saved)))
        }
    }

    /// The edit that makes `task` due on `day`, at `time` when given, or nil
    /// when it's already there.
    private static func edit(
        _ task: LifrTask?, to day: CalendarDate, time: TimeOfDay??
    ) -> (UUID, Workspace.TaskEdit)? {
        guard let task, task.deletedAt == nil else { return nil }
        var edit = Workspace.TaskEdit.due(day, for: task)
        if let time, time != task.dueTime {
            edit = edit ?? Workspace.TaskEdit()
            edit?.dueTime = .some(time)
        }
        return edit.map { (task.id, $0) }
    }

    private func select(_ day: CalendarDate, in state: inout State) {
        state.selected = day
        state.month = day.firstOfMonth
        state.list.query = .calendarDay(day, filter: state.filter)
        if let snapshot = state.list.snapshot { state.list.show(snapshot, now: now, timeZone: timeZone) }
        layOut(&state)
    }

    private func refilter(_ state: inout State) {
        guard let selected = state.selected else { return }
        select(selected, in: &state)
    }

    /// Works out the days of the month or the week shown, not both.
    private func layOut(_ state: inout State) {
        guard let month = state.month, let selected = state.selected, let workspace = state.list.workspace
        else { return }
        switch state.mode {
        case .month:
            state.weeks = workspace.calendarWeeks(
                month: month, firstWeekday: state.firstWeekday, weekend: state.weekend, filter: state.filter, now: now,
                timeZone: timeZone)
            state.week = []
        case .week:
            state.week = workspace.calendarWeek(
                of: selected, firstWeekday: state.firstWeekday, weekend: state.weekend, filter: state.filter, now: now,
                timeZone: timeZone)
            state.weeks = []
        }
        state.tray = state.showsTray ? workspace.unscheduled(state.filter) : []
        if let shown = state.months, let first = shown.first?.id {
            state.months = months(around: first.firstOfMonth(adding: Self.monthsAround), in: workspace, state)
        }
    }

    /// The months either side of `month`, with the tasks due on each day.
    private func months(around month: CalendarDate, in workspace: Workspace, _ state: State) -> [CalendarMonth] {
        // The long page shows dots, not bars.
        var filter = state.filter
        filter.showsStartDates = false
        let due = workspace.calendarDue(filter)
        return (-Self.monthsAround...Self.monthsAround).map { offset in
            let first = month.firstOfMonth(adding: offset)
            return CalendarMonth(
                id: first,
                weeks: workspace.calendarWeeks(
                    month: first, firstWeekday: state.firstWeekday, weekend: state.weekend, now: now,
                    timeZone: timeZone, due: due))
        }
    }
}
