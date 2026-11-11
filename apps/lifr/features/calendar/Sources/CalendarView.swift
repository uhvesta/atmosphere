import ComposableArchitecture
import LifrCore
import LifrTaskList
import LifrUI
import SwiftUI

/// The month over the selected day's tasks. `add` opens Quick Add for the
/// selected day.
package struct CalendarView: View {
    @Bindable package var store: StoreOf<CalendarFeature>
    private let add: () -> Void

    package init(store: StoreOf<CalendarFeature>, add: @escaping () -> Void) {
        self.store = store
        self.add = add
    }

    package var body: some View {
        TaskListView(store: store.scope(state: \.list, action: \.list)) {
            Section {
                MonthGrid(store: store)
            }
        }
        .emptyAction("Add a Task", perform: add)
        .emptyBelowHeader()
        .navigationTitle("Calendar")
        .toolbar {
            ToolbarItem(placement: .secondaryAction) {
                Button("Today") { store.send(.todayTapped) }
                    .keyboardShortcut("t", modifiers: .command)
            }
            #if os(iOS)
                ToolbarItem(placement: .topBarLeading) {
                    Button("All Months", systemImage: "calendar") { store.send(.monthsTapped) }
                }
            #endif
        }
        #if os(iOS)
            .navigationDestination(
                isPresented: Binding(
                    get: { store.months != nil },
                    set: { if !$0 { store.send(.monthsDismissed) } })
            ) {
                MonthsPage(store: store)
            }
        #endif
    }
}

/// Every month on one page, scrolled up and down, opened at the month
/// shown. Picking a day goes back to it with its tasks.
private struct MonthsPage: View {
    var store: StoreOf<CalendarFeature>
    @Environment(\.calendar) private var calendar
    /// The month at the top, which starts as the one open.
    @State private var top: CalendarDate?

    init(store: StoreOf<CalendarFeature>) {
        self.store = store
        _top = State(initialValue: store.month)
    }

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 28) {
                ForEach(store.months ?? []) { month in
                    MonthBlock(month: month, selected: store.selected, store: store)
                }
            }
            .scrollTargetLayout()
            .padding(.vertical, 12)
        }
        .scrollPosition(id: $top, anchor: .top)
        // Outside the scroll, so a month scrolled to starts below it.
        .safeAreaInset(edge: .top, spacing: 0) {
            VStack(spacing: 0) {
                WeekdayHeader(calendar: calendar).padding(.vertical, 6)
                Divider()
            }
            .background(.bar)
        }
        .onChange(of: store.month) { _, month in
            withAnimation { top = month }
        }
        .navigationTitle("Months")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Today") { store.send(.todayTapped) }
            }
        }
    }
}

/// A month on the long page: its name, then its weeks with a line over
/// each. Days of the months either side are left out.
private struct MonthBlock: View {
    var month: CalendarMonth
    var selected: CalendarDate?
    var store: StoreOf<CalendarFeature>
    @Environment(\.locale) private var locale

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(month.id.monthTitle(locale))
                .font(.title2.bold())
                .padding(.horizontal)
                .padding(.bottom, 8)
                .accessibilityAddTraits(.isHeader)
            ForEach(Array(month.weeks.enumerated()), id: \.offset) { _, week in
                Divider()
                HStack(spacing: 0) {
                    ForEach(week) { day in
                        if day.isInMonth {
                            DayCell(day: day, isSelected: day.date == selected, store: store)
                        } else {
                            Color.clear.frame(maxWidth: .infinity, minHeight: 46)
                        }
                    }
                }
                .padding(.vertical, 6)
            }
        }
    }
}

/// The short weekday names, from the person's first weekday.
private struct WeekdayHeader: View {
    var calendar: Calendar

    var body: some View {
        HStack(spacing: 0) {
            ForEach(Array(weekdaySymbols.enumerated()), id: \.offset) { _, symbol in
                Text(symbol)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity)
            }
        }
        .accessibilityHidden(true)
    }

    private var weekdaySymbols: [String] {
        let symbols = calendar.veryShortStandaloneWeekdaySymbols
        let first = calendar.firstWeekday - 1
        return Array(symbols[first...] + symbols[..<first])
    }
}

