import Foundation
import LifrCore

/// What a task row shows, worked out from the records at one moment.
package struct TaskRowData: Identifiable, Hashable, Sendable {
    /// When the task is due, relative to the moment the row was made.
    package enum Due: Hashable, Sendable {
        case overdue(CalendarDate, TimeOfDay?)
        case today(TimeOfDay?)
        case later(CalendarDate, TimeOfDay?)
    }

    /// Closed and total Sub tasks, for tasks that have any.
    package struct Progress: Hashable, Sendable {
        package var closed: Int
        package var total: Int
    }

    package var id: UUID
    /// The code shown, such as HEALTH-7K2Q, or 7K2Q where the Avenue is
    /// plain from the screen.
    package var code: String
    /// The whole code, which VoiceOver reads wherever the row is.
    package let fullCode: String
    package var title: String
    /// The effective status, so derived Procrastinated shows.
    package var status: TaskStatus
    package var priority: Priority
    package var due: Due?
    package var avenue: String
    package var subtasks: Progress?
    /// Whether closing it brings the next one.
    package var repeats = false
    /// The day it starts, while that's still ahead.
    package var starts: CalendarDate?

    package init(_ task: LifrTask, in workspace: Workspace, now: Date, timeZone: TimeZone) {
        let today = CalendarDate(now, in: timeZone)
        id = task.id
        code = workspace.displayCode(of: task)
        fullCode = code
        title = task.title
        status = task.effectiveStatus(now: now, settings: workspace.settings)
        priority = task.priority
        avenue = workspace.avenues[task.avenueID]?.name ?? ""
        repeats = task.recurrence != nil
        if let start = task.startDate, start > today { starts = start }
        if let dueDate = task.dueDate {
            if task.isOverdue(today: today, time: TimeOfDay(now, in: timeZone)) {
                due = .overdue(dueDate, task.dueTime)
            } else if dueDate == today {
                due = .today(task.dueTime)
            } else {
                due = .later(dueDate, task.dueTime)
            }
        }
        let subtasks = workspace.subtasks(of: task.id)
        if !subtasks.isEmpty {
            self.subtasks = Progress(closed: subtasks.count { !$0.isOpen }, total: subtasks.count)
        }
    }
}
