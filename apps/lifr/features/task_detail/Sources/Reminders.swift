import Foundation
import LifrCore

/// One of a task's reminders, as the detail shows it.
package struct ReminderRow: Identifiable, Hashable, Sendable {
    package var id: UUID
    /// How long before the due date, for one that follows it.
    package var beforeDue: String?
    /// When it rings. `nil` if the task lost its due date.
    package var fireAt: Date?

    package init(_ reminder: Reminder, fireAt: Date?) {
        id = reminder.id
        switch reminder.trigger {
        case .at: beforeDue = nil
        case .beforeDue(let seconds): beforeDue = ReminderChoice.describe(beforeDue: seconds)
        }
        self.fireAt = fireAt
    }
}

/// The reminders the detail offers.
package enum ReminderChoice: CaseIterable, Hashable, Sendable {
    case inOneHour
    case tomorrowMorning
    case hourBeforeDue
    case dayBeforeDue

    package var title: String {
        switch self {
        case .inOneHour: String(localized: "In 1 hour")
        case .tomorrowMorning: String(localized: "Tomorrow at 9:00")
        case .hourBeforeDue: "1 hour before due"
        case .dayBeforeDue: "1 day before due"
        }
    }

    package func trigger(now: Date, timeZone: TimeZone) -> Reminder.Trigger {
        switch self {
        case .inOneHour:
            return .at(now + 60 * 60)
        case .tomorrowMorning:
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = timeZone
            // The calendar only fails on dates far outside any real one;
            // a day and nine hours from midnight is close enough then.
            let midnight = calendar.startOfDay(for: now)
            let tomorrow = calendar.date(byAdding: .day, value: 1, to: midnight) ?? midnight + 24 * 60 * 60
            return .at(calendar.date(bySettingHour: 9, minute: 0, second: 0, of: tomorrow) ?? tomorrow + 9 * 60 * 60)
        case .hourBeforeDue:
            return .beforeDue(seconds: 60 * 60)
        case .dayBeforeDue:
            return .beforeDue(seconds: 24 * 60 * 60)
        }
    }

    static func describe(beforeDue seconds: Int) -> String {
        let formatter = DateComponentsFormatter()
        formatter.unitsStyle = .full
        formatter.maximumUnitCount = 1
        return "\(formatter.string(from: TimeInterval(seconds)) ?? "\(seconds) seconds") before due"
    }
}
