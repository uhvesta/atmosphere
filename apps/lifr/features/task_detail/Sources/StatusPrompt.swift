import ComposableArchitecture
import Foundation
import LifrCore

/// The answers to a status change that needs the person's OK first.
package enum StatusPrompt: Equatable, Sendable {
    /// Open the task to see its open Sub tasks.
    case showSubtasks(task: UUID)
    /// Close the open Sub tasks as Won't Do, then the task as Done.
    case finishDroppingSubtasks(task: UUID)
    /// Close the task and its open Sub tasks as Won't Do.
    case dropWithSubtasks(task: UUID)

    /// The change the answer asks for, or nil when it only shows something.
    package var retry: (status: TaskStatus, task: UUID)? {
        switch self {
        case .showSubtasks: nil
        case .finishDroppingSubtasks(let task): (.done, task)
        case .dropWithSubtasks(let task): (.wontDo, task)
        }
    }
}

extension ConfirmationDialogState where Action == StatusPrompt {
    /// The prompt for a change Core refused until the person confirms, or
    /// nil for other errors. `canShow` offers "Show them" for lists, where
    /// the Sub tasks aren't on screen.
    /// `title` looks up a Sub task's title by ID.
    package static func forRefusal(
        _ error: any Error, canShow: Bool, title: (UUID) -> String?
    ) -> Self? {
        switch error as? LifrError {
        case .openSubtasks(let id, let open)?:
            let titles = open.compactMap(title)
            return ConfirmationDialogState(titleVisibility: .visible) {
                TextState("\(titles.count) Sub task(s) are still open")
            } actions: {
                if canShow {
                    ButtonState(action: .showSubtasks(task: id)) { TextState("Show Them") }
                }
                ButtonState(role: .destructive, action: .finishDroppingSubtasks(task: id)) {
                    TextState("Won't Do + Finish")
                }
                ButtonState(role: .cancel) { TextState("Cancel") }
            } message: {
                TextState(
                    "\(titles.map { "\"\($0)\"" }.joined(separator: ", ")) still open. A task can be Done only "
                        + "when its Sub tasks are closed. Won't Do + Finish closes them as Won't Do first.")
            }
        case .wontDoNeedsCascade(let id, let open)?:
            return ConfirmationDialogState(titleVisibility: .visible) {
                TextState("Close \(open.count) open Sub task(s) too?")
            } actions: {
                ButtonState(role: .destructive, action: .dropWithSubtasks(task: id)) {
                    TextState("Mark All Won't Do")
                }
                ButtonState(role: .cancel) { TextState("Cancel") }
            } message: {
                TextState("A closed task can't keep open Sub tasks, so they'll be marked Won't Do too.")
            }
        default:
            return nil
        }
    }
}

extension AlertState where Action == Never {
    /// A change failed for a reason the person can't confirm away.
    package static func failed(_ title: String, _ error: any Error) -> Self {
        AlertState {
            TextState(title)
        } actions: {
            ButtonState(role: .cancel) { TextState("OK") }
        } message: {
            TextState((error as? LifrError)?.description ?? error.localizedDescription)
        }
    }
}
