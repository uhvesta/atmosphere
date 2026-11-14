import Foundation
import LifrCore
import LifrTaskList

/// A task on a day of the calendar.
package struct CalendarChip: Identifiable, Hashable, Sendable {
    package var id: UUID
    package var title: String
    /// Its Avenue's colour.
    package var color: String
    /// When in the day it's due, or nil for any time that day.
    package var dueTime: TimeOfDay?
    package var isOverdue: Bool
    package var status: TaskStatus

    /// Done or Won't Do, shown only with Show completed.
    package var isClosed: Bool { status.isClosed }
}

/// A Project due on a day of the calendar.
package struct CalendarFlag: Identifiable, Hashable, Sendable {
    package var id: UUID
    package var name: String
    /// Its Avenue's colour.
    package var color: String
}

/// A day's piece of a task's bar from its start date to its due date.
package struct CalendarSpan: Identifiable, Hashable, Sendable {
    package var id: UUID
    package var title: String
    /// Its Avenue's colour.
    package var color: String
    /// Whether the bar starts on this day, or at the start of its week when
    /// it started before: where its title shows.
    package var isFirst: Bool
    /// Whether the task starts on this day.
    package var startsHere: Bool
    /// Whether the task is due on this day.
    package var endsHere: Bool
}

/// A day in the month grid or the week.
package struct CalendarDay: Identifiable, Hashable, Sendable {
    package var date: CalendarDate
    /// Whether the day is in the month shown, rather than one filling its
    /// first or last week.
    package var isInMonth: Bool
    package var isToday: Bool
    /// A day of the weekend where the person is, shaded slightly.
    package var isWeekend: Bool
    /// The tasks due that day: those due any time first, in list order,
    /// then those due at a time, by the time.
    package var chips: [CalendarChip]
    /// The Avenue colours of the open tasks due that day.
    package var colors: [String]
    /// Whether any of them is overdue.
    package var hasOverdue: Bool
    /// The active Projects due that day.
    package var flags: [CalendarFlag]
    /// With Show start dates, the open tasks with only a start date that
    /// start that day.
    package var starts: [CalendarChip] = []
    /// With Show start dates, the day's piece of each bar by lane, the same
    /// lanes across its week so bars line up; nil where a lane is empty.
    package var spans: [CalendarSpan?] = []
    /// How many more bars cross the day than the lanes show.
    package var moreSpans = 0

    package var id: CalendarDate { date }

    /// How many lanes of bars a week shows before counting the rest.
    package static let spanLanes = 3

    /// `weekend` holds the weekend's weekdays, 1 for Sunday through 7 for
    /// Saturday.
    package init(
        date: CalendarDate, isInMonth: Bool, isToday: Bool, weekend: Set<Int> = Calendar.saturdayAndSunday,
        chips: [CalendarChip] = [], flags: [CalendarFlag] = [], starts: [CalendarChip] = []
    ) {
        self.date = date
        self.isInMonth = isInMonth
        self.isToday = isToday
        isWeekend = weekend.contains(date.weekday)
        self.chips = chips
        self.flags = flags
        self.starts = starts
        colors = chips.lazy.filter { !$0.isClosed }.map(\.color)
        hasOverdue = chips.contains(where: \.isOverdue)
    }
}

/// A month on the long page of months: its first day, and its weeks.
package struct CalendarMonth: Identifiable, Hashable, Sendable {
    package var id: CalendarDate
    package var weeks: [[CalendarDay]]
}

/// Which tasks the calendar shows.
package struct CalendarFilter: Hashable, Sendable {
    /// The Avenues shown; empty shows them all.
    package var avenues: Set<UUID> = []
    /// Whether Done and Won't Do tasks show, on the day they were due.
    package var showsCompleted = false
    /// Whether tasks with start dates show as bars to their due dates, and
    /// those with only a start date on the day they start.
    package var showsStartDates = false

    package init(avenues: Set<UUID> = [], showsCompleted: Bool = false, showsStartDates: Bool = false) {
        self.avenues = avenues
        self.showsCompleted = showsCompleted
        self.showsStartDates = showsStartDates
    }

    func shows(_ task: LifrTask) -> Bool {
        (showsCompleted || task.isOpen) && (avenues.isEmpty || avenues.contains(task.avenueID))
    }
}

/// What the calendar shows on each day, worked out once for many months.
package struct CalendarDue: Sendable {
    var tasks: [CalendarDate: [LifrTask]] = [:]
    var projects: [CalendarDate: [Project]] = [:]
    /// Tasks that start before they're due, by start date, with Show start
    /// dates.
    var spans: [LifrTask] = []
    /// Open tasks with a start date and no due date, by start date.
    var starts: [CalendarDate: [LifrTask]] = [:]
}

extension Workspace {
    /// The weeks that show `month`'s days, each starting on `firstWeekday`
    /// (1 for Sunday through 7 for Saturday), with the days of the months
    /// either side that fill the first and last week.
    /// `weekend` holds the weekdays shaded as the weekend.
    package func calendarWeeks(
        month: CalendarDate, firstWeekday: Int, weekend: Set<Int> = Calendar.saturdayAndSunday,
        filter: CalendarFilter = CalendarFilter(), now: Date, timeZone: TimeZone
    ) -> [[CalendarDay]] {
        calendarWeeks(
            month: month, firstWeekday: firstWeekday, weekend: weekend, now: now, timeZone: timeZone,
            due: calendarDue(filter))
    }

