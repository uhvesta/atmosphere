import Foundation
import LifrCore

/// The order of rows within each section of a list.
package enum TaskSort: String, CaseIterable, Hashable, Sendable {
    /// Higher priority first, then earlier due date, then manual order.
    case smart
    /// The order rows were dragged into.
    case manual
    /// Earliest due first; undated last.
    case due
    /// Highest priority first.
    case priority
    /// Newest first.
    case created
    /// A to Z.
    case title

    package var title: String {
        switch self {
        case .smart: String(localized: "Smart")
        case .manual: String(localized: "Manual")
        case .due: String(localized: "Due Date")
        case .priority: String(localized: "Priority")
        case .created: String(localized: "Date Added")
        case .title: String(localized: "Title")
        }
    }

    /// Whether `lhs` comes before `rhs`. Ties fall back to the smart order.
    package func areInOrder(_ lhs: LifrTask, _ rhs: LifrTask) -> Bool {
        switch self {
        case .smart:
            return LifrTask.listOrder(lhs, rhs)
        case .manual:
            return (lhs.sortOrder, lhs.id.uuidString) < (rhs.sortOrder, rhs.id.uuidString)
        case .due:
            guard lhs.dueDate != rhs.dueDate else { return LifrTask.listOrder(lhs, rhs) }
            guard let left = lhs.dueDate else { return false }
            guard let right = rhs.dueDate else { return true }
            return left < right
        case .priority:
            guard lhs.priority != rhs.priority else { return LifrTask.listOrder(lhs, rhs) }
            return lhs.priority > rhs.priority
        case .created:
            guard lhs.createdAt != rhs.createdAt else { return LifrTask.listOrder(lhs, rhs) }
            return lhs.createdAt > rhs.createdAt
        case .title:
            let order = lhs.title.localizedStandardCompare(rhs.title)
            guard order != .orderedSame else { return LifrTask.listOrder(lhs, rhs) }
            return order == .orderedAscending
        }
    }
}
