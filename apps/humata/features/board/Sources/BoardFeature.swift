import ComposableArchitecture
import Foundation
import HumataDaemonClient
import HumataProjects
import HumataSession

/// A project's board: its tasks and subtasks in a column per status, read
/// again every few seconds as their sessions move them. Dropping a card on
/// a column gives it that status; a card opens its item's page.
@Reducer
package struct BoardFeature {
    /// Identified by its project, so opening another board in its place
    /// stops what this one was doing.
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var id: Project.ID { project.id }
        package var project: Project
        package var items: IdentifiedArrayOf<Item> = []
        /// The project's sessions.
        package var sessions: IdentifiedArrayOf<Session> = []
        /// Why the board couldn't be read, and what to do; nil once it was.
        package var failure: String?
        /// Why a change made here failed, kept until the person closes it,
        /// as reading the board again doesn't make it right.
        package var actionFailure: String?
        /// The items work is starting on, so a second tap starts no second
        /// session.
        package var starting: Set<Item.ID> = []
        @Presents package var destination: Destination.State?
        /// The item to open once the board is read, as when a search found it.
        package var opening: Item.ID?
        /// Its repositories' issues are being made tasks.
        package var importing = false
        /// The last change made here, which Undo takes back.
        package var undo: UndoToast?
        /// An undo waiting on the daemon; a second ⌘Z waits for it.
        package var undoing = false
        /// Cards dropped on a column, shown there until the daemon answers,
        /// over any read that started before it did.
        package var moving: [Item.ID: ItemStatus] = [:]
        /// The first change Undo may take back: none made before the board
        /// opened. Nil until the board's log is read.
        package var undoFrom: BoardChange.ID?
        /// What agents outside Humata proposed through `humata mcp`, which
        /// reaches the board only once the person applies it here.
        package var proposals: ProposalsFeature.State
        /// Spec comments not sent yet, by item, kept when its page closes
        /// so opening it again shows them.
        var keptComments: [Item.ID: SpecCommentsFeature.State] = [:]
        /// Titles and specs being edited and not saved, by item, kept when
        /// another page opens in its place so opening it again shows them.
        var keptDrafts: [Item.ID: ItemFeature.Draft] = [:]
        /// An alert about something that finished while a page or sheet was
        /// open, shown once it goes.
        var heldAlert: AlertState<BoardAlert>?

        package init(project: Project, opening: Item.ID? = nil) {
            self.project = project
            self.opening = opening
            self.proposals = ProposalsFeature.State(outsideAgentsOf: project.id)
        }

        /// Shows `alert` now, or once the page or sheet that's open goes, so
        /// a late result doesn't take away what the person is doing there.
        mutating func present(_ alert: AlertState<BoardAlert>) {
            switch destination {
            case nil, .alert: destination = .alert(alert)
            default: heldAlert = alert
            }
        }

        /// The items the board shows in `status`'s column, in the board's order.
        package func cards(in status: ItemStatus) -> [Item] {
            items.filter { $0.status == status }
        }

        package func parent(of item: Item) -> Item? {
            item.parentID.flatMap { items[id: $0] }
        }

        package func subtasks(of item: Item) -> [Item] {
            items.filter { $0.parentID == item.id }
        }

        package func sessions(on item: Item) -> [Session] {
            item.sessionIDs.compactMap { sessions[id: $0] }
        }

        /// `item`'s page, with the spec comments and the unsaved edit it was
        /// closed with.
        func page(of item: Item) -> ItemFeature.State {
            var page = ItemFeature.State(
                project: project, item: item, parent: parent(of: item), subtasks: subtasks(of: item),
                sessions: sessions(on: item))
            if var kept = keptComments[item.id] {
                kept.title = item.title
                page.comments = kept
            }
            page.draft = keptDrafts[item.id]
            return page
        }

        /// Keeps what was written on `page`'s spec once it closes. Comments
        /// being sent aren't kept: the session may have them already.
        mutating func keepComments(of page: ItemFeature.State) {
            var comments = page.comments
            comments.alert = nil
            let unsent = !comments.drafts.isEmpty || !comments.comment.isEmpty
            keptComments[page.item.id] = unsent && !comments.sending ? comments : nil
        }

        /// Keeps the title and spec being edited on `page` once it closes,
        /// if they differ from the item's. Discarding them clears the draft
        /// first, so nothing discarded is kept.
        mutating func keepDraft(of page: ItemFeature.State) {
            keptDrafts[page.item.id] = page.draftChanged ? page.draft : nil
        }

        /// The board to keep while something else shows in its place. What it
        /// was waiting on stops when it goes, so nothing kept says it's still
        /// in progress: the answer that would clear it never comes.
        package func settled() -> Self {
            var kept = self
            kept.starting = []
            kept.importing = false
            kept.undoing = false
            kept.moving = [:]
            kept.heldAlert = nil
            switch kept.destination {
            case .item(var item):
                item.settle()
                kept.destination = .item(item)
            case .newItem(var new):
                new.creating = false
                kept.destination = .newItem(new)
            default:
                break
            }
            return kept
        }
    }

    @Reducer
    package enum Destination {
        case item(ItemFeature)
        case newItem(NewItemFeature)
        case log(BoardLogFeature)
        case alert(AlertState<BoardAlert>)
    }

    package enum Action {
        /// The board appeared; reads it now and again every few seconds,
        /// until it goes.
        case task
        case refresh
        case loaded(items: [Item], sessions: [Session])
        /// The board couldn't be read.
        case failed(String)
        /// A change made here failed.
        case actionFailed(String)
        case actionFailureDismissed
        /// Work started on the item, or failed to.
        case startFinished(Item.ID)
        /// Adds a task, or a subtask under `parent`.
        case addTapped(parent: Item.ID?)
        case cardTapped(Item.ID)
        /// Starts a session on the item with the last New Session settings,
        /// and has it start, staying on the board.
        case startWorkTapped(Item.ID)
        /// Start Work's defaults leave a local model to pick; the item's page
        /// asks.
        case startNeedsModel(Item.ID, NewSessionFeature.State)
        case dropped(Item.ID, on: ItemStatus)
        case moved(Item)
        /// A drop the daemon refused: the card goes back to `was`.
        case moveFailed(Item.ID, was: ItemStatus, String)
        /// Asks to delete the item, then does.
        case deleteTapped(Item.ID)
        case logTapped
        /// The board's newest change when it opened.
        case logStarted(newest: BoardChange.ID?)
        /// Something was changed here; offers to undo it.
        case changedHere
        case offerUndo(BoardChange)
        /// ⌘Z, or Undo in the toast: reverts the last change made here not
        /// yet undone.
        case undoTapped
        case undone(Reverted)
        /// Undo found nothing to take back.
        case undoEnded
        case undoExpired
        /// Makes a task of each open GitHub issue of the project's
        /// repositories not imported before.
        case importIssuesTapped
        case imported(ImportedIssues)
        case destination(PresentationAction<Destination.Action>)
        case proposals(ProposalsFeature.Action)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case openSession(Session)
        }
    }

    /// How often the board is read again, for its sessions' states.
    static let refreshInterval = Duration.seconds(2)

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock

    private enum CancelID { case refresh, undo }

    /// How long the toast offers Undo; ⌘Z still works after.
    static let undoInterval = Duration.seconds(8)

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            // However a task's page closes, or another opens in its place,
            // its unsent spec comments and its unsaved edit stay for when it
            // opens again.
            let open = state.destination?.item
            let effect = core.reduce(into: &state, action: action)
            if let open, state.destination?.item?.item.id != open.item.id {
                state.keepComments(of: open)
                state.keepDraft(of: open)
            }
            // An open page holds its own edit; one kept is for a closed page.
            if let id = state.destination?.item?.item.id, state.keptDrafts[id] != nil {
                state.keptDrafts[id] = nil
            }
            // A held alert shows once what it waited behind is gone.
            if state.destination == nil, let alert = state.heldAlert {
                state.heldAlert = nil
                state.destination = .alert(alert)
            }
            return effect
        }
    }

    @ReducerBuilder<State, Action>
    private var core: some ReducerOf<Self> {
        Scope(state: \.proposals, action: \.proposals) { ProposalsFeature() }
        Reduce { state, action in
            switch action {
            case .task:
                let project = state.project.id
                let refresh = Effect<Action>.run { send in
                    while !Task.isCancelled {
                        await send(await load(project))
                        await send(.proposals(.refresh))
                        try await clock.sleep(for: Self.refreshInterval)
                    }
                }
                .cancellable(id: CancelID.refresh, cancelInFlight: true)
                guard state.undoFrom == nil else { return refresh }
                return .merge(
                    refresh,
                    .run { send in
                        // Only the newest change's id is used, so only it is read.
                        await send(.logStarted(newest: try await daemon.boardLog(project, nil, 1).first?.id))
                    } catch: { _, _ in
                    })

            case .logStarted(let newest):
                state.undoFrom = newest.map { $0 + 1 } ?? 0
                return .none

            case .changedHere:
                guard let from = state.undoFrom else { return .none }
                let project = state.project.id
                return .run { send in
                    if let change = Self.undoable(try await daemon.boardLog(project, from, nil), from: from) {
                        await send(.offerUndo(change))
                    }
                } catch: { _, _ in
                }

            case .offerUndo(let change):
                guard state.undo?.change != change.id else { return .none }
                state.undo = UndoToast(change: change.id, message: BoardLogFeature.describe(change, items: state.items))
                return .run { send in
                    try await clock.sleep(for: Self.undoInterval)
                    await send(.undoExpired)
                }
                .cancellable(id: CancelID.undo, cancelInFlight: true)

            case .undoExpired:
                state.undo = nil
                return .none

            case .undoTapped:
                guard let from = state.undoFrom, !state.undoing else { return .none }
                state.undo = nil
                state.undoing = true
                let project = state.project.id
                return .merge(
                    .cancel(id: CancelID.undo),
                    .run { send in
                        guard let change = Self.undoable(try await daemon.boardLog(project, from, nil), from: from)
                        else {
                            await send(.undoEnded)
                            return
                        }
                        await send(.undone(try await daemon.revert(change.id)))
                    } catch: { error, send in
                        await send(.undoEnded)
                        await send(.actionFailed("Couldn't undo: \(error.localizedDescription)"))
                    })

            case .undoEnded:
                state.undoing = false
                return .none

            case .undone(let reverted):
                state.undoing = false
                if !reverted.kept.isEmpty {
                    state.present(BoardLogFeature.keptAlert(reverted.kept))
                }
                return .send(.refresh)

            case .refresh:
                let project = state.project.id
                return .run { send in await send(await load(project)) }

            case .loaded(let items, let sessions):
                // Read every two seconds and mostly the same: set only what
                // changed, so the board isn't drawn again for nothing.
                if state.failure != nil { state.failure = nil }
                var items = IdentifiedArray(items, uniquingIDsWith: { _, last in last })
                for (id, status) in state.moving {
                    items[id: id]?.fields.status = status
                }
                let sessions = IdentifiedArray(sessions, uniquingIDsWith: { _, last in last })
                let itemsChanged = items != state.items
                let sessionsChanged = !Self.showsTheSame(sessions, as: state.sessions)
                guard itemsChanged || sessionsChanged else {
                    if let opening = state.opening {
                        state.opening = nil
                        return .send(.cardTapped(opening))
                    }
                    return .none
                }
                if itemsChanged { state.items = items }
                if sessionsChanged { state.sessions = sessions }
                // A deleted item's comments and edit have nothing to go on.
                if !state.keptComments.isEmpty {
                    state.keptComments = state.keptComments.filter { items[id: $0.key] != nil }
                }
                if !state.keptDrafts.isEmpty {
                    state.keptDrafts = state.keptDrafts.filter { items[id: $0.key] != nil }
                }
                if case .item(var page) = state.destination, let item = state.items[id: page.item.id] {
                    let before = page
                    page.item = item
                    page.parent = state.parent(of: item)
                    page.subtasks = state.subtasks(of: item)
                    page.sessions = state.sessions(on: item)
                    if page != before { state.destination = .item(page) }
                }
                if let opening = state.opening {
                    state.opening = nil
                    return .send(.cardTapped(opening))
                }
                return .none

            case .failed(let message):
                state.failure = message
                return .none

            case .actionFailed(let message):
                state.importing = false
                state.actionFailure = message
                return .none

            case .actionFailureDismissed:
                state.actionFailure = nil
                return .none

            case .startFinished(let id):
                state.starting.remove(id)
                return .none

            case .importIssuesTapped:
                guard !state.importing else { return .none }
                state.importing = true
                let project = state.project.id
                return .run { send in
                    await send(.imported(try await daemon.importIssues(project)))
                } catch: { error, send in
                    await send(.actionFailed(error.localizedDescription))
                }

            case .imported(let imported):
                state.importing = false
                let made = imported.items.count
                let before = imported.skipped == 0 ? "" : " \(imported.skipped) were imported before."
                state.present(
                    AlertState {
                        TextState(
                            made == 0 ? "No New Issues" : made == 1 ? "Imported 1 Issue" : "Imported \(made) Issues")
                    } message: {
                        TextState(
                            (made == 0
                                ? "Every open issue of the project's repositories on GitHub is a task already."
                                : "Each is a task in To Do, its spec the issue's text.") + before)
                    })
                return made == 0 ? .none : .send(.refresh)

            case .addTapped(let parent):
                state.destination = .newItem(
                    NewItemFeature.State(project: state.project, parent: parent.flatMap { state.items[id: $0] }))
                return .none

            case .cardTapped(let id):
                // Its page is open already; opening it again would drop what
                // is being written there.
                if case .item(let page)? = state.destination, page.item.id == id { return .none }
                guard let item = state.items[id: id] else { return .none }
                state.destination = .item(state.page(of: item))
                return .none

            case .startWorkTapped(let id):
                guard let item = state.items[id: id], !state.starting.contains(id) else { return .none }
                let project = state.project
                state.starting.insert(id)
                return .run { send in
                    let settings = await startWorkSettings(project: project, item: item, with: daemon)
                    guard !settings.needsModel else {
                        await send(.startFinished(id))
                        await send(.startNeedsModel(id, settings))
                        return
                    }
                    _ = try await daemon.startWork(settings.request)
                    await send(.startFinished(id))
                    await send(await load(project.id))
                } catch: { error, send in
                    await send(.startFinished(id))
                    await send(.actionFailed("Couldn't start work on \(item.title): \(error.localizedDescription)"))
                }

            case .startNeedsModel(let id, let settings):
                guard let item = state.items[id: id] else { return .none }
                var page = state.page(of: item)
                page.destination = .newSession(settings)
                state.destination = .item(page)
                return .none

            case .dropped(let id, let status):
                guard let item = state.items[id: id], item.fields.status != status else { return .none }
                let live = state.sessions(on: item).filter { !$0.archived }
                if status.closed, !live.isEmpty {
                    state.destination = .alert(.closing(item, to: status, sessions: live.count))
                    return .none
                }
                return move(id, to: status, archiving: [], in: &state)

            case .destination(.presented(.alert(.stopAndArchive(let id, let status)))):
                guard let item = state.items[id: id] else { return .none }
                return move(id, to: status, archiving: state.sessions(on: item).filter { !$0.archived }, in: &state)

            case .destination(.presented(.alert(.keep(let id, let status)))):
                return move(id, to: status, archiving: [], in: &state)

            case .moveFailed(let id, let was, let message):
                state.moving[id] = nil
                state.items[id: id]?.fields.status = was
                state.actionFailure = message
                return .none

            case .moved(let item):
                state.moving[item.id] = nil
                state.items[id: item.id] = item
                // A task shows its subtasks' sessions, so read it all again.
                return .merge(.send(.refresh), .send(.changedHere))

            case .deleteTapped(let id):
                guard let item = state.items[id: id] else { return .none }
                state.destination = .alert(.deleting(item))
                return .none

            case .destination(.presented(.alert(.delete(let id)))):
                return .run { send in
                    try await daemon.deleteItem(id)
                    await send(.refresh)
                    await send(.changedHere)
                } catch: { error, send in
                    await send(.actionFailed(error.localizedDescription))
                }

            case .destination(.presented(.item(.delegate(.deleted)))):
                // Its kept comments go once the board reads it's gone.
                state.destination = nil
                return .merge(.send(.refresh), .send(.changedHere))

            case .destination(.presented(.item(.delegate(.openItem(let id))))):
                return .send(.cardTapped(id))

            case .logTapped:
                state.destination = .log(
                    BoardLogFeature.State(project: state.project, items: state.items, sessions: state.sessions))
                return .none

            case .destination(.presented(.newItem(.delegate(.created)))):
                state.destination = nil
                return .merge(.send(.refresh), .send(.changedHere))

            case .destination(.presented(.item(.delegate(.changed)))):
                return .merge(.send(.refresh), .send(.changedHere))

            case .destination(.presented(.log(.delegate(.changed)))):
                return .send(.refresh)

            case .destination(.presented(.item(.delegate(.openSession(let session))))):
                state.destination = nil
                return .send(.delegate(.openSession(session)))

            case .proposals(.decided(_, .applied)):
                return .send(.refresh)

            case .destination, .proposals, .delegate:
                return .none
            }
        }
        .ifLet(\.$destination, action: \.destination)
    }

    /// The newest change made in this app from `from` on that isn't
    /// itself a revert and hasn't been reverted, in `changes`, newest first.
    /// A revert is always newer than the change it reverts, so the changes
    /// from `from` on are all it needs.
    static func undoable(_ changes: [BoardChange], from: BoardChange.ID) -> BoardChange? {
        let reverted = Set(changes.compactMap(\.reverts))
        return changes.first { change in
            change.author == .app && change.reverts == nil && change.id >= from && !reverted.contains(change.id)
        }
    }

    /// Whether the board shows `sessions` as it does `shown`. When a session
    /// last did something and what it last said change with every event of
    /// a working one, and show nowhere on the board, so they don't count.
    /// The sessions are kept whole all the same, for whoever opens one.
    private static func showsTheSame(
        _ sessions: IdentifiedArrayOf<Session>, as shown: IdentifiedArrayOf<Session>
    ) -> Bool {
        sessions.count == shown.count
            && zip(sessions, shown).allSatisfy { new, old in
                var new = new
                new.lastActive = old.lastActive
                new.lastMessage = old.lastMessage
                return new == old
            }
    }

    private func load(_ project: Project.ID) async -> Action {
        do {
            async let items = daemon.items(project)
            async let sessions = daemon.sessions(project)
            return .loaded(items: try await items, sessions: try await sessions)
        } catch {
            return .failed(error.localizedDescription)
        }
    }

    /// Moves the item to `status`, stopping and archiving `sessions` first;
    /// the card shows there at once and goes back if the daemon refuses.
    private func move(
        _ id: Item.ID, to status: ItemStatus, archiving sessions: [Session], in state: inout State
    ) -> Effect<Action> {
        guard let was = state.items[id: id]?.fields.status else { return .none }
        state.moving[id] = status
        state.items[id: id]?.fields.status = status
        return .run { send in
            try await stopAndArchive(sessions, with: daemon)
            await send(.moved(try await daemon.updateItem(id, ItemUpdate(status: status))))
        } catch: { error, send in
            await send(.moveFailed(id, was: was, error.localizedDescription))
        }
    }
}

