import LifrCore
import LifrGlance
import SwiftUI
import WidgetKit

/// How many tasks are on Today, and how many of them are overdue, with
/// nothing else. Counts show while the app lock is on too. Tapping it opens
/// Today.
package struct CountTileView: View {
    package var glance: TodayGlance?

    package init(glance: TodayGlance?) {
        self.glance = glance
    }

    package var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Label("Today", systemImage: "sun.max.fill")
                .font(.headline)
                .foregroundStyle(.orange)
            Spacer(minLength: 0)
            if let glance {
                Text(glance.count, format: .number)
                    .font(.system(size: 56, weight: .semibold, design: .rounded))
                    .monospacedDigit()
                    .minimumScaleFactor(0.5)
                    .lineLimit(1)
                    .contentTransition(.numericText())
                if glance.overdue > 0 {
                    Text("\(glance.overdue) overdue")
                        .font(.subheadline)
                        .foregroundStyle(.red)
                } else {
                    Text(glance.count == 0 ? "All clear" : "due today")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            } else {
                Text("Open Lifr to see Today here.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .accessibilityElement(children: .combine)
        .widgetURL(DeepLink.today.url)
    }
}

/// A button-sized tile that opens Quick Add. It shows no tasks, so it looks
/// the same while the app lock is on.
package struct QuickAddTileView: View {
    package init() {}

    package var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "plus")
                .font(.system(size: 34, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 64, height: 64)
                .background(Circle().fill(.orange))
                .accessibilityHidden(true)
            Text("New Task")
                .font(.headline)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .widgetURL(DeepLink.add(title: nil).url)
    }
}
