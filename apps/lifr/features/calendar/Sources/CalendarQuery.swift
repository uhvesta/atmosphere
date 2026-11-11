import Foundation
import LifrCore
import LifrTaskList

/// A day in the month grid.
package struct CalendarDay: Identifiable, Hashable, Sendable {
    package var date: CalendarDate
    /// Whether the day is in the month shown, rather than one filling its
    /// first or last week.
    package var isInMonth: Bool
    package var isToday: Bool
    /// The Avenue colours of the open tasks due that day, in list order.
    package var colors: [String]
    /// Whether any of them is overdue.
    package var hasOverdue: Bool

    package var id: CalendarDate { date }
}

extension Workspace {
    /// The weeks that show `month`'s days, each starting on `firstWeekday`
    /// (1 for Sunday through 7 for Saturday), with the days of the months
    /// either side that fill the first and last week.
    package func calendarWeeks(
        month: CalendarDate, firstWeekday: Int, now: Date, timeZone: TimeZone
    ) -> [[CalendarDay]] {
        let first = month.firstOfMonth
        let next = first.firstOfMonth(adding: 1)
        let start = first.adding(days: -((first.weekday - firstWeekday + 7) % 7))
        let weeks = (start.days(until: next) + 6) / 7
        let end = start.adding(days: weeks * 7)
        let today = CalendarDate(now, in: timeZone)
        let time = TimeOfDay(now, in: timeZone)
        var due: [CalendarDate: [LifrTask]] = [:]
        for task in listedTasks where task.isOpen {
            guard let date = task.dueDate, date >= start, date < end else { continue }
            due[date, default: []].append(task)
        }
        return (0..<weeks).map { week in
            (0..<7).map { offset in
                let date = start.adding(days: week * 7 + offset)
                let tasks = due[date] ?? []
                return CalendarDay(
                    date: date, isInMonth: date.firstOfMonth == first, isToday: date == today,
                    colors: tasks.map { avenues[$0.avenueID]?.color ?? "" },
                    hasOverdue: tasks.contains { $0.isOverdue(today: today, time: time) })
            }
        }
    }
}

extension TaskQuery {
    /// Open tasks due on `day`, for the calendar's selected day.
    package static func calendarDay(_ day: CalendarDate?) -> TaskQuery {
        TaskQuery(
            key: "calendar-\(day?.description ?? "none")",
            empty: .init(
                "Nothing due", systemImage: "calendar",
                description: String(
                    localized: "Tasks due on this day show up here. To move one, drag it onto another day.")),
            adds: NewTaskDefaults(dueDate: day)
        ) { workspace, now, timeZone in
            guard let day else { return [] }
            let tasks = workspace.listedTasks.filter { $0.isOpen && $0.dueDate == day }
            return [
                TaskSection(
                    id: day.description, title: day.readable(),
                    rows: workspace.rows(tasks, now: now, timeZone: timeZone))
            ]
        }
    }
}
