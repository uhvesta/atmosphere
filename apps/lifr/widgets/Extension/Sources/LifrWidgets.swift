import LifrGlance
import LifrWidgetViews
import OSLog
import SwiftUI
import WidgetKit

/// Lifr's widgets.
@main
struct LifrWidgets: WidgetBundle {
    var body: some Widget {
        TodayWidget()
    }
}

/// How many tasks are on Today, and the first few where there's room.
struct TodayWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "Today", provider: TodayProvider()) { entry in
            TodayEntryView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Today")
        .description("What's due today, and how much is overdue.")
        .supportedFamilies([.systemSmall, .systemMedium, .systemLarge])
    }
}

struct TodayEntry: TimelineEntry {
    var date: Date
    var glance: TodayGlance?
}

/// Reads what the app last wrote: Today, and what it will be after
/// midnight, so the widget turns over without the app. The app redraws the
/// widgets after each change, so the timeline only asks again once the
/// days it has run out.
struct TodayProvider: TimelineProvider {
    func placeholder(in context: Context) -> TodayEntry {
        TodayEntry(
            date: Date(),
            glance: TodayGlance(asOf: Date(), from: .distantPast, until: .distantFuture, rows: [], locked: true))
    }

    func getSnapshot(in context: Context, completion: @escaping (TodayEntry) -> Void) {
        let now = Date()
        completion(TodayEntry(date: now, glance: glance()?.at(now)))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<TodayEntry>) -> Void) {
        let now = Date()
        let days = glance()?.days.filter { $0.until > now } ?? []
        let entries = days.map { TodayEntry(date: max($0.from, now), glance: $0) }
        // Past the last day only the app knows Today: say to open it.
        let end = days.last?.until ?? Calendar.current.startOfDay(for: now).addingTimeInterval(24 * 60 * 60)
        completion(Timeline(entries: entries + [TodayEntry(date: max(end, now), glance: nil)], policy: .after(end)))
    }

    private func glance() -> TodayGlance? {
        do throws(GlanceError) {
            return try GlanceFile.shared?.read()
        } catch {
            Logger(subsystem: "lifr", category: "widgets").error("\(error.description, privacy: .public)")
            return nil
        }
    }
}

struct TodayEntryView: View {
    @Environment(\.widgetFamily) private var family
    var entry: TodayEntry

    var body: some View {
        TodayWidgetView(glance: entry.glance, size: size)
    }

    private var size: TodayWidgetView.Size {
        switch family {
        case .systemMedium: .medium
        case .systemLarge, .systemExtraLarge: .large
        default: .small
        }
    }
}
