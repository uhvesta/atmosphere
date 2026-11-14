import LifrCore
import SwiftUI

/// One task in a list: status, title, code, due date, Sub task progress and
/// priority. The status glyph opens a menu of the statuses the task can
/// move to, so a tap alone never changes it. VoiceOver reads the row as one
/// element, with an action named for each of those statuses, or Change
/// Status when it can't pick one directly. A task that starts later is
/// drawn dimmed, and in a list that mixes Avenues a thin bar in the
/// Avenue's color runs down its leading edge.
package struct TaskRow: View {
    package var row: TaskRowData
    /// What the glyph does when it isn't a menu: no `pick`, or `menu` off.
    package var toggle: () -> Void
    package var pick: ((TaskStatus) -> Void)?
    /// Whether the glyph opens the status menu, given `pick`; off while
    /// rows are being picked, where a tap on the glyph picks the row.
    package var menu: Bool

    @Environment(\.locale) private var locale
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    package init(
        _ row: TaskRowData, menu: Bool = true, toggle: @escaping () -> Void, pick: ((TaskStatus) -> Void)? = nil
    ) {
        self.row = row
        self.menu = menu
        self.toggle = toggle
        self.pick = pick
    }

    /// The statuses the task can move to, in the order every picker uses.
    private var nextStatuses: [TaskStatus] { TaskStatus.pickOrder.filter { $0 != row.status } }

    /// Starts later, so not yet something to do.
    private var isLater: Bool { row.starts != nil && !row.status.isClosed }

    package var body: some View {
        let layout =
            dynamicTypeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 8))
            : AnyLayout(HStackLayout(alignment: .firstTextBaseline, spacing: 12))
        layout {
            glyph
                .modifier(StatusHover())
            VStack(alignment: .leading, spacing: 4) {
                Text(row.title)
                    .lineLimit(2)
                    .strikethrough(row.status.isClosed)
                    .foregroundStyle(row.status.isClosed ? .secondary : .primary)
                    .animation(reduceMotion ? nil : .easeOut(duration: 0.3), value: row.status)
                // One line when it fits, otherwise one item per line.
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 8) { details(oneLine: true) }
                    VStack(alignment: .leading, spacing: 4) { details(oneLine: false) }
                }
                if !row.labels.isEmpty {
                    // As many chips as fit on one line; VoiceOver reads them all.
                    ViewThatFits(in: .horizontal) {
                        ForEach((1...row.labels.count).reversed(), id: \.self) { count in
                            HStack(spacing: 4) {
                                ForEach(row.labels.prefix(count), id: \.name) { LabelChip($0) }
                                if count < row.labels.count {
                                    Text("+\(row.labels.count - count)")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                        .fixedSize()
                                        // On a Mac, pointing at it names the rest.
                                        .help(row.labels.dropFirst(count).map { "#\($0.name)" }.joined(separator: " "))
                                }
                            }
                        }
                        EmptyView()
                    }
                }
            }
        }
        .opacity(isLater ? 0.55 : 1)
        .padding(.leading, row.avenueColor == nil ? 0 : 10)
        .background(alignment: .leading) {
            if let color = row.avenueColor {
                Capsule()
                    .fill(Color(avenue: color))
                    .frame(width: 3)
                    .padding(.vertical, 2)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(row.title)
        .accessibilityValue(row.accessibilityValue(locale))
        .accessibilityIdentifier(row.code)
        .accessibilityAddTraits(.isButton)
        .accessibilityActions {
            if let pick {
                // Each names its status, so it acts at once, as a swipe does.
                ForEach(nextStatuses, id: \.self) { status in
                    Button(statusActionName(status)) { pick(status) }
                }
            } else {
                Button("Change Status", action: toggle)
            }
        }
    }

    /// The glyph: a menu of the statuses to move to, or a plain button
    /// where the row can't pick one, or is being picked itself.
    @ViewBuilder private var glyph: some View {
        if let pick, menu {
            Menu {
                ForEach(nextStatuses, id: \.self) { status in
                    Button {
                        pick(status)
                    } label: {
                        Label {
                            Text(statusMenuName(status))
                        } icon: {
                            StatusGlyph(status, font: .body)
                        }
                    }
                }
            } label: {
                StatusGlyph(row.status)
            }
            .menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
            .fixedSize()
        } else {
            Button(action: toggle) {
                StatusGlyph(row.status)
            }
            .buttonStyle(.plain)
        }
    }

    /// A menu item's name: Restart, for an idle task going back to In
    /// Progress, as its swipe says.
    private func statusMenuName(_ status: TaskStatus) -> String {
        if status == .inProgress, row.status == .procrastinated { return String(localized: "Restart") }
        return status.title
    }

    private func statusActionName(_ status: TaskStatus) -> LocalizedStringKey {
        if status == .inProgress, row.status == .procrastinated { return "Restart" }
        return "Mark \(status.title)"
    }

    /// The code, due date, repeat, Sub task progress and priority. Each stays on one
    /// line, unless they're stacked, where one too wide for the row wraps.
    @ViewBuilder
    private func details(oneLine: Bool) -> some View {
        TaskCodeLabel(row.code)
            .fixedSize(horizontal: oneLine, vertical: true)
        if let starts = row.starts {
            Text("Starts \(starts.formatted(locale))")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: oneLine, vertical: true)
        }
        if let due = row.due {
            Text(due.text(locale))
                .font(.caption)
                .foregroundStyle(due.isOverdue ? .red : .secondary)
                .fixedSize(horizontal: oneLine, vertical: true)
        }
        if row.repeats {
            Image(systemName: "repeat")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        if let subtasks = row.subtasks {
            // Not a Label: in a stacked layout it shows the icon alone.
            HStack(spacing: 2) {
                Image(systemName: "checklist")
                Text("\(subtasks.closed)/\(subtasks.total)")
                    .contentTransition(.numericText(value: Double(subtasks.closed)))
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: oneLine, vertical: true)
        }
        PriorityBadge(row.priority)
            .fixedSize(horizontal: oneLine, vertical: true)
    }
}

/// A label's name on a soft tint of its color; with Increase Contrast, a
/// stronger tint and an outline in the color, so the chip's edge shows on
/// any background.
private struct LabelChip: View {
    var chip: TaskRowData.Chip
    @Environment(\.colorSchemeContrast) private var contrast

    init(_ chip: TaskRowData.Chip) {
        self.chip = chip
    }

    var body: some View {
        Text(chip.name)
            .font(.caption2)
            .lineLimit(1)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(Color(avenue: chip.color).opacity(contrast == .increased ? 0.3 : 0.18), in: Capsule())
            .overlay {
                if contrast == .increased { Capsule().strokeBorder(Color(avenue: chip.color)) }
            }
            .fixedSize()
    }
}

/// Shows that the status circle is its own button when a pointer is over
/// it: a soft circle behind it and a tooltip on the Mac, the pointer's lift
/// on iPad.
private struct StatusHover: ViewModifier {
    @State private var isHovered = false

    func body(content: Content) -> some View {
        #if os(macOS)
            content
                .background {
                    Circle()
                        .fill(Color.primary.opacity(isHovered ? 0.1 : 0))
                        .padding(-4)
                }
                .onHover { isHovered = $0 }
                .help(Text("Change Status"))
        #else
            content
                .contentShape(.hoverEffect, Circle().inset(by: -6))
                .hoverEffect(.highlight)
        #endif
    }
}
