import ComposableArchitecture
import LifrCore
import LifrTaskList
import LifrUI
import SwiftUI

/// The calendar's top row: the month's title, Month or Week, the filter,
/// and the arrows that page.
struct CalendarHeader: View {
    var store: StoreOf<CalendarFeature>
    @Environment(\.locale) private var locale
    #if os(macOS)
        @State private var picksMonth = false
    #endif

    var body: some View {
        HStack(spacing: 8) {
            title
            Spacer(minLength: 4)
            Picker(
                "Show",
                selection: Binding(get: { store.mode }, set: { store.send(.modePicked($0), animation: .default) })
            ) {
                Text("Month").tag(CalendarMode.month)
                Text("Week").tag(CalendarMode.week)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .fixedSize()
            CalendarFilterMenu(store: store)
            PageArrow(steps: -1, store: store)
            PageArrow(steps: 1, store: store)
        }
        .labelStyle(.iconOnly)
        .buttonStyle(.borderless)
    }

    @ViewBuilder private var title: some View {
        let text = Text(store.month?.monthTitle(locale) ?? "")
            .font(.headline)
            .lineLimit(1)
            .minimumScaleFactor(0.5)
        #if os(iOS)
            Button {
                store.send(.monthsTapped)
            } label: {
                text.foregroundStyle(.primary)
            }
            .buttonStyle(.plain)
            .accessibilityHint("Shows every month to scroll through")
            .accessibilityAddTraits(.isHeader)
        #else
            Button {
                picksMonth = true
            } label: {
                text.foregroundStyle(.primary)
            }
            .buttonStyle(.plain)
            .accessibilityHint("Picks a day to go to")
            .accessibilityAddTraits(.isHeader)
            .popover(isPresented: $picksMonth) {
                DatePicker(
                    "Go to",
                    selection: Binding(
                        get: { (store.selected ?? CalendarDate(Date(), in: .gmt)).noonUTC },
                        set: {
                            picksMonth = false
                            store.send(.daySelected(CalendarDate($0, in: .gmt)))
                        }),
                    displayedComponents: .date
                )
                .datePickerStyle(.graphical)
                .labelsHidden()
                // Days are kept without a time zone, so the picker reads
                // them in UTC, where noon is still the same day.
                .environment(\.timeZone, .gmt)
                .padding()
            }
        #endif
    }
}

/// Previous or next month or week. Holding a dragged task over it for a
/// moment pages too, to drop the task further away.
private struct PageArrow: View {
    var steps: Int
    var store: StoreOf<CalendarFeature>
    @State private var isTargeted = false

    var body: some View {
        Button {
            store.send(.stepped(steps))
        } label: {
            Label(title, systemImage: steps < 0 ? "chevron.left" : "chevron.right")
                .padding(4)
                .background(isTargeted ? Color.accentColor.opacity(0.2) : .clear, in: Circle())
        }
        .keyboardShortcut(steps < 0 ? .leftArrow : .rightArrow, modifiers: .command)
        .dropDestination(for: String.self) { _, _ in
            false
        } isTargeted: {
            isTargeted = $0
        }
        .task(id: isTargeted) {
            // Pages every second while a drag stays over the arrow.
            while isTargeted {
                do { try await Task.sleep(for: .milliseconds(700)) } catch { return }
                guard isTargeted else { return }
                store.send(.stepped(steps), animation: .default)
            }
        }
    }

    private var title: LocalizedStringKey {
        switch (store.mode, steps < 0) {
        case (.month, true): "Previous Month"
        case (.month, false): "Next Month"
        case (.week, true): "Previous Week"
        case (.week, false): "Next Week"
        }
    }
}

/// Which Avenues show, and whether closed tasks do.
private struct CalendarFilterMenu: View {
    var store: StoreOf<CalendarFeature>

