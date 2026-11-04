import ComposableArchitecture
import Foundation
import LifrCore
import LifrTaskList
import LifrUI

/// What Search looks for. Every word of `text` must appear in a task's
/// title, details or one of its comments, and the task must pass every
/// filter that's set.
package struct SearchFilter: Hashable, Sendable {
    /// Which due dates match.
    package enum Due: String, CaseIterable, Hashable, Sendable {
        case any, overdue, today, nextSevenDays, none

        package var title: String {
            switch self {
            case .any: "Any date"
            case .overdue: "Overdue"
            case .today: "Today"
            case .nextSevenDays: "Next 7 days"
            case .none: "No due date"
            }
        }
    }

    package var text = ""
    /// Empty means every open status.
    package var statuses: Set<TaskStatus> = []
    /// Empty means any priority.
    package var priorities: Set<Priority> = []
    /// Label names; a task needs at least one of them. Empty means any.
    package var labels: Set<String> = []
    package var due = Due.any

    package init() {}

    /// No text and no filters: Search shows nothing yet.
    package var isEmpty: Bool { self == SearchFilter() }

    /// How many filters besides the text are set.
    package var filterCount: Int {
        [!statuses.isEmpty, !priorities.isEmpty, !labels.isEmpty, due != .any].count { $0 }
    }

    /// The filter as text, the same each run.
    var key: String {
        [
            text, statuses.map(\.rawValue).sorted().joined(separator: ","),
            priorities.map { "\($0)" }.sorted().joined(separator: ","), labels.sorted().joined(separator: ","),
            due.rawValue,
        ].joined(separator: "|")
    }

    package func matches(_ task: LifrTask, in workspace: Workspace, now: Date, timeZone: TimeZone) -> Bool {
        let status = task.effectiveStatus(now: now, settings: workspace.settings)
        guard statuses.isEmpty ? !status.isClosed : statuses.contains(status) else { return false }
        guard priorities.isEmpty || priorities.contains(task.priority) else { return false }
        if !labels.isEmpty {
            guard task.labelIDs.contains(where: { workspace.labels[$0].map { labels.contains($0.name) } == true })
            else { return false }
        }
        let today = CalendarDate(now, in: timeZone)
        switch due {
        case .any: break
        case .overdue: guard let date = task.dueDate, date < today else { return false }
        case .today: guard task.dueDate == today else { return false }
        case .nextSevenDays:
            let last = CalendarDate(now + 7 * 24 * 60 * 60, in: timeZone)
            guard let date = task.dueDate, date >= today, date <= last else { return false }
        case .none: guard task.dueDate == nil else { return false }
        }
        let words = text.lowercased().split(whereSeparator: \.isWhitespace)
        guard !words.isEmpty else { return true }
        let comments = workspace.comments.values
            .filter { $0.taskID == task.id && $0.deletedAt == nil }
            .map(\.body)
        let haystack = ([task.title, task.details] + comments).joined(separator: "\n").lowercased()
        return words.allSatisfy { haystack.contains($0) }
    }
}

extension TaskQuery {
    /// Tasks and Sub tasks that match `filter`, in list order. Nothing when
    /// the filter is empty.
    package static func search(_ filter: SearchFilter) -> TaskQuery {
        TaskQuery(
            key: "search-\(filter.key)",
            empty: filter.isEmpty
                ? .init(
                    "Search your tasks", systemImage: "magnifyingglass",
                    description: "Search titles, details and comments, or filter by status, priority, label and date.")
                : .init(
                    "No matches", systemImage: "magnifyingglass",
                    description: "Try other words, or clear some filters.")
        ) { workspace, now, timeZone in
            guard !filter.isEmpty else { return [] }
            let tasks = workspace.listedTasks.filter { filter.matches($0, in: workspace, now: now, timeZone: timeZone) }
            return [
                TaskSection(id: "results", title: "Results", rows: workspace.rows(tasks, now: now, timeZone: timeZone))
            ]
        }
    }
}

/// Search: words and filters over every task.
@Reducer
package struct SearchFeature {
    @ObservableState
    package struct State: Equatable {
        package var filter = SearchFilter()
        package var list: TaskListFeature.State

        package init() {
            list = TaskListFeature.State(query: .search(SearchFilter()))
        }

        package init(filter: SearchFilter, workspace: Workspace, now: Date, timeZone: TimeZone) {
            self.filter = filter
            list = TaskListFeature.State(query: .search(filter), workspace: workspace, now: now, timeZone: timeZone)
        }

        /// Every label name, for the label filter.
        package var labelNames: [String] {
            (list.workspace?.labels.values.map(\.name) ?? []).sorted()
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case list(TaskListFeature.Action)
        case clearFiltersTapped
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
        }
    }

    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Scope(state: \.list, action: \.list) { TaskListFeature() }
        Reduce { state, action in
            switch action {
            case .clearFiltersTapped:
                let text = state.filter.text
                state.filter = SearchFilter()
                state.filter.text = text
                refilter(&state)
                return .none
            case .list(.delegate(.saved(let workspace))):
                return .send(.delegate(.saved(workspace)))
            case .binding:
                refilter(&state)
                return .none
            case .list, .delegate:
                return .none
            }
        }
    }

    /// Runs the list's query again for the current filter.
    private func refilter(_ state: inout State) {
        state.list.query = .search(state.filter)
        if let workspace = state.list.workspace {
            state.list.show(workspace, now: now, timeZone: timeZone)
        }
    }
}
