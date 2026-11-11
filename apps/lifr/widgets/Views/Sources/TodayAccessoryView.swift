import LifrCore
import LifrGlance
import SwiftUI
import WidgetKit

/// Today on the Lock Screen and in StandBy: the count in a circle, the
/// count and the next two titles, or one line. Tapping opens Today. Only counts
/// while the app lock is on.
package struct TodayAccessoryView: View {
    /// Which Lock Screen shape it fills.
    package enum Shape: Hashable, Sendable {
        case circular, rectangular, inline
    }

    package var glance: TodayGlance?
    package var shape: Shape

    package init(glance: TodayGlance?, shape: Shape) {
        self.glance = glance
        self.shape = shape
    }

    package var body: some View {
        content.widgetURL(DeepLink.today.url)
    }

    @ViewBuilder private var content: some View {
        switch shape {
        case .circular: circular
        case .rectangular: rectangular
        case .inline: inline
        }
    }

    private var circular: some View {
        ZStack {
            #if os(iOS)
                AccessoryWidgetBackground()
            #endif
            VStack(spacing: 0) {
                Image(systemName: "sun.max.fill")
                    .font(.caption2)
                if let glance {
                    Text(glance.count, format: .number)
                        .font(.system(.title2, design: .rounded, weight: .semibold))
                        .minimumScaleFactor(0.5)
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(summary)
    }

    private var rectangular: some View {
        VStack(alignment: .leading, spacing: 1) {
            Label {
                if let glance, glance.count > 0 { Text("Today · \(glance.count)") } else { Text("Today") }
            } icon: {
                Image(systemName: "sun.max.fill")
            }
            .font(.headline)
            if let glance {
                if glance.isLocked {
                    Text("Unlock Lifr to see them")
                } else if glance.count == 0 {
                    Text("Nothing due")
                } else {
                    ForEach(glance.rows.prefix(2)) { row in
                        Text(row.isOverdue ? "! \(row.title)" : row.title)
                    }
                }
            } else {
                Text("Open Lifr to see Today")
            }
        }
        .font(.caption)
        .lineLimit(1)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }

    private var inline: some View {
        Label {
            summary
        } icon: {
            Image(systemName: "sun.max.fill")
        }
    }

    /// "3 due · 1 overdue", or why there's no count.
    private var summary: Text {
        guard let glance else { return Text("Open Lifr") }
        if glance.count == 0 { return Text("Nothing due") }
        if glance.overdue > 0 { return Text("\(glance.count) due · \(glance.overdue) overdue") }
        return Text("\(glance.count) due")
    }
}
