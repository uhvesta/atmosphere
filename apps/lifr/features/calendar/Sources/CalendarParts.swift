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
            .background { ModeShortcuts(store: store) }
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

/// ⌃⌘1 and ⌃⌘2 pick Month and Week while the calendar shows; ⌘1 to ⌘3
/// already go to lists.
private struct ModeShortcuts: View {
    var store: StoreOf<CalendarFeature>

    var body: some View {
        ZStack {
            Button("Show Month") { store.send(.modePicked(.month), animation: .default) }
                .keyboardShortcut("1", modifiers: [.command, .control])
            Button("Show Week") { store.send(.modePicked(.week), animation: .default) }
                .keyboardShortcut("2", modifiers: [.command, .control])
        }
        // The segmented picker is the control; these only take the keys.
        .opacity(0)
        .accessibilityHidden(true)
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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

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
            LiftedChip(title: chip.title, color: chip.color, reduceMotion: reduceMotion) {
                StatusGlyph(chip.status, font: .caption)
            }
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

/// A chip lifted off the calendar while it's dragged: tilted, with a
/// shadow, as list rows are.
private struct LiftedChip<Icon: View>: View {
    var title: String
    var color: String
    var reduceMotion: Bool
    @ViewBuilder var icon: Icon

    var body: some View {
        HStack(spacing: 4) {
            icon
            Text(title).font(.callout).lineLimit(1)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .frame(maxWidth: 240, alignment: .leading)
        .background(Color(avenue: color).opacity(0.18), in: .rect(cornerRadius: 8))
        .background(.background, in: .rect(cornerRadius: 8))
        .rotationEffect(.degrees(reduceMotion ? 0 : 2))
        .shadow(color: .black.opacity(0.2), radius: 8, y: 4)
        .padding(12)
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
/// ``CalendarFeature/Action/dropped(_:on:time:)`` has it, and a bar's
/// leading edge, to move its start. With `hours`, the view is those hours
/// from top to bottom, each an equal share of `height`, and where the drop
/// lands picks the hour and its quarter.
struct DayDrop: ViewModifier {
    var day: CalendarDate
    var time: TimeOfDay??
    var hours: Range<Int>?
    var height: CGFloat = 0
    var isOn = true
    @Binding var isTargeted: Bool
    var store: StoreOf<CalendarFeature>

    func body(content: Content) -> some View {
        if isOn {
            content.dropDestination(for: String.self) { items, location in
                switch CalendarPayload(items) {
                case .tasks(let ids):
                    store.send(.dropped(ids, on: day, time: time(at: location.y)), animation: .default)
                case .start(let id):
                    store.send(.startDropped(id, on: day), animation: .default)
                case nil:
                    return false
                }
                return true
            } isTargeted: {
                isTargeted = $0
            }
        } else {
            content
        }
    }

    private func time(at offset: CGFloat) -> TimeOfDay?? {
        guard let hours, !hours.isEmpty, height > 0 else { return time }
        let quarters = hours.count * 4
        let index = min(quarters - 1, max(0, Int((offset / height * CGFloat(quarters)).rounded(.down))))
        return .some(TimeOfDay(clampingHour: hours.lowerBound + index / 4, minute: index % 4 * 15))
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
                            .padding(.leading, span.startsHere ? 7 : 3)
                            .padding(.trailing, 3)
                    }
                }
                .overlay(alignment: .leading) {
                    if isWide, span.startsHere {
                        StartEdge(span: span, height: height)
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

/// A bar's leading edge, on the day its task starts: dragging it onto
/// another day moves the start alone, leaving the due date.
private struct StartEdge: View {
    var span: CalendarSpan
    var height: CGFloat
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Capsule()
            .fill(Color(avenue: span.color))
            .frame(width: 3, height: height - 4)
            .padding(.horizontal, 2)
            .frame(height: height)
            .contentShape(Rectangle())
            .draggable(CalendarPayload.dragging(startOf: span.id)) {
                LiftedChip(title: span.title, color: span.color, reduceMotion: reduceMotion) {
                    Text("Starts").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                }
            }
            #if os(macOS)
                .pointerStyle(.frameResize(position: .leading))
            #endif
            .help(Text("Drag to another day to change when it starts"))
    }
}

/// A task with only a start date, on the day it starts: a hollow chip
/// labelled Starts.
struct CalendarStartView: View {
    var chip: CalendarChip
    var store: StoreOf<CalendarFeature>

    var body: some View {
        Button {
            store.send(.list(.rowTapped(chip.id)))
        } label: {
            HStack(spacing: 3) {
                Text("Starts")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(Color(avenue: chip.color))
                    .lineLimit(1)
                    .fixedSize()
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
