import ComposableArchitecture
import Foundation
import LifrCore
import LifrTaskList
import LifrUI

extension TaskQuery {
    /// Open tasks due today through the next 14 days, one section per day.
    /// Overdue tasks are on Today.
    package static let upcoming = TaskQuery(
        key: "upcoming",
        empty: .init(
            "Nothing coming up", systemImage: "calendar",
            description: String(localized: "Tasks due today and in the next two weeks show up here, by day."))
    ) { workspace, now, timeZone in
        let today = CalendarDate(now, in: timeZone)
        let days = (0...14).map { today.adding(days: $0) }
        let open = workspace.listedTasks.filter { $0.isOpen && $0.dueDate.map { $0 >= today } == true }
        return days.enumerated().map { offset, day in
            let title =
                switch offset {
                case 0: String(localized: "Today")
                case 1: String(localized: "Tomorrow")
                default: day.weekdayTitle
                }
            return TaskSection(
                id: day.description, title: title,
                rows: workspace.rows(open.filter { $0.dueDate == day }, now: now, timeZone: timeZone))
        }
    }
}

/// Upcoming: open tasks due in the next 14 days, by date.
@Reducer
package struct UpcomingFeature {
    @ObservableState
    package struct State: Equatable {
        package var list: TaskListFeature.State

        package init() {
            list = TaskListFeature.State(query: .upcoming)
        }

        package init(workspace: Workspace, now: Date, timeZone: TimeZone) {
            list = TaskListFeature.State(query: .upcoming, workspace: workspace, now: now, timeZone: timeZone)
        }
    }

    package enum Action {
        case list(TaskListFeature.Action)
    }

    package init() {}

    package var body: some ReducerOf<Self> {
        Scope(state: \.list, action: \.list) { TaskListFeature() }
    }
}

extension CalendarDate {
    /// Such as "Thursday, Nov 5" in English, as the person's language
    /// writes it.
    var weekdayTitle: String {
        noonUTC.formatted(
            Date.FormatStyle(locale: .current, calendar: CalendarDate.utcCalendar, timeZone: .gmt)
                .weekday(.wide).month(.abbreviated).day())
    }
}
