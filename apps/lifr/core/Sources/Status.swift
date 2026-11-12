import Foundation

/// Why a task counts as Procrastinated.
package enum Procrastination: Hashable, Sendable {
    /// Someone set the status by hand. It stays until the status changes.
    case setByHand
    /// In Progress with no activity for this many whole days, longer than
    /// `Settings.procrastinatedAfterDays`. Derived, never stored, so devices
    /// can't conflict over it.
    case inactive(days: Int)
}

extension LifrTask {
    package func procrastination(now: Date, settings: Settings) -> Procrastination? {
        switch status {
        case .procrastinated:
            return .setByHand
        case .inProgress:
            let idle = now.timeIntervalSince(lastActivityAt)
            let threshold = TimeInterval(settings.procrastinatedAfterDays) * 24 * 60 * 60
            return idle > threshold ? .inactive(days: Int(idle / (24 * 60 * 60))) : nil
        case .todo, .done, .wontDo:
            return nil
        }
    }

    /// The status lists, filters and counts use: the stored one, or
    /// Procrastinated when it's derived.
    package func effectiveStatus(now: Date, settings: Settings) -> TaskStatus {
        procrastination(now: now, settings: settings) == nil ? status : .procrastinated
    }

    /// Open, and the due date (and time, if set) has passed. `today` and
    /// `time` are the device's current day and time.
    package func isOverdue(today: CalendarDate, time: TimeOfDay) -> Bool {
        guard isOpen, let dueDate else { return false }
        if dueDate != today { return dueDate < today }
        return dueTime.map { $0 < time } ?? false
    }

    /// List order: higher priority first, then earlier due date (undated
    /// last), then manual order.
    package static func listOrder(_ lhs: LifrTask, _ rhs: LifrTask) -> Bool {
        if lhs.priority != rhs.priority { return lhs.priority > rhs.priority }
        switch (lhs.dueDate, rhs.dueDate) {
        case (let left?, let right?) where left != right: return left < right
        case (.some, nil): return true
        case (nil, .some): return false
        default: return siblingOrder(lhs, rhs)
        }
    }

    /// Manual order, then ID for ties. Spells out the ID only on a tie,
    /// which sorting thousands of tasks would otherwise do every compare.
    package static func siblingOrder(_ lhs: LifrTask, _ rhs: LifrTask) -> Bool {
        lhs.sortOrder != rhs.sortOrder ? lhs.sortOrder < rhs.sortOrder : lhs.id.uuidString < rhs.id.uuidString
    }
}
