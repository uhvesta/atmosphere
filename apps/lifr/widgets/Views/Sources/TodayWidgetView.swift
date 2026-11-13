import LifrCore
import LifrGlance
import SwiftUI
import WidgetKit

/// Today in a widget: how many tasks, how many overdue, and the first few
/// titles where there's room. Only counts while the app lock is on.
///
/// Tapping the header opens Today, a title opens its task, a task's circle
/// opens it with the statuses to pick, and + opens Quick Add.
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
        content.widgetURL(DeepLink.today.url)
    }

    @ViewBuilder private var content: some View {
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
        HStack(alignment: .top) {
            count(glance)
            if size == .large { Spacer(minLength: 0) }
            if size != .small { add }
        }
    }

    /// Such as "3 tasks", the number large, read as one phrase so the
    /// word agrees with it and sits where the language puts it.
    private func counted(_ count: Int) -> AttributedString {
        var text = AttributedString(localized: "^[\(count) task](inflect: true)")
        if let number = text.range(of: count.formatted()) {
            text[number].font = .system(size: size == .small ? 40 : 28, weight: .semibold, design: .rounded)
            text[number].foregroundColor = .primary
        }
        return text
    }

    private func count(_ glance: TodayGlance) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Label("Today", systemImage: "sun.max.fill")
                .font(.headline)
                .foregroundStyle(.orange)
            Text(counted(glance.count))
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .contentTransition(.numericText())
            if glance.overdue > 0 {
                Text("\(glance.overdue) overdue")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.red)
            }
        }
        .accessibilityElement(children: .combine)
    }

    /// Opens Quick Add.
    @ViewBuilder private var add: some View {
        if let url = DeepLink.add(title: nil).url {
            Link(destination: url) {
                Image(systemName: "plus.circle.fill")
                    .font(.title3)
                    .foregroundStyle(.orange)
            }
            .accessibilityLabel("Add a Task")
        }
    }

    private var locked: some View {
        Label("Unlock Lifr to see them", systemImage: "lock.fill")
            .font(.caption)
            .lineLimit(2)
            .fixedSize(horizontal: false, vertical: true)
            .foregroundStyle(.secondary)
    }

    private func list(_ glance: TodayGlance) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(glance.rows.prefix(size.rows)) { row in
                HStack(spacing: 6) {
                    link(.task(row.id, picksStatus: true)) {
                        Image(systemName: "circle")
                            .font(.subheadline)
                            .foregroundStyle(row.isOverdue ? Color.red : Color.orange)
                    }
                    .accessibilityLabel(Text("Change Status of \(row.title)"))
                    link(.task(row.id, picksStatus: false)) {
                        Text(row.title)
                            .font(.subheadline)
                            .foregroundStyle(Color.primary)
                            .lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .accessibilityLabel(row.isOverdue ? Text("\(row.title), overdue") : Text(row.title))
                }
            }
            if glance.rows.count > size.rows {
                Text("\(glance.rows.count - size.rows) more")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    /// `label`, opening `link` when tapped.
    @ViewBuilder private func link(_ link: DeepLink, @ViewBuilder label: () -> some View) -> some View {
        if let url = link.url {
            Link(destination: url, label: label)
        } else {
            label()
        }
    }
}
