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
        CountWidget()
        QuickAddWidget()
        #if os(iOS)
            NewTaskControl()
            FocusActivityWidget()
        #endif
    }
}

/// How many tasks are on Today, and the first few where there's room. On
/// iPhone it can be set to complete a task with one tap.
struct TodayWidget: Widget {
    var body: some WidgetConfiguration {
        #if os(iOS)
            AppIntentConfiguration(
                kind: "Today", intent: TodaySettingsIntent.self, provider: TodaySettingsProvider()
            ) { entry in
                TodayEntryView(entry: entry)
                    .containerBackground(.fill.tertiary, for: .widget)
            }
            .configurationDisplayName("Today")
            .description("What's due today, and how much is overdue.")
            .supportedFamilies(families)
        #else
            StaticConfiguration(kind: "Today", provider: TodayProvider()) { entry in
                TodayEntryView(entry: entry)
                    .containerBackground(.fill.tertiary, for: .widget)
            }
            .configurationDisplayName("Today")
            .description("What's due today, and how much is overdue.")
            .supportedFamilies(families)
        #endif
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

/// Each Avenue's open count, and its first titles where there's room. On
/// iPhone it can show one Avenue instead; the Mac's widget extensions
/// can't be built with the App Intents a choice needs.
struct AvenuesWidget: Widget {
    var body: some WidgetConfiguration {
        #if os(iOS)
            AppIntentConfiguration(
                kind: "Avenues", intent: PickAvenueIntent.self, provider: AvenueProvider()
            ) { entry in
                AvenuesEntryView(entry: entry.today, picked: entry.picked)
                    .containerBackground(.fill.tertiary, for: .widget)
            }
            .configurationDisplayName("Avenues")
            .description("How many tasks are open in each Avenue, or in the one you pick.")
            .supportedFamilies([.systemSmall, .systemMedium, .systemLarge])
        #else
            StaticConfiguration(kind: "Avenues", provider: TodayProvider()) { entry in
                AvenuesEntryView(entry: entry, picked: nil)
                    .containerBackground(.fill.tertiary, for: .widget)
            }
            .configurationDisplayName("Avenues")
            .description("How many tasks are open in each Avenue.")
            .supportedFamilies([.systemSmall, .systemMedium, .systemLarge])
        #endif
    }
}

#if os(iOS)
    /// An Avenue or Project the Avenues widget can show, from what Lifr
    /// last wrote.
    struct AvenueOption: AppEntity {
        static let typeDisplayRepresentation: TypeDisplayRepresentation = "Avenue, Project or Filter"
        static let defaultQuery = Query()

        var id: UUID
        var name: String
        var kind = Kind.avenue

        enum Kind {
            case avenue, project, filter
        }

        var displayRepresentation: DisplayRepresentation {
            switch kind {
            case .avenue: DisplayRepresentation(title: "\(name)")
            case .project: DisplayRepresentation(title: "\(name)", subtitle: "Project")
            case .filter: DisplayRepresentation(title: "\(name)", subtitle: "Saved Filter")
            }
        }

        struct Query: EntityQuery {
            func entities(for identifiers: [UUID]) async throws -> [AvenueOption] {
                options().filter { identifiers.contains($0.id) }
            }

            func suggestedEntities() async throws -> [AvenueOption] {
                options()
            }

            private func options() -> [AvenueOption] {
                let glance = readGlance()
                return (glance?.avenues ?? []).map { AvenueOption(id: $0.id, name: $0.name) }
                    + (glance?.projects ?? []).map { AvenueOption(id: $0.id, name: $0.name, kind: .project) }
                    + (glance?.filters ?? []).map { AvenueOption(id: $0.id, name: $0.name, kind: .filter) }
            }
        }
    }

    /// Which Avenue the Avenues widget shows: all of them when none is
    /// picked.
    struct PickAvenueIntent: WidgetConfigurationIntent {
        static let title: LocalizedStringResource = "Pick an Avenue, Project or Filter"
        static let description = IntentDescription(
            "Shows one Avenue's, Project's or saved filter's tasks, or every Avenue when none is picked.")

        @Parameter(title: "Avenue, Project or Filter")
        var avenue: AvenueOption?

        init() {}
    }

    struct AvenueEntry: TimelineEntry {
        var today: TodayEntry
        /// The picked Avenue, or nil for all of them.
        var picked: UUID?

        var date: Date { today.date }
    }

    /// Today's timeline, with the Avenue the widget was set to show.
    struct AvenueProvider: AppIntentTimelineProvider {
        func placeholder(in context: TimelineProviderContext) -> AvenueEntry {
            AvenueEntry(
                today: TodayEntry(
                    date: Date(),
                    glance: TodayGlance(
                        asOf: Date(), from: .distantPast, until: .distantFuture, rows: [], locked: true)),
                picked: nil)
        }

        func snapshot(for configuration: PickAvenueIntent, in context: TimelineProviderContext) async -> AvenueEntry {
            let now = Date()
            let today = TodayEntry(date: now, glance: readGlance()?.at(now))
            return AvenueEntry(today: today, picked: configuration.avenue?.id)
        }

        func timeline(
            for configuration: PickAvenueIntent, in context: TimelineProviderContext
        ) async -> Timeline<AvenueEntry> {
            let today = todayTimeline(at: Date())
            let entries = today.entries.map { AvenueEntry(today: $0, picked: configuration.avenue?.id) }
            return Timeline(entries: entries, policy: today.policy)
        }
    }
#endif

#if os(iOS)
    /// What tapping a task's circle in the Today widget does.
    struct TodaySettingsIntent: WidgetConfigurationIntent {
        static let title: LocalizedStringResource = "Today"
        static let description = IntentDescription("Choose what tapping a task's circle does.")

        @Parameter(
            title: "Complete with One Tap",
            description: """
                Tapping a task's circle marks it Done, and tapping it again undoes that. Lifr saves it the \
                next time it opens. When off, the circle opens the task with its statuses to pick.
                """,
            default: false)
        var completesWithOneTap: Bool

        init() {}
    }

    /// Marks a task Done from the Today widget, or unmarks it, for Lifr to
    /// save the next time it opens.
    struct MarkDoneIntent: AppIntent {
        static let title: LocalizedStringResource = "Mark Done from the Widget"
        static let isDiscoverable = false

        @Parameter(title: "Task")
        var task: String

        init() {}

        init(task: UUID) {
            self.task = task.uuidString
        }

        func perform() async throws -> some IntentResult {
            guard let id = UUID(uuidString: task), let file = MarksFile.shared else { return .result() }
            try file.toggle(id)
            return .result()
        }
    }

    /// Today's timeline, with what the circle does.
    struct TodaySettingsProvider: AppIntentTimelineProvider {
        func placeholder(in context: TimelineProviderContext) -> TodayEntry {
            TodayProvider().placeholder(in: context)
        }

        func snapshot(
            for configuration: TodaySettingsIntent, in context: TimelineProviderContext
        ) async -> TodayEntry {
            let now = Date()
            return TodayEntry(
                date: now, glance: readGlance()?.at(now), marked: readMarks(),
                completesWithOneTap: configuration.completesWithOneTap)
        }

        func timeline(
            for configuration: TodaySettingsIntent, in context: TimelineProviderContext
        ) async -> Timeline<TodayEntry> {
            let today = todayTimeline(at: Date())
            let entries = today.entries.map { entry in
                var entry = entry
                entry.completesWithOneTap = configuration.completesWithOneTap
                return entry
            }
            return Timeline(entries: entries, policy: today.policy)
        }
    }
#endif

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

/// How many tasks are on Today and how many are overdue, and nothing else.
struct CountWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "Count", provider: TodayProvider()) { entry in
            CountTileView(glance: entry.glance)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Today's Count")
        .description("How many tasks are due today, and how many are overdue.")
        .supportedFamilies([.systemSmall])
    }
}

