#if os(iOS)
    import ActivityKit
    import AppIntents
    import LifrCore
    import LifrGlance
    import SwiftUI
    import WidgetKit

    /// The task the person is focusing on, on the lock screen and in the
    /// Dynamic Island: its title, unless the lock is on, its code and how
    /// many Sub tasks are closed, with a named Done button.
    struct FocusActivityWidget: Widget {
        var body: some WidgetConfiguration {
            ActivityConfiguration(for: FocusAttributes.self) { context in
                FocusLockScreenView(task: context.attributes.taskID, focus: context.state)
                    .padding()
                    .activityBackgroundTint(nil)
            } dynamicIsland: { context in
                DynamicIsland {
                    DynamicIslandExpandedRegion(.leading) {
                        Image(systemName: "scope")
                            .foregroundStyle(.orange)
                            .accessibilityHidden(true)
                    }
                    DynamicIslandExpandedRegion(.trailing) {
                        FocusProgressView(focus: context.state)
                    }
                    DynamicIslandExpandedRegion(.center) {
                        FocusTitleView(focus: context.state)
                    }
                    DynamicIslandExpandedRegion(.bottom) {
                        FocusDoneButton(task: context.attributes.taskID, focus: context.state)
                    }
                } compactLeading: {
                    Image(systemName: "scope")
                        .foregroundStyle(.orange)
                        .accessibilityLabel("Focusing")
                } compactTrailing: {
                    FocusProgressView(focus: context.state)
                } minimal: {
                    Image(systemName: context.state.isDone ? "checkmark" : "scope")
                        .foregroundStyle(.orange)
                        .accessibilityLabel("Focusing")
                }
                .widgetURL(DeepLink.task(context.attributes.taskID, picksStatus: false).url)
            }
        }
    }

    /// The lock screen's banner.
    private struct FocusLockScreenView: View {
        var task: UUID
        var focus: FocusedTask

        var body: some View {
            HStack(alignment: .center, spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    FocusTitleView(focus: focus)
                    FocusProgressView(focus: focus)
                }
                Spacer(minLength: 0)
                FocusDoneButton(task: task, focus: focus)
            }
            .widgetURL(DeepLink.task(task, picksStatus: false).url)
        }
    }

    /// The title, or "A task" while the lock is on, with the code.
    private struct FocusTitleView: View {
        var focus: FocusedTask

        var body: some View {
            VStack(alignment: .leading, spacing: 2) {
                Text(focus.code)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                if focus.title.isEmpty {
                    Text("A task")
                        .font(.headline)
                } else {
                    Text(focus.title)
                        .font(.headline)
                        .lineLimit(2)
                }
            }
            .accessibilityElement(children: .combine)
        }
    }

    /// How many Sub tasks are closed, when it has any.
    private struct FocusProgressView: View {
        var focus: FocusedTask

        var body: some View {
            if focus.total > 0 {
                Text("\(focus.closed)/\(focus.total)")
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("\(focus.closed) of \(focus.total) Sub tasks closed")
            }
        }
    }

    /// Marks the task Done, saved when Lifr next opens; once it's marked,
    /// says so instead.
    private struct FocusDoneButton: View {
        var task: UUID
        var focus: FocusedTask

        var body: some View {
            if focus.isDone {
                Label("Done", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
            } else {
                Button(intent: FocusDoneIntent(task: task)) {
                    Label("Done", systemImage: "checkmark.circle")
                }
                .tint(.orange)
            }
        }
    }
#endif