extension BoardFeature.Destination.State: Equatable {}

/// What Start Work starts a session on `item` with: every project's
/// defaults with the project's over them, as New Session starts from, or
/// only the project's when every project's can't be read.
private func startWorkSettings(
    project: Project, item: Item, with daemon: DaemonClient
) async -> NewSessionFeature.State {
    var settings = NewSessionFeature.State(project: project, item: item)
    if let every = try? await daemon.sessionDefaults() { settings.applyDefaults(every) }
    return settings
}

/// A task's or subtask's page: its spec, shown as Markdown and edited in
/// place; its status and priority; its subtasks; and its sessions with how
/// many files each changed. Starts a session on it, with the specs it reads.
@Reducer
package struct ItemFeature {
    /// Identified by its item, so another item's page opened in its place
    /// stops what this one was reading, and none of it lands there.
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var id: Item.ID { item.id }
        package var project: Project
        package var item: Item
        /// The task above a subtask.
        package var parent: Item?
        package var subtasks: [Item]
        package var sessions: [Session]
        /// How many files each session changed, by session id, once read.
        package var changedFiles: [Session.ID: Int] = [:]
        /// Its sessions' pull requests, as last read.
        package var pullRequests: [PullRequest] = []
        package var readingPullRequests = false
        /// Whether Humata reads them only on Refresh, as Settings › GitHub
        /// can have it, so the page says so.
        package var pullsReadOnRefreshOnly = false
        /// The title and spec being edited; nil while they're only shown.
        package var draft: Draft?
        package var saving = false
        /// Work is starting on it, so a second tap starts no second session.
        package var startingWork = false
        /// Comments on the spec's blocks, to send to one of its sessions.
        package var comments: SpecCommentsFeature.State
        @Presents package var destination: Destination.State?
        /// An alert about something that finished while a sheet was open,
        /// shown once the sheet goes.
        var heldAlert: AlertState<BoardAlert>?

        package init(project: Project, item: Item, parent: Item?, subtasks: [Item], sessions: [Session]) {
            self.project = project
            self.item = item
            comments = SpecCommentsFeature.State(title: item.title)
            self.parent = parent
            self.subtasks = subtasks
            self.sessions = sessions
        }

        /// The sessions spec comments can go to: archived ones don't work.
        package var commentTargets: [Session] { sessions.filter { !$0.archived } }

        /// Its sessions, those waiting on the person first, then those
        /// working, then the rest, and archived ones last.
        package var sortedSessions: [Session] {
            func rank(_ session: Session) -> Int {
                if session.archived { return 3 }
                if session.asking { return 0 }
                switch session.state {
                case .waitingForApproval, .failed: return 0
                case .working: return 1
                case .idle: return 2
                }
            }
            return sessions.enumerated()
                .sorted { (rank($0.element), $0.offset) < (rank($1.element), $1.offset) }
                .map(\.element)
        }

        /// Whether the draft differs from the item, so cancelling loses work.
        var draftChanged: Bool {
            guard let draft else { return false }
            return draft.title != item.title || draft.spec != item.spec
        }

        /// Shows `alert` now, or once the sheet that's open goes, so a late
        /// failure doesn't take away what the person is filling in.
        mutating func present(_ alert: AlertState<BoardAlert>) {
            switch destination {
            case nil, .alert: destination = .alert(alert)
            default: heldAlert = alert
            }
        }

        /// Clears what was in progress when its board was put away.
        fileprivate mutating func settle() {
            saving = false
            startingWork = false
            readingPullRequests = false
            comments.sending = false
            heldAlert = nil
            if case .newSubtask(var new) = destination {
                new.creating = false
                destination = .newSubtask(new)
            }
        }
    }

    package struct Draft: Equatable {
        package var title: String
        package var spec: String
        /// Whether the spec shows rendered instead of as Markdown to edit.
        package var previewing: Bool

        package init(title: String, spec: String, previewing: Bool = false) {
            self.title = title
            self.spec = spec
            self.previewing = previewing
        }
    }

    @Reducer
    package enum Destination {
        case newSession(NewSessionFeature)
        case newSubtask(NewItemFeature)
        case alert(AlertState<BoardAlert>)
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The page appeared: reads what each session changed.
        case task
        case changesRead(Session.ID, files: Int)
        case pullRequestsRead([PullRequest])
        case pullRefreshRead(PullRefresh)
        /// Reads its pull requests again from GitHub.
        case refreshPullRequestsTapped
        case editTapped
        case cancelEditTapped
        case saveTapped
        case statusPicked(ItemStatus)
        case priorityPicked(Priority)
        case updated(Item)
        /// The draft's save answered: it closes.
        case saved(Item)
        case failed(String)
        case sessionTapped(Session.ID)
        case startSessionTapped
        /// Starts a session on the item with the last New Session settings,
        /// and has it start, staying on the page.
        case startWorkTapped
        case workStarted
        /// Start Work's defaults leave a local model to pick; New Session
        /// asks.
        case startNeedsModel(NewSessionFeature.State)
        case startWorkFailed(String)
        case addSubtaskTapped
        case deleteTapped
        /// A subtask, or the task above a subtask: its page opens instead.
        case itemTapped(Item.ID)
        case comments(SpecCommentsFeature.Action)
        case destination(PresentationAction<Destination.Action>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            /// The board changed; read it again.
            case changed
            case openSession(Session)
            /// The item was deleted; the page closes.
            case deleted
            case openItem(Item.ID)
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Scope(state: \.comments, action: \.comments) { SpecCommentsFeature() }
        Reduce { state, action in
            switch action {
            case .binding, .delegate, .comments:
                return .none

            case .task:
                let sessions = state.sessions.map(\.id)
                let item = state.item.id
                return .run { send in
                    // A task that never landed has none; GitHub is read on
                    // asking, not every time the page opens.
                    if let pulls = try? await daemon.pullRequests(.item(item), false) {
                        await send(.pullRequestsRead(pulls))
                    }
                    // Unread, the page just doesn't say they wait for Refresh.
                    if let refresh = try? await daemon.pullRefresh() {
                        await send(.pullRefreshRead(refresh))
                    }
                    // Only the files are counted, so no lines around each
                    // change are read, and the sessions are read side by side.
                    await withTaskGroup(of: Void.self) { group in
                        for session in sessions {
                            group.addTask {
                                // A session whose copy is gone, or whose host
                                // is away, just shows no count.
                                guard let changes = try? await daemon.changes(session, ChangesQuery(context: 0))
                                else { return }
                                await send(.changesRead(session, files: changes.map(\.files.count).reduce(0, +)))
                            }
                        }
                    }
                }

            case .refreshPullRequestsTapped:
                guard !state.readingPullRequests else { return .none }
                state.readingPullRequests = true
                let item = state.item.id
                return .run { send in
                    await send(.pullRequestsRead(try await daemon.pullRequests(.item(item), true)))
                    await send(.delegate(.changed))
                } catch: { error, send in
                    // The pull requests that did refresh are kept; show them.
                    if let pulls = try? await daemon.pullRequests(.item(item), false) {
                        await send(.pullRequestsRead(pulls))
                    }
                    await send(.failed(error.localizedDescription))
                }

            case .pullRequestsRead(let pulls):
                state.readingPullRequests = false
                state.pullRequests = pulls
                return .none

            case .pullRefreshRead(let refresh):
                state.pullsReadOnRefreshOnly = !refresh.automatic
                return .none

            case .changesRead(let session, let files):
                state.changedFiles[session] = files
                return .none

            case .editTapped:
                state.draft = Draft(title: state.item.title, spec: state.item.spec)
                return .none

            case .cancelEditTapped:
                guard state.draftChanged else {
                    state.draft = nil
                    return .none
                }
                state.destination = .alert(.discarding(state.item))
                return .none

            case .destination(.presented(.alert(.discard))):
                state.draft = nil
                return .none

            case .saveTapped:
                guard let draft = state.draft, !state.saving else { return .none }
                let title = draft.title.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !title.isEmpty else {
                    return .send(.failed("A task needs a title. Give it one, or cancel to keep the old one."))
                }
                state.saving = true
                // The spec only when it changed, so a rename reaches no agent.
                let spec = draft.spec == state.item.spec ? nil : draft.spec
                let id = state.item.id
                return .run { send in
                    await send(.saved(try await daemon.updateItem(id, ItemUpdate(title: title, spec: spec))))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .statusPicked(let status):
                guard status != state.item.fields.status else { return .none }
                let live = state.sessions.filter { !$0.archived }
                if status.closed, !live.isEmpty {
                    state.destination = .alert(.closing(state.item, to: status, sessions: live.count))
                    return .none
                }
                return update(state.item.id, ItemUpdate(status: status))

            case .destination(.presented(.alert(.stopAndArchive(_, let status)))):
                let live = state.sessions.filter { !$0.archived }
                let id = state.item.id
                return .run { send in
                    try await stopAndArchive(live, with: daemon)
                    await send(.updated(try await daemon.updateItem(id, ItemUpdate(status: status))))
                    await send(.delegate(.changed))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .destination(.presented(.alert(.keep(_, let status)))):
                return update(state.item.id, ItemUpdate(status: status))

            case .deleteTapped:
                state.destination = .alert(.deleting(state.item))
                return .none

            case .itemTapped(let id):
                return .send(.delegate(.openItem(id)))

            case .destination(.presented(.alert(.delete(let id)))):
                return .run { send in
                    try await daemon.deleteItem(id)
                    await send(.delegate(.deleted))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .priorityPicked(let priority):
                guard priority != state.item.priority else { return .none }
                return update(state.item.id, ItemUpdate(priority: priority))

            case .updated(let item):
                // A priority or status picked while editing keeps the draft.
                state.item = item
                state.comments.title = item.title
                return .send(.delegate(.changed))

            case .saved(let item):
                state.item = item
                state.comments.title = item.title
                state.draft = nil
                state.saving = false
                return .send(.delegate(.changed))

            case .failed(let message):
                let title =
                    state.readingPullRequests
                    ? String(localized: "Couldn't refresh the pull requests")
                    : String(localized: "Couldn't change \(state.item.title)")
                state.saving = false
                state.readingPullRequests = false
                state.present(
                    AlertState {
                        TextState(title)
                    } message: {
                        TextState(message)
                    })
                return .none

            case .sessionTapped(let id):
                guard let session = state.sessions.first(where: { $0.id == id }) else { return .none }
                return .send(.delegate(.openSession(session)))

            case .startSessionTapped:
                state.destination = .newSession(NewSessionFeature.State(project: state.project, item: state.item))
                return .none

            case .startWorkTapped:
                guard !state.startingWork else { return .none }
                state.startingWork = true
                let (project, item) = (state.project, state.item)
                return .run { send in
                    let settings = await startWorkSettings(project: project, item: item, with: daemon)
                    guard !settings.needsModel else {
                        await send(.workStarted)
                        await send(.startNeedsModel(settings))
                        return
                    }
                    _ = try await daemon.startWork(settings.request)
                    await send(.workStarted)
                    await send(.delegate(.changed))
                } catch: { error, send in
                    await send(.startWorkFailed(error.localizedDescription))
                }

            case .workStarted:
                state.startingWork = false
                return .none

            case .startNeedsModel(let settings):
                state.destination = .newSession(settings)
                return .none

            case .startWorkFailed(let message):
                state.startingWork = false
                state.present(
                    AlertState {
                        TextState("Couldn't start work on \(state.item.title)")
                    } message: {
                        TextState(message)
                    })
                return .none

            case .addSubtaskTapped:
                state.destination = .newSubtask(NewItemFeature.State(project: state.project, parent: state.item))
                return .none

            case .destination(.presented(.newSession(.delegate(.started(let session))))):
                state.destination = nil
                return .send(.delegate(.openSession(session)))

            case .destination(.presented(.newSubtask(.delegate(.created(let subtask))))):
                state.destination = nil
                state.subtasks.append(subtask)
                return .send(.delegate(.changed))

            case .destination:
                return .none
            }
        }
        .ifLet(\.$destination, action: \.destination)
        // A held alert shows once the sheet it waited behind is gone.
        Reduce { state, _ in
            guard state.destination == nil, let alert = state.heldAlert else { return .none }
            state.heldAlert = nil
            state.destination = .alert(alert)
            return .none
        }
    }

    private func update(_ id: Item.ID, _ update: ItemUpdate) -> Effect<Action> {
        .run { send in
            await send(.updated(try await daemon.updateItem(id, update)))
        } catch: { error, send in
            await send(.failed(error.localizedDescription))
        }
    }
}

extension ItemFeature.Destination.State: Equatable {}

/// A new task, or a subtask of one: its title, spec and priority.
@Reducer
package struct NewItemFeature {
    @ObservableState
    package struct State: Equatable {
        package var project: Project
        /// The task it goes under; nil for a task.
        package var parent: Item?
        package var title = ""
        /// Markdown.
        package var spec = ""
        /// Whether the spec shows rendered instead of as Markdown to edit.
        package var previewingSpec = false
        package var priority = Priority.none
        package var creating = false
        @Presents package var alert: AlertState<Never>?

        package init(project: Project, parent: Item? = nil) {
            self.project = project
            self.parent = parent
        }

        package var canCreate: Bool {
            !creating && !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case createTapped
        case failed(String)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case created(Item)
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding, .alert, .delegate:
                return .none

            case .createTapped:
                guard state.canCreate else { return .none }
                state.creating = true
                let project = state.project.id
                let fields = ItemFields(
                    parentID: state.parent?.id, title: state.title.trimmingCharacters(in: .whitespacesAndNewlines),
                    spec: state.spec, priority: state.priority)
                return .run { send in
                    await send(.delegate(.created(try await daemon.createItem(project, fields))))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .failed(let message):
                state.creating = false
                state.alert = AlertState {
                    TextState(state.parent == nil ? "Couldn't add the task" : "Couldn't add the subtask")
                } message: {
                    TextState(message)
                }
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }
}

/// The last change made on the board in this app, which the toast offers
/// to undo.
package struct UndoToast: Equatable, Sendable {
    package var change: BoardChange.ID
    package var message: String
}

/// The board's journal: every change to a project's board, newest first,
/// with who made it and what it changed, each of which can be reverted.
@Reducer
package struct BoardLogFeature {
    @ObservableState
    package struct State: Equatable {
        package var project: Project
        /// The board's items when the log opened, for the titles of changes
        /// whose fields don't name them.
        package var items: IdentifiedArrayOf<Item>
        /// The project's sessions, for the names of those whose proposals
        /// or requests made changes.
        package var sessions: IdentifiedArrayOf<Session>
        package var changes: [BoardChange] = []
        /// Only this author's changes; nil for everyone's.
        package var author: BoardAuthor?
        /// The changes were read, so none means the board has none.
        package var loaded = false
        /// The changes couldn't be read and none are shown; the alert says
        /// why, and the list offers to read them again.
        package var unread = false
        /// The change being reverted.
        package var reverting: BoardChange.ID?
        @Presents package var alert: AlertState<Never>?

        package init(
            project: Project, items: IdentifiedArrayOf<Item>, sessions: IdentifiedArrayOf<Session> = []
        ) {
            self.project = project
            self.items = items
            self.sessions = sessions
        }

        /// Everyone who made a change, in the order of their latest one.
        package var authors: [BoardAuthor] {
            var seen = Set<BoardAuthor>()
            return changes.map(\.author).filter { seen.insert($0).inserted }
        }

        /// The changes the filter lets through.
        package var shown: [BoardChange] {
            guard let author else { return changes }
            return changes.filter { $0.author == author }
        }

        /// The change that reverted `change`, if one did.
        package func revert(of change: BoardChange) -> BoardChange? {
            changes.first { $0.reverts == change.id }
        }

        /// Who made changes, in words.
        package func name(of author: BoardAuthor) -> String {
            switch author {
            case .app: String(localized: "You, in Humata")
            case .commandLine: String(localized: "You, with the humata command")
            case .mcp: String(localized: "An outside agent, through humata mcp")
            case .humata: String(localized: "Humata")
            case .session(let id):
                sessions[id: id].map { String(localized: "\($0.name), approved by you") }
                    ?? String(localized: "A session, approved by you")
            case .unknown: String(localized: "Not recorded")
            }
        }
    }

    package enum Action: BindableAction {
        case task
        case loaded([BoardChange])
        case revertTapped(BoardChange.ID)
        case reverted(Reverted)
        case failed(String)
        case binding(BindingAction<State>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case changed
        }
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .task:
                state.unread = false
                return read(state.project.id)

            case .loaded(let changes):
                state.loaded = true
                state.unread = false
                state.changes = changes
                if let author = state.author, !changes.contains(where: { $0.author == author }) {
                    state.author = nil
                }
                return .none

            case .revertTapped(let id):
                guard state.reverting == nil else { return .none }
                state.reverting = id
                return .run { send in
                    await send(.reverted(try await daemon.revert(id)))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .reverted(let reverted):
                state.reverting = nil
                if !reverted.kept.isEmpty {
                    state.alert = Self.keptAlert(reverted.kept)
                }
                return .merge(read(state.project.id), .send(.delegate(.changed)))

            case .failed(let message):
                let title =
                    state.reverting == nil
                    ? String(localized: "Couldn't read the board's history")
                    : String(localized: "Couldn't revert the change")
                if state.reverting == nil, !state.loaded { state.unread = true }
                state.reverting = nil
                state.alert = AlertState {
                    TextState(title)
                } message: {
                    TextState(message)
                }
                return .none

            case .binding, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    /// Says which fields a revert left alone, and why.
    static func keptAlert<Action>(_ kept: [ItemField]) -> AlertState<Action> {
        let names = kept.map(\.name).formatted(.list(type: .and))
        return AlertState {
            TextState("Reverted, except the \(names)")
        } message: {
            TextState(
                "The \(names) changed again after this change, so reverting it there would undo that later "
                    + "change too. Revert the later change first to put it back as well.")
        }
    }

    private func read(_ project: Project.ID) -> Effect<Action> {
        .run { send in
            await send(.loaded(try await daemon.boardLog(project, nil, nil)))
        } catch: { error, send in
            await send(.failed(error.localizedDescription))
        }
    }
}

extension ItemField {
    /// The field's name, in a sentence.
    var name: String {
        switch self {
        case .title: String(localized: "title")
        case .spec: String(localized: "spec")
        case .status: String(localized: "status")
        case .priority: String(localized: "priority")
        }
    }
}

extension DaemonClient {
    /// What a session started from a card is first told.
    static let startWorkMessage = "Do this task as its spec says."

    /// Starts a session and has it start on its item.
    func startWork(_ request: StartRequest) async throws -> Session {
        let session = try await start(request)
        try await send(session.id, Self.startWorkMessage, .enqueue)
        return session
    }
}
