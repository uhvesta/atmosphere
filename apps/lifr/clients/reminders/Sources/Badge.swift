import ComposableArchitecture
import Foundation
import LifrCore

/// What the number on the app icon counts. This device only.
package enum BadgeCount: String, CaseIterable, Hashable, Sendable {
    /// Overdue and due today.
    case today
    case overdue
    case off

    package var title: String {
        switch self {
        case .today: String(localized: "Due today and overdue")
        case .overdue: String(localized: "Overdue only")
        case .off: String(localized: "Off")
        }
    }
}

extension SharedKey where Self == AppStorageKey<BadgeCount>.Default {
    package static var badgeCount: Self {
        Self[.appStorage("lifrBadgeCount"), default: .today]
    }
}

/// The buttons a reminder's notification offers, reached with a long press
/// so each is a choice.
package enum ReminderAction: String, CaseIterable, Sendable {
    case done = "lifrDone"
    case snooze = "lifrSnooze"
    /// Opens Lifr with the task's statuses to pick from.
    case changeStatus = "lifrChangeStatus"

    /// The notification category with every action, and the one with only
    /// Snooze for a notification that hides its title.
    package static let category = "lifrReminder"
    package static let hiddenCategory = "lifrReminderHidden"

    /// How long Snooze puts a reminder off.
    package static let snoozeInterval: TimeInterval = 60 * 60
}

extension Workspace {
    /// How many open tasks, outside the trash and archived Avenues, `kind`
    /// counts at `now`.
    package func badgeCount(_ kind: BadgeCount, now: Date, timeZone: TimeZone) -> Int {
        guard kind != .off else { return 0 }
        let today = CalendarDate(now, in: timeZone)
        let time = TimeOfDay(now, in: timeZone)
        return tasks.values.count { task in
            guard task.deletedAt == nil, task.isOpen, avenues[task.avenueID]?.archivedAt == nil,
                let due = task.dueDate
            else { return false }
            return task.isOverdue(today: today, time: time) || (kind == .today && due == today)
        }
    }
}
