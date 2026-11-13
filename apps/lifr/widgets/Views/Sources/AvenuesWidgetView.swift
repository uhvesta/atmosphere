import LifrCore
import LifrGlance
import SwiftUI
import WidgetKit

/// The Avenues in a widget: how many open tasks each has, and the first
/// few titles where there's room. Only counts while the app lock is on.
///
/// Tapping an Avenue opens its page, a title opens its task, and a task's
/// circle opens it with the statuses to pick.
package struct AvenuesWidgetView: View {
    /// The open Avenues in the sidebar's order, or nil before Lifr wrote
    /// them.
    package var avenues: [AvenueGlance]?
    package var size: TodayWidgetView.Size

    package init(avenues: [AvenueGlance]?, size: TodayWidgetView.Size) {
        self.avenues = avenues
        self.size = size
    }

    package var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("Avenues", systemImage: "square.grid.2x2.fill")
                .font(.headline)
                .foregroundStyle(.blue)
            if let avenues, avenues.isEmpty {
                Text("No Avenues yet")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            } else if let avenues {
                content(avenues)
            } else {
                Text("Open Lifr to see your Avenues here.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    @ViewBuilder private func content(_ avenues: [AvenueGlance]) -> some View {
        switch size {
        case .small:
            VStack(alignment: .leading, spacing: 6) {
                ForEach(avenues.prefix(4)) { avenue in
                    linked(DeepLink.avenue(avenue.id).url) { line(avenue) }
                }
            }
        case .medium:
            LazyVGrid(columns: [GridItem(.flexible(), spacing: 12), GridItem(.flexible())], spacing: 10) {
                ForEach(avenues.prefix(6)) { avenue in
                    linked(DeepLink.avenue(avenue.id).url) {
                        VStack(alignment: .leading, spacing: 2) {
                            line(avenue)
                            Text(avenue.rows.first?.title ?? " ")
                                .font(.caption)
                                .foregroundStyle(Color.secondary)
                                .lineLimit(1)
                                .accessibilityHidden(avenue.rows.isEmpty)
                        }
                    }
                }
            }
        case .large:
            VStack(alignment: .leading, spacing: 4) {
                ForEach(sections(avenues), id: \.avenue.id) { section in
                    linked(DeepLink.avenue(section.avenue.id).url) { line(section.avenue) }
                    ForEach(section.rows) { row in
                        title(row, color: Color(widgetAvenue: section.avenue.color))
                    }
                }
                if avenues.contains(where: { $0.count > 0 && $0.rows.isEmpty }) {
                    Label("Unlock Lifr to see them", systemImage: "lock.fill")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    /// The Avenues that fit, each with the titles that fit under it: a
    /// large widget has room for twelve lines.
    private func sections(_ avenues: [AvenueGlance]) -> [(avenue: AvenueGlance, rows: [TodayGlance.Row])] {
        var room = 12
        var sections: [(avenue: AvenueGlance, rows: [TodayGlance.Row])] = []
        for avenue in avenues where room > 0 {
            // Up to three titles each, so later Avenues get a line too.
            let rows = Array(avenue.rows.prefix(min(3, room - 1)))
            sections.append((avenue, rows))
            room -= 1 + rows.count
        }
        return sections
    }

    /// The Avenue's symbol, name and open count.
    private func line(_ avenue: AvenueGlance) -> some View {
        HStack(spacing: 6) {
            Image(systemName: avenue.symbol)
                .font(.subheadline)
                .foregroundStyle(Color(widgetAvenue: avenue.color))
                .frame(width: 18)
            Text(avenue.name)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(Color.primary)
                .lineLimit(1)
            Spacer(minLength: 4)
            Text(avenue.count, format: .number)
                .font(.subheadline)
                .foregroundStyle(avenue.overdue > 0 ? Color.red : Color.secondary)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label(avenue))
    }

    private func label(_ avenue: AvenueGlance) -> Text {
        let tasks = String(AttributedString(localized: "^[\(avenue.count) open task](inflect: true)").characters)
        return avenue.overdue > 0
            ? Text("\(avenue.name), \(tasks), \(avenue.overdue) overdue") : Text("\(avenue.name), \(tasks)")
    }

    private func title(_ row: TodayGlance.Row, color: Color) -> some View {
        HStack(spacing: 6) {
            linked(DeepLink.task(row.id, picksStatus: true).url) {
                Image(systemName: "circle")
                    .font(.subheadline)
                    .foregroundStyle(row.isOverdue ? Color.red : color)
            }
            .accessibilityLabel(Text("Change Status of \(row.title)"))
            linked(DeepLink.task(row.id, picksStatus: false).url) {
                Text(row.title)
                    .font(.subheadline)
                    .foregroundStyle(Color.primary)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .accessibilityLabel(row.isOverdue ? Text("\(row.title), overdue") : Text(row.title))
        }
        .padding(.leading, 24)
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

extension Color {
    /// An Avenue's color, stored by name. The app's own copy is in LifrUI,
    /// which the widgets don't link to stay small.
    init(widgetAvenue name: String) {
        self =
            switch name {
            case "red": .red
            case "orange": .orange
            case "yellow": .yellow
            case "green": .green
            case "mint": .mint
            case "teal": .teal
            case "cyan": .cyan
            case "indigo": .indigo
            case "purple": .purple
            case "pink": .pink
            case "brown": .brown
            case "gray": .gray
            default: .blue
            }
    }
}
