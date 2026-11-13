import LifrCore
import SwiftUI

/// One task in a list: status, title, code, due date, Sub task progress and
/// priority. VoiceOver reads it as one element, with an action named for
/// each status the task can move to, or Change Status when it can't pick
/// one directly.
package struct TaskRow: View {
    package var row: TaskRowData
    package var toggle: () -> Void
    package var pick: ((TaskStatus) -> Void)?

    @Environment(\.locale) private var locale
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    package init(_ row: TaskRowData, toggle: @escaping () -> Void, pick: ((TaskStatus) -> Void)? = nil) {
        self.row = row
        self.toggle = toggle
        self.pick = pick
    }

    package var body: some View {
        let layout =
            dynamicTypeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 8))
            : AnyLayout(HStackLayout(alignment: .firstTextBaseline, spacing: 12))
        layout {
            Button(action: toggle) {
                StatusGlyph(row.status)
            }
            .buttonStyle(.plain)
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
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(row.title)
        .accessibilityValue(row.accessibilityValue(locale))
        .accessibilityIdentifier(row.code)
        .accessibilityAddTraits(.isButton)
        .accessibilityActions {
            if let pick {
                // Each names its status, so it acts at once, as a swipe does.
                ForEach(TaskStatus.pickOrder.filter { $0 != row.status }, id: \.self) { status in
                    Button(statusActionName(status)) { pick(status) }
                }
            } else {
                Button("Change Status", action: toggle)
            }
        }
    }

    private func statusActionName(_ status: TaskStatus) -> LocalizedStringKey {
        if status == .inProgress, row.status == .procrastinated { return "Restart" }
        return "Mark \(status.title)"
    }

    /// The code, due date, repeat, Sub task progress and priority. Each stays on one
    /// line, unless they're stacked, where one too wide for the row wraps.
    @ViewBuilder
    private func details(oneLine: Bool) -> some View {
        HStack(spacing: 4) {
            if let color = row.avenueColor {
                Circle()
                    .fill(Color(avenue: color))
                    .frame(width: 6, height: 6)
            }
            TaskCodeLabel(row.code)
        }
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

/// A label's name on a soft tint of its color.
private struct LabelChip: View {
    var chip: TaskRowData.Chip

    init(_ chip: TaskRowData.Chip) {
        self.chip = chip
    }

    var body: some View {
        Text(chip.name)
            .font(.caption2)
            .lineLimit(1)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(Color(avenue: chip.color).opacity(0.18), in: Capsule())
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
