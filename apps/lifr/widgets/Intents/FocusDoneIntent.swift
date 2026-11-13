#if os(iOS)
    import ActivityKit
    import AppIntents
    import Foundation
    import LifrGlance

    /// Done on the Live Activity. It runs in Lifr, which marks the task for
    /// saving as Done the next time it's open, as the widgets' marks are,
    /// and ends the activity.
    ///
    /// Built into both the app and the widgets, as App Intents needs: the
    /// widgets' button names it, and the app runs it.
    struct FocusDoneIntent: LiveActivityIntent {
        static let title: LocalizedStringResource = "Mark the Task You're Focusing On Done"
        static let isDiscoverable = false

        @Parameter(title: "Task")
        var task: String

        init() {}

        init(task: UUID) {
            self.task = task.uuidString
        }

        func perform() async throws -> some IntentResult {
            guard let id = UUID(uuidString: task) else { return .result() }
            guard let file = MarksFile.shared else { throw MarksError.noAppGroup }
            try file.mark(id)
            for activity in Activity<FocusAttributes>.activities where activity.attributes.taskID == id {
                var state = activity.content.state
                state.isDone = true
                await activity.end(ActivityContent(state: state, staleDate: nil), dismissalPolicy: .default)
            }
            return .result()
        }
    }
#endif
