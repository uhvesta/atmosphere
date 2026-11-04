import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAvenue
import LifrBoard
import LifrConversations
import LifrCore
import LifrProcrastinated
import LifrQuickAdd
import LifrRemindersClient
import LifrSearch
import LifrSessionClient
import LifrSettings
import LifrStoreClient
import LifrTaskDetail
import LifrTaskList
import LifrToday
import LifrUpcoming

/// The whole app: the iPhone's tabs or the Mac's sidebar, with quick add,
/// the sync banner and the approval sheet over them. It loads the records
/// once and passes every change on to each screen, so they never disagree,
/// and reschedules this device's reminders after each change.
@Reducer
package struct RootFeature {
    package enum Platform: Equatable, Sendable {
        /// Tabs, and no agents: only the Mac runs the CLI.
        case phone
        /// A sidebar, the list or board, and the task detail.
        case mac
    }

    package enum Tab: Hashable, Sendable {
        case today, upcoming, avenues, conversations, search
    }

    /// What the Mac's sidebar can show.
    package enum SidebarItem: Hashable, Sendable {
        case today, upcoming, procrastinated, conversations, search
        case scope(AvenueScope)
    }

    /// How the Mac shows an Avenue or Project.
    package enum Content: String, CaseIterable, Hashable, Sendable {
        case list, board

        package var title: String {
            switch self {
            case .list: String(localized: "List")
            case .board: String(localized: "Board")
            }
        }
    }

    @ObservableState
    package struct State: Equatable {
        package let platform: Platform
        /// `nil` until the records load.
        package var workspace: Workspace?
        package var tab = Tab.today
        package var selection: SidebarItem? = .today
        package var content = Content.list
        package var today: TodayFeature.State
        package var upcoming = UpcomingFeature.State()
        package var procrastinated = ProcrastinatedFeature.State()
        package var avenues = AvenuesFeature.State()
        package var search = SearchFeature.State()
        package var conversations = ConversationsFeature.State()
        /// The Mac's selected Avenue or Project, as a list.
        package var avenue: AvenueFeature.State?
        /// The same Avenue or Project, as a board.
        package var board: BoardFeature.State?
        package var settings: SettingsFeature.State
        package var isSettingsShown = false
        /// What the last sync fixed, until dismissed.
        package var syncBanner: SyncBanner?
        /// Apps with a grant; the menu bar shows a dot while one is connected.
        package var apps: [ConnectedApp] = []
        /// Apps asking while another's approval sheet is up.
        package var queuedApprovals: [ApprovalRequest] = []
        /// Whether the CLI can connect. The menu bar turns it on and off.
        package var isAPIEnabled = true
        /// Why turning the local API on or off failed, until it next works.
        package var apiSwitchError: String?
        @Presents package var quickAdd: QuickAddFeature.State?
        @Presents package var approval: ApprovalFeature.State?
        /// Why Undo couldn't revert the last change.
        @Presents package var undoAlert: AlertState<Never>?
        @Shared(.lastChange) package var lastChange
        /// The last change whose toast went away; Undo stays in the Edit menu.
        package var dismissedToast: UUID?

        /// The last change, while its toast is up.
        package var toast: LastChange? {
            lastChange.flatMap { $0.showsToast && $0.id != dismissedToast ? $0 : nil }
        }

        package init(platform: Platform) {
            self.platform = platform
            today = TodayFeature.State()
            settings = SettingsFeature.State(showsAgents: platform == .mac)
        }

        package var hasConnectedApp: Bool { apps.contains(where: \.isConnected) }

        /// The list the person is looking at, which keyboard shortcuts act on.
        package var currentList: ListSlot? {
            switch platform {
            case .phone:
                switch tab {
                case .today: .today
                case .upcoming: .upcoming
                case .avenues: avenues.avenue == nil ? nil : .openAvenue
                case .search: .search
                case .conversations: nil
                }
            case .mac:
                switch selection {
                case .today: .today
                case .upcoming: .upcoming
                case .procrastinated: .procrastinated
                case .search: .search
                case .conversations: nil
                case .scope: content == .board ? .board : .avenue
                case nil: nil
                }
            }
        }

        package subscript(list slot: ListSlot) -> TaskListFeature.State? {
            switch slot {
            case .today: today.list
            case .upcoming: upcoming.list
            case .procrastinated: procrastinated.list
            case .search: search.list
            case .avenue: avenue?.list
            case .board: board?.list
            case .openAvenue: avenues.avenue?.list
            }
        }

        /// The task open in the current list's detail.
        package var openTaskID: UUID? {
            currentList.flatMap { self[list: $0]?.detail?.id }
        }
    }

    /// Where a task list lives in `State`.
    package enum ListSlot: Hashable, Sendable {
        case today, upcoming, procrastinated, search, avenue, board
        /// The Avenue opened from the iPhone's Avenues tab.
        case openAvenue
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The app launched: load the records and listen for changes.
        case task
        case loaded(Result<Workspace, any Error>)
        /// Another device or process changed the records.
        case updated(Update)
        case appsLoaded([ConnectedApp])
        case apiEnabledLoaded(Bool)
        /// The menu bar's Local API switch.
        case apiSwitched(Bool)
        case apiSwitchFailed(String)
        case sessionEvent(SessionEvent)
        case selected(SidebarItem?)
        /// ⌘N, or Task in the + menu.
        case newTaskTapped
        /// ⌘⇧N: Sub tasks of the open task. Does nothing without one.
        case newSubtaskTapped
        /// A sidebar Avenue's or Project's New Task.
        case newTaskInTapped(AvenueScope)
        /// The Mac sidebar's New Avenue.
        case newAvenueTapped
        /// Project in the + menu, or a sidebar Avenue's New Project: in that
        /// Avenue, or the one on screen.
        case newProjectTapped(UUID?)
        /// A status shortcut, for the open task.
        case statusShortcut(TaskStatus)
        /// The toast's Undo, ⌘Z or a shake, for the change with this ID.
        case undoTapped(UUID)
        case undone(Result<Saved, any Error>)
        case toastExpired(UUID)
        case undoAlert(PresentationAction<Never>)
        case syncBannerDismissed
        /// This device's notifications match the records again.
        case remindersScheduled
        /// The approval sheet closed: show the next app's, if one is waiting.
        case nextApproval
        case today(TodayFeature.Action)
        case upcoming(UpcomingFeature.Action)
        case procrastinated(ProcrastinatedFeature.Action)
        case avenues(AvenuesFeature.Action)
        case search(SearchFeature.Action)
        case conversations(ConversationsFeature.Action)
        case avenue(AvenueFeature.Action)
        case board(BoardFeature.Action)
        case settings(SettingsFeature.Action)
        case quickAdd(PresentationAction<QuickAddFeature.Action>)
        case approval(PresentationAction<ApprovalFeature.Action>)
    }

    private enum CancelID { case reminders }

    @Dependency(\.store) private var store
    @Dependency(\.session) private var session
    @Dependency(\.reminders) private var reminders
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        CombineReducers { screens }
            .transformDependency(\.self) { dependencies in
                dependencies.store = dependencies.store.recordingChanges(
                    into: Shared(.lastChange), uuid: dependencies.uuid)
            }
    }

    @ReducerBuilder<State, Action>
    private var screens: some ReducerOf<Self> {
        BindingReducer()
        Scope(state: \.today, action: \.today) { TodayFeature() }
        Scope(state: \.upcoming, action: \.upcoming) { UpcomingFeature() }
        Scope(state: \.procrastinated, action: \.procrastinated) { ProcrastinatedFeature() }
        Scope(state: \.avenues, action: \.avenues) { AvenuesFeature() }
        Scope(state: \.search, action: \.search) { SearchFeature() }
        Scope(state: \.conversations, action: \.conversations) { ConversationsFeature() }
        Scope(state: \.settings, action: \.settings) { SettingsFeature() }
        Reduce { state, action in
            switch action {
            case .task:
                var effects: [Effect<Action>] = [
                    .run { [store] send in await send(.loaded(Result { try await store.load() })) },
                    .run { [store] send in
                        for await update in await store.updates() { await send(.updated(update)) }
                    },
                ]
                if state.platform == .mac {
                    effects.append(
                        .run { [session] send in
                            if let apps = try? await session.apps() { await send(.appsLoaded(apps)) }
                            await send(.apiEnabledLoaded(await session.isAPIEnabled()))
                            for await event in await session.events() { await send(.sessionEvent(event)) }
                        })
                }
                return .merge(effects)

            case .loaded(.success(let workspace)):
                return show(workspace, in: &state)

            case .loaded(.failure):
                // Each screen loads on its own too, and says what failed.
                return .none

            case .updated(let update):
                if !update.fixes.isEmpty {
                    state.syncBanner = SyncBanner(update.fixes, in: update.workspace)
                }
                return show(update.workspace, in: &state)

            case .appsLoaded(let apps), .sessionEvent(.appsChanged(let apps)):
                state.apps = apps
                return .none

            case .apiEnabledLoaded(let enabled), .sessionEvent(.apiEnabledChanged(let enabled)):
                state.isAPIEnabled = enabled
                state.apiSwitchError = nil
                return .none

            case .apiSwitched(let enabled):
                state.isAPIEnabled = enabled
                return .run { [session] send in
                    do {
                        try await session.setAPIEnabled(enabled)
                    } catch {
                        await send(.apiEnabledLoaded(await session.isAPIEnabled()))
                        await send(.apiSwitchFailed(String(describing: error)))
                    }
                }

            case .apiSwitchFailed(let message):
                state.apiSwitchError = message
                return .none

            case .sessionEvent(.recordsChanged(let workspace)):
                return show(workspace, in: &state)

            case .sessionEvent(.approvalRequested(let request)):
                if state.approval == nil {
                    state.approval = ApprovalFeature.State(request: request, workspace: state.workspace ?? Workspace())
                } else {
                    state.queuedApprovals.append(request)
                }
                return .none

            case .sessionEvent(.approvalWithdrawn(let id)):
                state.queuedApprovals.removeAll { $0.id == id }
                if state.approval?.request.id == id {
                    state.approval = nil
                    showNextApproval(&state)
                }
                return .none

            case .approval(.dismiss):
                // The sheet is still up while its dismissal runs.
                return .send(.nextApproval)

            case .nextApproval:
                showNextApproval(&state)
                return .none

            case .selected(let item):
                state.selection = item
                state.avenue = nil
                state.board = nil
                guard case .scope(let scope) = item, let workspace = state.workspace else { return .none }
                let avenue = AvenueFeature.State(scope: scope, workspace: workspace, now: now, timeZone: timeZone)
                state.avenue = avenue
                state.board = BoardFeature.State(
                    scope: scope, title: avenue.title, workspace: workspace, now: now, timeZone: timeZone)
                return .none

            case .newTaskTapped:
                showQuickAdd(.task, in: &state)
                return .none

            case .newTaskInTapped(let scope):
                showQuickAdd(.task, NewTaskDefaults(scope: scope.newTaskScope), in: &state)
                return .none

            case .newAvenueTapped:
                return .send(.avenues(.newAvenueTapped))

            case .newProjectTapped(let avenueID):
                showQuickAdd(.project, avenueID.map { NewTaskDefaults(scope: .avenue($0)) }, in: &state)
                return .none

            case .newSubtaskTapped:
                // Sub tasks go under a task, so only with one open.
                guard let workspace = state.workspace, let id = state.openTaskID, let task = workspace.tasks[id]
                else { return .none }
                // A Sub task can't have Sub tasks: add to its parent.
                let parent = task.parentID.flatMap { workspace.tasks[$0] } ?? task
                var quickAdd = QuickAddFeature.State(workspace: workspace, now: now)
                quickAdd.parent = QuickAddFeature.Choice(id: parent.id, name: parent.title)
                state.quickAdd = quickAdd
                return .none

            case .statusShortcut(let status):
                guard let slot = state.currentList, state[list: slot]?.detail != nil else { return .none }
                return .send(list(slot, .detail(.presented(.statusPicked(status)))))

            case .undoTapped(let id):
                guard let last = state.lastChange, last.id == id, let workspace = state.workspace else {
                    return .none
                }
                guard last.isCurrent(in: workspace) else {
                    state.$lastChange.withLock { $0 = nil }
                    state.undoAlert = AlertState {
                        TextState("Couldn't undo")
                    } actions: {
                        ButtonState(role: .cancel) { TextState("OK") }
                    } message: {
                        TextState(
                            """
                            The task changed again since, here or on another device, and undoing would lose that \
                            change. Open the task to change it back by hand.
                            """)
                    }
                    return .none
                }
                return .run { [store] send in
                    await send(.undone(Result { try await store.undo(last.change) }))
                }

            case .undone(.success(let saved)):
                // Undoing isn't a change to undo in turn.
                state.$lastChange.withLock { $0 = nil }
                return show(saved.workspace, in: &state)

            case .undone(.failure(let error)):
                state.undoAlert = .failed("Couldn't undo", error)
                return .none

            case .toastExpired(let id):
                state.dismissedToast = id
                return .none

            case .syncBannerDismissed:
                state.syncBanner = nil
                return .none

            case .today(.delegate(.saved(let workspace))),
                .upcoming(.list(.delegate(.saved(let workspace)))),
                .procrastinated(.list(.delegate(.saved(let workspace)))),
                .avenues(.delegate(.saved(let workspace))),
                .search(.delegate(.saved(let workspace))),
                .avenue(.delegate(.saved(let workspace))),
                .board(.list(.delegate(.saved(let workspace)))),
                .settings(.delegate(.saved(let workspace))),
                .conversations(.delegate(.saved(let workspace))),
                .quickAdd(.presented(.delegate(.saved(let workspace)))):
                return show(workspace, in: &state)

            case .binding, .remindersScheduled, .today, .upcoming, .procrastinated, .avenues, .search, .avenue, .board,
                .conversations, .settings, .quickAdd, .approval, .undoAlert:
                return .none
            }
        }
        .ifLet(\.avenue, action: \.avenue) { AvenueFeature() }
        .ifLet(\.board, action: \.board) { BoardFeature() }
        .ifLet(\.$quickAdd, action: \.quickAdd) { QuickAddFeature() }
        .ifLet(\.$approval, action: \.approval) { ApprovalFeature() }
        .ifLet(\.$undoAlert, action: \.undoAlert)
    }

    /// Shows `workspace` on every screen, then schedules its reminders.
    private func show(_ workspace: Workspace, in state: inout State) -> Effect<Action> {
        state.workspace = workspace
        let effects: [Effect<Action>] = [
            TodayFeature().reduce(into: &state.today, action: .list(.show(workspace))).map(Action.today),
            UpcomingFeature().reduce(into: &state.upcoming, action: .list(.show(workspace))).map(Action.upcoming),
            ProcrastinatedFeature().reduce(into: &state.procrastinated, action: .list(.show(workspace)))
                .map(Action.procrastinated),
            AvenuesFeature().reduce(into: &state.avenues, action: .show(workspace)).map(Action.avenues),
            SearchFeature().reduce(into: &state.search, action: .list(.show(workspace))).map(Action.search),
        ]
        state.avenue?.list.show(workspace, now: now, timeZone: timeZone)
        state.board?.list.show(workspace, now: now, timeZone: timeZone)
        state.settings.show(workspace)
        state.conversations.show(workspace)
        let scheduled = workspace.scheduledReminders(now: now, timeZone: timeZone)
        return .merge(
            effects + [
                .run { [reminders] send in
                    // Notifications that are off, or a device that refuses
                    // them, leave the reminders saved and ringing elsewhere;
                    // the detail screen says so when one is added.
                    try? await reminders.schedule(scheduled)
                    await send(.remindersScheduled)
                }
                .cancellable(id: CancelID.reminders, cancelInFlight: true)
            ])
    }

    /// Quick Add for `kind`, starting where `adds` says, or in the list on
    /// screen so what's added shows where it was added.
    private func showQuickAdd(_ kind: QuickAddFeature.Kind, _ adds: NewTaskDefaults? = nil, in state: inout State) {
        guard let workspace = state.workspace else { return }
        let adds = adds ?? state.currentList.flatMap { state[list: $0]?.query.adds } ?? NewTaskDefaults()
        var quickAdd = QuickAddFeature.State(workspace: workspace, now: now)
        quickAdd.kind = kind
        switch adds.scope {
        case .avenue(let id):
            quickAdd.avenueID = id
        case .project(let id):
            if let project = workspace.projects[id] {
                quickAdd.avenueID = project.avenueID
                quickAdd.projectID = id
            }
        case nil:
            break
        }
        if adds.isDueToday { quickAdd.due = .today }
        state.quickAdd = quickAdd
    }

    private func showNextApproval(_ state: inout State) {
        guard state.approval == nil, !state.queuedApprovals.isEmpty else { return }
        let next = state.queuedApprovals.removeFirst()
        state.approval = ApprovalFeature.State(request: next, workspace: state.workspace ?? Workspace())
    }

    /// `action` for the list in `slot`.
    private func list(_ slot: ListSlot, _ action: TaskListFeature.Action) -> Action {
        switch slot {
        case .today: .today(.list(action))
        case .upcoming: .upcoming(.list(action))
        case .procrastinated: .procrastinated(.list(action))
        case .search: .search(.list(action))
        case .avenue: .avenue(.list(action))
        case .board: .board(.list(action))
        case .openAvenue: .avenues(.avenue(.presented(.list(action))))
        }
    }
}
