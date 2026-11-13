import Foundation

/// What the widgets show: Today's open tasks as Lifr last saw them, and
/// what Today will be on each of the next two weeks. The app writes it after each change;
/// the widgets only read it.
package struct TodayGlance: Codable, Hashable, Sendable {
    /// One of Today's tasks.
    package struct Row: Codable, Hashable, Sendable, Identifiable {
        package var id: UUID
        package var title: String
        /// Its code, such as HEALTH-0004.
        package var code: String
        package var isOverdue: Bool
        /// Whether it's In Progress.
        package var isInProgress: Bool
        /// The time it's due, as minutes after midnight, if it has one.
        package var dueMinutes: Int?
        /// Its Avenue's color, by name, such as "teal".
        package var color: String?

        package init(
            id: UUID, title: String, code: String, isOverdue: Bool, isInProgress: Bool = false,
            dueMinutes: Int? = nil, color: String? = nil
        ) {
            self.id = id
            self.title = title
            self.code = code
            self.isOverdue = isOverdue
            self.isInProgress = isInProgress
            self.dueMinutes = dueMinutes
            self.color = color
        }

        private enum CodingKeys: String, CodingKey {
            case id, title, code, isOverdue, isInProgress, dueMinutes, color
        }

        /// Reads a row, including one written before rows had a status, a
        /// time and a color.
        package init(from decoder: any Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            id = try container.decode(UUID.self, forKey: .id)
            title = try container.decode(String.self, forKey: .title)
            code = try container.decode(String.self, forKey: .code)
            isOverdue = try container.decode(Bool.self, forKey: .isOverdue)
            isInProgress = try container.decodeIfPresent(Bool.self, forKey: .isInProgress) ?? false
            dueMinutes = try container.decodeIfPresent(Int.self, forKey: .dueMinutes)
            color = try container.decodeIfPresent(String.self, forKey: .color)
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
    /// Each open Avenue in the sidebar's order, or nil from a Lifr that
    /// didn't write them yet.
    package var avenues: [AvenueGlance]?
    /// Each active Project, Avenue by Avenue, in the same shape as an
    /// Avenue, or nil from a Lifr that didn't write them yet.
    package var projects: [AvenueGlance]?
    /// How many open tasks are due on each day of this month and the
    /// next, by YYYY-MM-DD, or nil from a Lifr that didn't write them yet.
    package var dueCounts: [String: Int]?

    /// Today's tasks for the day from `from` until `until`, or only how
    /// many while `locked`.
    package init(
        asOf: Date, from: Date, until: Date, rows: [Row], locked: Bool, later: [TodayGlance] = [],
        avenues: [AvenueGlance]? = nil, projects: [AvenueGlance]? = nil, dueCounts: [String: Int]? = nil
    ) {
        self.asOf = asOf
        self.from = from
        self.until = until
        count = rows.count
        overdue = rows.filter(\.isOverdue).count
        self.rows = locked ? [] : rows
        isLocked = locked
        self.later = later
        self.avenues = avenues.map { $0.map { locked ? $0.locked : $0 } }
        self.projects = projects.map { $0.map { locked ? $0.locked : $0 } }
        self.dueCounts = dueCounts
    }

    /// Each day it has, in order, each with the Avenues and due counts as
    /// they were written.
    package var days: [TodayGlance] {
        var today = self
        today.later = []
        return [today]
            + later.flatMap(\.days).map { day in
                var day = day
                day.avenues = day.avenues ?? avenues
                day.projects = day.projects ?? projects
                day.dueCounts = day.dueCounts ?? dueCounts
                return day
            }
    }

    /// The tasks due on its day itself, without those overdue from before.
    package var dueRows: [Row] {
        rows.filter { !$0.isOverdue }
    }

    /// How many tasks are due on its day itself, kept while the lock is on.
    package var dueCount: Int {
        count - overdue
    }

    /// The day `date` falls in and those after it, or none once `date` is
    /// past every day it has.
    package func days(from date: Date) -> [TodayGlance] {
        days.filter { $0.until > date }
    }

    /// Today as it is at `date`, or nil once `date` is past every day it
    /// has, when only the app knows.
    package func at(_ date: Date) -> TodayGlance? {
        days.first { $0.from <= date && date < $0.until }
    }
}

/// An Avenue or a Project in the widgets: how many open tasks it has and
/// the first few.
package struct AvenueGlance: Codable, Hashable, Sendable, Identifiable {
    package var id: UUID
    package var name: String
    /// An SF Symbol name.
    package var symbol: String
    /// Its color, by name, such as "teal".
    package var color: String
    /// How many open tasks it has.
    package var count: Int
    /// How many of them are overdue.
    package var overdue: Int
    /// Its first open tasks in the list's order. Empty while the lock is on.
    package var rows: [TodayGlance.Row]

    package init(
        id: UUID, name: String, symbol: String, color: String, count: Int, overdue: Int, rows: [TodayGlance.Row]
    ) {
        self.id = id
        self.name = name
        self.symbol = symbol
        self.color = color
        self.count = count
        self.overdue = overdue
        self.rows = rows
    }

    /// The same, with only how many.
    var locked: AvenueGlance {
        var avenue = self
        avenue.rows = []
        return avenue
    }
}
