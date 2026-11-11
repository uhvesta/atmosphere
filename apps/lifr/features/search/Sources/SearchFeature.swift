import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskList
import LifrUI

/// Search's words and filters.
package typealias SearchFilter = TaskFilter

extension TaskFilter.Due {
    package var title: String {
        switch self {
        case .any: String(localized: "Any date")
        case .overdue: String(localized: "Overdue")
        case .today: String(localized: "Today")
        case .nextSevenDays: String(localized: "Next 7 days")
        case .none: String(localized: "No due date")
        }
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
                    description:
                        "Search titles, details and comments, or filter by Avenue, status, priority, label and date.")
                : .init(
                    "No matches", systemImage: "magnifyingglass",
                    description: String(localized: "Try other words, or clear the filters.")),
            adds: nil
        ) { workspace, now, timeZone in
            guard !filter.isEmpty else { return [] }
            let tasks = workspace.listedTasks.filter { filter.matches($0, in: workspace, now: now, timeZone: timeZone) }
            return [
                TaskSection(
                    id: "results", title: String(localized: "Results"),
                    rows: workspace.rows(tasks, now: now, timeZone: timeZone))
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
        /// Whether the search field has the keyboard, as after ⌘F.
        package var isFieldFocused = false
        package var list: TaskListFeature.State
        /// The saved filter this search was opened from, if any.
        package var savedFilterID: UUID?
        @Presents package var editor: SavedFilterEditor.State?
        /// Why updating the saved filter failed.
        package var error: String?

        package init() {
            list = TaskListFeature.State(query: .search(SearchFilter()))
        }

        /// The search a saved filter keeps.
        package init(saved: SavedFilter, workspace: Workspace, now: Date, timeZone: TimeZone) {
            self.init(filter: saved.filter, workspace: workspace, now: now, timeZone: timeZone)
            savedFilterID = saved.id
        }

        /// The saved filter this search was opened from, as it's saved.
        package var savedFilter: SavedFilter? {
            savedFilterID.flatMap { list.workspace?.savedFilters[$0] }
        }

        /// Whether the words or filters differ from the saved filter's.
        package var differsFromSaved: Bool {
            savedFilter.map { $0.filter != filter } ?? false
        }

        package init(filter: SearchFilter, workspace: Workspace, now: Date, timeZone: TimeZone) {
            self.filter = filter
            list = TaskListFeature.State(query: .search(filter), workspace: workspace, now: now, timeZone: timeZone)
        }

        /// Every Avenue in sidebar order, for the Avenue filter.
        package var avenueChoices: [Avenue] {
            (list.workspace?.avenues.values.sorted { $0.sortOrder < $1.sortOrder }) ?? []
        }

        /// Every label name, for the label filter.
        package var labelNames: [String] {
            (list.workspace?.labels.values.map(\.name) ?? []).sorted()
        }

        /// Each filter value that's set, in the order the filter menu lists
        /// them.
        package var chips: [FilterChip] {
            var chips: [FilterChip] = avenueChoices.filter { filter.avenues.contains($0.id) }
                .map { FilterChip(.avenue($0.id), $0.name) }
            for status in TaskStatus.allCases where filter.statuses.contains(status) {
                chips.append(FilterChip(.status(status), status.title))
            }
            for priority in Priority.allCases where filter.priorities.contains(priority) {
                chips.append(FilterChip(.priority(priority), priority.title ?? String(localized: "No priority")))
            }
            for name in filter.labels.sorted() {
                chips.append(FilterChip(.label(name), "#\(name)"))
            }
            if filter.due != .any { chips.append(FilterChip(.due, filter.due.title)) }
            return chips
        }
    }

    /// One set filter value, shown above the results.
    package struct FilterChip: Hashable, Identifiable, Sendable {
        package enum Kind: Hashable, Sendable {
            case avenue(UUID)
            case status(TaskStatus)
            case priority(Priority)
            case label(String)
            case due
        }

        package var kind: Kind
        package var title: String
        package var id: Kind { kind }

        init(_ kind: Kind, _ title: String) {
            self.kind = kind
            self.title = title
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case list(TaskListFeature.Action)
        case clearFiltersTapped
        /// Clears one filter value.
        case chipRemoved(FilterChip.Kind)
        /// Keep this search under a name.
        case saveFilterTapped
        /// Rename, re-symbol or delete the saved filter shown.
        case editSavedFilterTapped
        /// Save the words and filters shown into the saved filter.
        case updateSavedFilterTapped
        case updated(Result<Saved, any Error>)
        case editor(PresentationAction<SavedFilterEditor.Action>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Snapshot)
        }
    }

    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone
    @Dependency(\.store) private var store

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
            case .chipRemoved(let kind):
                switch kind {
                case .avenue(let id): state.filter.avenues.remove(id)
                case .status(let status): state.filter.statuses.remove(status)
                case .priority(let priority): state.filter.priorities.remove(priority)
                case .label(let name): state.filter.labels.remove(name)
                case .due: state.filter.due = .any
                }
                refilter(&state)
                return .none
            case .saveFilterTapped:
                guard !state.filter.isEmpty else { return .none }
                state.editor = SavedFilterEditor.State(saving: state.filter)
                return .none
            case .editSavedFilterTapped:
                guard let saved = state.savedFilter else { return .none }
                state.editor = SavedFilterEditor.State(editing: saved)
                return .none
            case .updateSavedFilterTapped:
                guard let saved = state.savedFilter else { return .none }
                return .run { [store, filter = state.filter] send in
                    await send(
                        .updated(
                            Result {
                                try await store.edit(
                                    savedFilter: saved.id, name: saved.name, symbol: saved.symbol, filter: filter)
                            }))
                }
            case .updated(.success(let saved)):
                state.error = nil
                return .send(.delegate(.saved(saved.snapshot)))
            case .updated(.failure(let error)):
                state.error = (error as? LifrError)?.description ?? error.localizedDescription
                return .none
            case .list(.delegate(.saved(let snapshot))),
                .editor(.presented(.delegate(.saved(let snapshot)))),
                .editor(.presented(.delegate(.deleted(let snapshot)))):
                return .send(.delegate(.saved(snapshot)))
            case .binding:
                state.error = nil
                refilter(&state)
                return .none
            case .list, .editor, .delegate:
                return .none
            }
        }
        .ifLet(\.$editor, action: \.editor) { SavedFilterEditor() }
    }

    /// Runs the list's query again for the current filter.
    private func refilter(_ state: inout State) {
        state.list.query = .search(state.filter)
        if let snapshot = state.list.snapshot {
            state.list.show(snapshot, now: now, timeZone: timeZone)
        }
    }
}
