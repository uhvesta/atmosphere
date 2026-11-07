import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAvenue
import LifrBoard
import LifrBugReport
import LifrConversations
import LifrCore
import LifrGlance
import LifrGlanceClient
import LifrLockClient
import LifrProcrastinated
import LifrQuickAdd
import LifrRemindersClient
import LifrSearch
import LifrSessionClient
import LifrSettings
import LifrStoreClient
import LifrSyncClient
import LifrSyncReview
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

    package enum Tab: Hashable, Codable, Sendable {
        case today, upcoming, avenues, conversations, search
    }

    /// What the Mac's sidebar can show.
    package enum SidebarItem: Hashable, Codable, Sendable {
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
        /// Where sync is, and what waits for review.
        package var sync = SyncState()
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
        @Presents package var syncReview: SyncReviewFeature.State?
        /// Why Undo couldn't revert the last change.
        @Presents package var undoAlert: AlertState<Never>?
        @Shared(.lastChange) package var lastChange
        /// The last change whose toast went away; Undo stays in the Edit menu.
        package var dismissedToast: UUID?
        /// Whether Lifr asks for Face ID, Touch ID or the passcode.
        @Shared(.locksApp) package var locksApp
        /// Whether the records are hidden until the person confirms it's
        /// them.
        package var isLocked = false
        /// What this device confirms with, which Unlock is named for.
        package var biometry = Biometry.faceID
        /// Whether the system is asking the person to confirm.
        package var isUnlocking = false
        /// Why the last try to unlock failed.
        package var lockMessage: String?
        /// When the app last went to the background.
        package var awayAt: Date?
        /// Why the lock turned itself off.
        @Presents package var lockAlert: AlertState<Never>?
        /// Whether the steps to a bug are being recorded, which the
        /// Recording bar shows.
        package var isRecordingSteps = false
        /// The iPhone's shake menu: report a bug, record steps, or undo.
        package var isBugMenuShown = false
        /// Describing a bug, after a recording or straight from the menu.
        @Presents package var bugReport: BugReportFeature.State?
        /// Where the person was when Lifr last ran.
        @Shared(.place) package var place
        /// The place to go back to, until the records are in.
        package var restoring: Place?

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

        /// This device, as the review names it.
        package var thisDevice: String {
            platform == .phone ? String(localized: "This iPhone") : String(localized: "This Mac")
        }

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

        /// Where the person is now.
        package var here: Place {
            platform == .phone
                ? Place(tab: tab, task: openTaskID) : Place(sidebar: selection, task: openTaskID)
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
        case syncChanged(SyncState)
        /// The app came to the front: lock it if it was away long enough,
        /// and sync what changed meanwhile.
        case foregrounded
        /// The app went to the background.
        case backgrounded
        /// The lock screen's Unlock, or the lock coming on.
        case unlockTapped
        case unlocked(Result<Void, any Error>)
        case lockAlert(PresentationAction<Never>)
        /// The Mac's File › Sync Now.
        case syncNowTapped
        /// The banner's Review.
        case syncReviewTapped
        case syncReview(PresentationAction<SyncReviewFeature.Action>)
        /// This device's notifications match the records again.
        case remindersScheduled
        /// The approval sheet closed: show the next app's, if one is waiting.
        case nextApproval
        /// The iPhone was shaken: the bug menu, or the end of a recording.
        case shaken
        /// Report Bug, from the shake menu or Help: describe it now, with
        /// the steps if they were being recorded.
        case reportBugTapped
        /// Record Steps: log what happens until Stop.
        case recordStepsTapped
        /// The Recording bar's Stop, Help's Stop Recording, or a shake.
        case stopRecordingTapped
        case bugReport(PresentationAction<BugReportFeature.Action>)
        /// Opens the task the person had open when Lifr last ran.
        case reopen(UUID)
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

    private enum CancelID { case reminders, glance }

    /// How often sync runs while the app is open, for what other devices
    /// sent.
    static let syncInterval = Duration.seconds(15 * 60)

    @Dependency(\.store) private var store
    @Dependency(\.session) private var session
    @Dependency(\.sync) private var sync
    @Dependency(\.continuousClock) private var clock
    @Dependency(\.reminders) private var reminders
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone
    @Dependency(\.lock) private var lock
    @Dependency(\.glance) private var glance
    @Dependency(\.bugRecorder) private var bugRecorder

    package init() {}

    package var body: some ReducerOf<Self> {
        CombineReducers { screens }
            // Kept as it changes, once Lifr has gone back to where it was.
            .onChange(of: \.here) { _, here in
                Reduce { state, _ in
                    if state.restoring == nil { state.$place.withLock { $0 = here } }
                    return .none
                }
            }
            // The widgets show only counts while the lock is on.
            .onChange(of: \.locksApp) { _, _ in
                Reduce { state, _ in writeGlance(state) }
            }
            .transformDependency(\.self) { dependencies in
                dependencies.store = dependencies.store.telling(dependencies.sync).recordingChanges(
                    into: Shared(.lastChange), uuid: dependencies.uuid)
            }
            .recordingBugs()
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
        Reduce(locking)
        Reduce(reportingBugs)
        Reduce { (state: inout State, action: Action) -> Effect<Action> in
            // Menus and shortcuts reach the app while it's locked; what they'd
            // show or change waits until it's unlocked.
            if state.isLocked, action.isCommand { return .none }
            switch action {
            case .task:
                var effects: [Effect<Action>] = [
                    .run { [store] send in await send(.loaded(Result { try await store.load() })) },
                    .run { [store] send in
                        for await update in await store.updates() { await send(.updated(update)) }
                    },
                    .run { [sync] send in
                        for await state in await sync.states() { await send(.syncChanged(state)) }
                    },
                    .run { [sync, clock] _ in
                        _ = await sync.sync()
                        for await _ in clock.timer(interval: Self.syncInterval) { _ = await sync.sync() }
                    },
                ]
                state.biometry = lock.biometry()
                if state.place != Place() {
                    state.restoring = state.place
                    if state.platform == .phone, let tab = state.place.tab { state.tab = tab }
                }
                if state.locksApp {
                    state.isLocked = true
                    effects.append(.send(.unlockTapped))
                }
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
                let shown = show(workspace, in: &state)
                guard let place = state.restoring else { return shown }
                state.restoring = nil
                let selects: Effect<Action> =
                    if state.platform == .mac, let item = place.sidebar, item != state.selection {
                        .send(.selected(item))
                    } else {
                        .none
                    }
                let reopens: Effect<Action> = place.task.map { .send(.reopen($0)) } ?? .none
                return .merge(shown, .concatenate(selects, reopens))

            case .reopen(let id):
                guard let slot = state.currentList, state.workspace?.tasks[id] != nil else { return .none }
                return .send(list(slot, .rowTapped(id)))

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
                let left = state.selection
                state.selection = item
                state.avenue = nil
                state.board = nil
                // What the detail column showed belongs to the list left, so
                // it goes with it.
                if left != item {
                    state.today.list.detail = nil
                    state.upcoming.list.detail = nil
                    state.procrastinated.list.detail = nil
                    state.search.list.detail = nil
                }
                let closesChat: Effect<Action> =
                    left == .conversations && item != .conversations ? .send(.conversations(.closed)) : .none
                guard case .scope(let scope) = item, let workspace = state.workspace else { return closesChat }
                let avenue = AvenueFeature.State(scope: scope, workspace: workspace, now: now, timeZone: timeZone)
                state.avenue = avenue
                state.board = BoardFeature.State(
                    scope: scope, title: avenue.title, workspace: workspace, now: now, timeZone: timeZone)
                return closesChat

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
                            It changed again since, here or on another device, and undoing would lose that \
                            change. Open it to change it back by hand.
                            """)
                    }
                    return .none
                }
                return .run { [store] send in
                    if let entry = last.entry {
                        await send(.undone(Result { try await store.revert(entries: [entry]) }))
                    } else {
                        await send(.undone(Result { try await store.undo(last.change) }))
                    }
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

            case .syncChanged(let sync):
                state.sync = sync
                state.settings.sync = sync
                guard state.syncReview != nil else { return .none }
                return .send(.syncReview(.presented(.show(sync.review, state.workspace ?? Workspace()))))

            case .foregrounded:
                let away = state.awayAt
                state.awayAt = nil
                var effects: [Effect<Action>] = []
                if state.locksApp, !state.isLocked, AppLock.locks(away: away, now: now) {
                    state.isLocked = true
                    effects.append(.send(.unlockTapped))
                }
                if state.sync.status != .syncing {
                    effects.append(.run { [sync] _ in _ = await sync.sync() })
                }
                return .merge(effects)

            case .syncNowTapped:
                guard state.sync.status != .syncing else { return .none }
                return .run { [sync] _ in _ = await sync.sync() }

            case .backgrounded, .unlockTapped, .unlocked:
                return .none

            case .syncReviewTapped:
                guard !state.sync.review.isEmpty else { return .none }
                state.syncReview = SyncReviewFeature.State(
                    items: state.sync.review, workspace: state.workspace ?? Workspace(),
                    thisDevice: state.thisDevice)
                return .none

            case .syncReview(.presented(.delegate(.resolved))):
                // The choices were saved through sync, not a screen.
                return .run { [store] send in await send(.loaded(Result { try await store.load() })) }

            case .syncReview(.presented(.delegate(.finished))):
                state.syncReview = nil
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

            case .board(.addTapped):
                guard let scope = state.board?.scope else { return .none }
                return .send(.newTaskInTapped(scope))

            case .shaken, .reportBugTapped, .recordStepsTapped, .stopRecordingTapped, .bugReport:
                return .none

            case .binding, .remindersScheduled, .today, .upcoming, .procrastinated, .avenues, .search, .avenue, .board,
                .conversations, .settings, .quickAdd, .approval, .undoAlert, .syncReview, .lockAlert:
                return .none
            }
        }
        .ifLet(\.avenue, action: \.avenue) { AvenueFeature() }
        .ifLet(\.board, action: \.board) { BoardFeature() }
        .ifLet(\.$quickAdd, action: \.quickAdd) { QuickAddFeature() }
        .ifLet(\.$approval, action: \.approval) { ApprovalFeature() }
        .ifLet(\.$syncReview, action: \.syncReview) { SyncReviewFeature() }
        .ifLet(\.$undoAlert, action: \.undoAlert)
        .ifLet(\.$lockAlert, action: \.lockAlert)
        .ifLet(\.$bugReport, action: \.bugReport) { BugReportFeature() }
    }

    /// The shake menu, recording the steps to a bug, and describing it.
    private func reportingBugs(_ state: inout State, _ action: Action) -> Effect<Action> {
        switch action {
        case .shaken:
            if state.isRecordingSteps { return .send(.stopRecordingTapped) }
            guard !state.isLocked, state.bugReport == nil else { return .none }
            state.isBugMenuShown = true
            return .none

        case .reportBugTapped:
            if state.isRecordingSteps { return .send(.stopRecordingTapped) }
            guard !state.isLocked, state.bugReport == nil else { return .none }
            state.bugReport = .new(recording: nil)
            return .none

        case .recordStepsTapped:
            guard !state.isLocked, !state.isRecordingSteps, state.bugReport == nil else { return .none }
            state.isRecordingSteps = true
            bugRecorder.start(from: state)
            return .none

        case .stopRecordingTapped:
            guard state.isRecordingSteps else { return .none }
            state.isRecordingSteps = false
            state.bugReport = .new(recording: bugRecorder.stop())
            return .none

        default:
            return .none
        }
    }

    /// Away, unlocking and what happens when it fails.
    private func locking(_ state: inout State, _ action: Action) -> Effect<Action> {
        switch action {
        case .backgrounded:
            // The iPhone passes through inactive on its way to the
            // background; away starts at the first.
            if state.awayAt == nil { state.awayAt = now }
            return .none

        case .unlockTapped:
            guard state.isLocked, !state.isUnlocking else { return .none }
            state.isUnlocking = true
            state.lockMessage = nil
            let reason = String(localized: "Unlock Lifr to see your tasks")
            return .run { [lock] send in
                await send(.unlocked(Result { try await lock.authenticate(reason) }))
            }

        case .unlocked(.success):
            state.isLocked = false
            state.isUnlocking = false
            return .none

        case .unlocked(.failure(let error)):
            state.isUnlocking = false
            switch error as? LockError {
            case .cancelled:
                return .none
            case .noPasscode:
                // Without a passcode nothing protects the device, so the
                // lock can't either; better to say so than to lock the
                // person out.
                state.isLocked = false
                state.$locksApp.withLock { $0 = false }
                state.lockAlert = AlertState {
                    TextState("Lifr's lock is off")
                } actions: {
                    ButtonState(role: .cancel) { TextState("OK") }
                } message: {
                    TextState(LockError.noPasscode.description)
                }
                return .none
            case .failed, nil:
                state.lockMessage = String(describing: error)
                return .none
            }
        default:
            return .none
        }
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
        state.avenue?.show(workspace, now: now, timeZone: timeZone)
        state.board?.list.show(workspace, now: now, timeZone: timeZone)
        state.settings.show(workspace)
        state.conversations.show(workspace)
        let scheduled = workspace.scheduledReminders(now: now, timeZone: timeZone)
        return .merge(
            effects + [
                writeGlance(state),
                .run { [reminders] send in
                    // Notifications that are off, or a device that refuses
                    // them, leave the reminders saved and ringing elsewhere;
                    // the detail screen says so when one is added.
                    try? await reminders.schedule(scheduled)
                    await send(.remindersScheduled)
                }
                .cancellable(id: CancelID.reminders, cancelInFlight: true),
            ])
    }

    /// Gives the widgets Today as `state` has it, and tomorrow's for after
    /// midnight: the titles, or only how many while the lock is on.
    private func writeGlance(_ state: State) -> Effect<Action> {
        guard let workspace = state.workspace else { return .none }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        let start = calendar.startOfDay(for: now)
        guard let tomorrow = calendar.date(byAdding: .day, value: 1, to: start),
            let after = calendar.date(byAdding: .day, value: 2, to: start)
        else { return .none }
        func rows(at date: Date) -> [TodayGlance.Row] {
            TaskQuery.today.sections(workspace, date, timeZone).flatMap(\.rows).map { row in
                let isOverdue = if case .overdue = row.due { true } else { false }
                return TodayGlance.Row(id: row.id, title: row.title, code: row.code, isOverdue: isOverdue)
            }
        }
        let next = TodayGlance(
            asOf: now, from: tomorrow, until: after, rows: rows(at: tomorrow), locked: state.locksApp)
        let today = TodayGlance(
            asOf: now, from: start, until: tomorrow, rows: rows(at: now), locked: state.locksApp, later: [next])
        return .run { [glance] _ in await glance.write(today) }
            .cancellable(id: CancelID.glance)
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

extension RootFeature.Action {
    /// What a menu, a keyboard shortcut or the menu bar sends.
    fileprivate var isCommand: Bool {
        switch self {
        case .newTaskTapped, .newTaskInTapped, .newAvenueTapped, .newProjectTapped, .newSubtaskTapped,
            .statusShortcut, .undoTapped, .syncReviewTapped, .apiSwitched, .reportBugTapped, .recordStepsTapped:
            true
        default:
            false
        }
    }
}
