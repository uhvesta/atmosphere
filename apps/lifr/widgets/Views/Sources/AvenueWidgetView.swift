import LifrCore
import LifrGlance
import SwiftUI
import WidgetKit

/// One Avenue in a widget: its open count and its first titles, or only the
/// count while the app lock is on.
///
/// Tapping the widget opens the Avenue's page; on the larger sizes a title
/// opens its task, and a task's circle opens it with the statuses to pick.
package struct AvenueWidgetView: View {
    /// What the widget shows.
    package enum Content: Hashable, Sendable {
        /// Lifr hasn't written its Avenues yet.
        case notWritten
        /// The picked Avenue or Project was archived, finished or deleted.
        case gone
        case avenue(AvenueGlance)
        case project(AvenueGlance)
        case filter(AvenueGlance)
    }

    package var content: Content
    package var size: TodayWidgetView.Size

    package init(content: Content, size: TodayWidgetView.Size) {
        self.content = content
        self.size = size
    }

    package var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            switch content {
            case .notWritten:
                note("Open Lifr to see this Avenue here.")
            case .gone:
                note("What this widget showed was archived, finished or deleted. Edit the widget to pick another.")
            case .avenue(let avenue), .project(let avenue), .filter(let avenue):
                header(avenue)
                titles(avenue)
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .widgetURL(url)
    }

    private var url: URL? {
        switch content {
        case .avenue(let avenue): return DeepLink.avenue(avenue.id).url
        case .project(let project): return DeepLink.project(project.id).url
        case .filter(let filter): return DeepLink.filter(filter.id).url
        case .notWritten, .gone: return nil
        }
    }

    private func note(_ text: LocalizedStringResource) -> some View {
        Text(text)
            .font(.subheadline)
            .foregroundStyle(.secondary)
    }

    private func header(_ avenue: AvenueGlance) -> some View {
        let color = Color(widgetAvenue: avenue.color)
        return HStack(alignment: .firstTextBaseline, spacing: 6) {
            Image(systemName: avenue.symbol)
                .foregroundStyle(color)
            Text(avenue.name)
                .foregroundStyle(color)
                .lineLimit(1)
            Spacer(minLength: 4)
            Text(avenue.count, format: .number)
                .foregroundStyle(avenue.overdue > 0 ? Color.red : Color.secondary)
        }
        .font(.headline)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label(avenue))
    }

    private func label(_ avenue: AvenueGlance) -> Text {
        let tasks = String(AttributedString(localized: "^[\(avenue.count) open task](inflect: true)").characters)
        return avenue.overdue > 0
            ? Text("\(avenue.name), \(tasks), \(avenue.overdue) overdue") : Text("\(avenue.name), \(tasks)")
    }

    @ViewBuilder private func titles(_ avenue: AvenueGlance) -> some View {
        if avenue.count == 0 {
            note("Nothing open")
        } else if avenue.rows.isEmpty {
            Label("Unlock Lifr to see them", systemImage: "lock.fill")
                .font(.caption)
                .foregroundStyle(.secondary)
        } else {
            VStack(alignment: .leading, spacing: size == .small ? 4 : 6) {
                ForEach(avenue.rows.prefix(room)) { row in
                    title(row, color: Color(widgetAvenue: avenue.color))
                }
                if avenue.count > room {
                    Text("\(avenue.count - room) more")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    /// How many titles fit under the header.
    private var room: Int {
        switch size {
        case .small: 3
        case .medium: 3
        case .large: 8
        }
    }

    @ViewBuilder private func title(_ row: TodayGlance.Row, color: Color) -> some View {
        let text = Text(row.title)
            .font(size == .small ? .caption : .subheadline)
            .foregroundStyle(Color.primary)
            .lineLimit(1)
            .frame(maxWidth: .infinity, alignment: .leading)
        // A small widget opens one link only, so its rows are plain.
        if size == .small {
            HStack(spacing: 4) {
                Image(systemName: "circle")
                    .font(.caption)
                    .foregroundStyle(row.isOverdue ? Color.red : color)
                text
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(row.isOverdue ? Text("\(row.title), overdue") : Text(row.title))
        } else {
            HStack(spacing: 6) {
                linked(DeepLink.task(row.id, picksStatus: true).url) {
                    Image(systemName: "circle")
                        .font(.subheadline)
                        .foregroundStyle(row.isOverdue ? Color.red : color)
                }
                .accessibilityLabel(Text("Change Status of \(row.title)"))
                linked(DeepLink.task(row.id, picksStatus: false).url) { text }
                    .accessibilityLabel(row.isOverdue ? Text("\(row.title), overdue") : Text(row.title))
            }
        }
    }

    /// `label`, opening `url` when tapped.
    @ViewBuilder private func linked(_ url: URL?, @ViewBuilder label: () -> some View) -> some View {
        if let url {
            Link(destination: url, label: label)
        } else {
            label()
        }
    }
}