    /// The week, starting on `firstWeekday`, that has `day` in it.
    package func calendarWeek(
        of day: CalendarDate, firstWeekday: Int, weekend: Set<Int> = Calendar.saturdayAndSunday,
        filter: CalendarFilter = CalendarFilter(), now: Date, timeZone: TimeZone
    ) -> [CalendarDay] {
        let start = day.adding(days: -((day.weekday - firstWeekday + 7) % 7))
        return week(
            from: start, month: day.firstOfMonth, weekend: weekend, now: now, timeZone: timeZone,
            due: calendarDue(filter))
    }

    /// The tasks `filter` shows due on each day, in list order, and the
    /// active Projects due on each day in the Avenues it shows.
    package func calendarDue(_ filter: CalendarFilter) -> CalendarDue {
        var due = CalendarDue()
        for task in listedTasks where filter.shows(task) {
            if let date = task.dueDate { due.tasks[date, default: []].append(task) }
            guard filter.showsStartDates, let start = task.startDate else { continue }
            if let date = task.dueDate {
                // One that starts after it's due, from before the rule, shows
                // on its due day only.
                if start < date { due.spans.append(task) }
            } else if task.isOpen {
                due.starts[start, default: []].append(task)
            }
        }
        // Earliest first, and the longer of two starting together, so long
        // bars take the top lanes.
        due.spans.sort { lhs, rhs in
            guard let left = lhs.startDate, let right = rhs.startDate else { return false }
            if left != right { return left < right }
            return (lhs.dueDate ?? left) > (rhs.dueDate ?? right)
        }
        for project in projects.values where project.status == .active {
            guard let date = project.dueDate, avenues[project.avenueID]?.archivedAt == nil,
                filter.avenues.isEmpty || filter.avenues.contains(project.avenueID)
            else { continue }
            due.projects[date, default: []].append(project)
        }
        for date in due.projects.keys { due.projects[date]?.sort { ($0.sortOrder, $0.name) < ($1.sortOrder, $1.name) } }
        return due
    }

    package func calendarWeeks(
        month: CalendarDate, firstWeekday: Int, weekend: Set<Int> = Calendar.saturdayAndSunday, now: Date,
        timeZone: TimeZone, due: CalendarDue
    ) -> [[CalendarDay]] {
        let first = month.firstOfMonth
        let next = first.firstOfMonth(adding: 1)
        let start = first.adding(days: -((first.weekday - firstWeekday + 7) % 7))
        let weeks = (start.days(until: next) + 6) / 7
        return (0..<weeks).map { week in
            self.week(
                from: start.adding(days: week * 7), month: first, weekend: weekend, now: now, timeZone: timeZone,
                due: due)
        }
    }

    /// Seven days from `start`, with the bars that cross them in lanes.
    private func week(
        from start: CalendarDate, month: CalendarDate, weekend: Set<Int>, now: Date, timeZone: TimeZone,
        due: CalendarDue
    ) -> [CalendarDay] {
        var days = (0..<7).map { offset in
            calendarDay(
                start.adding(days: offset), month: month, weekend: weekend, now: now, timeZone: timeZone, due: due)
        }
        guard !due.spans.isEmpty, let end = days.last?.date else { return days }
        // Each lane holds the last column its bar reaches.
        var lanes: [Int] = []
        var placed: [(task: LifrTask, lane: Int, from: Int, to: Int)] = []
        for task in due.spans {
            guard let first = task.startDate, let last = task.dueDate, first <= end, last >= start else { continue }
            let from = max(0, start.days(until: first))
            let to = min(6, start.days(until: last))
            if let lane = lanes.firstIndex(where: { $0 < from }) {
                lanes[lane] = to
                placed.append((task, lane, from, to))
            } else {
                lanes.append(to)
                placed.append((task, lanes.count - 1, from, to))
            }
        }
        guard !placed.isEmpty else { return days }
        let shown = min(lanes.count, CalendarDay.spanLanes)
        for column in days.indices {
            days[column].spans = Array(repeating: nil, count: shown)
        }
        for (task, lane, from, to) in placed {
            for column in from...to {
                guard lane < shown else {
                    days[column].moreSpans += 1
                    continue
                }
                days[column].spans[lane] = CalendarSpan(
                    id: task.id, title: task.title, color: avenues[task.avenueID]?.color ?? "",
                    isFirst: column == from, startsHere: task.startDate == days[column].date,
                    endsHere: task.dueDate == days[column].date)
            }
        }
        return days
    }

