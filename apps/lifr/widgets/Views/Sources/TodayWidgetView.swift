import LifrCore
import LifrGlance
import SwiftUI
import WidgetKit

/// Today in a widget: how many tasks, how many overdue, and the first few
/// titles where there's room. Only counts while the app lock is on.
///
/// Tapping the header opens Today, a title opens its task, a task's circle
/// opens it with the statuses to pick, and + opens Quick Add. Set to
/// complete with one tap, the circle marks the task Done instead and the row
/// stays, struck through, so a mis-tap shows and another tap undoes it.
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
    /// The tasks marked Done from the widget that Lifr hasn't saved yet.
    package var marked: Set<UUID>
    /// Wraps a task's circle in the button that marks it, or unmarks it
    /// when it's marked; nil while the circle opens the statuses to pick.
    package var marks: ((TodayGlance.Row, AnyView) -> AnyView)?

    package init(
        glance: TodayGlance?, size: Size, marked: Set<UUID> = [],
        marks: ((TodayGlance.Row, AnyView) -> AnyView)? = nil
    ) {
        self.glance = glance
        self.size = size
        self.marked = marked
        self.marks = marks
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
                let isMarked = marked.contains(row.id)
                HStack(spacing: 6) {
                    glyph(row, isMarked: isMarked)
                    link(.task(row.id, picksStatus: false)) {
                        Text(row.title)
                            .font(.subheadline)
                            .strikethrough(isMarked)
                            .foregroundStyle(isMarked ? Color.secondary : Color.primary)
                            .lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .accessibilityLabel(label(row, isMarked: isMarked))
                    if let due = dueTime(row, on: glance), !isMarked {
                        Text(due, format: .dateTime.hour().minute())
                            .font(.caption2)
                            .monospacedDigit()
                            .foregroundStyle(.secondary)
                    }
                }
            }
            if glance.rows.count > size.rows {
                Text("\(glance.rows.count - size.rows) more")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    /// The task's circle: a button that marks it when set to complete with
    /// one tap, otherwise a link that opens the statuses to pick.
    @ViewBuilder private func glyph(_ row: TodayGlance.Row, isMarked: Bool) -> some View {
        let symbol = isMarked ? "checkmark.circle.fill" : row.isInProgress ? "circle.lefthalf.filled" : "circle"
        let tint = row.isOverdue ? Color.red : row.color.map { Color(widgetAvenue: $0) } ?? Color.orange
        let circle = Image(systemName: symbol)
            .font(.subheadline)
            .foregroundStyle(isMarked ? Color.secondary : tint)
        if let marks {
            marks(row, AnyView(circle))
                .accessibilityLabel(isMarked ? Text("Reopen \(row.title)") : Text("Mark \(row.title) Done"))
        } else {
            link(.task(row.id, picksStatus: true)) { circle }
                .accessibilityLabel(Text("Change Status of \(row.title)"))
        }
    }

    private func label(_ row: TodayGlance.Row, isMarked: Bool) -> Text {
        if isMarked { return Text("\(row.title), Done") }
        if row.isOverdue { return Text("\(row.title), overdue") }
        return row.isInProgress ? Text("\(row.title), In Progress") : Text(row.title)
    }

    /// When on its day `row` is due, if it has a time; an overdue one's
    /// time was on an earlier day, so it has none here.
    private func dueTime(_ row: TodayGlance.Row, on glance: TodayGlance) -> Date? {
        guard let minutes = row.dueMinutes, !row.isOverdue else { return nil }
        return Calendar.current.date(byAdding: .minute, value: minutes, to: glance.from)
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
