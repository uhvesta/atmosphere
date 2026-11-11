import AppIntents
import LifrCore
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
        UpcomingWidget()
        AvenuesWidget()
        CalendarWidget()
        #if os(iOS)
            NewTaskControl()
        #endif
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
        .supportedFamilies(families)
    }

    private var families: [WidgetFamily] {
        #if os(iOS)
            [.systemSmall, .systemMedium, .systemLarge, .accessoryCircular, .accessoryRectangular, .accessoryInline]
        #else
            [.systemSmall, .systemMedium, .systemLarge]
        #endif
    }
}

/// How many tasks are due on each of the next seven days, and their
/// titles where there's room.
struct UpcomingWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "Upcoming", provider: UpcomingProvider()) { entry in
            UpcomingEntryView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Upcoming")
        .description("What's due on each of the next seven days.")
        .supportedFamilies([.systemSmall, .systemMedium, .systemLarge])
    }
}

/// Each Avenue's open count, and its first titles where there's room.
struct AvenuesWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "Avenues", provider: TodayProvider()) { entry in
            AvenuesEntryView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Avenues")
        .description("How many tasks are open in each Avenue.")
        .supportedFamilies([.systemSmall, .systemMedium, .systemLarge])
    }
}

/// This month, with the days that have tasks due marked.
struct CalendarWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "Calendar", provider: TodayProvider()) { entry in
            CalendarEntryView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Calendar")
        .description("This month, with the days that have tasks due.")
        .supportedFamilies([.systemSmall, .systemLarge])
    }
}

#if os(iOS)
    /// A Control Center and Action button control that opens Quick Add.
    struct NewTaskControl: ControlWidget {
        var body: some ControlWidgetConfiguration {
            StaticControlConfiguration(kind: "NewTask") {
                ControlWidgetButton(action: Self.openQuickAdd) {
                    Label("New Task", systemImage: "plus.circle")
                }
            }
            .displayName("New Task")
            .description("Opens Quick Add in Lifr.")
        }

        private static var openQuickAdd: OpenURLIntent {
            var intent = OpenURLIntent()
            if let url = DeepLink.add(title: nil).url { intent.url = url }
            return intent
        }
    }
#endif

struct TodayEntry: TimelineEntry {
    var date: Date
    var glance: TodayGlance?
}

/// Reads what the app last wrote: Today, and what it will be after
/// midnight, so the widget turns over without the app. The app redraws the
/// widgets after each change, so the timeline only asks again once the
/// days it has run out.
struct TodayProvider: TimelineProvider {
    func placeholder(in context: TimelineProviderContext) -> TodayEntry {
        TodayEntry(
            date: Date(),
            glance: TodayGlance(asOf: Date(), from: .distantPast, until: .distantFuture, rows: [], locked: true))
    }

    func getSnapshot(in context: TimelineProviderContext, completion: @escaping @Sendable (TodayEntry) -> Void) {
        let now = Date()
        completion(TodayEntry(date: now, glance: glance()?.at(now)))
    }

    func getTimeline(
        in context: TimelineProviderContext, completion: @escaping @Sendable (Timeline<TodayEntry>) -> Void
    ) {
        let now = Date()
        let days = glance()?.days.filter { $0.until > now } ?? []
        let entries = days.map { TodayEntry(date: max($0.from, now), glance: $0) }
        // Past the last day only the app knows Today: say to open it.
        let end = days.last?.until ?? Calendar.current.startOfDay(for: now).addingTimeInterval(24 * 60 * 60)
        completion(Timeline(entries: entries + [TodayEntry(date: max(end, now), glance: nil)], policy: .after(end)))
    }

    private func glance() -> TodayGlance? {
        readGlance()
    }
}

/// What the app last wrote, or nil when it hasn't or the file can't be read.
private func readGlance() -> TodayGlance? {
    do throws(GlanceError) {
        return try GlanceFile.shared?.read()
    } catch {
        Logger(subsystem: "lifr", category: "widgets").error("\(error.description, privacy: .public)")
        return nil
    }
}

struct AvenuesEntryView: View {
    @Environment(\.widgetFamily) private var family
    var entry: TodayEntry

    var body: some View {
        switch family {
        case .systemMedium: AvenuesWidgetView(avenues: entry.glance?.avenues, size: .medium)
        case .systemLarge, .systemExtraLarge: AvenuesWidgetView(avenues: entry.glance?.avenues, size: .large)
        default: AvenuesWidgetView(avenues: entry.glance?.avenues, size: .small)
        }
    }
}

struct CalendarEntryView: View {
    @Environment(\.widgetFamily) private var family
    var entry: TodayEntry

    var body: some View {
        CalendarWidgetView(
            today: CalendarDate(entry.date, in: .current), dueCounts: entry.glance?.dueCounts,
            size: family == .systemSmall ? .small : .large)
    }
}

struct UpcomingEntry: TimelineEntry {
    var date: Date
    /// The days from `date` on, or nil when the app hasn't written them or
    /// they ran out.
    var days: [TodayGlance]?
}

/// Reads the two weeks the app last wrote and moves on a day at each
/// midnight, showing fewer days once fewer than seven are left.
struct UpcomingProvider: TimelineProvider {
    func placeholder(in context: TimelineProviderContext) -> UpcomingEntry {
        UpcomingEntry(
            date: Date(),
            days: [TodayGlance(asOf: Date(), from: Date(), until: .distantFuture, rows: [], locked: true)])
    }

    func getSnapshot(in context: TimelineProviderContext, completion: @escaping @Sendable (UpcomingEntry) -> Void) {
        let now = Date()
        completion(entry(at: now, readGlance()))
    }

    func getTimeline(
        in context: TimelineProviderContext, completion: @escaping @Sendable (Timeline<UpcomingEntry>) -> Void
    ) {
        let now = Date()
        let glance = readGlance()
        let days = glance?.days(from: now) ?? []
        let entries = days.map { entry(at: max($0.from, now), glance) }
        // Past the last day only the app knows: say to open it.
        let end = days.last?.until ?? Calendar.current.startOfDay(for: now).addingTimeInterval(24 * 60 * 60)
        completion(Timeline(entries: entries + [entry(at: max(end, now), glance)], policy: .after(end)))
    }

    private func entry(at date: Date, _ glance: TodayGlance?) -> UpcomingEntry {
        let days = glance?.days(from: date) ?? []
        return UpcomingEntry(date: date, days: days.isEmpty ? nil : days)
    }
}

struct UpcomingEntryView: View {
    @Environment(\.widgetFamily) private var family
    var entry: UpcomingEntry

    var body: some View {
        switch family {
        case .systemMedium: UpcomingWidgetView(days: entry.days, size: .medium)
        case .systemLarge, .systemExtraLarge: UpcomingWidgetView(days: entry.days, size: .large)
        default: UpcomingWidgetView(days: entry.days, size: .small)
        }
    }
}

struct TodayEntryView: View {
    @Environment(\.widgetFamily) private var family
    var entry: TodayEntry

    var body: some View {
        switch family {
        case .accessoryCircular: TodayAccessoryView(glance: entry.glance, shape: .circular)
        case .accessoryRectangular: TodayAccessoryView(glance: entry.glance, shape: .rectangular)
        case .accessoryInline: TodayAccessoryView(glance: entry.glance, shape: .inline)
        case .systemMedium: TodayWidgetView(glance: entry.glance, size: .medium)
        case .systemLarge, .systemExtraLarge: TodayWidgetView(glance: entry.glance, size: .large)
        default: TodayWidgetView(glance: entry.glance, size: .small)
        }
    }
}
