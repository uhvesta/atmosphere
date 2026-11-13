import LifrCore
import LifrGlance
import SwiftUI
import WidgetKit

/// The next seven days in a widget: how many tasks are due on each, and
/// their titles where there's room. Only counts while the app lock is on.
///
/// Tapping a day opens it in the calendar, a title opens its task, and a
/// task's circle opens it with the statuses to pick.
package struct UpcomingWidgetView: View {
    /// The days from today on, as `TodayGlance.days(from:)` gives them, or
    /// nil before Lifr wrote them or once they ran out.
    package var days: [TodayGlance]?
    package var size: TodayWidgetView.Size

    package init(days: [TodayGlance]?, size: TodayWidgetView.Size) {
        self.days = days.flatMap { $0.isEmpty ? nil : Array($0.prefix(7)) }
        self.size = size
    }

    package var body: some View {
        content.widgetURL(days?.first.flatMap { link(to: $0) })
    }

    @ViewBuilder private var content: some View {
        if let days {
            VStack(alignment: .leading, spacing: 8) {
                header(days)
                if size == .small {
                    week(days)
                } else if days.allSatisfy({ $0.dueCount == 0 }) {
                    Text("Nothing due this week")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                } else {
                    list(days)
                    if days.contains(where: \.isLocked) {
                        Label("Unlock Lifr to see them", systemImage: "lock.fill")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        } else {
            VStack(alignment: .leading, spacing: 6) {
                title
                Text("Open Lifr to see Upcoming here.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    private var title: some View {
        Label("Upcoming", systemImage: "calendar")
            .font(.headline)
            .foregroundStyle(.red)
    }

    private func header(_ days: [TodayGlance]) -> some View {
        let total = days.map(\.dueCount).reduce(0, +)
        let tasks = Text(count(total))
            .font(.subheadline)
            .foregroundStyle(.secondary)
        // A small widget is too narrow for both on one line.
        return Group {
            if size == .small {
                VStack(alignment: .leading, spacing: 2) {
                    title
                    tasks
                }
            } else {
                HStack(alignment: .firstTextBaseline) {
                    title
                    Spacer(minLength: 4)
                    tasks
                }
            }
        }
        .accessibilityElement(children: .combine)
    }

    /// Each day's initial over how many are due, with today first.
    private func week(_ days: [TodayGlance]) -> some View {
        HStack(spacing: 0) {
            ForEach(Array(days.enumerated()), id: \.offset) { index, day in
                VStack(spacing: 4) {
                    Text(day.from.formatted(.dateTime.weekday(.narrow)))
                        .font(.caption2.weight(.medium))
                        .foregroundStyle(index == 0 ? Color.red : Color.secondary)
                    Text(day.dueCount, format: .number)
                        .font(.system(.callout, design: .rounded).weight(.semibold))
                        .foregroundStyle(day.dueCount == 0 ? Color.secondary : Color.primary)
                }
                .frame(maxWidth: .infinity)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(Text("\(name(of: day, at: index)), \(count(day.dueCount))"))
            }
        }
        .padding(.top, 8)
    }

    /// The days with something due, each with as many titles as fit.
    private func list(_ days: [TodayGlance]) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(lines(days), id: \.index) { line in
                dayRow(line.day, at: line.index, titles: line.titles)
            }
        }
    }

    /// The days with something due that fit, each with the titles that fit
    /// under it: a medium widget has room for four lines and a large one twelve, less
    /// one for the lock's note.
    private func lines(_ days: [TodayGlance]) -> [(index: Int, day: TodayGlance, titles: [TodayGlance.Row])] {
        var room = (size == .large ? 12 : 4) - (days.contains(where: \.isLocked) ? 1 : 0)
        var lines: [(index: Int, day: TodayGlance, titles: [TodayGlance.Row])] = []
        for (index, day) in days.enumerated() where day.dueCount > 0 && room > 0 {
            let titles = size == .large ? Array(day.dueRows.prefix(room - 1)) : []
            lines.append((index, day, titles))
            room -= 1 + titles.count
        }
        return lines
    }

    @ViewBuilder private func dayRow(_ day: TodayGlance, at index: Int, titles: [TodayGlance.Row]) -> some View {
        let name = name(of: day, at: index)
        linked(link(to: day)) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(name)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(index == 0 ? Color.red : Color.primary)
                if size == .medium, let first = day.dueRows.first {
                    Text(first.title)
                        .font(.subheadline)
                        .foregroundStyle(Color.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 4)
                Text(day.dueCount, format: .number)
                    .font(.subheadline)
                    .foregroundStyle(Color.secondary)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(Text("\(name), \(count(day.dueCount))"))
        }
        ForEach(titles) { row in
            HStack(spacing: 6) {
                linked(DeepLink.task(row.id, picksStatus: true).url) {
                    Image(systemName: "circle")
                        .font(.subheadline)
                        .foregroundStyle(.red)
                }
                .accessibilityLabel(Text("Change Status of \(row.title)"))
                linked(DeepLink.task(row.id, picksStatus: false).url) {
                    Text(row.title)
                        .font(.subheadline)
                        .foregroundStyle(Color.primary)
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(.leading, 8)
        }
    }

    /// Today, Tomorrow, or the weekday.
    private func name(of day: TodayGlance, at index: Int) -> String {
        switch index {
        case 0: String(localized: "Today")
        case 1: String(localized: "Tomorrow")
        default: day.from.formatted(.dateTime.weekday(.wide))
        }
    }

    private func count(_ count: Int) -> String {
        String(AttributedString(localized: "^[\(count) task](inflect: true)").characters)
    }

    /// The link that opens `day` in the calendar.
    private func link(to day: TodayGlance) -> URL? {
        DeepLink.calendar(CalendarDate(day.from, in: .current)).url
    }

    /// `label`, opening `url` when tapped.
    @ViewBuilder private func linked(_ url: URL?, @ViewBuilder label: () -> some View) -> some View {
        if let url {
            Link(destination: url, label: label)
        } else {
            label()
        }
    }
}