/// A tile that opens Quick Add.
struct QuickAddWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "QuickAdd", provider: TodayProvider()) { _ in
            QuickAddTileView()
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("New Task")
        .description("Opens Quick Add in Lifr.")
        .supportedFamilies([.systemSmall])
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
    /// The tasks marked Done from the widget that Lifr hasn't saved yet.
    var marked: Set<UUID> = []
    /// Whether a task's circle marks it Done rather than opening Lifr.
    var completesWithOneTap = false
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
        completion(TodayEntry(date: now, glance: readGlance()?.at(now)))
    }

    func getTimeline(
        in context: TimelineProviderContext, completion: @escaping @Sendable (Timeline<TodayEntry>) -> Void
    ) {
        completion(todayTimeline(at: Date()))
    }
}

/// Each day the app last wrote from `now` on, then one saying to open Lifr.
private func todayTimeline(at now: Date) -> Timeline<TodayEntry> {
    let days = readGlance()?.days.filter { $0.until > now } ?? []
    let marked = readMarks()
    let entries = days.map { TodayEntry(date: max($0.from, now), glance: $0, marked: marked) }
    // Past the last day only the app knows Today: say to open it.
    let end = days.last?.until ?? Calendar.current.startOfDay(for: now).addingTimeInterval(24 * 60 * 60)
    return Timeline(entries: entries + [TodayEntry(date: max(end, now), glance: nil)], policy: .after(end))
}

