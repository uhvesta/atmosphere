import Foundation
import LifrCore
import SwiftUI

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
    case atDue
    case hourBeforeDue
    case dayBeforeDue

    package var title: String {
        switch self {
        case .inOneHour: String(localized: "In 1 hour")
        case .tomorrowMorning: String(localized: "Tomorrow at 9:00")
        case .atDue: String(localized: "At time of due")
        case .hourBeforeDue: String(localized: "1 hour before due")
        case .dayBeforeDue: String(localized: "1 day before due")
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
        case .atDue:
            return .beforeDue(seconds: 0)
        case .hourBeforeDue:
            return .beforeDue(seconds: 60 * 60)
        case .dayBeforeDue:
            return .beforeDue(seconds: 24 * 60 * 60)
        }
    }

    static func describe(beforeDue seconds: Int) -> String {
        guard seconds > 0 else { return String(localized: "At time of due") }
        let formatter = DateComponentsFormatter()
        formatter.unitsStyle = .full
        formatter.maximumUnitCount = 1
        let amount = formatter.string(from: TimeInterval(seconds)) ?? "\(seconds) seconds"
        return String(localized: "\(amount) before due")
    }
}

/// A reminder being set up in the Custom sheet: at a date and time, or
/// some minutes, hours or days before the due date.
package struct ReminderDraft: Equatable, Sendable {
    package enum Kind: String, Hashable, Sendable {
        case at, beforeDue
    }

    package enum Unit: Int, Hashable, Sendable, CaseIterable {
        case minutes = 60
        case hours = 3600
        case days = 86400
    }

    package var kind: Kind
    package var date: Date
    package var amount = 30
    package var unit = Unit.minutes
    /// Before the due date is offered only when the task has one.
    package let hasDueDate: Bool

    /// Starts before the due date when there is one, else at the next
    /// whole hour after `now`.
    package init(now: Date, hasDueDate: Bool) {
        self.hasDueDate = hasDueDate
        kind = hasDueDate ? .beforeDue : .at
        // The next whole hour.
        let hours = (now.timeIntervalSinceReferenceDate / 3600).rounded(.down)
        date = Date(timeIntervalSinceReferenceDate: hours * 3600 + 3600)
    }

    package var trigger: Reminder.Trigger {
        switch kind {
        case .at: .at(date)
        case .beforeDue: .beforeDue(seconds: amount * unit.rawValue)
        }
    }

    /// A set time must be in the future; before the due date needs one.
    package func canSave(now: Date) -> Bool {
        switch kind {
        case .at: date > now
        case .beforeDue: hasDueDate && amount >= 0
        }
    }
}

/// The sheet for a custom reminder.
struct ReminderEditor: View {
    @Binding var draft: ReminderDraft
    let save: () -> Void
    let cancel: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                if draft.hasDueDate {
                    Picker("Remind", selection: $draft.kind) {
                        Text("Before due").tag(ReminderDraft.Kind.beforeDue)
                        Text("At a time").tag(ReminderDraft.Kind.at)
                    }
                    .pickerStyle(.segmented)
                }
                switch draft.kind {
                case .at:
                    DatePicker("At", selection: $draft.date, in: Date()...)
                case .beforeDue:
                    Section {
                        Stepper(value: $draft.amount, in: 0...999) {
                            Text("\(draft.amount)")
                        }
                        Picker("Unit", selection: $draft.unit) {
                            Text("Minutes").tag(ReminderDraft.Unit.minutes)
                            Text("Hours").tag(ReminderDraft.Unit.hours)
                            Text("Days").tag(ReminderDraft.Unit.days)
                        }
                    } footer: {
                        Text("Before the due time, or 9:00 on the due day when it has no time.")
                    }
                }
            }
            .navigationTitle("Custom reminder")
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel", action: cancel) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save", action: save).disabled(!draft.canSave(now: Date()))
                }
            }
        }
    }
}
