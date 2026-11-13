import Foundation

#if os(iOS)
    import ActivityKit
#endif

/// What the Live Activity for a task the person is focusing on shows.
package struct FocusedTask: Codable, Hashable, Sendable {
    /// Its title; empty while the app lock is on, so the lock screen never
    /// shows what the lock hides.
    package var title: String
    /// Its code, such as HEALTH-0004.
    package var code: String
    /// How many of its Sub tasks are closed, of how many.
    package var closed: Int
    package var total: Int
    /// Whether it was marked Done from the Live Activity.
    package var isDone: Bool

    package init(title: String, code: String, closed: Int, total: Int, isDone: Bool = false) {
        self.title = title
        self.code = code
        self.closed = closed
        self.total = total
        self.isDone = isDone
    }
}

#if os(iOS)
    /// A task the person is focusing on, on the lock screen and in the
    /// Dynamic Island.
    package struct FocusAttributes: ActivityAttributes {
        package typealias ContentState = FocusedTask

        package var taskID: UUID

        package init(taskID: UUID) {
            self.taskID = taskID
        }
    }
#endif
