import Foundation

/// What the widgets show: Today's open tasks as Lifr last saw them, and
/// what Today will be from midnight. The app writes it after each change;
/// the widgets only read it.
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
    /// When the day it's Today for starts and ends.
    package var from: Date
    package var until: Date
    /// How many open tasks are overdue or due today.
    package var count: Int
    /// How many of them are overdue.
    package var overdue: Int
    /// Today's tasks in Today's order. Empty while the lock is on, so the
    /// file never holds what the lock hides.
    package var rows: [Row]
    /// Whether the app lock was on.
    package var isLocked: Bool
    /// Today on the days after, as it will be if nothing changes before
    /// then, so the widgets turn over at midnight without the app.
    package var later: [TodayGlance]

    /// Today's tasks for the day from `from` until `until`, or only how
    /// many while `locked`.
    package init(
        asOf: Date, from: Date, until: Date, rows: [Row], locked: Bool, later: [TodayGlance] = []
    ) {
        self.asOf = asOf
        self.from = from
        self.until = until
        count = rows.count
        overdue = rows.filter(\.isOverdue).count
        self.rows = locked ? [] : rows
        isLocked = locked
        self.later = later
    }

    /// Each day it has, in order.
    package var days: [TodayGlance] {
        var today = self
        today.later = []
        return [today] + later.flatMap(\.days)
    }

    /// Today as it is at `date`, or nil once `date` is past every day it
    /// has, when only the app knows.
    package func at(_ date: Date) -> TodayGlance? {
        days.first { $0.from <= date && date < $0.until }
    }
}
