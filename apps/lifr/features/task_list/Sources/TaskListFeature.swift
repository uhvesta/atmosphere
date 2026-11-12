import ComposableArchitecture
import Foundation
import LifrCore
import LifrFeedbackClient
import LifrPasteboardClient
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
        /// Why sync failed, as a pull last said, so the same failure isn't
        /// told on every pull.
        package var explainedSync: String?
        /// Tasks just closed here, kept where they were for a moment so a
        /// mis-pick can be undone in place.
        package var lingering: Set<UUID> = []
        /// Why the records couldn't load, shown in place of the list with
        /// a way to try again.
        package var loadError: String?
        /// Whether any task was closed today, for a list that says so when
        /// it empties.
        package var closedToday = false
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

        /// What the empty list says: that the day's work is done, once
        /// tasks were closed today, where the list says so.
        package var emptyMessage: TaskQuery.Empty {
            closedToday ? query.finished ?? query.empty : query.empty
        }

        /// Whether rows are being filtered by the search field.
        package var isSearching: Bool { query.searches && !searchTerm.isEmpty }

        private var searchTerm: String { searchText.trimmingCharacters(in: .whitespacesAndNewlines) }

        package var rows: [TaskRowData] { sections.flatMap(\.rows) }

        /// The list as a Markdown checklist, with a heading over each
        /// section, to copy or share.
        package var markdown: String {
            sections.filter { !$0.rows.isEmpty }.map { section in
                let lines = section.rows.map(\.markdown).joined(separator: "\n")
                return section.title.isEmpty ? lines : "## \(section.title)\n\n\(lines)"
            }
            .joined(separator: "\n\n")
        }

        /// The picked tasks, in the order shown.
        package var selectedIDs: [UUID] { rows.map(\.id).filter(selection.contains) }

        /// The row the keyboard is on: the last one moved to or picked, or
        /// else the open task.
        package var cursor: UUID? {
            let ids = rows.map(\.id)
            return [selectionAnchor, detail?.id].compactMap { $0 }.first(where: ids.contains)
        }

        /// The records shown, with their place in the store's order.
        package var snapshot: Snapshot? { workspace.map { Snapshot($0, number: saves.latest) } }

        /// Shows `snapshot`'s records through the query, and the open task
        /// with them, unless later ones are shown already.
        package mutating func show(_ snapshot: Snapshot, now: Date, timeZone: TimeZone) {
            guard saves.isLatest(snapshot) else { return }
            let previous = sections
            let before = rows.map(\.id)
            let wasOn = cursor
            workspace = snapshot.workspace
            sections = query.sections(snapshot.workspace, now, timeZone)
            keepLingering(from: previous, in: snapshot.workspace)
            if query.finished != nil {
                let today = CalendarDate(now, in: timeZone)
                closedToday = snapshot.workspace.listedTasks.contains {
                    $0.closedAt.map { CalendarDate($0, in: timeZone) == today } ?? false
                }
            }
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
            // The keyboard row, gone, moves to the row that took its place.
            let shown = Set(rows.map(\.id))
            if let wasOn, !shown.contains(wasOn), let index = before.firstIndex(of: wasOn) {
                selectionAnchor =
                    before[(index + 1)...].first(where: shown.contains) ?? before[..<index].last(where: shown.contains)
            }
            // A task trashed, here or elsewhere, closes.
            if var detail {
                self.detail = detail.refresh(from: snapshot, now: now, timeZone: timeZone) ? detail : nil
            }
        }

        /// Puts tasks closed here back where `previous` showed them, with
        /// their new status, while they linger.
        private mutating func keepLingering(from previous: [TaskSection], in workspace: Workspace) {
            lingering = lingering.filter { workspace.tasks[$0].map { $0.deletedAt == nil } ?? false }
            for id in lingering where !rows.contains(where: { $0.id == id }) {
                guard let task = workspace.tasks[id],
                    let before = previous.firstIndex(where: { $0.rows.contains { $0.id == id } }),
                    var row = previous[before].rows.first(where: { $0.id == id }),
                    let index = previous[before].rows.firstIndex(where: { $0.id == id })
                else { continue }
                row.status = task.status
                let section = previous[before]
                if let at = sections.firstIndex(where: { $0.id == section.id }) {
                    sections[at].rows.insert(row, at: min(index, sections[at].rows.count))
                } else {
                    let at = min(before, sections.count)
                    sections.insert(TaskSection(id: section.id, title: section.title, rows: [row]), at: at)
                }
            }
        }
    }

    package enum Action {
        /// The view appeared: load the records if they aren't yet.
        case task
        /// A task closed here has stayed in place long enough.
        case lingerEnded(UUID)
        /// Try loading the records again, after they couldn't load.
        case retryTapped
        /// Pull to sync; ends when the sync does.
        case pulled
        case pullFailed(String)
        case pullSucceeded
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
        /// ↑ or ↓ on the Mac, with ⇧ to pick the rows passed too.
        case cursorMoved(up: Bool, extending: Bool)
        /// Return on the Mac: opens the row the keyboard is on.
        case openCursorTapped
        /// Space on the Mac: the statuses for the row the keyboard is on.
        case cursorStatusTapped
        /// ⌫ on the Mac: the picked tasks, or the row the keyboard is on,
        /// to the Trash.
        case trashKeyPressed
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
        /// Tasks dropped onto a row, to become its Sub tasks.
        case droppedOnto(UUID, tasks: [UUID])
        case bannerDismissed
        /// Copy as Markdown: the task as a line of a checklist, with its code.
        case copyTapped(UUID)
        /// Copy List: every row, as `markdown` puts them.
        case copyListTapped
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
    @Dependency(\.pasteboard) private var pasteboard
    @Dependency(\.sync) private var sync
    @Dependency(\.continuousClock) private var clock
    @Dependency(\.closedRowPause) private var closedRowPause
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

            case .lingerEnded(let id):
                state.lingering.remove(id)
                guard let snapshot = state.snapshot else { return .none }
                state.show(snapshot, now: now, timeZone: timeZone)
                return .none

            case .retryTapped:
                state.loadError = nil
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
                state.loadError = (error as? any CustomStringConvertible)?.description ?? error.localizedDescription
                return .none

            case .rowTapped(let id):
                if state.isSelecting { return .send(.selectionToggled(id)) }
                // The open task's row keeps what's typed in it.
                guard state.detail?.id != id, let workspace = state.workspace else { return .none }
                let leaving = state.detail
                state.detail = TaskDetailFeature.State(id: id, workspace: workspace, now: now, timeZone: timeZone)
                // Another task opens in its place: its typed title and
                // description are saved first, as leaving the field does.
                guard let leaving, let edit = leaving.unsavedEdit else { return .none }
                return .run { [store] send in
                    await send(.saved(Result { try await store.edit(leaving.id, edit) }))
                }

            case .selectTapped, .selectionToggled, .selectionExtended, .selectAllTapped, .bulk, .bulkDue, .bulkSaved,
                .bulkPrompt(.presented), .cursorMoved, .openCursorTapped, .cursorStatusTapped, .trashKeyPressed:
                return picking(&state, action)

            case .toggled(let id):
                if state.isSelecting { return .send(.selectionToggled(id)) }
                // Closing a task is a choice, so the checkbox asks which
                // status rather than finishing it outright.
                guard let row = state.rows.first(where: { $0.id == id }) else { return .none }
                state.statusPicker = .statuses(for: row.id, title: row.title, status: row.status)
                return .none

            case .rowAction(let id, .status(let status)):
                return setStatus(status, id, in: &state)

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
                return setStatus(pick.status, pick.task, in: &state)

            case .reschedule(.presented(.delegate(.picked(let id, let date)))):
                // As the bulk bar does: a task that runs over days moves
                // as a whole, and No due date also clears the time.
                return save { try await $0.apply(.dueDate(date), to: [id]) }

            case .prompt(.presented(.showSubtasks(let id))):
                return .send(.rowTapped(id))

            case .prompt(.presented(let answer)):
                guard let retry = answer.retry else { return .none }
                return setStatus(retry.status, retry.task, cascade: true, in: &state)

            case .pulled:
                return .run { [sync] send in
                    switch await sync.sync().status {
                    case .failed(let error): await send(.pullFailed(error.description))
                    default: await send(.pullSucceeded)
                    }
                }

            case .pullSucceeded:
                state.explainedSync = nil
                return .none

            case .pullFailed(let reason):
                // Each failure once, until a sync works or it fails another way.
                guard state.explainedSync != reason else { return .none }
                state.explainedSync = reason
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

            case .droppedOnto(let parent, let ids):
                let ids = ids.filter { $0 != parent && state.workspace?.tasks[$0]?.parentID != parent }
                guard let first = ids.first else { return .none }
                return save { store in
                    var saved = try await store.setParent(parent, of: first)
                    for id in ids.dropFirst() { saved = try await store.setParent(parent, of: id) }
                    return saved
                }

            case .bannerDismissed:
                state.banner = nil
                return .none

            case .copyListTapped:
                guard !state.isEmpty else { return .none }
                return .run { [pasteboard, text = state.markdown] _ in await pasteboard.copy(text) }

            case .copyTapped(let id):
                guard let row = state.rows.first(where: { $0.id == id }) else { return .none }
                return .run { [pasteboard, line = row.markdown] _ in await pasteboard.copy(line) }

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

        case .cursorMoved(let up, let extending):
            let ids = state.rows.map(\.id)
            guard !ids.isEmpty else { return .none }
            let next: UUID
            if let cursor = state.cursor, let index = ids.firstIndex(of: cursor) {
                next = ids[up ? max(index - 1, 0) : min(index + 1, ids.count - 1)]
                if extending {
                    state.isSelecting = true
                    state.selection.formUnion([cursor, next])
                }
            } else {
                next = (up ? ids.last : ids.first) ?? ids[0]
                if extending {
                    state.isSelecting = true
                    state.selection.insert(next)
                }
            }
            state.selectionAnchor = next
            return .none

        case .openCursorTapped:
            guard let cursor = state.cursor else { return .none }
            return .send(.rowTapped(cursor))

        case .cursorStatusTapped:
            guard let cursor = state.cursor else { return .none }
            return .send(.rowAction(cursor, .pickStatus))

        case .trashKeyPressed:
            let ids = state.isSelecting ? state.selectedIDs : state.cursor.map { [$0] } ?? []
            guard !ids.isEmpty else { return .none }
            return bulk(.trash, ids)

        case .bulk(let edit):
            return bulk(edit, state.selectedIDs)

        case .bulkDue(let days):
            let today = CalendarDate(now, in: timeZone)
            return bulk(.dueDate(days.map { today.adding(days: $0) }), state.selectedIDs)

        case .bulkSaved(_, .success(let saved)):
            // The keyboard row stays, so the next key goes on from there.
            state.isSelecting = false
            state.selection = []
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

    /// Saves `status`; a task closed from a row stays there for a moment.
    private func setStatus(
        _ status: TaskStatus, _ id: UUID, cascade: Bool = false, in state: inout State
    ) -> Effect<Action> {
        let lingers = status.isClosed && closedRowPause > .zero && state.rows.contains { $0.id == id }
        if lingers { state.lingering.insert(id) }
        return .run { [store, feedback, clock, closedRowPause] send in
            let result = await Result { try await store.setStatus(status, id, cascade) }
            if case .success = result { await feedback(Feedback(status: status)) }
            await send(.saved(result))
            guard lingers else { return }
            if case .success = result { try await clock.sleep(for: closedRowPause) }
            await send(.lingerEnded(id))
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

/// How long a task closed from a row stays where it was before it leaves
/// the list. None in tests, where a closed row leaves at once unless the test
/// says otherwise.
private enum ClosedRowPauseKey: DependencyKey {
    static let liveValue = Duration.milliseconds(1500)
    static let testValue = Duration.zero
}

extension DependencyValues {
    package var closedRowPause: Duration {
        get { self[ClosedRowPauseKey.self] }
        set { self[ClosedRowPauseKey.self] = newValue }
    }
}