/// What the app last wrote, without the tasks marked Done from the widget
/// in its counts, or nil when it hasn't written or the file can't be read.
private func readGlance() -> TodayGlance? {
    do throws(GlanceError) {
        return try GlanceFile.shared?.read()?.marking(readMarks())
    } catch {
        Logger(subsystem: "lifr", category: "widgets").error("\(error.description, privacy: .public)")
        return nil
    }
}

/// The tasks marked Done from the widget that Lifr hasn't saved yet.
private func readMarks() -> Set<UUID> {
    do throws(MarksError) {
        return try MarksFile.shared?.read() ?? []
    } catch {
        Logger(subsystem: "lifr", category: "widgets").error("\(error.description, privacy: .public)")
        return []
    }
}

struct AvenuesEntryView: View {
    @Environment(\.widgetFamily) private var family
    var entry: TodayEntry
    /// The one Avenue to show, or nil for all of them.
    var picked: UUID?

    var body: some View {
        if let picked {
            AvenueWidgetView(content: content(picked), size: size)
        } else {
            AvenuesWidgetView(avenues: entry.glance?.avenues, size: size)
        }
    }

    private var size: TodayWidgetView.Size {
        switch family {
        case .systemMedium: .medium
        case .systemLarge, .systemExtraLarge: .large
        default: .small
        }
    }

    private func content(_ id: UUID) -> AvenueWidgetView.Content {
        guard let avenues = entry.glance?.avenues else { return .notWritten }
        if let avenue = avenues.first(where: { $0.id == id }) { return .avenue(avenue) }
        if let project = entry.glance?.projects?.first(where: { $0.id == id }) { return .project(project) }
        return entry.glance?.filters?.first { $0.id == id }.map { .filter($0) } ?? .gone
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
        case .systemMedium: TodayWidgetView(glance: entry.glance, size: .medium, marked: entry.marked, marks: marks)
        case .systemLarge, .systemExtraLarge:
            TodayWidgetView(glance: entry.glance, size: .large, marked: entry.marked, marks: marks)
        default: TodayWidgetView(glance: entry.glance, size: .small)
        }
    }

    /// The button a task's circle becomes when set to complete with one tap.
    private var marks: ((TodayGlance.Row, AnyView) -> AnyView)? {
        #if os(iOS)
            guard entry.completesWithOneTap else { return nil }
            return { row, circle in
                AnyView(Button(intent: MarkDoneIntent(task: row.id)) { circle }.buttonStyle(.plain))
            }
        #else
            nil
        #endif
    }
}
