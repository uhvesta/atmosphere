import Foundation
import LifrCore

/// What reconciling synced records changed, in words, so a change nobody
/// made on this device doesn't go unexplained.
package struct SyncBanner: Equatable, Sendable {
    package var messages: [String]

    package init(_ fixes: [Fix], in workspace: Workspace) {
        messages = fixes.map { fix in
            func title(_ id: UUID) -> String {
                workspace.tasks[id].map { "“\($0.title)”" } ?? String(localized: "A task")
            }
            switch fix {
            case .reopenedParent(let task):
                return String(
                    localized: """
                        \(title(task)) is In Progress again: a Sub task was added on one device \
                        while it was closed on another.
                        """)
            case .reparented(let task, _, let parent?):
                return String(
                    localized: "\(title(task)) moved under \(title(parent)), because its parent became a Sub task.")
            case .reparented(let task, _, nil):
                return String(
                    localized: """
                        \(title(task)) is a task of its own now, because its parents were Sub tasks of each other.
                        """)
            case .alignedWithParent(let task):
                return String(localized: "\(title(task)) moved to its parent's Avenue and Project.")
            case .recoded(let task, let from, let to):
                return String(
                    localized: """
                        \(title(task)) is \(to.rawValue) now, because another task was also \(from.rawValue).
                        """)
            case .suffixedAvenueKey(let avenue, let from, let to):
                let name = workspace.avenues[avenue]?.name ?? String(localized: "An Avenue")
                return String(
                    localized: """
                        \(name)'s key is \(to.rawValue) now, because another Avenue was also \(from.rawValue). \
                        Its tasks' codes changed with it.
                        """)
            }
        }
    }
}
