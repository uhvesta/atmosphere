import Foundation
import LifrCore
import LifrUI

/// Which tasks a list shows, in which sections. Each screen defines its own,
/// such as Today's overdue and due today.
package struct TaskQuery: Equatable, Sendable {
    /// What a list shows when the query matches nothing.
    package struct Empty: Equatable, Sendable {
        package var title: String
        package var systemImage: String
        package var description: String

        package init(_ title: String, systemImage: String, description: String) {
            self.title = title
            self.systemImage = systemImage
            self.description = description
        }
    }

    /// Identifies the query, so two queries with the same key are equal.
    package var key: String
    package var empty: Empty
    /// The actions a row offers when swiped left.
    package var trailingActions: [RowAction]
    package var sections: @Sendable (_ workspace: Workspace, _ now: Date, _ timeZone: TimeZone) -> [TaskSection]

    package init(
        key: String, empty: Empty,
        trailingActions: [RowAction] = [.status(.done), .status(.procrastinated), .status(.wontDo)],
        sections: @escaping @Sendable (Workspace, Date, TimeZone) -> [TaskSection]
    ) {
        self.key = key
        self.empty = empty
        self.trailingActions = trailingActions
        self.sections = sections
    }

    package static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.key == rhs.key && lhs.empty == rhs.empty && lhs.trailingActions == rhs.trailingActions
    }
}

/// Something a row offers besides opening the task.
package enum RowAction: Hashable, Sendable {
    case status(TaskStatus)
    /// Pick a new due date.
    case reschedule
}

/// A titled group of rows.
package struct TaskSection: Identifiable, Hashable, Sendable {
    package var id: String
    package var title: String
    package var rows: [TaskRowData]
    /// A line under a row, such as why it's Procrastinated.
    package var notes: [UUID: String]

    package init(id: String, title: String, rows: [TaskRowData], notes: [UUID: String] = [:]) {
        self.id = id
        self.title = title
        self.rows = rows
        self.notes = notes
    }
}

extension Workspace {
    /// Tasks that are in the trash or in an archived Avenue appear in no list.
    package var listedTasks: [LifrTask] {
        tasks.values
            .filter { $0.deletedAt == nil && avenues[$0.avenueID]?.archivedAt == nil }
            .sorted(by: LifrTask.listOrder)
    }

    /// `tasks` as rows, as of `now`.
    package func rows(_ tasks: [LifrTask], now: Date, timeZone: TimeZone) -> [TaskRowData] {
        tasks.map { TaskRowData($0, in: self, now: now, timeZone: timeZone) }
    }
}
