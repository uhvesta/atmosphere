import ComposableArchitecture
import LifrCore
import LifrTaskList
import LifrUI
import SwiftUI

/// The month or the week over the selected day's tasks. `add` opens Quick
/// Add for the selected day.
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
                VStack(spacing: 8) {
                    CalendarHeader(store: store)
                    switch store.mode {
                    case .month: MonthGrid(store: store, add: add)
                    case .week: WeekGrid(store: store, add: add)
                    }
                }
                .padding(.vertical, 4)
            }
            if store.showsDragTip {
                GestureTipCard(
                    symbol: "hand.point.up.left", title: "Drag a task onto a day",
                    message: "Drag a task from the list below and drop it on a day to make it due then."
                ) {
                    store.send(.dragTipClosed, animation: .default)
                }
            }
        }
        .emptyAction("Add a Task", perform: add)
        .emptyBelowHeader()
        .navigationTitle("Calendar")
        .inspector(
            isPresented: Binding(
                get: { store.showsTray },
                set: { if !$0 { store.send(.trayDismissed) } })
        ) {
            UnscheduledTray(store: store)
                .presentationDetents([.medium, .large])
                .presentationBackgroundInteraction(.enabled(upThrough: .medium))
                .inspectorColumnWidth(min: 200, ideal: 240, max: 320)
        }
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

/// Pages on a sideways swipe, sliding the days across, or fading them
/// with Reduce Motion.
private struct SwipePaging: ViewModifier {
    var page: CalendarDate?
    var store: StoreOf<CalendarFeature>
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var forward = true

