import ComposableArchitecture
import Foundation
import LifrCore
import LifrTaskList
import LifrUI

extension TaskQuery {
    /// Open tasks due from tomorrow through the next 14 days, one section per
    /// day. Today and overdue tasks are on Today.
    package static let upcoming = TaskQuery(
        key: "upcoming",
        empty: .init(
            "Nothing coming up", systemImage: "calendar",
            description: "Tasks due in the next two weeks show up here, by day.")
    ) { workspace, now, timeZone in
        let today = CalendarDate(now, in: timeZone)
        let days = (1...14).map { CalendarDate(now + Double($0) * 24 * 60 * 60, in: timeZone) }
        let open = workspace.listedTasks.filter { $0.isOpen && $0.dueDate.map { $0 > today } == true }
        return days.enumerated().map { offset, day in
            TaskSection(
                id: day.description, title: offset == 0 ? "Tomorrow" : day.weekdayTitle,
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
    /// Such as "Thursday, Nov 5". English only for now.
    var weekdayTitle: String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .gmt
        let date = calendar.date(from: DateComponents(year: year, month: month, day: day))!
        return date.formatted(
            Date.FormatStyle(locale: Locale(identifier: "en_US"), calendar: calendar, timeZone: .gmt)
                .weekday(.wide).month(.abbreviated).day())
    }
}
