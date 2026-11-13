import Foundation
import LifrCore

/// What reconciling synced records changed, in words, so a change nobody
/// made on this device doesn't go unexplained.
package struct SyncBanner: Equatable, Sendable {
    package var messages: [String]
    /// Whether it says sync keeps failing, with Sync Now, rather than what
    /// a sync fixed.
    package var isFailure = false

    /// How many fixes the banner lists before it sums them up instead.
    package static let listed = 3

    /// The sum of what the fixes changed when there are too many to list,
    /// with the list a tap away; nil when they're listed.
    package var summary: String? {
        guard !isFailure, messages.count > Self.listed else { return nil }
        return String(
            AttributedString(localized: "^[\(messages.count) change](inflect: true) were adjusted while syncing")
                .characters)
    }

    /// This banner with `later`'s messages after its own, so fixes from a
    /// sync that came before the person read them stay.
    package func adding(_ later: SyncBanner) -> SyncBanner {
        guard !isFailure, !later.isFailure else { return later }
        var banner = self
        banner.messages += later.messages
        return banner
    }

    package init(failure reason: String) {
        messages = [reason]
        isFailure = true
    }

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
