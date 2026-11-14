import Foundation

/// Tasks dragged out of a list, carried as text: one task's ID, or the IDs
/// of every picked task one per line, in the order the list shows them.
package enum TaskDrag {
    /// The text that carries `ids`.
    package static func text(_ ids: [UUID]) -> String {
        ids.map(\.uuidString).joined(separator: "\n")
    }

    /// The tasks `items` carry, or nil when any line of them isn't a
    /// task's ID, such as text dragged from another app.
    package static func ids(in items: [String]) -> [UUID]? {
        let lines = items.flatMap { $0.split(whereSeparator: \.isNewline) }
        guard !lines.isEmpty else { return nil }
        let ids = lines.compactMap { UUID(uuidString: String($0)) }
        return ids.count == lines.count ? ids : nil
    }
}
