import Foundation
import LifrCore
import SwiftUI

/// A repeat the person can pick in one tap, read from the task's due day.
package struct RepeatPreset: Hashable, Sendable {
    package var rule: Recurrence.Rule
    package var interval: Int

    package init(rule: Recurrence.Rule, interval: Int) {
        self.rule = rule
        self.interval = interval
    }

    /// Such as "Every week on Wed", in the words `Recurrence` uses.
    package var title: String {
        Recurrence(rule: rule, interval: interval, seriesID: UUID()).description
    }

    /// The usual repeats for a task due `day`: daily, on weekdays, weekly
    /// and every two weeks on its weekday, monthly on its date and on its
    /// weekday ("the second Wed"), and yearly.
    package static func presets(for day: CalendarDate) -> [RepeatPreset] {
        [
            RepeatPreset(rule: .daily, interval: 1),
            RepeatPreset(rule: .weekly(weekdays: [2, 3, 4, 5, 6]), interval: 1),
            RepeatPreset(rule: .weekly(weekdays: [day.weekday]), interval: 1),
            RepeatPreset(rule: .weekly(weekdays: [day.weekday]), interval: 2),
            RepeatPreset(rule: .monthly(day: day.day), interval: 1),
            RepeatPreset(
                rule: .monthlyWeekday(ordinal: RepeatDraft.ordinal(of: day), weekday: day.weekday), interval: 1),
            RepeatPreset(rule: .yearly(month: day.month, day: day.day), interval: 1),
        ]
    }
}

/// A custom repeat being set up: every N days, weeks, months or years on
/// a schedule, or N of them after the task is closed.
package struct RepeatDraft: Equatable, Sendable {
    package enum Kind: String, Hashable, Sendable, CaseIterable {
        case schedule, afterClosing
    }

    package var kind: Kind = .schedule
    package var unit: Recurrence.Unit = .week
    package var interval = 1
    /// For weekly: 1 for Sunday through 7 for Saturday.
    package var weekdays: Set<Int>
    /// For monthly: on the anchor's weekday ("the second Wed") instead of
    /// its date.
    package var onWeekday = false
    /// The day the schedule is read from: the due day, or today.
    package var anchor: CalendarDate

    package init(anchor: CalendarDate, from recurrence: Recurrence?) {
        self.anchor = anchor
        weekdays = [anchor.weekday]
        guard let recurrence else { return }
        interval = recurrence.interval
        switch recurrence.rule {
        case .daily: unit = .day
        case .weekly(let days):
            unit = .week
            weekdays = days
        case .monthly: unit = .month
        case .monthlyWeekday:
            unit = .month
            onWeekday = true
        case .yearly: unit = .year
        case .afterClosing(let closing):
            kind = .afterClosing
            unit = closing
        }
    }

    package var rule: Recurrence.Rule {
        switch (kind, unit) {
        case (.afterClosing, let unit): .afterClosing(unit)
        case (.schedule, .day): .daily
        case (.schedule, .week): .weekly(weekdays: weekdays)
        case (.schedule, .month):
            onWeekday
                ? .monthlyWeekday(ordinal: Self.ordinal(of: anchor), weekday: anchor.weekday)
                : .monthly(day: anchor.day)
        case (.schedule, .year): .yearly(month: anchor.month, day: anchor.day)
        }
    }

    /// Saving needs at least one weekday for a weekly schedule.
    package var canSave: Bool {
        interval >= 1 && !(kind == .schedule && unit == .week && weekdays.isEmpty)
    }

    /// Which of its weekday in the month `day` is: 1 to 4, or -1 for the
    /// last, so the 29th to 31st repeat on the month's last one.
    package static func ordinal(of day: CalendarDate) -> Int {
        let ordinal = (day.day - 1) / 7 + 1
        return ordinal > 4 ? -1 : ordinal
    }
}

/// The sheet for a custom repeat.
struct RepeatEditor: View {
    @Binding var draft: RepeatDraft
    let save: () -> Void
    let cancel: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                Picker("Repeat", selection: $draft.kind) {
                    Text("On a schedule").tag(RepeatDraft.Kind.schedule)
                    Text("After closing").tag(RepeatDraft.Kind.afterClosing)
                }
                .pickerStyle(.segmented)
                Section {
                    Stepper(value: $draft.interval, in: 1...99) {
                        Text("Every \(draft.interval)")
                    }
                    Picker("Unit", selection: $draft.unit) {
                        Text(draft.interval == 1 ? "Day" : "Days").tag(Recurrence.Unit.day)
                        Text(draft.interval == 1 ? "Week" : "Weeks").tag(Recurrence.Unit.week)
                        Text(draft.interval == 1 ? "Month" : "Months").tag(Recurrence.Unit.month)
                        Text(draft.interval == 1 ? "Year" : "Years").tag(Recurrence.Unit.year)
                    }
                }
                if draft.kind == .schedule, draft.unit == .week {
                    Section("On") {
                        ForEach(Self.mondayFirst, id: \.self) { weekday in
                            Toggle(Self.weekdayName(weekday), isOn: weekdayBinding(weekday))
                        }
                    }
                }
                if draft.kind == .schedule, draft.unit == .month {
                    Section {
                        Picker("On", selection: $draft.onWeekday) {
                            Text("Day \(draft.anchor.day)").tag(false)
                            Text(weekdayTitle).tag(true)
                        }
                        .pickerStyle(.inline)
                        .labelsHidden()
                    }
                }
                Section {
                    Text(Recurrence(rule: draft.rule, interval: draft.interval, seriesID: UUID()).description)
                        .foregroundStyle(.secondary)
                }
            }
            .navigationTitle("Custom repeat")
            #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel", action: cancel) }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save", action: save).disabled(!draft.canSave)
                }
            }
        }
    }

    /// Such as "The second Wednesday".
    private var weekdayTitle: String {
        let ordinal = RepeatDraft.ordinal(of: draft.anchor)
        let which = [-1: "last", 1: "first", 2: "second", 3: "third", 4: "fourth"][ordinal] ?? ""
        return "The \(which) \(Self.weekdayName(draft.anchor.weekday))"
    }

    private func weekdayBinding(_ weekday: Int) -> Binding<Bool> {
        Binding(
            get: { draft.weekdays.contains(weekday) },
            set: { on in
                if on { draft.weekdays.insert(weekday) } else { draft.weekdays.remove(weekday) }
            })
    }

    private static let mondayFirst = [2, 3, 4, 5, 6, 7, 1]

    private static func weekdayName(_ weekday: Int) -> String {
        let symbols = CalendarDate.utcCalendar.weekdaySymbols
        return symbols.indices.contains(weekday - 1) ? symbols[weekday - 1] : ""
    }
}