    var body: some View {
        Menu {
            Toggle(
                "Show Completed",
                isOn: Binding(get: { store.showsCompleted }, set: { _ in store.send(.showsCompletedToggled) }))
            Toggle(
                "Show Start Dates",
                isOn: Binding(get: { store.showsStartDates }, set: { _ in store.send(.showsStartDatesToggled) }))
            Toggle(
                "Show Unscheduled",
                isOn: Binding(get: { store.showsTray }, set: { _ in store.send(.trayToggled) }))
            Section("Avenues") {
                Toggle(
                    "All Avenues",
                    isOn: Binding(get: { store.avenues.isEmpty }, set: { _ in store.send(.allAvenuesShown) }))
                ForEach(store.avenueChoices) { avenue in
                    Toggle(
                        avenue.name,
                        isOn: Binding(
                            get: { store.avenues.contains(avenue.id) },
                            set: { _ in store.send(.avenueToggled(avenue.id)) }))
                }
            }
        } label: {
            Label(
                "Filter",
                systemImage: isFiltered
                    ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
        }
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityValue(isFiltered ? Text("Filtered") : Text("All tasks"))
    }

    private var isFiltered: Bool { !store.avenues.isEmpty || store.showsCompleted || store.showsStartDates }
}

/// A task on a day, on the Mac: its status, its time in the week, and its
/// title, in its Avenue's colour. It opens the task, drags onto another day,
/// and moves a day or a week with ⌥ and the arrows.
struct CalendarChipView: View {
    var chip: CalendarChip
    var showsTime = false
    var store: StoreOf<CalendarFeature>
    @Environment(\.locale) private var locale
    @Environment(\.colorSchemeContrast) private var contrast

