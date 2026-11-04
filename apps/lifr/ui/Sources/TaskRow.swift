import LifrCore
import SwiftUI

/// One task in a list: status, title, code, due date, Sub task progress and
/// priority. VoiceOver reads it as one element, with toggling Done as an
/// action.
package struct TaskRow: View {
    package var row: TaskRowData
    package var toggle: () -> Void

    @Environment(\.locale) private var locale
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    package init(_ row: TaskRowData, toggle: @escaping () -> Void) {
        self.row = row
        self.toggle = toggle
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
            VStack(alignment: .leading, spacing: 4) {
                Text(row.title)
                    .strikethrough(row.status.isClosed)
                    .foregroundStyle(row.status.isClosed ? .secondary : .primary)
                // One line when it fits, otherwise one item per line.
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 8) { details }
                    VStack(alignment: .leading, spacing: 4) { details }
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(row.title)
        .accessibilityValue(row.accessibilityValue(locale))
        .accessibilityIdentifier(row.code)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction(named: row.status.isClosed ? "Mark To Do" : "Mark Done", toggle)
    }

    /// The code, due date, Sub task progress and priority. Each stays on one
    /// line.
    @ViewBuilder
    private var details: some View {
        TaskCodeLabel(row.code)
            .fixedSize()
        if let due = row.due {
            Text(due.text(locale))
                .font(.caption)
                .foregroundStyle(due.isOverdue ? .red : .secondary)
                .fixedSize()
        }
        if let subtasks = row.subtasks {
            SwiftUI.Label("\(subtasks.closed)/\(subtasks.total)", systemImage: "checklist")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize()
        }
        PriorityBadge(row.priority)
            .fixedSize()
    }
}
