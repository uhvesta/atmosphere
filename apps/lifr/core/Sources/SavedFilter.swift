import Foundation

/// Which tasks a search or a saved filter shows. Every word of `text` must
/// appear in a task's title, details or one of its comments, and the task
/// must pass every filter that's set.
package struct TaskFilter: Hashable, Codable, Sendable {
    /// Which due dates match.
    package enum Due: String, CaseIterable, Hashable, Codable, Sendable {
        case any, overdue, today, nextSevenDays, none
    }

    package var text = ""
    /// Empty means every open status.
    package var statuses: Set<TaskStatus> = []
    /// Empty means any priority.
    package var priorities: Set<Priority> = []
    /// Label names; a task needs at least one of them. Empty means any.
    package var labels: Set<String> = []
    /// Avenue IDs; a task must be in one of them. Empty means any.
    package var avenues: Set<UUID> = []
    package var due = Due.any

    package init(
        text: String = "", statuses: Set<TaskStatus> = [], priorities: Set<Priority> = [],
        labels: Set<String> = [], avenues: Set<UUID> = [], due: Due = .any
    ) {
        self.text = text
        self.statuses = statuses
        self.priorities = priorities
        self.labels = labels
        self.avenues = avenues
        self.due = due
    }

    /// No text and no filters: it matches nothing.
    package var isEmpty: Bool { self == TaskFilter() }

    /// How many filters besides the text are set.
    package var filterCount: Int {
        [!statuses.isEmpty, !priorities.isEmpty, !labels.isEmpty, !avenues.isEmpty, due != .any].count { $0 }
    }

    /// The filter as text, the same each run.
    package var key: String {
        [
            text, statuses.map(\.rawValue).sorted().joined(separator: ","),
            priorities.map { "\($0)" }.sorted().joined(separator: ","), labels.sorted().joined(separator: ","),
            avenues.map(\.uuidString).sorted().joined(separator: ","), due.rawValue,
        ].joined(separator: "|")
    }

    package func matches(_ task: LifrTask, in workspace: Workspace, now: Date, timeZone: TimeZone) -> Bool {
        matches(task, Prepared(self, workspace, now: now, timeZone: timeZone, only: [task.id]))
    }

    /// The listed tasks that match, in list order. What every task is
    /// checked against is worked out once, not once a task.
    package func matching(in workspace: Workspace, now: Date, timeZone: TimeZone) -> [LifrTask] {
        let prepared = Prepared(self, workspace, now: now, timeZone: timeZone, only: nil)
        return workspace.listedTasks.filter { matches($0, prepared) }
    }

    /// What a run checks every task against.
    private struct Prepared {
        let workspace: Workspace
        let now: Date
        let today: CalendarDate
        let time: TimeOfDay
        let words: [Substring]
        /// Each task's comments, when there are words to look for.
        let comments: [UUID: [String]]

        init(_ filter: TaskFilter, _ workspace: Workspace, now: Date, timeZone: TimeZone, only: Set<UUID>?) {
            self.workspace = workspace
            self.now = now
            today = CalendarDate(now, in: timeZone)
            time = TimeOfDay(now, in: timeZone)
            words = filter.text.lowercased().split(whereSeparator: \.isWhitespace)
            var comments: [UUID: [String]] = [:]
            if !words.isEmpty {
                for comment in workspace.comments.values
                where comment.deletedAt == nil && only?.contains(comment.taskID) != false {
                    comments[comment.taskID, default: []].append(comment.body)
                }
            }
            self.comments = comments
        }
    }

    private func matches(_ task: LifrTask, _ prepared: Prepared) -> Bool {
        let workspace = prepared.workspace
        let status = task.effectiveStatus(now: prepared.now, settings: workspace.settings)
        guard statuses.isEmpty ? !status.isClosed : statuses.contains(status) else { return false }
        guard priorities.isEmpty || priorities.contains(task.priority) else { return false }
        guard avenues.isEmpty || avenues.contains(task.avenueID) else { return false }
        if !labels.isEmpty {
            guard task.labelIDs.contains(where: { workspace.labels[$0].map { labels.contains($0.name) } == true })
            else { return false }
        }
        let today = prepared.today
        switch due {
        case .any: break
        case .overdue: guard task.isOverdue(today: today, time: prepared.time) else { return false }
        case .today: guard task.dueDate == today else { return false }
        case .nextSevenDays:
            let last = today.adding(days: 7)
            guard let date = task.dueDate, date >= today, date <= last else { return false }
        case .none: guard task.dueDate == nil else { return false }
        }
        guard !prepared.words.isEmpty else { return true }
        let haystack = ([task.title, task.details] + (prepared.comments[task.id] ?? [])).joined(separator: "\n")
            .lowercased()
        return prepared.words.allSatisfy { haystack.contains($0) }
    }
}

