import LifrGlance
import SwiftUI

/// Today in a widget: how many tasks, how many overdue, and the first few
/// titles where there's room. Only counts while the app lock is on.
package struct TodayWidgetView: View {
    /// How much room the widget has.
    package enum Size: Hashable, Sendable {
        case small, medium, large

        /// How many titles fit.
        var rows: Int {
            switch self {
            case .small: 0
            case .medium: 3
            case .large: 7
            }
        }
    }

    package var glance: TodayGlance?
    package var size: Size

    package init(glance: TodayGlance?, size: Size) {
        self.glance = glance
        self.size = size
    }

    package var body: some View {
        if let glance, size == .medium {
            // Side by side: a medium widget is too short to stack them.
            HStack(alignment: .top, spacing: 16) {
                header(glance)
                    .frame(maxHeight: .infinity, alignment: .topLeading)
                detail(glance)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
        } else if let glance {
            VStack(alignment: .leading, spacing: 8) {
                header(glance)
                detail(glance)
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        } else {
            VStack(alignment: .leading, spacing: 6) {
                Label("Today", systemImage: "sun.max.fill")
                    .font(.headline)
                    .foregroundStyle(.orange)
                Text("Open Lifr to see Today here.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    /// The titles, or why there are none.
    @ViewBuilder private func detail(_ glance: TodayGlance) -> some View {
        if glance.isLocked {
            locked
        } else if glance.count == 0 {
            Text("Nothing due today")
                .font(.subheadline)
                .foregroundStyle(.secondary)
        } else if size != .small {
            list(glance)
        }
    }

    private func header(_ glance: TodayGlance) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Label("Today", systemImage: "sun.max.fill")
                .font(.headline)
                .foregroundStyle(.orange)
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(glance.count, format: .number)
                    .font(.system(size: size == .small ? 40 : 28, weight: .semibold, design: .rounded))
                    .contentTransition(.numericText())
                Text(glance.count == 1 ? "task" : "tasks")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            if glance.overdue > 0 {
                Text("\(glance.overdue) overdue")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.red)
            }
        }
        .accessibilityElement(children: .combine)
    }

    private var locked: some View {
        Label("Unlock Lifr to see them", systemImage: "lock.fill")
            .font(.caption)
            .foregroundStyle(.secondary)
    }

    private func list(_ glance: TodayGlance) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(glance.rows.prefix(size.rows)) { row in
                HStack(spacing: 6) {
                    Circle()
                        .fill(row.isOverdue ? Color.red : Color.orange)
                        .frame(width: 6, height: 6)
                        .accessibilityHidden(true)
                    Text(row.title)
                        .font(.subheadline)
                        .lineLimit(1)
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(row.isOverdue ? Text("\(row.title), overdue") : Text(row.title))
            }
            if glance.rows.count > size.rows {
                Text("\(glance.rows.count - size.rows) more")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}
