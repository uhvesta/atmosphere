import ComposableArchitecture
import Foundation
import LifrCore

/// The reminder Quick Add gives a new task with a due date. This device
/// only; off unless the person picks one.
package enum DefaultReminder: String, CaseIterable, Hashable, Sendable {
    case never
    /// At the due time, or 9:00 on the due day when it has no time.
    case atDue
    /// 9:00 on the due day, or at the due time when that's earlier.
    case dueDayMorning

    package var title: String {
        switch self {
        case .never: String(localized: "Never")
        case .atDue: String(localized: "At the due time")
        case .dueDayMorning: String(localized: "At 9:00 on the due day")
        }
    }

    /// The reminder for a task due at `dueTime`, if any.
    package func trigger(dueTime: TimeOfDay?) -> Reminder.Trigger? {
        switch self {
        case .never: return nil
        case .atDue: return .beforeDue(seconds: 0)
        case .dueDayMorning:
            guard let dueTime else { return .beforeDue(seconds: 0) }
            let minutes = dueTime.hour * 60 + dueTime.minute - 9 * 60
            return .beforeDue(seconds: max(0, minutes) * 60)
        }
    }
}

extension SharedKey where Self == AppStorageKey<DefaultReminder>.Default {
    package static var defaultReminder: Self {
        Self[.appStorage("lifrDefaultReminder"), default: .never]
    }
}
