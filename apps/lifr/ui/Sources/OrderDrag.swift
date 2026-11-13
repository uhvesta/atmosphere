import Foundation

/// An Avenue or Project dragged to a new place in its list, carried as
/// text so a list row can take it. Drops that take tasks or text leave it
/// alone, so it never lands as a task named after it.
package enum OrderDrag: Hashable, Sendable {
    case avenue(UUID)
    case project(UUID)

    private static let avenuePrefix = "lifr-avenue:"
    private static let projectPrefix = "lifr-project:"

    package var text: String {
        switch self {
        case .avenue(let id): Self.avenuePrefix + id.uuidString
        case .project(let id): Self.projectPrefix + id.uuidString
        }
    }

    package init?(_ text: String) {
        if text.hasPrefix(Self.avenuePrefix),
            let id = UUID(uuidString: String(text.dropFirst(Self.avenuePrefix.count)))
        {
            self = .avenue(id)
        } else if text.hasPrefix(Self.projectPrefix),
            let id = UUID(uuidString: String(text.dropFirst(Self.projectPrefix.count)))
        {
            self = .project(id)
        } else {
            return nil
        }
    }

    /// Whether any of `items` is an Avenue or Project being moved.
    package static func isAmong(_ items: [String]) -> Bool {
        items.contains { OrderDrag($0) != nil }
    }
}
