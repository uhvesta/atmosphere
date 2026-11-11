import ComposableArchitecture
import Foundation
import LifrCore
import LifrFeedbackClient
import LifrStoreClient
import LifrSyncClient
import LifrTaskDetail
import LifrUI

/// A list of tasks in sections, shared by Today, Upcoming, Procrastinated,
/// the Avenue view, search results and the Mac's board. Rows open the task,
/// toggle Done, and change status by swipe; refused changes ask first, and
/// a reopened parent offers Undo.
@Reducer
package struct TaskListFeature {
    @ObservableState
    package struct State: Equatable {
        package var query: TaskQuery
        /// The latest save shown, so an earlier one's late reply is ignored.
        package var saves = SaveOrder()
        /// `nil` until the records load.
        package var workspace: Workspace?
        package var sections: [TaskSection] = []
        package var banner: Banner?
        @Presents package var detail: TaskDetailFeature.State?
        @Presents package var reschedule: RescheduleFeature.State?
        @Presents package var prompt: AlertState<StatusPrompt>?
        /// Every status, from a row's Status swipe.
        @Presents package var statusPicker: ConfirmationDialogState<StatusPick>?
        @Presents package var alert: AlertState<Never>?
        /// Whether a pull already said why sync can't run, so the next pull
        /// just ends.
        package var explainedSync = false
        /// The order within sections, remembered per list on this device.
        @Shared package var sort: TaskSort
        /// What the search field filters rows by, in a list that searches.
        package var searchText = ""
        /// Picking tasks to change at once: tapping a row picks it.
        package var isSelecting = false
        /// The tasks picked, of those shown.
        package var selection: Set<UUID> = []
        /// The row a ⇧-click picks from.
        package var selectionAnchor: UUID?
        /// Asks before closing picked tasks' open Sub tasks.
        @Presents package var bulkPrompt: AlertState<BulkAnswer>?

        package init(query: TaskQuery) {
            self.query = query
            // App storage keys can't have periods.
            let list = query.listID.replacingOccurrences(of: ".", with: "-")
            _sort = Shared(wrappedValue: .smart, .appStorage("lifrTaskSort-\(list)"))
        }

        package init(query: TaskQuery, workspace: Workspace, now: Date, timeZone: TimeZone) {
            self.init(query: query)
            show(Snapshot(workspace, number: 0), now: now, timeZone: timeZone)
        }

        /// The sorts this list offers: Manual only where rows can be dragged.
        package var sorts: [TaskSort] { TaskSort.allCases.filter { $0 != .manual || query.reorders } }

        /// Whether rows can be dragged into a new order now.
        package var canReorder: Bool { query.reorders && sort == .manual }

        package var isEmpty: Bool { sections.allSatisfy(\.rows.isEmpty) }

        /// Whether rows are being filtered by the search field.
        package var isSearching: Bool { query.searches && !searchTerm.isEmpty }

        private var searchTerm: String { searchText.trimmingCharacters(in: .whitespacesAndNewlines) }

        package var rows: [TaskRowData] { sections.flatMap(\.rows) }

        /// The picked tasks, in the order shown.
        package var selectedIDs: [UUID] { rows.map(\.id).filter(selection.contains) }

        /// The records shown, with their place in the store's order.
        package var snapshot: Snapshot? { workspace.map { Snapshot($0, number: saves.latest) } }

        /// Shows `snapshot`'s records through the query, and the open task
        /// with them, unless later ones are shown already.
        package mutating func show(_ snapshot: Snapshot, now: Date, timeZone: TimeZone) {
            guard saves.isLatest(snapshot) else { return }
            workspace = snapshot.workspace
            sections = query.sections(snapshot.workspace, now, timeZone)
            if isSearching {
                let term = searchTerm
                for index in sections.indices {
                    sections[index].rows.removeAll {
                        !$0.title.localizedStandardContains(term) && !$0.code.localizedStandardContains(term)
                    }
                }
            }
            sections.removeAll { $0.rows.isEmpty }
            let sort = sorts.contains(sort) ? sort : .smart
            if sort != .smart {
                let tasks = snapshot.workspace.tasks
                for index in sections.indices {
                    sections[index].rows.sort { lhs, rhs in
                        guard let left = tasks[lhs.id], let right = tasks[rhs.id] else { return false }
                        return sort.areInOrder(left, right)
                    }
                }
            }
            // Picked tasks no longer shown aren't picked.
            selection.formIntersection(rows.map(\.id))
            // A task trashed, here or elsewhere, closes.
            if var detail {
                self.detail = detail.refresh(from: snapshot, now: now, timeZone: timeZone) ? detail : nil
            }
        }
    }

    package enum Action {
        /// The view appeared: load the records if they aren't yet.
        case task
        /// Pull to sync; ends when the sync does.
        case pulled
        case pullFailed(String)
        case loaded(Result<Workspace, any Error>)
        /// The records changed elsewhere, such as in another tab or a sync.
        case show(Snapshot)
        case rowTapped(UUID)
        /// Select Tasks, or Done to stop picking.
        case selectTapped
        /// A row tapped while picking, or ⌘-clicked on the Mac.
        case selectionToggled(UUID)
        /// A row ⇧-clicked on the Mac: picks the rows from the last one picked.
        case selectionExtended(to: UUID)
        case selectAllTapped
        /// A change to every picked task.
        case bulk(BulkEdit)
        /// Due in this many days from today, or no due date, for every
        /// picked task.
        case bulkDue(days: Int?)
        case bulkSaved(BulkEdit, Result<Saved, any Error>)
        case bulkPrompt(PresentationAction<BulkAnswer>)
        case toggled(UUID)
        case rowAction(UUID, RowAction)
        case saved(Result<Saved, any Error>)
        case bannerUndoTapped
        case sortPicked(TaskSort)
        case searchChanged(String)
        /// Rows dragged within a section, as `onMove` reports them.
        case moved(section: String, from: IndexSet, to: Int)
        case bannerDismissed
        case detail(PresentationAction<TaskDetailFeature.Action>)
        case reschedule(PresentationAction<RescheduleFeature.Action>)
        case prompt(PresentationAction<StatusPrompt>)
        case statusPicker(PresentationAction<StatusPick>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            /// This list changed the records; other lists should show them.
            case saved(Snapshot)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.feedback) private var feedback
    @Dependency(\.sync) private var sync
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    private var core: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                guard state.workspace == nil else { return .none }
                return .run { [store] send in
                    await send(.loaded(Result { try await store.load() }))
                }

            case .loaded(.success(let workspace)):
                state.show(Snapshot(workspace, number: 0), now: now, timeZone: timeZone)
                return .none

            case .show(let snapshot):
                state.show(snapshot, now: now, timeZone: timeZone)
                return .none

            case .loaded(.failure(let error)):
                state.alert = .failed("Couldn't load your tasks", error)
                return .none

            case .rowTapped(let id):
                if state.isSelecting { return .send(.selectionToggled(id)) }
                guard let workspace = state.workspace else { return .none }
                state.detail = TaskDetailFeature.State(id: id, workspace: workspace, now: now, timeZone: timeZone)
                return .none

            case .selectTapped, .selectionToggled, .selectionExtended, .selectAllTapped, .bulk, .bulkDue, .bulkSaved,
                .bulkPrompt(.presented):
                return picking(&state, action)

            case .toggled(let id):
                if state.isSelecting { return .send(.selectionToggled(id)) }
                // Closing a task is a choice, so the checkbox asks which
                // status rather than finishing it outright.
                guard let row = state.rows.first(where: { $0.id == id }) else { return .none }
                state.statusPicker = .statuses(for: row.id, title: row.title, status: row.status)
                return .none

            case .rowAction(let id, .status(let status)):
                return setStatus(status, id)

            case .rowAction(let id, .reschedule):
                guard let row = state.rows.first(where: { $0.id == id }) else { return .none }
                state.reschedule = RescheduleFeature.State(
                    id: id, title: row.title, date: now.addingTimeInterval(24 * 60 * 60))
                return .none

            case .rowAction(let id, .pickStatus):
                guard let row = state.rows.first(where: { $0.id == id }) else { return .none }
                state.statusPicker = .statuses(for: row.id, title: row.title, status: row.status)
                return .none

            case .statusPicker(.presented(let pick)):
                return setStatus(pick.status, pick.task)

            case .reschedule(.presented(.delegate(.picked(let id, let date)))):
                return save { try await $0.edit(id, Workspace.TaskEdit(dueDate: .some(date))) }

            case .prompt(.presented(.showSubtasks(let id))):
                return .send(.rowTapped(id))

            case .prompt(.presented(let answer)):
                guard let retry = answer.retry else { return .none }
                return setStatus(retry.status, retry.task, cascade: true)

            case .pulled:
                return .run { [sync] send in
                    if case .failed(let error) = await sync.sync().status { await send(.pullFailed(error.description)) }
                }

            case .pullFailed(let reason):
                guard !state.explainedSync else { return .none }
                state.explainedSync = true
                state.alert = AlertState {
                    TextState("Couldn't sync")
                } actions: {
                    ButtonState(role: .cancel) { TextState("OK") }
                } message: {
                    TextState(reason)
                }
                return .none

            case .bannerUndoTapped:
                guard let change = state.banner?.undo else { return .none }
                state.banner = nil
                return save { [feedback] in
                    let saved = try await $0.undo(change)
                    await feedback(.tap)
                    return saved
                }

            case .sortPicked(let sort):
                state.$sort.withLock { $0 = sort }
                if let snapshot = state.snapshot { state.show(snapshot, now: now, timeZone: timeZone) }
                return .none

            case .searchChanged(let text):
                state.searchText = text
                if let snapshot = state.snapshot { state.show(snapshot, now: now, timeZone: timeZone) }
                return .none

            case .moved(let sectionID, let source, let destination):
                guard state.canReorder, source.count == 1, let from = source.first,
                    let index = state.sections.firstIndex(where: { $0.id == sectionID }),
                    state.sections[index].rows.indices.contains(from)
                else { return .none }
                var rows = state.sections[index].rows
                let row = rows.remove(at: from)
                let to = min(destination > from ? destination - 1 : destination, rows.count)
                rows.insert(row, at: to)
                guard to != from else { return .none }
                // Shown at once; the save's reply confirms it.
                state.sections[index].rows = rows
                let lower = to > 0 ? rows[to - 1].id : nil
                let upper = to + 1 < rows.count ? rows[to + 1].id : nil
                return save { try await $0.reorder(task: row.id, after: lower, before: upper) }

            case .bannerDismissed:
                state.banner = nil
                return .none

            case .saved(.success(let saved)):
                let isLatest = saved.number >= state.saves.latest
                state.show(saved.snapshot, now: now, timeZone: timeZone)
                if let parentID = saved.change.reopenedParent, let parent = saved.workspace.tasks[parentID] {
                    state.banner = .reopened(parent, by: saved.change)
                }
                return isLatest ? .send(.delegate(.saved(saved.snapshot))) : .none

            case .saved(.failure(let error)):
                let workspace = state.workspace
                if let prompt = AlertState.forRefusal(
                    error, canShow: true, title: { workspace?.tasks[$0]?.title })
                {
                    state.prompt = prompt
                } else {
                    state.alert = .failed("Couldn't change the task", error)
                }
                return .run { [feedback] _ in await feedback(.warning) }

            case .detail(.presented(.delegate(.saved(let snapshot)))):
                state.show(snapshot, now: now, timeZone: timeZone)
                return .send(.delegate(.saved(snapshot)))

            case .detail, .reschedule, .prompt, .statusPicker, .alert, .bulkPrompt, .delegate:
                return .none
            }
        }
    }

    package var body: some ReducerOf<Self> {
        core
            .ifLet(\.$detail, action: \.detail) { TaskDetailFeature() }
            .ifLet(\.$reschedule, action: \.reschedule) { RescheduleFeature() }
            .ifLet(\.$prompt, action: \.prompt)
            .ifLet(\.$statusPicker, action: \.statusPicker)
            .ifLet(\.$alert, action: \.alert)
            .ifLet(\.$bulkPrompt, action: \.bulkPrompt)
    }

    /// Picking tasks and changing the picked ones at once.
    private func picking(_ state: inout State, _ action: Action) -> Effect<Action> {
        switch action {
        case .selectTapped:
            state.isSelecting.toggle()
            state.selection = []
            state.selectionAnchor = nil
            return .none

        case .selectionToggled(let id):
            if !state.isSelecting {
                // ⌘-click with a task open picks that one too.
                state.isSelecting = true
                state.selection = state.detail.map { [$0.id] } ?? []
            }
            if state.selection.remove(id) == nil { state.selection.insert(id) }
            state.selectionAnchor = id
            return .none

        case .selectionExtended(let id):
            let ids = state.rows.map(\.id)
            let anchor = state.selectionAnchor ?? state.detail?.id ?? ids.first
            guard let to = ids.firstIndex(of: id), let from = anchor.flatMap(ids.firstIndex(of:)) else {
                return .none
            }
            state.isSelecting = true
            state.selection.formUnion(ids[min(from, to)...max(from, to)])
            return .none

        case .selectAllTapped:
            let ids = Set(state.rows.map(\.id))
            state.selection = state.selection == ids ? [] : ids
            return .none

        case .bulk(let edit):
            return bulk(edit, state.selectedIDs)

        case .bulkDue(let days):
            let today = CalendarDate(now, in: timeZone)
            return bulk(.dueDate(days.map { today.adding(days: $0) }), state.selectedIDs)

        case .bulkSaved(_, .success(let saved)):
            state.isSelecting = false
            state.selection = []
            state.selectionAnchor = nil
            return .send(.saved(.success(saved)))

        case .bulkSaved(.status(let status), .failure(let error))
        where (error as? LifrError)?.needsCascade == true:
            let ids = state.selectedIDs
            var titles: [String] = []
            if let workspace = state.workspace {
                for id in ids {
                    let open = workspace.openSubtasks(of: id).contains { !ids.contains($0.id) }
                    if open, let title = workspace.tasks[id]?.title { titles.append(title) }
                }
            }
            state.bulkPrompt = .closingSubtasks(of: titles, status: status)
            return .none

        case .bulkSaved(_, .failure(let error)):
            state.alert = .failed("Couldn't change the tasks", error)
            return .none

        case .bulkPrompt(.presented(.closeSubtasks(let status))):
            return bulk(.status(status), state.selectedIDs, cascade: true)

        default:
            return .none
        }
    }

    private func bulk(_ edit: BulkEdit, _ ids: [UUID], cascade: Bool = false) -> Effect<Action> {
        guard !ids.isEmpty else { return .none }
        return .run { [store] send in
            await send(.bulkSaved(edit, Result { try await store.apply(edit, to: ids, cascade: cascade) }))
        }
    }

    private func setStatus(_ status: TaskStatus, _ id: UUID, cascade: Bool = false) -> Effect<Action> {
        save { [feedback] in
            let saved = try await $0.setStatus(status, id, cascade)
            await feedback(Feedback(status: status))
            return saved
        }
    }

    private func save(_ operation: @escaping @Sendable (StoreClient) async throws -> Saved) -> Effect<Action> {
        .run { [store] send in
            await send(.saved(Result { try await operation(store) }))
        }
    }
}