/// The month's title, its arrows, and its weeks of days.
private struct MonthGrid: View {
    var store: StoreOf<CalendarFeature>
    @Environment(\.calendar) private var calendar
    @Environment(\.locale) private var locale

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                #if os(iOS)
                    Button {
                        store.send(.monthsTapped)
                    } label: {
                        Text(store.month?.monthTitle(locale) ?? "")
                            .font(.headline)
                            .foregroundStyle(.primary)
                    }
                    .buttonStyle(.plain)
                    .accessibilityHint("Shows every month to scroll through")
                    .accessibilityAddTraits(.isHeader)
                #else
                    Text(store.month?.monthTitle(locale) ?? "")
                        .font(.headline)
                        .accessibilityAddTraits(.isHeader)
                #endif
                Spacer()
                Button {
                    store.send(.monthStepped(-1))
                } label: {
                    Label("Previous Month", systemImage: "chevron.left")
                }
                .keyboardShortcut(.leftArrow, modifiers: .command)
                Button {
                    store.send(.monthStepped(1))
                } label: {
                    Label("Next Month", systemImage: "chevron.right")
                }
                .keyboardShortcut(.rightArrow, modifiers: .command)
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            WeekdayHeader(calendar: calendar)
            ForEach(Array(store.weeks.enumerated()), id: \.offset) { _, week in
                HStack(spacing: 0) {
                    ForEach(week) { day in
                        DayCell(day: day, isSelected: day.date == store.selected, store: store)
                    }
                }
            }
        }
        .padding(.vertical, 4)
        .task(id: calendar.firstWeekday) { store.send(.firstWeekdayChanged(calendar.firstWeekday)) }
    }
}

/// A day: its number, a dot for each task due, and somewhere to drop a task.
private struct DayCell: View {
    var day: CalendarDay
    var isSelected: Bool
    var store: StoreOf<CalendarFeature>
    @State private var isTargeted = false

    var body: some View {
        Button {
            store.send(.daySelected(day.date))
        } label: {
            VStack(spacing: 3) {
                Text("\(day.date.day)")
                    .font(.callout.weight(day.isToday ? .bold : .regular))
                    .monospacedDigit()
                    .foregroundStyle(numberStyle)
                    .frame(width: 30, height: 30)
                    .background {
                        if isSelected {
                            Circle().fill(day.isToday ? Color.red : Color.accentColor)
                        } else if day.isToday {
                            Circle().strokeBorder(Color.red, lineWidth: 1.5)
                        }
                    }
                dots
            }
            .frame(maxWidth: .infinity, minHeight: 46)
            .contentShape(Rectangle())
            .background {
                RoundedRectangle(cornerRadius: 8)
                    .fill(isTargeted ? Color.accentColor.opacity(0.2) : Color.clear)
            }
            .opacity(day.isInMonth ? 1 : 0.4)
        }
        .buttonStyle(.plain)
        .dropDestination(for: String.self) { ids, _ in
            let dropped = ids.compactMap(UUID.init(uuidString:))
            for id in dropped { store.send(.dropped(id, on: day.date)) }
            return !dropped.isEmpty
        } isTargeted: {
            isTargeted = $0
        }
        .accessibilityLabel(accessibilityLabel)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var numberStyle: Color {
        if isSelected { return .white }
        return day.hasOverdue ? .red : .primary
    }

    /// Up to three dots in the Avenues' colours, then how many more.
    @ViewBuilder private var dots: some View {
        HStack(spacing: 2) {
            ForEach(Array(day.colors.prefix(3).enumerated()), id: \.offset) { _, color in
                Circle().fill(Color(avenue: color)).frame(width: 5, height: 5)
            }
            if day.colors.count > 3 {
                Text("+\(day.colors.count - 3)")
                    .font(.system(size: 8))
                    .foregroundStyle(.secondary)
            }
        }
        .frame(height: 6)
    }

    private var accessibilityLabel: Text {
        let date = Text(day.date.readable())
        let tasks = day.colors.isEmpty ? Text("nothing due") : Text("\(day.colors.count) due")
        return day.hasOverdue ? Text("\(date), \(tasks), overdue") : Text("\(date), \(tasks)")
    }
}
