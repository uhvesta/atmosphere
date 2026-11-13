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
    /// Every status but the task's own, always in the same order so the
    /// hand learns where each one is. A Procrastinated task offers Restart
    /// in place of In Progress, as its swipe does.
    package static func statuses(for task: UUID, title: String, status: TaskStatus) -> Self {
        ConfirmationDialogState(titleVisibility: .visible) {
            TextState(title)
        } actions: {
            for next in TaskStatus.pickOrder where next != status {
                ButtonState(action: StatusPick(task: task, status: next)) {
                    next == .inProgress && status == .procrastinated ? TextState("Restart") : TextState(next.title)
                }
            }
            ButtonState(role: .cancel) { TextState("Cancel") }
        } message: {
            TextState("Currently \(status.title).")
        }
    }
}
