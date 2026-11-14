import Foundation
import LifrCore
import LifrUI

/// How a list that offers grouping puts its rows in sections, in place of
/// the sections its query makes.
package enum TaskGroup: String, CaseIterable, Hashable, Sendable {
    /// The list's own sections.
    case list
    /// One section per status, in the order a status picker lists them.
    case status
    /// One section per Avenue, in the Avenues' order.
    case avenue
    /// Most urgent first, then those with no priority.
    case priority
    /// Overdue, today, the next 7 days, later, then those with no due date.
    case due
    /// One section per label, by name: a task with several goes under the
    /// first. Then those with none.
    case label

    package var title: String {
        switch self {
        case .list: String(localized: "None")
        case .status: String(localized: "Status")
        case .avenue: String(localized: "Avenue")
        case .priority: String(localized: "Priority")
        case .due: String(localized: "Due Date")
        case .label: String(localized: "Label")
        }
    }

    /// `sections`' rows in this grouping's sections, keeping each row's
    /// place among the rest. Empty sections are left out.
    func regroup(_ sections: [TaskSection], in workspace: Workspace, today: CalendarDate) -> [TaskSection] {
        guard self != .list else { return sections }
        let rows = sections.flatMap(\.rows)
        let notes = sections.reduce(into: [UUID: String]()) { $0.merge($1.notes) { first, _ in first } }
        var keyed: [Key: [TaskRowData]] = [:]
        for row in rows { keyed[key(of: row, in: workspace, today: today), default: []].append(row) }
        return keyed.keys.sorted().compactMap { key in
            guard let rows = keyed[key] else { return nil }
            return TaskSection(
                id: "\(rawValue)-\(key.id)", title: key.title, rows: rows,
                notes: notes.filter { id, _ in rows.contains { $0.id == id } })
        }
    }

    /// A section's place, identity and title.
    private struct Key: Hashable, Comparable {
        var rank: Int
        var name: String
        var id: String
        var title: String

        static func < (lhs: Self, rhs: Self) -> Bool {
            (lhs.rank, lhs.name, lhs.id) < (rhs.rank, rhs.name, rhs.id)
        }
    }

    private func key(of row: TaskRowData, in workspace: Workspace, today: CalendarDate) -> Key {
        switch self {
        case .list, .status:
            let rank = TaskStatus.pickOrder.firstIndex(of: row.status) ?? 0
            return Key(rank: rank, name: "", id: row.status.rawValue, title: row.status.title)
        case .avenue:
            guard let task = workspace.tasks[row.id], let avenue = workspace.avenues[task.avenueID] else {
                return Key(rank: 1, name: "", id: "none", title: String(localized: "No Avenue"))
            }
            return Key(rank: 0, name: avenue.sortOrder, id: avenue.id.uuidString, title: avenue.name)
        case .priority:
            let title = row.priority.title ?? String(localized: "No Priority")
            let rank = -(Priority.allCases.firstIndex(of: row.priority) ?? 0)
            return Key(rank: rank, name: "", id: row.priority.rawValue, title: title)
        case .due:
            switch row.due {
            case .overdue?: return Key(rank: 0, name: "", id: "overdue", title: String(localized: "Overdue"))
            case .today?: return Key(rank: 1, name: "", id: "today", title: String(localized: "Today"))
            case .later(let date, _)? where date <= today.adding(days: 7):
                return Key(rank: 2, name: "", id: "week", title: String(localized: "Next 7 Days"))
            case .later?: return Key(rank: 3, name: "", id: "later", title: String(localized: "Later"))
            case nil: return Key(rank: 4, name: "", id: "none", title: String(localized: "No Due Date"))
            }
        case .label:
            guard let label = row.labels.first else {
                return Key(rank: 1, name: "", id: "none", title: String(localized: "No Label"))
            }
            return Key(rank: 0, name: label.name.lowercased(), id: label.name, title: "#\(label.name)")
        }
    }
}