    var body: some View {
        Button {
            store.send(.list(.rowTapped(chip.id)))
        } label: {
            HStack(spacing: 3) {
                StatusGlyph(chip.status, font: .caption2)
                if showsTime, let time = chip.dueTime {
                    Text(time.formatted(locale))
                        .font(.caption2.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                Text(chip.title)
                    .font(.caption)
                    .strikethrough(chip.isClosed)
                    .foregroundStyle(chip.isOverdue ? Color.red : .primary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 4)
            .padding(.vertical, 1)
            .background(
                Color(avenue: chip.color).opacity(contrast == .increased ? 0.32 : 0.18),
                in: RoundedRectangle(cornerRadius: 4)
            )
            .opacity(chip.isClosed ? 0.55 : 1)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .draggable(chip.id.uuidString) {
            Text(chip.title).font(.caption).padding(4)
        }
        .onKeyPress(keys: [.leftArrow, .rightArrow, .upArrow, .downArrow]) { press in
            guard press.modifiers.contains(.option) else { return .ignored }
            let days =
                switch press.key {
                case .leftArrow: -1
                case .rightArrow: 1
                case .upArrow: -7
                default: 7
                }
            store.send(.moved(chip.id, days: days))
            return .handled
        }
        .accessibilityLabel(accessibilityLabel)
        .accessibilityAction(named: "Move to Next Day") { store.send(.moved(chip.id, days: 1)) }
        .accessibilityAction(named: "Move to Previous Day") { store.send(.moved(chip.id, days: -1)) }
        .accessibilityAction(named: "Move to Next Week") { store.send(.moved(chip.id, days: 7)) }
    }

    private var accessibilityLabel: Text {
        let title = Text(chip.title)
        let status = Text(chip.status.title)
        if let time = chip.dueTime {
            return chip.isOverdue
                ? Text("\(title), \(status), \(time.formatted(locale)), overdue")
                : Text("\(title), \(status), \(time.formatted(locale))")
        }
        return chip.isOverdue ? Text("\(title), \(status), overdue") : Text("\(title), \(status)")
    }
}

/// A Project due on a day: a flag that opens it.
struct CalendarFlagView: View {
    var flag: CalendarFlag
    var store: StoreOf<CalendarFeature>

    var body: some View {
        Button {
            store.send(.projectTapped(flag.id))
        } label: {
            Label {
                Text(flag.name).font(.caption).lineLimit(1)
            } icon: {
                Image(systemName: "flag.fill").font(.caption2).foregroundStyle(Color(avenue: flag.color))
            }
            .labelStyle(.titleAndIcon)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text("Project \(flag.name), due"))
    }
}

/// Takes tasks dropped on a day, when `isOn`, at `time` as
/// ``CalendarFeature/Action/dropped(_:on:time:)`` has it. With an
/// `hourHeight`, `time` is an hour, and where in it the drop lands picks
/// the quarter.
struct DayDrop: ViewModifier {
    var day: CalendarDate
    var time: TimeOfDay??
    var hourHeight: CGFloat?
    var isOn = true
    @Binding var isTargeted: Bool
    var store: StoreOf<CalendarFeature>

    func body(content: Content) -> some View {
        if isOn {
            content.dropDestination(for: String.self) { ids, location in
                let dropped = ids.compactMap(UUID.init(uuidString:))
                guard !dropped.isEmpty else { return false }
                var time = time
                if let hourHeight, hourHeight > 0, case .some(.some(let hour)) = time {
                    let quarter = min(3, max(0, Int((location.y / hourHeight * 4).rounded(.down))))
                    time = .some(TimeOfDay(clampingHour: hour.hour, minute: quarter * 15))
                }
                store.send(.dropped(dropped, on: day, time: time), animation: .default)
                return true
            } isTargeted: {
                isTargeted = $0
            }
        } else {
            content
        }
    }
}

/// A day's bars, one row per lane so they line up across the week. On the
/// Mac each bar shows its title where it begins; on the iPhone it's a thin
/// line under the day's number.
struct SpanLanes: View {
    var day: CalendarDay
    @Environment(\.colorSchemeContrast) private var contrast
    @WideCalendar private var isWide

    var body: some View {
        if !day.spans.isEmpty || day.moreSpans > 0 {
            VStack(alignment: .leading, spacing: 1) {
                ForEach(day.spans.indices, id: \.self) { lane in
                    piece(day.spans[lane])
                }
                if day.moreSpans > 0 {
                    Text("+\(day.moreSpans)")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(accessibilityLabel)
        }
    }

    @ViewBuilder private func piece(_ span: CalendarSpan?) -> some View {
        let height: CGFloat = isWide ? 14 : 3
        if let span {
            let shape = UnevenRoundedRectangle(
                topLeadingRadius: span.startsHere ? 3 : 0, bottomLeadingRadius: span.startsHere ? 3 : 0,
                bottomTrailingRadius: span.endsHere ? 3 : 0, topTrailingRadius: span.endsHere ? 3 : 0)
            shape
                .fill(Color(avenue: span.color).opacity(contrast == .increased ? 0.6 : 0.35))
                .frame(height: height)
                .overlay(alignment: .leading) {
                    if isWide, span.isFirst {
                        Text(span.title)
                            .font(.caption2)
                            .lineLimit(1)
                            .padding(.horizontal, 3)
                    }
                }
        } else {
            Color.clear.frame(height: height)
        }
    }

    private var accessibilityLabel: Text {
        let titles = day.spans.compactMap { $0?.title }.joined(separator: ", ")
        return day.moreSpans > 0
            ? Text("Under way: \(titles), and \(day.moreSpans) more")
            : Text("Under way: \(titles)")
    }
}

/// A task with only a start date, on the day it starts: a hollow chip.
struct CalendarStartView: View {
    var chip: CalendarChip
    var store: StoreOf<CalendarFeature>

    var body: some View {
        Button {
            store.send(.list(.rowTapped(chip.id)))
        } label: {
            HStack(spacing: 3) {
                Image(systemName: "play.circle").font(.caption2)
                Text(chip.title).font(.caption).lineLimit(1)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 4)
            .padding(.vertical, 1)
            .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(Color(avenue: chip.color), lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text("\(chip.title), starts"))
    }
}

/// The open tasks with no due date, to drag onto a day.
struct UnscheduledTray: View {
    var store: StoreOf<CalendarFeature>

    var body: some View {
        List {
            if store.tray.isEmpty {
                Text("Every open task has a due date.")
                    .foregroundStyle(.secondary)
            }
            ForEach(store.tray) { chip in
                CalendarChipView(chip: chip, store: store)
            }
        }
        .navigationTitle("Unscheduled")
    }
}

/// Whether the calendar has room for chips with titles and a column a day:
/// on the Mac, and on an iPad at a regular width, not in a narrow split.
@propertyWrapper
struct WideCalendar: DynamicProperty {
    #if os(iOS)
        @Environment(\.horizontalSizeClass) private var sizeClass
    #endif

    init() {}

    @MainActor var wrappedValue: Bool {
        #if os(iOS)
            // A large iPhone on its side is regular width too, but too
            // short for a column a day.
            UIDevice.current.userInterfaceIdiom == .pad && sizeClass == .regular
        #else
            true
        #endif
    }
}
