import ComposableArchitecture
import Foundation
import LifrCore

/// When this device says each morning what's due that day, or never.
/// Off until picked.
package struct DailySummary: RawRepresentable, Hashable, Sendable {
    package var time: TimeOfDay?

    package init(time: TimeOfDay?) {
        self.time = time
    }

    /// "08:00", or empty when off.
    package init(rawValue: String) {
        time = try? TimeOfDay(rawValue)
    }

    package var rawValue: String { time?.description ?? "" }

    /// The time it starts at when turned on.
    package static let morning = TimeOfDay(clampingHour: 8, minute: 0)

    /// How many days ahead are scheduled. Each launch and background
    /// refresh schedules them again with the records as they are then.
    package static let days = 7
}

extension SharedKey where Self == AppStorageKey<DailySummary>.Default {
    package static var dailySummary: Self {
        Self[.appStorage("lifrDailySummary"), default: DailySummary(time: nil)]
    }
}

extension Workspace {
    /// A summary at `time` on each of the next `DailySummary.days` days,
    /// from today if `time` hasn't passed: how many open tasks are due that
    /// day and how many are overdue by then, outside the trash and archived
    /// Avenues. A day with nothing due has none. With `hidingTitles`, no
    /// task is named.
    func dailySummaries(
        at time: TimeOfDay, now: Date, timeZone: TimeZone, hidingTitles: Bool, locale: Locale
    ) -> [ScheduledReminder] {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        let today = CalendarDate(now, in: timeZone)
        let open = tasks.values.filter { task in
            task.deletedAt == nil && task.isOpen && task.dueDate != nil
                && avenues[task.avenueID]?.archivedAt == nil
        }
        // Those due any time first, then by time, as Today lists them.
        .sorted { (minutes($0), $0.title) < (minutes($1), $1.title) }
        return (0...DailySummary.days).compactMap { offset -> ScheduledReminder? in
            let day = today.adding(days: offset)
            let components = DateComponents(
                year: day.year, month: day.month, day: day.day, hour: time.hour, minute: time.minute)
            guard let fireAt = calendar.date(from: components), fireAt > now else { return nil }
            let due = open.filter { $0.dueDate == day }
            let overdue = open.count { task in task.dueDate.map { $0 < day } ?? false }
            guard !due.isEmpty || overdue > 0 else { return nil }
            let title =
                if due.isEmpty {
                    String(AttributedString(localized: "^[\(overdue) task](inflect: true) overdue").characters)
                } else if overdue == 0 {
                    String(AttributedString(localized: "^[\(due.count) task](inflect: true) due today").characters)
                } else {
                    String(
                        AttributedString(
                            localized: "^[\(due.count) task](inflect: true) due today, \(overdue) overdue"
                        ).characters)
                }
            let names = due.prefix(3).map(\.title).formatted(.list(type: .and).locale(locale))
            let body =
                if hidingTitles || due.isEmpty {
                    String(localized: "Open Lifr to plan your day.")
                } else if due.count > 3 {
                    String(localized: "\(names) and \(due.count - 3) more")
                } else {
                    names
                }
            return ScheduledReminder(
                id: Self.summaryID(day), taskID: Self.summaryID(day), title: title, body: body, fireAt: fireAt,
                hidesTitle: hidingTitles, isSummary: true)
        }
        .prefix(DailySummary.days)
        .map { $0 }
    }

    /// Minutes into the day of its due time; -1 for any time.
    private func minutes(_ task: LifrTask) -> Int {
        task.dueTime.map { $0.hour * 60 + $0.minute } ?? -1
    }

    /// The same for a day each time it's scheduled, so scheduling again
    /// replaces it.
    private static func summaryID(_ day: CalendarDate) -> UUID {
        let digits = String(day.year * 10_000 + day.month * 100 + day.day)
        let padded = String(repeating: "0", count: max(0, 12 - digits.count)) + digits
        return UUID(uuidString: "00000000-0000-4000-8000-" + padded) ?? UUID()
    }
}
