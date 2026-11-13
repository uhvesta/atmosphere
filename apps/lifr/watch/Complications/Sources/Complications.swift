import LifrGlance
import SwiftUI
import WidgetKit

@main
struct LifrComplications: WidgetBundle {
    var body: some Widget {
        TodayCountComplication()
        NextTaskComplication()
    }
}

/// Today as the Watch kept it from the iPhone, at one moment.
struct ComplicationEntry: TimelineEntry {
    var date: Date
    /// Nil before the iPhone sent Today, or once the days it sent are past.
    var today: TodayGlance?
}

/// An entry at the start of each day the iPhone sent, so the face turns
/// over at midnight; the Watch asks again when the iPhone sends more.
struct ComplicationTimeline: TimelineProvider {
    func placeholder(in context: Context) -> ComplicationEntry {
        ComplicationEntry(date: .now, today: nil)
    }

    func getSnapshot(in context: Context, completion: @escaping (ComplicationEntry) -> Void) {
        completion(ComplicationEntry(date: .now, today: Self.read()?.at(.now)))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<ComplicationEntry>) -> Void) {
        let now = Date.now
        let days = Self.read()?.days(from: now) ?? []
        var entries = days.map { ComplicationEntry(date: max($0.from, now), today: $0) }
        // After the last day sent, until the iPhone sends again.
        entries.append(ComplicationEntry(date: max(days.last?.until ?? now, now), today: nil))
        completion(Timeline(entries: entries, policy: .never))
    }

    private static func read() -> TodayGlance? {
        try? GlanceFile.shared?.read()
    }
}

extension WidgetFamily {
    // The Mac has none of the Watch's families; the strings build compiles
    // these there.
    #if os(watchOS)
        fileprivate static let count: [Self] = [.accessoryCircular, .accessoryCorner, .accessoryInline]
        fileprivate static let nextTask: [Self] = [.accessoryRectangular]
    #else
        fileprivate static let count: [Self] = []
        fileprivate static let nextTask: [Self] = []
    #endif
}

/// How many tasks are due today or overdue.
struct TodayCountComplication: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "lifr.watch.count", provider: ComplicationTimeline()) { entry in
            CountView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Today")
        .description("How many tasks are due today or overdue.")
        .supportedFamilies(WidgetFamily.count)
    }
}

/// Today's first open task.
struct NextTaskComplication: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "lifr.watch.next", provider: ComplicationTimeline()) { entry in
            NextTaskView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Next Task")
        .description("Today's first open task, and how many are left.")
        .supportedFamilies(WidgetFamily.nextTask)
    }
}

private struct CountView: View {
    @Environment(\.widgetFamily) private var family
    var entry: ComplicationEntry

    var body: some View {
        if let today = entry.today {
            switch family {
            #if os(watchOS)
                case .accessoryInline:
                    Text("^[\(today.count) task](inflect: true) today")
                case .accessoryCorner:
                    Image(systemName: "checklist")
                        .widgetLabel { Text("^[\(today.count) task](inflect: true)") }
            #endif
            default:
                VStack(spacing: 0) {
                    Text(today.count, format: .number)
                        .font(.title2.bold())
                    Image(systemName: today.overdue > 0 ? "exclamationmark.circle" : "checklist")
                        .font(.caption2)
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(Text("^[\(today.count) task](inflect: true) today"))
            }
        } else {
            Image(systemName: "checklist")
                .accessibilityLabel(Text("Open Lifr on the iPhone"))
        }
    }
}

private struct NextTaskView: View {
    var entry: ComplicationEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Label("Lifr", systemImage: "checklist")
                .font(.headline)
                .widgetAccentable()
            if let today = entry.today {
                if today.isLocked {
                    Text("^[\(today.count) task](inflect: true) today")
                    Text("Locked")
                        .foregroundStyle(.secondary)
                } else if let next = today.rows.first {
                    Text(next.title)
                        .lineLimit(2)
                    if today.count > 1 {
                        Text("^[\(today.count - 1) more](inflect: true)")
                            .foregroundStyle(.secondary)
                    }
                } else {
                    Text("Nothing due today")
                        .foregroundStyle(.secondary)
                }
            } else {
                Text("Open Lifr on the iPhone")
                    .foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