/// The answer to closing picked tasks that have open Sub tasks.
package enum BulkAnswer: Equatable, Sendable {
    /// Close the open Sub tasks as Won't Do, then set the picked tasks to this.
    case closeSubtasks(TaskStatus)
}

extension LifrError {
    /// Whether Core refused only until the person agrees to close open Sub
    /// tasks.
    fileprivate var needsCascade: Bool {
        switch self {
        case .openSubtasks, .wontDoNeedsCascade: true
        default: false
        }
    }
}

extension AlertState where Action == BulkAnswer {
    /// Asks before closing the open Sub tasks of `titles` as Won't Do.
    fileprivate static func closingSubtasks(of titles: [String], status: TaskStatus) -> Self {
        let title: TextState =
            titles.count == 1
            ? TextState("“\(titles.joined())” has open Sub tasks")
            : TextState("\(titles.count) tasks have open Sub tasks")
        let names: String = titles.map { name in "“\(name)”" }.joined(separator: ", ")
        let message = TextState(
            "\(names). A closed task can't keep open Sub tasks, so they'll be marked Won't Do first. "
                + "Nothing changes if you cancel.")
        return AlertState {
            title
        } actions: {
            ButtonState(role: .destructive, action: .closeSubtasks(status)) {
                TextState("Won't Do + \(status.title)")
            }
            ButtonState(role: .cancel) { TextState("Cancel") }
        } message: {
            message
        }
    }
}