/// A search kept under a name, listed with the Avenues.
package struct SavedFilter: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    package var name: String
    /// An SF Symbol name.
    package var symbol: String
    package var filter: TaskFilter
    package var sortOrder: String
    package var createdAt: Date

    /// The symbol a new saved filter has.
    package static let defaultSymbol = "line.3.horizontal.decrease.circle"

    package init(
        id: UUID, name: String, symbol: String, filter: TaskFilter, sortOrder: String, createdAt: Date
    ) {
        self.id = id
        self.name = name
        self.symbol = symbol
        self.filter = filter
        self.sortOrder = sortOrder
        self.createdAt = createdAt
    }
}

extension Workspace {
    /// Saved filters in the order people put them.
    package var sortedSavedFilters: [SavedFilter] {
        savedFilters.values.sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
    }

    /// Saves `filter` under `name`, last in the list. An empty filter is
    /// refused: it would show nothing.
    package mutating func saveFilter(
        _ filter: TaskFilter, name: String, symbol: String, in context: Context
    ) throws(LifrError) -> SavedFilter {
        guard !filter.isEmpty else { throw .emptyFilter }
        let saved = SavedFilter(
            id: context.newID(), name: try Self.validName(name), symbol: symbol, filter: filter,
            sortOrder: SortKey.between(sortedSavedFilters.last?.sortOrder, nil), createdAt: context.now)
        savedFilters[saved.id] = saved
        return saved
    }

    /// Renames a saved filter, gives it another symbol or another filter.
    package mutating func edit(
        savedFilter id: UUID, name: String, symbol: String, filter: TaskFilter
    ) throws(LifrError) {
        guard var saved = savedFilters[id] else { throw .notFound(.savedFilter, id.uuidString) }
        guard !filter.isEmpty else { throw .emptyFilter }
        saved.name = try Self.validName(name)
        saved.symbol = symbol
        saved.filter = filter
        savedFilters[id] = saved
    }

    package mutating func delete(savedFilter id: UUID) throws(LifrError) {
        guard savedFilters[id] != nil else { throw .notFound(.savedFilter, id.uuidString) }
        savedFilters[id] = nil
    }

    /// Puts a saved filter between two others.
    package mutating func reorder(savedFilter id: UUID, after lower: UUID?, before upper: UUID?) throws(LifrError) {
        func filter(_ id: UUID) throws(LifrError) -> SavedFilter {
            guard let saved = savedFilters[id] else { throw .notFound(.savedFilter, id.uuidString) }
            return saved
        }
        var saved = try filter(id)
        saved.sortOrder = Self.key(
            after: try lower.map(filter)?.sortOrder, before: try upper.map(filter)?.sortOrder)
        savedFilters[id] = saved
    }

    /// Saved filters that name a label by `old` name it by `new` instead,
    /// or drop it when `new` is nil.
    mutating func renameLabel(inSavedFilters old: String, to new: String?) {
        for (id, saved) in savedFilters where saved.filter.labels.contains(old) {
            savedFilters[id]?.filter.labels.remove(old)
            if let new { savedFilters[id]?.filter.labels.insert(new) }
        }
    }
}
