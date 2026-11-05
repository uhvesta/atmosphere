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

/// Reads what the app last wrote. The app redraws the widgets after each
/// change, so the timeline only asks again in case it missed one.
struct TodayProvider: TimelineProvider {
    func placeholder(in context: Context) -> TodayEntry {
        TodayEntry(date: Date(), glance: TodayGlance(asOf: Date(), rows: [], locked: true))
    }

    func getSnapshot(in context: Context, completion: @escaping (TodayEntry) -> Void) {
        completion(entry())
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<TodayEntry>) -> Void) {
        let entry = entry()
        let next = Calendar.current.startOfDay(for: entry.date).addingTimeInterval(24 * 60 * 60)
        completion(Timeline(entries: [entry], policy: .after(next)))
    }

    private func entry() -> TodayEntry {
        do throws(GlanceError) {
            return TodayEntry(date: Date(), glance: try GlanceFile.shared?.read())
        } catch {
            Logger(subsystem: "lifr", category: "widgets").error("\(error.description, privacy: .public)")
            return TodayEntry(date: Date(), glance: nil)
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
