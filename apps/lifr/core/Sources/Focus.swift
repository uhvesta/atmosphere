import Foundation

/// The Avenues a Focus limits Lifr to, as the Focus filter set them, kept
/// in this device's settings. None means every Avenue shows.
package struct FocusAvenues: Hashable, Sendable, RawRepresentable {
    package var ids: Set<UUID>

    package init(_ ids: Set<UUID> = []) {
        self.ids = ids
    }

    /// The IDs, comma-separated and sorted, so the same Avenues always
    /// read the same.
    package var rawValue: String {
        ids.map(\.uuidString).sorted().joined(separator: ",")
    }

    /// Ignores any piece that isn't an ID.
    package init(rawValue: String) {
        ids = Set(rawValue.split(separator: ",").compactMap { UUID(uuidString: String($0)) })
    }

    /// The settings key the Focus filter writes and the app reads.
    package static let key = "lifrFocusAvenues"
}

extension Workspace {
    /// Only the Avenues in `focus`, with their Projects and tasks and those
    /// tasks' comments and reminders: what Lifr shows while a Focus limits
    /// it. The records stay as they are; this is a view of them. With no
    /// Avenues in `focus`, it's every record.
    package func focused(on focus: FocusAvenues) -> Workspace {
        guard !focus.ids.isEmpty else { return self }
        var workspace = self
        workspace.avenues = avenues.filter { focus.ids.contains($0.key) }
        workspace.projects = projects.filter { focus.ids.contains($0.value.avenueID) }
        workspace.tasks = tasks.filter { focus.ids.contains($0.value.avenueID) }
        workspace.comments = comments.filter { workspace.tasks[$0.value.taskID] != nil }
        workspace.reminders = reminders.filter { workspace.tasks[$0.value.taskID] != nil }
        return workspace
    }
}
