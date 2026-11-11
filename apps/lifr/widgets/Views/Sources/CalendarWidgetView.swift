import LifrCore
import LifrGlance
import SwiftUI
import WidgetKit

/// This month in a widget, with today ringed and a dot under each day that
/// has open tasks due, overdue ones included. The large size shows how many
/// instead of a dot.
///
/// Tapping the widget opens today in the calendar; on the large size,
/// tapping a day opens that day.
package struct CalendarWidgetView: View {
    /// The day the widget shows the month of.
    package var today: CalendarDate
    /// How many open tasks are due each day, by YYYY-MM-DD, or nil before
    /// Lifr wrote them.
    package var dueCounts: [String: Int]?
    package var size: TodayWidgetView.Size

    package init(today: CalendarDate, dueCounts: [String: Int]?, size: TodayWidgetView.Size) {
        self.today = today
        self.dueCounts = dueCounts
        self.size = size
    }

    package var body: some View {
        VStack(alignment: .leading, spacing: size == .large ? 10 : 4) {
            Text(today.monthTitle())
                .font(size == .large ? .headline : .caption.weight(.semibold))
                .foregroundStyle(.red)
            if dueCounts == nil {
                Text("Open Lifr to see your due dates here.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                grid
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .widgetURL(DeepLink.calendar(today).url)
    }

    private var grid: some View {
        let columns = Array(repeating: GridItem(.flexible(), spacing: 0), count: 7)
        return LazyVGrid(columns: columns, spacing: size == .large ? 6 : 1) {
            ForEach(Array(weekdays.enumerated()), id: \.offset) { _, symbol in
                Text(symbol)
                    .font(.system(size: size == .large ? 12 : 9, weight: .medium))
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
            }
            ForEach(Array(cells.enumerated()), id: \.offset) { _, day in
                if let day {
                    cell(day)
                } else {
                    Color.clear.frame(height: 1).accessibilityHidden(true)
                }
            }
        }
    }

    @ViewBuilder private func cell(_ day: CalendarDate) -> some View {
        let count = dueCounts?[day.description] ?? 0
        let isToday = day == today
        let label = VStack(spacing: 1) {
            Text(day.day, format: .number)
                .font(.system(size: size == .large ? 15 : 10, weight: isToday ? .bold : .regular))
                .foregroundStyle(isToday ? Color.white : day < today ? Color.secondary : Color.primary)
                .frame(width: size == .large ? 28 : 16, height: size == .large ? 28 : 16)
                .background(Circle().fill(isToday ? Color.red : Color.clear))
            if size == .large {
                Text(count > 0 ? "\(count)" : " ")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.red)
            } else {
                Circle()
                    .fill(count > 0 ? Color.red : Color.clear)
                    .frame(width: 3, height: 3)
            }
        }
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel(day, count: count))
        if size == .large, let url = DeepLink.calendar(day).url {
            Link(destination: url) { label }
        } else {
            label
        }
    }

    private func accessibilityLabel(_ day: CalendarDate, count: Int) -> Text {
        let date = day.readable()
        switch count {
        case 0: return Text(date)
        case 1: return Text("\(date), 1 task due")
        default: return Text("\(date), \(count) tasks due")
        }
    }

    /// The weekday initials, starting on the locale's first weekday.
    private var weekdays: [String] {
        let symbols = Calendar.current.veryShortStandaloneWeekdaySymbols
        let first = Calendar.current.firstWeekday - 1
        return Array(symbols[first...] + symbols[..<first])
    }

    /// This month's days in weeks, with nil before the first to line it up
    /// under its weekday.
    private var cells: [CalendarDate?] {
        let first = today.firstOfMonth
        let lead = (first.weekday - Calendar.current.firstWeekday + 7) % 7
        let count = first.days(until: today.firstOfMonth(adding: 1))
        return Array(repeating: nil, count: lead) + (0..<count).map { first.adding(days: $0) }
    }
}
