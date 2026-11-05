import Foundation

/// What the widgets show: Today's open tasks as Lifr last saw them. The
/// app writes it after each change; the widgets only read it.
package struct TodayGlance: Codable, Hashable, Sendable {
    /// One of Today's tasks.
    package struct Row: Codable, Hashable, Sendable, Identifiable {
        package var id: UUID
        package var title: String
        /// Its code, such as HEALTH-0004.
        package var code: String
        package var isOverdue: Bool

        package init(id: UUID, title: String, code: String, isOverdue: Bool) {
            self.id = id
            self.title = title
            self.code = code
            self.isOverdue = isOverdue
        }
    }

    /// When Lifr wrote it.
    package var asOf: Date
    /// How many open tasks are overdue or due today.
    package var count: Int
    /// How many of them are overdue.
    package var overdue: Int
    /// Today's tasks in Today's order. Empty while the lock is on, so the
    /// file never holds what the lock hides.
    package var rows: [Row]
    /// Whether the app lock was on.
    package var isLocked: Bool

    /// Today's tasks, or only how many while `locked`.
    package init(asOf: Date, rows: [Row], locked: Bool) {
        self.asOf = asOf
        count = rows.count
        overdue = rows.filter(\.isOverdue).count
        self.rows = locked ? [] : rows
        isLocked = locked
    }
}
