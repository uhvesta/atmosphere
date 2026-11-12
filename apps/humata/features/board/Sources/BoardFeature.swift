import ComposableArchitecture
import Foundation
import HumataDaemonClient
import HumataProjects

/// A project's board: its tasks and subtasks in a column per status, read
/// again every few seconds as their sessions move them. Dropping a card on
/// a column gives it that status; a card opens its item's page.
@Reducer
package struct BoardFeature {
    @ObservableState
    package struct State: Equatable {
        package var project: Project
        package var items: IdentifiedArrayOf<Item> = []
        /// The project's sessions.
        package var sessions: IdentifiedArrayOf<Session> = []
        /// Why the board couldn't be read or changed, and what to do; nil
        /// once it was.
        package var failure: String?
        @Presents package var destination: Destination.State?
        /// The item to open once the board is read, as when a search found it.
        package var opening: Item.ID?
        /// Its repositories' issues are being made tasks.
        package var importing = false

        package init(project: Project, opening: Item.ID? = nil) {
            self.project = project
            self.opening = opening
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
        case failed(String)
        /// Adds a task, or a subtask under `parent`.
        case addTapped(parent: Item.ID?)
        case cardTapped(Item.ID)
        /// Starts a session on the item with the last New Session settings,
        /// and has it start, staying on the board.
        case startWorkTapped(Item.ID)
        case dropped(Item.ID, on: ItemStatus)
        case moved(Item)
        /// Asks to delete the item, then does.
        case deleteTapped(Item.ID)
        case logTapped
        /// Makes a task of each open GitHub issue of the project's
        /// repositories not imported before.
        case importIssuesTapped
        case imported(ImportedIssues)
        case destination(PresentationAction<Destination.Action>)
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

    private enum CancelID { case refresh }

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                let project = state.project.id
                return .run { send in
                    while !Task.isCancelled {
                        await send(await load(project))
                        try await clock.sleep(for: Self.refreshInterval)
                    }
                }
                .cancellable(id: CancelID.refresh, cancelInFlight: true)

            case .refresh:
                let project = state.project.id
                return .run { send in await send(await load(project)) }

            case .loaded(let items, let sessions):
                state.failure = nil
                state.items = IdentifiedArray(items, uniquingIDsWith: { _, last in last })
                state.sessions = IdentifiedArray(sessions, uniquingIDsWith: { _, last in last })
                if case .item(var page) = state.destination, let item = state.items[id: page.item.id] {
                    page.item = item
                    page.parent = state.parent(of: item)
                    page.subtasks = state.subtasks(of: item)
                    page.sessions = state.sessions(on: item)
                    state.destination = .item(page)
                }
                if let opening = state.opening {
                    state.opening = nil
                    return .send(.cardTapped(opening))
                }
                return .none

            case .failed(let message):
                state.importing = false
                state.failure = message
                return .none

            case .importIssuesTapped:
                guard !state.importing else { return .none }
                state.importing = true
                let project = state.project.id
                return .run { send in
                    await send(.imported(try await daemon.importIssues(project)))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .imported(let imported):
                state.importing = false
                let made = imported.items.count
                let before = imported.skipped == 0 ? "" : " \(imported.skipped) were imported before."
                state.destination = .alert(
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
                guard let item = state.items[id: id] else { return .none }
                state.destination = .item(
                    ItemFeature.State(
                        project: state.project, item: item, parent: state.parent(of: item),
                        subtasks: state.subtasks(of: item), sessions: state.sessions(on: item)))
                return .none

            case .startWorkTapped(let id):
                guard let item = state.items[id: id] else { return .none }
                let settings = NewSessionFeature.State(project: state.project, item: item)
                guard !settings.needsModel else {
                    // A local model must be picked; the item's page asks.
                    var page = ItemFeature.State(
                        project: state.project, item: item, parent: state.parent(of: item),
                        subtasks: state.subtasks(of: item), sessions: state.sessions(on: item))
                    page.destination = .newSession(settings)
                    state.destination = .item(page)
                    return .none
                }
                let project = state.project.id
                return .run { send in
                    _ = try await daemon.startWork(settings.request)
                    await send(await load(project))
                } catch: { error, send in
                    await send(.failed("Couldn't start work on \(item.title): \(error.localizedDescription)"))
                }

            case .dropped(let id, let status):
                guard let item = state.items[id: id], item.fields.status != status else { return .none }
                let live = state.sessions(on: item).filter { !$0.archived }
                if status.closed, !live.isEmpty {
                    state.destination = .alert(.closing(item, to: status, sessions: live.count))
                    return .none
                }
                return move(id, to: status, archiving: [])

            case .destination(.presented(.alert(.stopAndArchive(let id, let status)))):
                guard let item = state.items[id: id] else { return .none }
                return move(id, to: status, archiving: state.sessions(on: item).filter { !$0.archived })

            case .destination(.presented(.alert(.keep(let id, let status)))):
                return move(id, to: status, archiving: [])

            case .moved(let item):
                state.items[id: item.id] = item
                // A task shows its subtasks' sessions, so read it all again.
                return .send(.refresh)

            case .deleteTapped(let id):
                guard let item = state.items[id: id] else { return .none }
                state.destination = .alert(.deleting(item))
                return .none

            case .destination(.presented(.alert(.delete(let id)))):
                return .run { send in
                    try await daemon.deleteItem(id)
                    await send(.refresh)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .destination(.presented(.item(.delegate(.deleted)))):
                state.destination = nil
                return .send(.refresh)

            case .destination(.presented(.item(.delegate(.openItem(let id))))):
                return .send(.cardTapped(id))

            case .logTapped:
                state.destination = .log(BoardLogFeature.State(project: state.project, items: state.items))
                return .none

            case .destination(.presented(.newItem(.delegate(.created)))):
                state.destination = nil
                return .send(.refresh)

            case .destination(.presented(.item(.delegate(.changed)))),
                .destination(.presented(.log(.delegate(.changed)))):
                return .send(.refresh)

            case .destination(.presented(.item(.delegate(.openSession(let session))))):
                state.destination = nil
                return .send(.delegate(.openSession(session)))

            case .destination, .delegate:
                return .none
            }
        }
        .ifLet(\.$destination, action: \.destination)
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

    /// Moves the item to `status`, stopping and archiving `sessions` first.
    private func move(_ id: Item.ID, to status: ItemStatus, archiving sessions: [Session]) -> Effect<Action> {
        .run { send in
            try await stopAndArchive(sessions, with: daemon)
            await send(.moved(try await daemon.updateItem(id, ItemUpdate(status: status))))
        } catch: { error, send in
            await send(.failed(error.localizedDescription))
        }
    }
}

extension BoardFeature.Destination.State: Equatable {}

/// A task's or subtask's page: its spec, shown as Markdown and edited in
/// place; its status and priority; its subtasks; and its sessions with how
/// many files each changed. Starts a session on it, with the specs it reads.
@Reducer
package struct ItemFeature {
    @ObservableState
    package struct State: Equatable {
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
        /// The title and spec being edited; nil while they're only shown.
        package var draft: Draft?
        package var saving = false
        /// Comments on the spec's blocks, to send to one of its sessions.
        package var comments: SpecCommentsFeature.State
        @Presents package var destination: Destination.State?

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
        /// Reads its pull requests again from GitHub.
        case refreshPullRequestsTapped
        case editTapped
        case cancelEditTapped
        case saveTapped
        case statusPicked(ItemStatus)
        case priorityPicked(Priority)
        case updated(Item)
        case failed(String)
        case sessionTapped(Session.ID)
        case startSessionTapped
        /// Starts a session on the item with the last New Session settings,
        /// and has it start, staying on the page.
        case startWorkTapped
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
                    for session in sessions {
                        // A session whose copy is gone, or whose host is away,
                        // just shows no count.
                        guard let changes = try? await daemon.changes(session, ChangesQuery()) else { continue }
                        await send(.changesRead(session, files: changes.map(\.files.count).reduce(0, +)))
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
                    await send(.failed(error.localizedDescription))
                }

            case .pullRequestsRead(let pulls):
                state.readingPullRequests = false
                state.pullRequests = pulls
                return .none

            case .changesRead(let session, let files):
                state.changedFiles[session] = files
                return .none

            case .editTapped:
                state.draft = Draft(title: state.item.title, spec: state.item.spec)
                return .none

            case .cancelEditTapped:
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
                return update(state.item.id, ItemUpdate(title: title, spec: spec))

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
                state.item = item
                state.comments.title = item.title
                state.draft = nil
                state.saving = false
                return .send(.delegate(.changed))

            case .failed(let message):
                state.saving = false
                state.readingPullRequests = false
                state.destination = .alert(
                    AlertState {
                        TextState("Couldn't change \(state.item.title)")
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
                let settings = NewSessionFeature.State(project: state.project, item: state.item)
                guard !settings.needsModel else {
                    state.destination = .newSession(settings)
                    return .none
                }
                return .run { send in
                    _ = try await daemon.startWork(settings.request)
                    await send(.delegate(.changed))
                } catch: { error, send in
                    await send(.startWorkFailed(error.localizedDescription))
                }

            case .startWorkFailed(let message):
                state.destination = .alert(
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

/// Every change to a project's board, newest first, each of which can be
/// reverted.
@Reducer
package struct BoardLogFeature {
    @ObservableState
    package struct State: Equatable {
        package var project: Project
        /// The board's items when the log opened, for the titles of changes
        /// whose fields don't name them.
        package var items: IdentifiedArrayOf<Item>
        package var changes: [BoardChange] = []
        /// The change being reverted.
        package var reverting: BoardChange.ID?
        @Presents package var alert: AlertState<Never>?

        package init(project: Project, items: IdentifiedArrayOf<Item>) {
            self.project = project
            self.items = items
        }
    }

    package enum Action {
        case task
        case loaded([BoardChange])
        case revertTapped(BoardChange.ID)
        case reverted
        case failed(String)
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
        Reduce { state, action in
            switch action {
            case .task:
                return read(state.project.id)

            case .loaded(let changes):
                state.changes = changes
                return .none

            case .revertTapped(let id):
                guard state.reverting == nil else { return .none }
                state.reverting = id
                return .run { send in
                    _ = try await daemon.revert(id)
                    await send(.reverted)
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .reverted:
                state.reverting = nil
                return .merge(read(state.project.id), .send(.delegate(.changed)))

            case .failed(let message):
                state.reverting = nil
                state.alert = AlertState {
                    TextState("Couldn't revert the change")
                } message: {
                    TextState(message)
                }
                return .none

            case .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$alert, action: \.alert)
    }

    private func read(_ project: Project.ID) -> Effect<Action> {
        .run { send in
            await send(.loaded(try await daemon.boardLog(project)))
        } catch: { error, send in
            await send(.failed(error.localizedDescription))
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