    private func calendarDay(
        _ date: CalendarDate, month: CalendarDate, weekend: Set<Int>, now: Date, timeZone: TimeZone,
        due: CalendarDue
    ) -> CalendarDay {
        let today = CalendarDate(now, in: timeZone)
        let time = TimeOfDay(now, in: timeZone)
        let tasks = due.tasks[date] ?? []
        // Stable: the untimed keep list order, ahead of the timed.
        let ordered = tasks.enumerated().sorted { lhs, rhs in
            switch (lhs.element.dueTime, rhs.element.dueTime) {
            case (nil, nil): lhs.offset < rhs.offset
            case (nil, _): true
            case (_, nil): false
            case (let left?, let right?): left == right ? lhs.offset < rhs.offset : left < right
            }
        }
        return CalendarDay(
            date: date, isInMonth: date.firstOfMonth == month, isToday: date == today, weekend: weekend,
            chips: ordered.map { _, task in
                CalendarChip(
                    id: task.id, title: task.title, color: avenues[task.avenueID]?.color ?? "", dueTime: task.dueTime,
                    isOverdue: task.isOpen && task.isOverdue(today: today, time: time), status: task.status)
            },
            flags: (due.projects[date] ?? []).map { project in
                CalendarFlag(id: project.id, name: project.name, color: avenues[project.avenueID]?.color ?? "")
            },
            starts: (due.starts[date] ?? []).map { task in
                CalendarChip(
                    id: task.id, title: task.title, color: avenues[task.avenueID]?.color ?? "", dueTime: nil,
                    isOverdue: false, status: task.status)
            })
    }
}

extension Workspace {
    /// The open tasks with no due date that `filter` shows, in list order,
    /// to drag onto a day. Sub tasks of closed parents are left out.
    package func unscheduled(_ filter: CalendarFilter) -> [CalendarChip] {
        listedTasks.compactMap { task in
            guard task.isOpen, task.dueDate == nil, filter.shows(task) else { return nil }
            if let parent = task.parentID, tasks[parent]?.isOpen != true { return nil }
            return CalendarChip(
                id: task.id, title: task.title, color: avenues[task.avenueID]?.color ?? "", dueTime: nil,
                isOverdue: false, status: task.status)
        }
    }
}

extension TaskQuery {
    /// Tasks due on `day` that `filter` shows, for the calendar's selected
    /// day.
    package static func calendarDay(_ day: CalendarDate?, filter: CalendarFilter = CalendarFilter()) -> TaskQuery {
        let avenues = filter.avenues.map(\.uuidString).sorted().joined(separator: ",")
        return TaskQuery(
            key: "calendar-\(day?.description ?? "none")-\(filter.showsCompleted)-\(avenues)",
            empty: .init(
                "Nothing due", systemImage: "calendar",
                description: String(
                    localized: "Tasks due on this day show up here. To move one, drag it onto another day.")),
            adds: NewTaskDefaults(dueDate: day)
        ) { workspace, now, timeZone in
            guard let day else { return [] }
            let tasks = workspace.listedTasks.filter { filter.shows($0) && $0.dueDate == day }
            return [
                TaskSection(
                    id: day.description, title: day.readable(),
                    rows: workspace.rows(tasks, now: now, timeZone: timeZone))
            ]
        }
    }
}

extension Calendar {
    /// Saturday and Sunday, the weekend where no other is known.
    package static let saturdayAndSunday: Set<Int> = [1, 7]

    /// The weekdays of this calendar's weekend, 1 for Sunday through 7 for
    /// Saturday, as its locale has them: Friday and Saturday in some places.
    package var weekendDays: Set<Int> {
        var days: Set<Int> = []
        // Seven days in a row, at noon so no time zone moves one.
        guard let start = date(from: DateComponents(year: 2026, month: 11, day: 1, hour: 12)) else {
            return Self.saturdayAndSunday
        }
        for offset in 0..<7 {
            guard let day = date(byAdding: .day, value: offset, to: start) else { continue }
            if isDateInWeekend(day) { days.insert(component(.weekday, from: day)) }
        }
        return days
    }
}

/// What a drag onto a day carries, as text: task IDs, one to a line, so
/// several selected rows can drag together, or a bar's leading edge, to
/// move its start alone.
package enum CalendarPayload: Equatable, Sendable {
    case tasks([UUID])
    case start(UUID)

    private static let startPrefix = "start:"

    /// The text a bar's leading edge drags as.
    package static func dragging(startOf id: UUID) -> String { startPrefix + id.uuidString }

    /// The text dragging `ids` together carries.
    package static func dragging(_ ids: [UUID]) -> String { ids.map(\.uuidString).joined(separator: "\n") }

    /// What `items` dropped together carry, or nil when they hold no task.
    package init?(_ items: [String]) {
        if let edge = items.first(where: { $0.hasPrefix(Self.startPrefix) }) {
            guard let id = UUID(uuidString: String(edge.dropFirst(Self.startPrefix.count))) else { return nil }
            self = .start(id)
            return
        }
        var seen: Set<UUID> = []
        let ids =
            items
            .flatMap { $0.split(whereSeparator: \.isNewline) }
            .compactMap { UUID(uuidString: $0.trimmingCharacters(in: .whitespaces)) }
            .filter { seen.insert($0).inserted }
        guard !ids.isEmpty else { return nil }
        self = .tasks(ids)
    }
}