    func body(content: Content) -> some View {
        content
            .id(page)
            .transition(
                reduceMotion
                    ? .opacity
                    : .asymmetric(
                        insertion: .move(edge: forward ? .trailing : .leading).combined(with: .opacity),
                        removal: .opacity)
            )
            #if os(iOS)
                .simultaneousGesture(
                    DragGesture(minimumDistance: 30).onEnded { drag in
                        let width = drag.translation.width
                        guard abs(width) > 60, abs(width) > abs(drag.translation.height) * 2 else { return }
                        forward = width < 0
                        store.send(.stepped(forward ? 1 : -1), animation: .easeInOut(duration: 0.25))
                    })
            #endif
            .onChange(of: page) { old, new in
                if let old, let new { forward = old < new }
            }
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
            Button {
                store.send(.monthPicked(month.id))
            } label: {
                Text(month.id.monthTitle(locale))
                    .font(.title2.bold())
                    .foregroundStyle(month.weeks.joined().contains(where: \.isToday) ? Color.red : .primary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.horizontal)
            .padding(.bottom, 8)
            .accessibilityAddTraits(.isHeader)
            .accessibilityHint("Shows this month")
            ForEach(Array(month.weeks.enumerated()), id: \.offset) { _, week in
                Divider()
                HStack(spacing: 0) {
                    ForEach(week) { day in
                        if day.isInMonth {
                            // Nothing can be dragged onto this page, so its
                            // days take only taps.
                            DayCell(day: day, isSelected: day.date == selected, acceptsDrops: false, store: store)
                        } else {
                            Color.clear.frame(maxWidth: .infinity, minHeight: 46)
                        }
                    }
                }
                .dynamicTypeSize(...DynamicTypeSize.accessibility1)
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
                    .dynamicTypeSize(...DynamicTypeSize.accessibility1)
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

/// The month's weeks of days: dots on the iPhone, chips on the Mac and
/// iPad.
private struct MonthGrid: View {
    var store: StoreOf<CalendarFeature>
    var add: () -> Void
    @Environment(\.calendar) private var calendar
    @WideCalendar private var isWide

    var body: some View {
        VStack(spacing: 8) {
            WeekdayHeader(calendar: calendar)
            VStack(spacing: 0) {
                ForEach(Array(store.weeks.enumerated()), id: \.offset) { _, week in
                    HStack(spacing: 0) {
                        ForEach(week) { day in
                            if isWide {
                                ChipDayCell(day: day, isSelected: day.date == store.selected, store: store, add: add)
                            } else {
                                DayCell(day: day, isSelected: day.date == store.selected, store: store, add: add)
                            }
                        }
                    }
                    // Seven days to a row leave no room past the first
                    // accessibility size.
                    .dynamicTypeSize(...DynamicTypeSize.accessibility1)
                }
            }
            .modifier(SwipePaging(page: store.month, store: store))
        }
        .task(id: calendar.firstWeekday) { store.send(.firstWeekdayChanged(calendar.firstWeekday)) }
    }
}

/// The selected day's week: a strip of days on the iPhone, over the day's
/// tasks, and seven columns of chips by time on the Mac and iPad.
private struct WeekGrid: View {
    var store: StoreOf<CalendarFeature>
    var add: () -> Void
    @Environment(\.calendar) private var calendar
    @WideCalendar private var isWide

    var body: some View {
        VStack(spacing: 8) {
            WeekdayHeader(calendar: calendar)
            Group {
                if isWide {
                    WeekColumns(week: store.week, store: store, add: add)
                } else {
                    HStack(spacing: 0) {
                        ForEach(store.week) { day in
                            DayCell(day: day, isSelected: day.date == store.selected, store: store, add: add)
                        }
                    }
                    .dynamicTypeSize(...DynamicTypeSize.accessibility1)
                }
            }
            .modifier(SwipePaging(page: store.week.first?.date, store: store))
        }
        .task(id: calendar.firstWeekday) { store.send(.firstWeekdayChanged(calendar.firstWeekday)) }
    }
}

/// A day: its number, a dot for each task due, and somewhere to drop a task.
/// Tapping the selected day when nothing is due adds a task to it.
private struct DayCell: View {
    var day: CalendarDay
    var isSelected: Bool
    var acceptsDrops = true
    var store: StoreOf<CalendarFeature>
    var add: (() -> Void)?
    @State private var isTargeted = false
    /// The circle grows with the number in it.
    @ScaledMetric(relativeTo: .callout) private var circle = 30.0

    var body: some View {
        Button {
            if isSelected, day.chips.isEmpty, let add {
                add()
            } else {
                store.send(.daySelected(day.date))
            }
        } label: {
            VStack(spacing: 3) {
                Text("\(day.date.day)")
                    .font(.callout.weight(day.isToday ? .bold : .regular))
                    .monospacedDigit()
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
                    .foregroundStyle(numberStyle)
                    .frame(width: circle, height: circle)
                    .background {
                        if isSelected {
                            Circle().fill(day.isToday ? Color.red : Color.accentColor)
                        } else if day.isToday {
                            Circle().strokeBorder(Color.red, lineWidth: 1.5)
                        }
                    }
                SpanLanes(day: day)
                dots
            }
            .frame(maxWidth: .infinity, minHeight: 46)
            .contentShape(Rectangle())
            .background {
                RoundedRectangle(cornerRadius: 8)
                    .fill(background)
            }
            .opacity(day.isInMonth ? 1 : 0.4)
        }
        .buttonStyle(.plain)
        .modifier(DayDrop(day: day.date, isOn: acceptsDrops, isTargeted: $isTargeted, store: store))
        // Holding a day shows it big, past the size the grid stops at.
        .accessibilityShowsLargeContentViewer {
            Text("\(day.date.day)")
        }
        .accessibilityLabel(accessibilityLabel)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var background: Color {
        if isTargeted { return Color.accentColor.opacity(0.2) }
        return day.isWeekend ? Color.secondary.opacity(0.06) : .clear
    }

    private var numberStyle: Color {
        if isSelected { return .white }
        return day.hasOverdue ? .red : .primary
    }

    /// Up to three dots in the Avenues' colours, then how many more, and a
    /// flag when a Project is due.
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
            if !day.starts.isEmpty {
                Circle().strokeBorder(Color(avenue: day.starts[0].color), lineWidth: 1).frame(width: 5, height: 5)
            }
            if !day.flags.isEmpty {
                Image(systemName: "flag.fill")
                    .font(.system(size: 7))
                    .foregroundStyle(Color(avenue: day.flags[0].color))
            }
        }
        .frame(height: 6)
    }

    private var accessibilityLabel: Text {
        let date = Text(day.date.readable())
        let tasks = day.colors.isEmpty ? Text("nothing due") : Text("\(day.colors.count) due")
        let summary = day.hasOverdue ? Text("\(date), \(tasks), overdue") : Text("\(date), \(tasks)")
        return day.flags.isEmpty ? summary : Text("\(summary), \(day.flags.count) Projects due")
    }
}

/// A day of the Mac's and iPad's month: its number, its tasks as chips up to three,
/// then how many more, which shows the day. Double-clicking it adds a
/// task to it.
private struct ChipDayCell: View {
    var day: CalendarDay
    var isSelected: Bool
    var store: StoreOf<CalendarFeature>
    var add: () -> Void
    @State private var isTargeted = false
    @Environment(\.locale) private var locale

    private static let shown = 3

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Spacer()
                Text("\(day.date.day)")
                    .font(.callout.weight(day.isToday ? .bold : .regular))
                    .monospacedDigit()
                    .foregroundStyle(numberStyle)
                    .frame(minWidth: 24, minHeight: 22)
                    .background {
                        if isSelected {
                            Capsule().fill(day.isToday ? Color.red : Color.accentColor)
                        } else if day.isToday {
                            Capsule().strokeBorder(Color.red, lineWidth: 1.5)
                        }
                    }
            }
            SpanLanes(day: day)
                .padding(.horizontal, -4)
            ForEach(day.flags) { flag in
                CalendarFlagView(flag: flag, store: store)
            }
            ForEach(day.chips.prefix(Self.shown)) { chip in
                CalendarChipView(chip: chip, store: store)
            }
            ForEach(day.starts) { chip in
                CalendarStartView(chip: chip, store: store)
            }
            if day.chips.count > Self.shown {
                Button("\(day.chips.count - Self.shown) more") { store.send(.daySelected(day.date)) }
                    .buttonStyle(.plain)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .padding(4)
        .frame(maxWidth: .infinity, minHeight: 96, alignment: .top)
        .background(background)
        .overlay(alignment: .top) { Rectangle().fill(.separator).frame(height: 1) }
        .opacity(day.isInMonth ? 1 : 0.5)
        .contentShape(Rectangle())
        .onTapGesture(count: 2) {
            store.send(.daySelected(day.date))
            add()
        }
        .onTapGesture { store.send(.daySelected(day.date)) }
        .modifier(DayDrop(day: day.date, isTargeted: $isTargeted, store: store))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text(day.date.readable(locale)))
        .accessibilityAddTraits(isSelected ? .isSelected : [])
        .accessibilityAction(named: "Show Day") { store.send(.daySelected(day.date)) }
        .accessibilityAction(named: "Add a Task") {
            store.send(.daySelected(day.date))
            add()
        }
    }

    private var background: Color {
        if isTargeted { return Color.accentColor.opacity(0.2) }
        if isSelected { return Color.accentColor.opacity(0.08) }
        return day.isWeekend ? Color.secondary.opacity(0.06) : .clear
    }

    private var numberStyle: Color {
        if isSelected { return .white }
        return day.hasOverdue ? .red : .primary
    }
}

/// The Mac's and iPad's week: a column a day, its tasks due any time on top, then
/// those due at a time against the hours that have any. Dropping on the
/// top takes the time away; dropping on an hour sets it.
private struct WeekColumns: View {
    var week: [CalendarDay]
    var store: StoreOf<CalendarFeature>
    var add: () -> Void
    @Environment(\.locale) private var locale
    /// One chip's height, which every row makes room for.
    @ScaledMetric(relativeTo: .caption) private var chipHeight = 19.0

