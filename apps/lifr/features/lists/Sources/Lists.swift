import Foundation
import LifrCore
import LifrTaskList
import LifrUI

extension LifrTask {
    /// Whether the task waits for a start date after `today`, so it's in
    /// Later rather than Anytime.
    package func startsAfter(_ today: CalendarDate) -> Bool {
        startDate.map { $0 > today } ?? false
    }
}

extension TaskQuery {
    /// Open tasks with no due date that can be started now, by Avenue.
    package static let anytime = TaskQuery(
        key: "anytime",
        empty: .init(
            "Nothing to do anytime", systemImage: "square.stack",
            description: String(
                localized: "Tasks with no due date show up here, once they can be started.")),
        trailingActions: [.reschedule, .status(.wontDo), .pickStatus], groups: true
    ) { workspace, now, timeZone in
        let today = CalendarDate(now, in: timeZone)
        let tasks = workspace.listedTasks.filter {
            $0.isOpen && !$0.isSubtask && $0.dueDate == nil && !$0.startsAfter(today)
        }
        return workspace.avenues.values
            .filter { $0.archivedAt == nil }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
            .map { avenue in
                TaskSection(
                    id: avenue.id.uuidString, title: avenue.name,
                    rows: workspace.rows(tasks.filter { $0.avenueID == avenue.id }, now: now, timeZone: timeZone))
            }
    }

    /// Open tasks that can't be started until a later day, by that day.
    package static let later = TaskQuery(
        key: "later",
        empty: .init(
            "Nothing for later", systemImage: "clock",
            description: String(
                localized: "Give a task a start date after today to put it away until then.")),
        adds: nil, groups: true
    ) { workspace, now, timeZone in
        let today = CalendarDate(now, in: timeZone)
        let tasks = workspace.listedTasks.filter { $0.isOpen && !$0.isSubtask && $0.startsAfter(today) }
        let days = Set(tasks.compactMap(\.startDate)).sorted()
        return days.map { day in
            TaskSection(
                id: day.description, title: String(localized: "Starts \(day.readable())"),
                rows: workspace.rows(tasks.filter { $0.startDate == day }, now: now, timeZone: timeZone))
        }
    }

    /// Closed tasks, the latest closed first, by when: Today, Yesterday,
    /// the rest of the week, then by month.
    package static let logbook = TaskQuery(
        key: "logbook",
        empty: .init(
            "Nothing closed yet", systemImage: "book.closed",
            description: String(localized: "Tasks you mark Done or Won’t Do show up here.")),
        trailingActions: [.pickStatus],
        adds: nil, searches: true, groups: true
    ) { workspace, now, timeZone in
        let today = CalendarDate(now, in: timeZone)
        let closed = workspace.listedTasks
            .filter { !$0.isOpen && !$0.isSubtask }
            .sorted { ($0.closedAt ?? .distantPast) > ($1.closedAt ?? .distantPast) }
        var sections: [TaskSection] = []
        for task in closed {
            let (id, title) = LogbookPeriod.of(task.closedAt.map { CalendarDate($0, in: timeZone) }, today: today)
            let row = TaskRowData(task, in: workspace, now: now, timeZone: timeZone)
            if sections.last?.id == id {
                sections[sections.count - 1].rows.append(row)
            } else {
                sections.append(TaskSection(id: id, title: title, rows: [row]))
            }
        }
        return sections
    }
}

/// Which Logbook section a task closed on a day goes in.
enum LogbookPeriod {
    static func of(_ day: CalendarDate?, today: CalendarDate) -> (id: String, title: String) {
        guard let day else { return ("earlier", String(localized: "Earlier")) }
        switch day.days(until: today) {
        case ...0: return ("today", String(localized: "Today"))
        case 1: return ("yesterday", String(localized: "Yesterday"))
        case 2..<7: return ("week", String(localized: "This Week"))
        default:
            let month = day.firstOfMonth
            return (month.description, month.monthTitle())
        }
    }
}
