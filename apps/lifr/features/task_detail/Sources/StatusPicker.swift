import ComposableArchitecture
import Foundation
import LifrCore
import LifrUI

/// A status picked for a task from its checkbox or Status swipe.
package struct StatusPick: Equatable, Sendable {
    package var task: UUID
    package var status: TaskStatus

    package init(task: UUID, status: TaskStatus) {
        self.task = task
        self.status = status
    }
}

extension ConfirmationDialogState where Action == StatusPick {
    /// Every status but the task's own: for an open task, the closing ones
    /// first; for a closed one, those that open it.
    package static func statuses(for task: UUID, title: String, status: TaskStatus) -> Self {
        let order: [TaskStatus] =
            status.isClosed
            ? [.todo, .inProgress, .procrastinated, .done, .wontDo]
            : [.done, .wontDo, .inProgress, .todo, .procrastinated]
        return ConfirmationDialogState(titleVisibility: .visible) {
            TextState(title)
        } actions: {
            for next in order where next != status {
                ButtonState(action: StatusPick(task: task, status: next)) { TextState(next.title) }
            }
            ButtonState(role: .cancel) { TextState("Cancel") }
        } message: {
            TextState("Currently \(status.title).")
        }
    }
}