    var body: some View {
        let layout = WeekLayout(week, chipHeight: chipHeight)
        HStack(alignment: .top, spacing: 0) {
            VStack(alignment: .trailing, spacing: 0) {
                Color.clear.frame(height: 28 + layout.allDayHeight)
                ForEach(layout.hours, id: \.hour) { row in
                    Text(TimeOfDay(clampingHour: row.hour, minute: 0).formatted(locale))
                        .font(.caption2.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(height: row.height, alignment: .top)
                }
            }
            .frame(width: 52)
            .accessibilityHidden(true)
            ForEach(week) { day in
                WeekColumn(
                    day: day, layout: layout, isSelected: day.date == store.selected, store: store, add: add)
            }
        }
    }
}

/// How tall the week's rows are, the same in every column so the hours
/// line up: tall enough for the day with the most in each.
private struct WeekLayout {
    var allDayHeight: CGFloat
    /// Only the hours that have a task due in them.
    var hours: [HourRow]

    struct HourRow: Hashable {
        var hour: Int
        var height: CGFloat
    }

    init(_ week: [CalendarDay], chipHeight: CGFloat) {
        let untimed =
            week.map { day in
                day.spans.count + (day.moreSpans > 0 ? 1 : 0) + day.flags.count + day.starts.count
                    + day.chips.count { $0.dueTime == nil }
            }
            .max() ?? 0
        allDayHeight = max(chipHeight * 3, CGFloat(untimed) * chipHeight + 6)
        var counts: [Int: Int] = [:]
        for day in week {
            var inDay: [Int: Int] = [:]
            for chip in day.chips { if let hour = chip.dueTime?.hour { inDay[hour, default: 0] += 1 } }
            counts.merge(inDay, uniquingKeysWith: max)
        }
        hours = counts.keys.sorted().map { hour in
            HourRow(hour: hour, height: max(chipHeight * 2, CGFloat(counts[hour] ?? 1) * chipHeight + 4))
        }
    }
}

private struct WeekColumn: View {
    var day: CalendarDay
    var layout: WeekLayout
    var isSelected: Bool
    var store: StoreOf<CalendarFeature>
    var add: () -> Void
    @State private var targetsAllDay = false
    @State private var targetedHour: Int?
    @Environment(\.locale) private var locale

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                store.send(.daySelected(day.date))
            } label: {
                Text("\(day.date.day)")
                    .font(.callout.weight(day.isToday ? .bold : .regular))
                    .monospacedDigit()
                    .foregroundStyle(isSelected ? .white : day.isToday ? .red : .primary)
                    .frame(minWidth: 24, minHeight: 22)
                    .background {
                        if isSelected {
                            Capsule().fill(day.isToday ? Color.red : Color.accentColor)
                        }
                    }
                    .frame(maxWidth: .infinity, minHeight: 28)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text(day.date.readable(locale)))
            .accessibilityAddTraits(isSelected ? .isSelected : [])
            VStack(alignment: .leading, spacing: 2) {
                SpanLanes(day: day)
                    .padding(.horizontal, -3)
                ForEach(day.flags) { flag in
                    CalendarFlagView(flag: flag, store: store)
                }
                ForEach(day.chips.filter { $0.dueTime == nil }) { chip in
                    CalendarChipView(chip: chip, store: store)
                }
                ForEach(day.starts) { chip in
                    CalendarStartView(chip: chip, store: store)
                }
                Spacer(minLength: 0)
            }
            .padding(3)
            .frame(maxWidth: .infinity, alignment: .top)
            .frame(height: layout.allDayHeight, alignment: .top)
            .background(targetsAllDay ? Color.accentColor.opacity(0.2) : .clear)
            .contentShape(Rectangle())
            .onTapGesture(count: 2) {
                store.send(.daySelected(day.date))
                add()
            }
            .modifier(DayDrop(day: day.date, time: .some(nil), isTargeted: $targetsAllDay, store: store))
            ForEach(layout.hours, id: \.hour) { row in
                HourSlot(
                    day: day.date, hour: row.hour, chips: day.chips.filter { $0.dueTime?.hour == row.hour },
                    height: row.height, store: store)
            }
        }
        .background(day.isWeekend ? Color.secondary.opacity(0.06) : .clear)
        .overlay(alignment: .leading) { Rectangle().fill(.separator).frame(width: 1) }
    }
}

/// An hour of a day in the week, with the tasks due in it; dropping a
/// task on it sets the time to the quarter hour it lands on.
private struct HourSlot: View {
    var day: CalendarDate
    var hour: Int
    var chips: [CalendarChip]
    var height: CGFloat
    var store: StoreOf<CalendarFeature>
    @State private var isTargeted = false

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            ForEach(chips) { chip in
                CalendarChipView(chip: chip, showsTime: true, store: store)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 3)
        .frame(maxWidth: .infinity, alignment: .top)
        .frame(height: height, alignment: .top)
        .background(isTargeted ? Color.accentColor.opacity(0.2) : .clear)
        .overlay(alignment: .top) { Rectangle().fill(.separator.opacity(0.5)).frame(height: 1) }
        .modifier(
            DayDrop(
                day: day, time: .some(TimeOfDay(clampingHour: hour, minute: 0)), hourHeight: height,
                isTargeted: $isTargeted, store: store))
    }
}
