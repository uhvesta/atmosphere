import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAvenue
import LifrBoard
import LifrBugReport
import LifrCalendar
import LifrConversations
import LifrCore
import LifrFeedbackClient
import LifrGlance
import LifrGlanceClient
import LifrLists
import LifrLockClient
import LifrOnboarding
import LifrPasteboardClient
import LifrProcrastinated
import LifrQuickAdd
import LifrRemindersClient
import LifrSearch
import LifrSessionClient
import LifrSettings
import LifrSpotlightClient
import LifrStoreClient
import LifrSyncClient
import LifrSyncReview
import LifrTaskDetail
import LifrTaskList
import LifrToday
import LifrTrash
import LifrUI
import LifrUpcoming
import OSLog

/// The whole app: the iPhone's tabs or the Mac's sidebar, with quick add,
/// the sync banner and the approval sheet over them. It loads the records
/// once and passes every change on to each screen, so they never disagree,
/// and reschedules this device's reminders after each change.
@Reducer
package struct RootFeature {
    /// How long the last change's Undo stays on screen.
    static let toastDuration = Duration.seconds(5)
    /// How long "Copied" stays: there's nothing to undo.
    static let copyNoticeDuration = Duration.seconds(2)

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
        case today, upcoming, calendar, procrastinated, conversations, search, trash
        case anytime, later, logbook
        case scope(AvenueScope)
        case savedFilter(UUID)
    }

    /// A list across every Avenue that isn't a screen of its own: tasks to
    /// do anytime, ones waiting to start, and closed ones.
    package enum Shelf: String, CaseIterable, Hashable, Codable, Sendable {
        case anytime, later, logbook

        package var title: String {
            switch self {
            case .anytime: String(localized: "Anytime")
            case .later: String(localized: "Later")
            case .logbook: String(localized: "Logbook")
            }
        }

        package var systemImage: String {
            switch self {
            case .anytime: "square.stack"
            case .later: "clock"
            case .logbook: "book.closed"
            }
        }

        package var sidebarItem: SidebarItem {
            switch self {
            case .anytime: .anytime
            case .later: .later
            case .logbook: .logbook
            }
        }

        package var slot: ListSlot {
            switch self {
            case .anytime: .anytime
            case .later: .later
            case .logbook: .logbook
            }
        }
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
        /// Whether this is one of the Mac's other windows: it goes where it
        /// likes and changes the records, while the first store syncs,
        /// schedules reminders and serves connected apps for the process.
        package let isWindow: Bool
        /// `nil` until the records load.
        package var workspace: Workspace?
        /// The latest records shown, so a save's reply or an update that
        /// arrives after later records is ignored.
        package var saves = SaveOrder()
        package var tab = Tab.today
        package var selection: SidebarItem? = .today
        /// The Mac's places before this one, for Back, latest last.
        package var back: [SidebarItem] = []
        /// The places Back left, for Forward, latest last.
        package var forward: [SidebarItem] = []
        package var content = Content.list
        package var today: TodayFeature.State
        package var upcoming = UpcomingFeature.State()
        package var calendar = CalendarFeature.State()
        package var trash = TrashFeature.State()
        /// Whether the iPhone's Avenues tab shows the trash.
        package var isTrashShown = false
        /// The list across Avenues the iPhone's Avenues tab shows.
        package var shownShelf: Shelf?
        package var anytime = TaskListFeature.State(query: .anytime)
        package var later = TaskListFeature.State(query: .later)
        package var logbook = TaskListFeature.State(query: .logbook)
        /// Whether the iPhone's Upcoming tab shows the calendar rather than
        /// the list.
        @Shared(.upcomingShowsCalendar) package var upcomingShowsCalendar
        package var procrastinated = ProcrastinatedFeature.State()
        package var avenues = AvenuesFeature.State()
        package var search = SearchFeature.State()
        /// The saved filter open: the Mac's selection, or pushed on the
        /// iPhone's Avenues tab.
        @Presents package var savedFilter: SearchFeature.State?
        package var conversations = ConversationsFeature.State()
        /// The Mac's selected Avenue or Project, as a list.
        package var avenue: AvenueFeature.State?
        /// The same Avenue or Project, as a board.
        package var board: BoardFeature.State?
        package var settings: SettingsFeature.State
        package var isSettingsShown = false
        /// What the last sync fixed, until dismissed.
        package var syncBanner: SyncBanner?
        /// What VoiceOver says next, such as that a sync asked for worked or
        /// that tasks were closed elsewhere.
        package var announcement: Announcement?
        /// Syncs in a row that failed for a reason the person can act on.
        package var syncFailures = 0
        /// The sync failure whose banner the person closed.
        package var dismissedSyncFailure: String?
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
        /// Quick Add opened over another app by the Mac's hotkey.
        @Presents package var capture: QuickAddFeature.State?
        @Presents package var approval: ApprovalFeature.State?
        @Presents package var syncReview: SyncReviewFeature.State?
        /// A short message: why Undo or Redo couldn't, or that a link
        /// named a task Lifr doesn't have.
        @Presents package var notice: AlertState<Never>?
        /// Whether reminders that couldn't be scheduled were told about,
        /// once per launch.
        package var explainedReminders = false
        @Shared(.lastChange) package var lastChange
        /// What a list last copied, while it's said.
        @Shared(.lastCopy) package var lastCopy
        /// The change Undo just took back, which Redo puts back, until
        /// something else changes.
        package var redo: LastChange?
        /// The change Undo is taking back, for Redo's words.
        package var undoing: LastChange?
        @Presents package var goToTask: GoToTaskFeature.State?
        /// Whether the list of keyboard shortcuts is up.
        package var isShortcutsShown = false
        /// The last change whose toast went away; Undo stays in the Edit menu.
        package var dismissedToast: UUID?
        /// Whether Lifr asks for Face ID, Touch ID or the passcode.
        @Shared(.locksApp) package var locksApp
        /// What the number on the app icon counts.
        @Shared(.badgeCount) package var badgeCount
        /// The Avenues the Focus that's on limits the lists, badge and
        /// widgets to.
        @Shared(.focusAvenues) package var focusAvenues
        /// When this device sums up the day, if it does.
        @Shared(.dailySummary) package var dailySummary
        /// This device's copy of the synced default reminder, for Quick Add,
        /// widgets and Shortcuts.
        @Shared(.defaultReminder) package var defaultReminder
        /// A task to show the statuses for once it opens, as Change Status…
        /// on its reminder asks.
        package var statusPickPending: UUID?
        /// A link opened while Lifr was locked, followed once it's unlocked.
        package var pendingLink: DeepLink?
        /// Whether sync runs often while open, rather than every so often.
        @Shared(.syncsContinuously) package var syncsContinuously
        /// Whether the records are hidden until the person confirms it's
        /// them.
        package var isLocked = false
        /// What this device confirms with, which Unlock is named for.
        package var biometry = Biometry.faceID
        /// Whether the system is asking the person to confirm.
        package var isUnlocking = false
        /// Whether the app's lifetime work, such as the sync timer, has
        /// started.
        package var isRunning = false
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
        /// The first launch on this device, until it's finished or skipped.
        @Presents package var onboarding: OnboardingFeature.State?
        @Shared(.hasOnboarded) package var hasOnboarded
        @Shared(.seenRelease) package var seenRelease
        /// Whether this release's notes are up.
        package var isWhatsNewShown = false
        /// Whether the first records decided on onboarding or What's New.
        package var introduced = false

        /// Whether `item` is still there to go to: a saved filter that
        /// wasn't deleted, an Avenue or Project that wasn't archived.
        func canShow(_ item: SidebarItem) -> Bool {
            guard let workspace else { return false }
            switch item {
            case .savedFilter(let id): return workspace.savedFilters[id] != nil
            case .scope(.avenue(let id)): return workspace.avenues[id].map { $0.archivedAt == nil } ?? false
            case .scope(.project(let id)): return workspace.projects[id].map { $0.status == .active } ?? false
            default: return true
            }
        }

        /// The last change, while its toast is up.
        package var toast: LastChange? {
            lastChange.flatMap { $0.showsToast && $0.id != dismissedToast ? $0 : nil }
        }

        package init(platform: Platform) {
            self.platform = platform
            isWindow = false
            today = TodayFeature.State()
            settings = SettingsFeature.State(showsAgents: platform == .mac)
        }

        /// Another Mac window, opening at `place`; where it goes after is
        /// its own, not kept for the next launch.
        package init(window place: Place) {
            platform = .mac
            isWindow = true
            today = TodayFeature.State()
            settings = SettingsFeature.State(showsAgents: true)
            _place = Shared(value: place)
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
                case .upcoming: upcomingShowsCalendar ? .calendar : .upcoming
                case .avenues:
                    shownShelf?.slot ?? (savedFilter != nil ? .savedFilter : avenues.avenue == nil ? nil : .openAvenue)
                case .search: .search
                case .conversations: nil
                }
            case .mac:
                switch selection {
                case .today: .today
                case .upcoming: .upcoming
                case .calendar: .calendar
                case .procrastinated: .procrastinated
                case .search: .search
                case .anytime: .anytime
                case .later: .later
                case .logbook: .logbook
                case .conversations, .trash: nil
                case .scope: content == .board ? .board : .avenue
                case .savedFilter: .savedFilter
                case nil: nil
                }
            }
        }

        /// The conversation on screen, for the Mac's plan commands. `nil`
        /// while locked or with none open.
        package var openChatID: UUID? {
            guard !isLocked, selection == .conversations || tab == .conversations else { return nil }
            return conversations.openID
        }

        /// The Mac's list on screen, with its title, to print. `nil` while
        /// locked, or with no list or an empty one shown.
        package var printable: (title: String, list: TaskListFeature.State)? {
            guard platform == .mac, !isLocked, let slot = currentList, let list = self[list: slot], !list.isEmpty
            else { return nil }
            let title: String? =
                switch selection {
                case .today: String(localized: "Today")
                case .upcoming: String(localized: "Upcoming")
                case .calendar: String(localized: "Calendar")
                case .procrastinated: String(localized: "Procrastinated")
                case .search: String(localized: "Search")
                case .anytime: Shelf.anytime.title
                case .later: Shelf.later.title
                case .logbook: Shelf.logbook.title
                case .scope: avenue?.title
                case .savedFilter(let id): workspace?.savedFilters[id]?.name
                case .conversations, .trash, nil: nil
                }
            return (title ?? "Lifr", list)
        }

        package subscript(list slot: ListSlot) -> TaskListFeature.State? {
            switch slot {
            case .today: today.list
            case .upcoming: upcoming.list
            case .calendar: calendar.list
            case .procrastinated: procrastinated.list
            case .anytime: anytime
            case .later: later
            case .logbook: logbook
            case .search: search.list
            case .savedFilter: savedFilter?.list
            case .avenue: avenue?.list
            case .board: board?.list
            case .openAvenue: avenues.avenue?.list
            }
        }

        /// The task open in the current list's detail.
        package var openTaskID: UUID? {
            currentList.flatMap { self[list: $0]?.detail?.id }
        }

        /// The list on screen, for the View menu's Sort By.
        package var currentRows: TaskListFeature.State? {
            currentList.flatMap { self[list: $0] }
        }

        /// The open task, for the Task menu.
        package var openDetail: TaskDetailFeature.State? {
            currentList.flatMap { self[list: $0]?.detail }
        }

        /// Whether an Avenue or Project is on screen, which can show as a
        /// list or a board.
        package var showsScope: Bool {
            if case .scope = selection { platform == .mac } else { false }
        }

        /// Whether Move Up and Move Down apply: a row the keyboard is on, in
        /// a list in manual order.
        package var canMoveCursorRow: Bool {
            guard let slot = currentList, let rows = self[list: slot] else { return false }
            return rows.canReorder && rows.cursor != nil
        }

        /// The task a status shortcut changes: the open one, or else the
        /// row the keyboard is on.
        package var statusTaskID: UUID? {
            openTaskID ?? currentList.flatMap { self[list: $0]?.cursor }
        }

        /// The open task, for Handoff to the person's other devices; none
        /// while the lock is on.
        package var handoffTaskID: UUID? {
            isLocked ? nil : openTaskID
        }

        /// Where the person is now.
        package var here: Place {
            platform == .phone
                ? Place(tab: tab, task: openTaskID) : Place(sidebar: selection, task: openTaskID)
        }
    }

    /// What the Edit and Task menus do to the list on screen.
    package enum ListShortcut: Hashable, Sendable {
        case duplicate, trash, selectAll, moveUp, moveDown
        case sort(TaskSort)
        /// Pick a due date for the open task, or the row the keyboard is on.
        case pickDue
    }

    /// What the Task menu sets on the open task.
    package enum DetailShortcut: Hashable, Sendable {
        case repeats(RepeatPreset?)
        case customRepeat
        case reminder(ReminderChoice)
        case customReminder
        case avenue(UUID)
        case label(UUID)
    }

    /// Where a task list lives in `State`.
    package enum ListSlot: Hashable, Sendable {
        case today, upcoming, calendar, procrastinated, search, avenue, board
        case anytime, later, logbook
        /// The saved filter open.
        case savedFilter
        /// The Avenue opened from the iPhone's Avenues tab.
        case openAvenue
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        /// The app launched: load the records and listen for changes.
        case task
        case loaded(Result<Snapshot, any Error>)
        /// Another device or process changed the records.
        case updated(Update)
        case appsLoaded([ConnectedApp])
        case apiEnabledLoaded(Bool)
        /// The menu bar's Local API switch.
        case apiSwitched(Bool)
        case apiSwitchFailed(String)
        case sessionEvent(SessionEvent)
        case selected(SidebarItem?)
        /// ⌘[: the Mac's place before this one.
        case backTapped
        /// ⌘]: the place Back left.
        case forwardTapped
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
        /// The title and description typed in the task left by choosing
        /// another sidebar item were saved, or couldn't be.
        case leftTaskSaved(Result<Saved, any Error>)
        /// ⇧⌘Z, for the change Undo took back with this ID.
        case redoTapped(UUID)
        case redone(String, Result<Saved, any Error>)
        /// A menu's Go to: Today, Upcoming, Calendar and the like.
        case goTo(SidebarItem)
        /// Edit and Task menu items for the list on screen.
        case listShortcut(ListShortcut)
        /// A priority shortcut, for the open task.
        case priorityShortcut(Priority)
        /// A due date shortcut, for the open task: today, tomorrow or none.
        case dueShortcut(DueShortcut)
        case detailShortcut(DetailShortcut)
        /// Go to Task…
        case goToTaskTapped
        /// A chat command from the menu, for the conversation on screen.
        case chatCommandChosen(ChatCommand)
        case goToTask(PresentationAction<GoToTaskFeature.Action>)
        /// Help › Keyboard Shortcuts.
        case shortcutsTapped
        /// The toast for the change with this ID came up; it goes after
        /// five seconds.
        case toastShown(UUID)
        case toastExpired(UUID)
        case copyNoticeShown(UUID)
        case copyNoticeExpired(UUID)
        case notice(PresentationAction<Never>)
        case syncBannerDismissed
        case syncChanged(SyncState)
        /// The app came to the front: lock it if it was away long enough,
        /// and sync what changed meanwhile.
        case foregrounded
        /// The tasks marked Done from a widget were saved: the last save,
        /// if any did, and how many couldn't be.
        case widgetMarksSaved(Saved?, unsaved: Int)
        /// The tasks added from the share sheet were saved, but `unsaved`
        /// of them couldn't be.
        case sharedTasksSaved(Saved?, unsaved: Int)
        /// Midnight passed, or the clock or time zone changed: what's due
        /// today, overdue or procrastinated is worked out again.
        case dayChanged
        /// The app went to the background.
        case backgrounded
        /// The lock screen's Unlock, or the lock coming on.
        case unlockTapped
        case unlocked(Result<Void, any Error>)
        /// Another window locked or unlocked: every window follows.
        case lockMirrored(Bool)
        /// The Mac app became the active app, or another app did. The lock
        /// follows Lifr as a whole, so it comes on even when the first
        /// window is closed and others are open.
        case appActivated(Bool)
        case lockAlert(PresentationAction<Never>)
        /// The Mac's File › Sync Now.
        case syncNowTapped
        case syncNowFailed(String)
        case syncNowSucceeded
        /// The system woke Lifr, away, to catch up.
        case refreshedInBackground
        /// The banner's Review.
        case syncReviewTapped
        case syncReview(PresentationAction<SyncReviewFeature.Action>)
        /// This device's notifications match the records again.
        /// Why some reminders couldn't be scheduled, if they couldn't.
        case remindersScheduled(failure: String?)
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
        case onboarding(PresentationAction<OnboardingFeature.Action>)
        case whatsNewDismissed
        /// Opens the task the person had open when Lifr last ran.
        case reopen(UUID)
        /// The person opened a reminder's notification: show its task, from
        /// Today if no list is showing.
        case reminderOpened(UUID)
        /// A `lifr://` link, from a widget, Shortcuts or another app.
        case urlOpened(URL)
        case linkOpened(DeepLink)
        /// Change Status… on a reminder: its task, with the statuses to pick.
        case reminderStatusRequested(UUID)
        /// The badge setting changed.
        case badgeCountChanged
        case dailySummaryChanged
        /// A Focus came on or went off.
        case focusChanged
        case today(TodayFeature.Action)
        case upcoming(UpcomingFeature.Action)
        case calendar(CalendarFeature.Action)
        case trash(TrashFeature.Action)
        /// The iPhone's Upcoming tab switched between its list and the
        /// calendar.
        case upcomingCalendarShown(Bool)
        case procrastinated(ProcrastinatedFeature.Action)
        case anytime(TaskListFeature.Action)
        case later(TaskListFeature.Action)
        case logbook(TaskListFeature.Action)
        case avenues(AvenuesFeature.Action)
        case search(SearchFeature.Action)
        /// A saved filter on the iPhone's Avenues tab.
        case savedFilterTapped(UUID)
        /// A saved filter's Move Up or Move Down.
        case savedFilterMoved(UUID, up: Bool)
        case savedFilterMoveFailed(any Error)
        /// Tasks dragged onto a sidebar item: an Avenue or Project moves
        /// them there, Today makes them due today and Upcoming tomorrow.
        case tasksDropped([UUID], on: SidebarItem)
        /// Text dragged in from another app onto a sidebar item: a task
        /// for each line, in that Avenue or Project, or in the Inbox due
        /// today or tomorrow.
        case textDropped(String, on: SidebarItem)
        /// Text sent from another app with the Mac's Services menu: Quick
        /// Add with one line as its title, or a task for each of several.
        case textSent(String)
        /// ⌘V on the Mac's list: reads the clipboard for `textPasted`.
        case pasteCommanded
        /// Text pasted or dropped on the list shown: a task for each line where
        /// a drop on its sidebar item puts them, or Quick Add elsewhere.
        case textPasted(String?)
        case savedFilter(PresentationAction<SearchFeature.Action>)
        case conversations(ConversationsFeature.Action)
        case avenue(AvenueFeature.Action)
        case board(BoardFeature.Action)
        case settings(SettingsFeature.Action)
        case quickAdd(PresentationAction<QuickAddFeature.Action>)
        /// The Mac's hotkey was pressed.
        case captureOpened
        case capture(PresentationAction<QuickAddFeature.Action>)
        case approval(PresentationAction<ApprovalFeature.Action>)
    }

    private enum CancelID { case reminders, glance, spotlight, syncTimer, copyNotice }

    /// How often sync runs while the app is open, for what other devices
    /// sent.
    static let syncInterval = Duration.seconds(15 * 60)
    /// How often it runs while Lifr syncs continuously.
    static let continuousSyncInterval = Duration.seconds(2 * 60)

    @Dependency(\.store) private var store
    @Dependency(\.session) private var session
    @Dependency(\.sync) private var sync
    @Dependency(\.continuousClock) private var clock
    @Dependency(\.reminders) private var reminders
    @Dependency(\.feedback) private var feedback
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone
    @Dependency(\.lock) private var lock
    @Dependency(\.glance) private var glance
    @Dependency(\.spotlight) private var spotlight
    @Dependency(\.bugRecorder) private var bugRecorder
    @Dependency(\.uuid) private var uuid
    @Dependency(\.pasteboard) private var pasteboard

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
            // Change Status… on a reminder picks once its task is open,
            // which waits for the records at launch.
            .onChange(of: \.openTaskID) { _, id in
                Reduce { state, _ in
                    guard let id, id == state.statusPickPending, let slot = state.currentList else { return .none }
                    state.statusPickPending = nil
                    return .send(list(slot, .detail(.presented(.statusPickerRequested))))
                }
            }
            // The widgets show only counts while the lock is on, and search
            // lists no task.
            .onChange(of: \.locksApp) { _, _ in
                Reduce { state, _ in .merge(writeGlance(state), indexSpotlight(state), scheduleReminders(state)) }
            }
            // The timer restarts at the new pace.
            .onChange(of: \.syncsContinuously) { _, _ in
                Reduce { state, _ in state.isRunning ? syncTimer(state) : .none }
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
        Scope(state: \.calendar, action: \.calendar) { CalendarFeature() }
        Scope(state: \.trash, action: \.trash) { TrashFeature() }
        Scope(state: \.procrastinated, action: \.procrastinated) { ProcrastinatedFeature() }
        Scope(state: \.anytime, action: \.anytime) { TaskListFeature() }
        Scope(state: \.later, action: \.later) { TaskListFeature() }
        Scope(state: \.logbook, action: \.logbook) { TaskListFeature() }
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
                // Started once for the whole process: on the Mac, Lifr keeps
                // syncing with no window open, and a second window must not
                // start a second timer.
                guard !state.isRunning else { return .none }
                if state.isWindow { return startWindow(&state) }
                var effects: [Effect<Action>] = [
                    tidy(ensuringInbox: true),
                    .run { [store] send in
                        for await update in await store.updates() { await send(.updated(update)) }
                    },
                    .run { [sync] send in
                        for await state in await sync.states() { await send(.syncChanged(state)) }
                    },
                    .run { [badgeCount = state.$badgeCount] send in
                        for await _ in badgeCount.publisher.dropFirst().values { await send(.badgeCountChanged) }
                    },
                    .run { [dailySummary = state.$dailySummary] send in
                        for await _ in dailySummary.publisher.dropFirst().values { await send(.dailySummaryChanged) }
                    },
                    watchFocus(state),
                    .run { [sync] _ in _ = await sync.sync() },
                    syncTimer(state),
                    saveWidgetMarks(),
                    saveSharedTasks(),
                ]
                state.isRunning = true
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

            case .loaded(.success(let snapshot)):
                introduce(snapshot.workspace, in: &state)
                var shown = show(snapshot, in: &state)
                if !state.isLocked, let link = state.pendingLink {
                    // A link that came before the records: follow it now.
                    state.pendingLink = nil
                    shown = .concatenate(shown, .send(.linkOpened(link)))
                }
                guard let place = state.restoring else { return shown }
                state.restoring = nil
                // Where Lifr opens isn't a step to go back from.
                let selects: Effect<Action> =
                    if state.platform == .mac, let item = place.sidebar, item != state.selection {
                        select(item, in: &state)
                    } else {
                        .none
                    }
                let reopens: Effect<Action> = place.task.map { .send(.reopen($0)) } ?? .none
                return .merge(shown, .concatenate(selects, reopens))

            case .reopen(let id):
                guard let slot = state.currentList, state.workspace?.tasks[id] != nil else { return .none }
                return .send(list(slot, .taskOpened(id)))

            case .reminderStatusRequested(let id):
                state.statusPickPending = id
                return .send(.reminderOpened(id))

            case .urlOpened(let url):
                guard let link = DeepLink(url) else {
                    state.notice = AlertState {
                        TextState("Lifr can't open this link")
                    } actions: {
                        ButtonState(role: .cancel) { TextState("OK") }
                    } message: {
                        TextState(
                            """
                            It doesn't name a task, a day or a list in Lifr. If another app made it, that app \
                            may need updating.
                            """)
                    }
                    return .none
                }
                return .send(.linkOpened(link))

            case .linkOpened(let link):
                guard !state.isLocked else {
                    state.pendingLink = link
                    return .none
                }
                return open(link, in: &state)

            case .badgeCountChanged, .dailySummaryChanged:
                return scheduleReminders(state)

            case .focusChanged:
                return reload()

            case .reminderOpened(let id):
                guard state.workspace != nil else {
                    // Still opening: go there once the records load.
                    let place =
                        state.platform == .phone ? Place(tab: .today, task: id) : Place(sidebar: .today, task: id)
                    state.$place.withLock { $0 = place }
                    if state.restoring != nil { state.restoring = place }
                    return .none
                }
                guard state.currentList == nil else { return .send(.reopen(id)) }
                switch state.platform {
                case .phone:
                    state.tab = .today
                    return .send(.reopen(id))
                case .mac:
                    return .concatenate(.send(.selected(.today)), .send(.reopen(id)))
                }

            case .loaded(.failure):
                // Each screen loads on its own too, and says what failed.
                return .none

            case .updated(let update):
                // A window hears its own saves too, already shown.
                guard update.number > state.saves.latest || !update.fixes.isEmpty else { return .none }
                if !update.fixes.isEmpty {
                    let banner = SyncBanner(update.fixes, in: update.workspace)
                    state.syncBanner = state.syncBanner.map { $0.adding(banner) } ?? banner
                }
                if let before = state.workspace, let closed = Announcement.closed(from: before, to: update.workspace) {
                    state.announce(closed)
                }
                // Another device's records arrived during onboarding:
                // there's nothing left to set up.
                let found: Effect<Action> =
                    state.onboarding != nil && update.workspace.hasRecordsOfItsOwn
                    ? .send(.onboarding(.presented(.foundExisting))) : .none
                return .merge(show(update.snapshot, in: &state), found)

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

            case .sessionEvent(.recordsChanged(let workspace, let number)):
                return show(Snapshot(workspace, number: number), in: &state)

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
                if item != state.selection, let left = state.selection {
                    state.back.append(left)
                    state.forward = []
                }
                return select(item, in: &state)

            case .backTapped:
                let backShown = state
                state.back.removeAll { !backShown.canShow($0) }
                guard let item = state.back.popLast() else { return .none }
                if let left = state.selection { state.forward.append(left) }
                return select(item, in: &state)

            case .forwardTapped:
                let forwardShown = state
                state.forward.removeAll { !forwardShown.canShow($0) }
                guard let item = state.forward.popLast() else { return .none }
                if let left = state.selection { state.back.append(left) }
                return select(item, in: &state)

            case .savedFilterTapped(let id):
                guard let workspace = state.workspace, let saved = workspace.savedFilters[id] else { return .none }
                state.savedFilter = SearchFeature.State(
                    saved: saved, workspace: workspace, now: now, timeZone: timeZone)
                return .none

            case .tasksDropped(let ids, let item):
                return dropped(ids, on: item, in: state)

            case .textDropped(let text, let item):
                return dropped(text, on: item, in: state)

            case .textSent(let text):
                let lines = QuickAddFeature.lines(text)
                guard lines.count > 1, !state.isLocked, state.workspace != nil else {
                    return .send(.linkOpened(.add(title: lines.first)))
                }
                showQuickAdd(.task, in: &state)
                return .send(.quickAdd(.presented(.linesPasted(text))))

            case .pasteCommanded:
                guard !state.isLocked else { return .none }
                return .run { [pasteboard] send in await send(.textPasted(await pasteboard.read())) }

            case .textPasted(let text):
                guard let text, !QuickAddFeature.lines(text).isEmpty, !state.isLocked else { return .none }
                switch state.selection {
                case .today, .upcoming, .scope(.avenue), .scope(.project):
                    return dropped(text, on: state.selection ?? .today, in: state)
                default:
                    return .send(.textSent(text))
                }

            case .savedFilterMoved(let id, let up):
                let ids = state.workspace?.sortedSavedFilters.map(\.id) ?? []
                guard let index = ids.firstIndex(of: id) else { return .none }
                // Between the one it passes and the one beyond that.
                let (after, before): (UUID?, UUID?) =
                    up
                    ? (index >= 2 ? ids[index - 2] : nil, index >= 1 ? ids[index - 1] : nil)
                    : (index + 1 < ids.count ? ids[index + 1] : nil, index + 2 < ids.count ? ids[index + 2] : nil)
                guard up ? before != nil : after != nil else { return .none }
                return .run { [store] send in
                    do {
                        let saved = try await store.reorder(savedFilter: id, after: after, before: before)
                        await send(.loaded(.success(saved.snapshot)))
                    } catch {
                        await send(.savedFilterMoveFailed(error))
                    }
                }

            case .savedFilterMoveFailed(let error):
                state.notice = .failed("Couldn't move the saved filter", error)
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
                var quickAdd = QuickAddFeature.State(workspace: workspace, now: now, timeZone: timeZone)
                quickAdd.parent = QuickAddFeature.Choice(id: parent.id, name: parent.title)
                state.quickAdd = quickAdd
                return .none

            case .statusShortcut(let status):
                guard let slot = state.currentList, let rows = state[list: slot] else { return .none }
                if rows.detail != nil { return .send(list(slot, .detail(.presented(.statusPicked(status))))) }
                // The shortcut names the status, so it acts on the row at once.
                guard let cursor = rows.cursor else { return .none }
                return .send(list(slot, .rowAction(cursor, .status(status))))

            case .listShortcut(let shortcut):
                guard let slot = state.currentList, let rows = state[list: slot] else { return .none }
                switch shortcut {
                case .duplicate:
                    return .send(list(slot, .duplicateTapped))
                case .trash:
                    return .send(list(slot, .trashKeyPressed))
                case .selectAll:
                    return .send(list(slot, .selectAllTapped))
                case .moveUp, .moveDown:
                    guard let cursor = rows.cursor else { return .none }
                    return .send(list(slot, .movedOnePlace(cursor, up: shortcut == .moveUp)))
                case .sort(let sort):
                    return .send(list(slot, .sortPicked(sort)))
                case .pickDue:
                    guard let id = rows.detail?.id ?? rows.cursor else { return .none }
                    return .send(list(slot, .rowAction(id, .reschedule)))
                }

            case .priorityShortcut(let priority):
                guard let slot = state.currentList, state[list: slot]?.detail != nil else { return .none }
                return .send(list(slot, .detail(.presented(.priorityPicked(priority)))))

            case .dueShortcut(let due):
                guard let slot = state.currentList, state[list: slot]?.detail != nil else { return .none }
                return .send(list(slot, .detail(.presented(.rescheduled(due.date(now: now, in: timeZone))))))

            case .detailShortcut(let shortcut):
                guard let slot = state.currentList, state[list: slot]?.detail != nil else { return .none }
                let action: TaskDetailFeature.Action =
                    switch shortcut {
                    case .repeats(let preset): .repeatPicked(preset)
                    case .customRepeat: .repeatCustomTapped
                    case .reminder(let choice): .reminderPicked(choice)
                    case .customReminder: .reminderCustomTapped
                    case .avenue(let id): .avenuePicked(id)
                    case .label(let id): .labelToggled(id)
                    }
                return .send(list(slot, .detail(.presented(action))))

            case .goTo(let item):
                // Find puts the cursor in the search field.
                if item == .search { state.search.isFieldFocused = true }
                switch state.platform {
                case .mac:
                    return .send(.selected(item))
                case .phone:
                    switch item {
                    case .today: state.tab = .today
                    case .search: state.tab = .search
                    case .conversations: state.tab = .conversations
                    case .upcoming, .calendar:
                        state.tab = .upcoming
                        return .send(.upcomingCalendarShown(item == .calendar))
                    default: break
                    }
                    return .none
                }

            case .chatCommandChosen(let command):
                guard let id = state.openChatID else { return .none }
                return .send(.conversations(.chats(.element(id: id, action: .commandChosen(command)))))

            case .goToTaskTapped:
                guard let workspace = state.workspace else { return .none }
                state.goToTask = GoToTaskFeature.State(workspace: workspace)
                return .none

            case .goToTask(.presented(.delegate(.open(let id)))):
                return .send(.reminderOpened(id))

            case .goToTask:
                return .none

            case .onboarding(.presented(.delegate(.finished(let snapshot)))):
                state.$hasOnboarded.withLock { $0 = true }
                state.$seenRelease.withLock { $0 = WhatsNew.release }
                state.onboarding = nil
                let shown = snapshot.map { show($0, in: &state) } ?? .none
                guard state.platform == .mac else {
                    state.tab = .today
                    return shown
                }
                return .concatenate(shown, .send(.selected(.today)))

            case .onboarding:
                return .none

            case .whatsNewDismissed:
                state.isWhatsNewShown = false
                state.$seenRelease.withLock { $0 = WhatsNew.release }
                return .none

            case .shortcutsTapped:
                state.isShortcutsShown = true
                return .none

            case .redoTapped(let id):
                guard let redo = state.redo, redo.id == id, state.lastChange == nil,
                    let workspace = state.workspace
                else { return .none }
                state.redo = nil
                guard redo.isCurrent(in: workspace) else {
                    state.notice = AlertState {
                        TextState("Couldn't redo")
                    } actions: {
                        ButtonState(role: .cancel) { TextState("OK") }
                    } message: {
                        TextState(
                            """
                            It changed again since, here or on another device, and redoing would lose that \
                            change. Open it to make the change again by hand.
                            """)
                    }
                    return .none
                }
                return .run { [store] send in
                    if let entry = redo.entry {
                        await send(.redone(redo.message, Result { try await store.revert(entries: [entry]) }))
                    } else {
                        await send(.redone(redo.message, Result { try await store.undo(redo.change) }))
                    }
                }

            case .redone(let message, .success(let saved)):
                // Undo takes the redone change back again.
                let redone = LastChange(reversing: saved, id: uuid(), message: message)
                state.$lastChange.withLock { last in
                    if last == nil { last = redone }
                }
                return .merge(show(saved.snapshot, in: &state), .run { [feedback] _ in await feedback(.tap) })

            case .redone(_, .failure(let error)):
                state.notice = .failed("Couldn't redo", error)
                return .none

            case .undoTapped(let id):
                guard let last = state.lastChange, last.id == id, let workspace = state.workspace else {
                    return .none
                }
                state.undoing = last
                guard last.isCurrent(in: workspace) else {
                    state.$lastChange.withLock { $0 = nil }
                    state.notice = AlertState {
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
                // Undoing isn't a change to undo in turn, but Redo puts it
                // back. Not when it added labels, Projects or Avenues along
                // with its tasks, which Undo took away and Redo wouldn't
                // bring back.
                state.$lastChange.withLock { $0 = nil }
                if let undone = state.undoing, undone.entry != nil || undone.change.added.allSatisfy(\.isTask) {
                    state.redo = LastChange(reversing: saved, id: uuid(), message: undone.message)
                }
                state.undoing = nil
                return .merge(show(saved.snapshot, in: &state), .run { [feedback] _ in await feedback(.tap) })

            case .undone(.failure(let error)):
                state.undoing = nil
                state.notice = .failed("Couldn't undo", error)
                return .none

            case .leftTaskSaved(.success(let saved)):
                return show(saved.snapshot, in: &state)

            case .leftTaskSaved(.failure(let error)):
                state.notice = .failed("Couldn't save the task", error)
                return .none

            case .toastShown(let id):
                return .run { [clock] send in
                    try await clock.sleep(for: Self.toastDuration)
                    await send(.toastExpired(id))
                }

            case .toastExpired(let id):
                state.dismissedToast = id
                return .none

            case .copyNoticeShown(let id):
                return .run { [clock] send in
                    try await clock.sleep(for: Self.copyNoticeDuration)
                    await send(.copyNoticeExpired(id))
                }
                .cancellable(id: CancelID.copyNotice, cancelInFlight: true)

            case .copyNoticeExpired(let id):
                if state.lastCopy?.id == id { state.$lastCopy.withLock { $0 = nil } }
                return .none

            case .syncBannerDismissed:
                if let banner = state.syncBanner, banner.isFailure {
                    state.dismissedSyncFailure = banner.messages.first
                }
                state.syncBanner = nil
                return .none

            case .syncChanged(let sync):
                // One notification per batch: the first review item while
                // Lifr is away, and none while it's locked, which would name
                // the device to whoever holds this one.
                let notifies =
                    state.sync.review.isEmpty && state.awayAt != nil && !state.locksApp && !state.isWindow
                let reviewFrom = notifies ? sync.review.first?.device : nil
                state.sync = sync
                state.settings.sync = sync
                state.onboarding?.sync = sync
                switch sync.status {
                case .failed(.notSetUp), .failed(.noAccount), .syncing:
                    break
                case .failed(let error):
                    // Once is often the network; twice in a row is told,
                    // until it works or fails another way.
                    state.syncFailures += 1
                    let reason = error.description
                    if state.syncFailures >= 2, state.dismissedSyncFailure != reason,
                        state.syncBanner == nil || state.syncBanner?.isFailure == true
                    {
                        state.syncBanner = SyncBanner(failure: reason)
                    }
                case .idle:
                    state.syncFailures = 0
                    state.dismissedSyncFailure = nil
                    if state.syncBanner?.isFailure == true { state.syncBanner = nil }
                }
                let notice: Effect<Action> =
                    reviewFrom.map { device in .run { [reminders] _ in await reminders.notifyReview(device) } } ?? .none
                guard state.syncReview != nil else { return notice }
                return .merge(
                    notice, .send(.syncReview(.presented(.show(sync.review, state.workspace ?? Workspace())))))

            case .foregrounded:
                let away = state.awayAt
                state.awayAt = nil
                var effects: [Effect<Action>] = []
                // Other windows follow the first one's lock.
                if !state.isWindow, state.locksApp, !state.isLocked, AppLock.locks(away: away, now: now) {
                    state.isLocked = true
                    effects.append(.send(.unlockTapped))
                }
                if state.sync.status != .syncing {
                    effects.append(.run { [sync] _ in _ = await sync.sync() })
                }
                // Away overnight, today is another day.
                effects.append(reload())
                effects.append(saveWidgetMarks())
                effects.append(saveSharedTasks())
                return .merge(effects)

            case .widgetMarksSaved(let saved, let unsaved):
                if unsaved > 0 {
                    state.notice = AlertState {
                        TextState("The widget couldn't mark ^[\(unsaved) task](inflect: true) Done")
                    } actions: {
                        ButtonState(role: .cancel) { TextState("OK") }
                    } message: {
                        TextState(
                            """
                            A task with open Sub tasks needs you to decide what happens to them, and a save can \
                            fail. Open the task in Today and pick its status.
                            """)
                    }
                }
                guard let saved else { return .none }
                return show(saved.snapshot, in: &state)

            case .sharedTasksSaved(let saved, let unsaved):
                if unsaved > 0 {
                    state.notice = AlertState {
                        TextState("Lifr couldn't add ^[\(unsaved) shared task](inflect: true)")
                    } actions: {
                        ButtonState(role: .cancel) { TextState("OK") }
                    } message: {
                        TextState(
                            """
                            They were taken from the share sheet but couldn't be saved. Share them again, or add \
                            them with Quick Add.
                            """)
                    }
                }
                guard let saved else { return .none }
                return show(saved.snapshot, in: &state)

            case .dayChanged:
                // Lifr can stay open for days: what has aged out of undo and
                // the trash goes on each new day, not only at launch.
                return tidy(ensuringInbox: false)

            case .syncNowTapped:
                guard state.sync.status != .syncing else { return .none }
                // Asked for, so a failure is told.
                return .run { [sync] send in
                    if case .failed(let error) = await sync.sync().status {
                        await send(.syncNowFailed(error.description))
                    } else {
                        await send(.syncNowSucceeded)
                    }
                }

            case .refreshedInBackground:
                guard !state.isWindow else { return .none }
                return refreshInBackground(state)

            case .syncNowSucceeded:
                state.announce(String(localized: "Synced"))
                return .none

            case .syncNowFailed(let reason):
                state.notice = AlertState {
                    TextState("Couldn't Sync")
                } actions: {
                    ButtonState(role: .cancel) { TextState("OK") }
                } message: {
                    TextState(reason)
                }
                return .none

            case .backgrounded:
                // What changed here reaches the other devices before Lifr
                // is suspended.
                guard state.syncsContinuously, state.sync.status != .syncing else { return .none }
                return .run { [sync] _ in _ = await sync.flush() }

            case .unlockTapped, .unlocked, .lockMirrored, .appActivated:
                return .none

            case .syncReviewTapped:
                guard !state.sync.review.isEmpty else { return .none }
                state.syncReview = SyncReviewFeature.State(
                    items: state.sync.review, workspace: state.workspace ?? Workspace(),
                    thisDevice: state.thisDevice)
                return .none

            case .syncReview(.presented(.delegate(.resolved))):
                // The choices were saved through sync, not a screen.
                return .run { [store] send in await send(.loaded(Result { try await store.snapshot() })) }

            case .syncReview(.presented(.delegate(.finished))):
                state.syncReview = nil
                return .none

            case .today(.delegate(.saved(let snapshot))),
                .upcoming(.list(.delegate(.saved(let snapshot)))),
                .calendar(.list(.delegate(.saved(let snapshot)))),
                .trash(.delegate(.saved(let snapshot))),
                .procrastinated(.list(.delegate(.saved(let snapshot)))),
                .anytime(.delegate(.saved(let snapshot))),
                .later(.delegate(.saved(let snapshot))),
                .logbook(.delegate(.saved(let snapshot))),
                .avenues(.delegate(.saved(let snapshot))),
                .search(.delegate(.saved(let snapshot))),
                .savedFilter(.presented(.delegate(.saved(let snapshot)))),
                .avenue(.delegate(.saved(let snapshot))),
                .board(.list(.delegate(.saved(let snapshot)))),
                .settings(.delegate(.saved(let snapshot))),
                .conversations(.delegate(.saved(let snapshot))),
                .quickAdd(.presented(.delegate(.saved(let snapshot)))),
                .capture(.presented(.delegate(.saved(let snapshot)))):
                return show(snapshot, from: ShownList(action), in: &state)

            case .captureOpened:
                guard state.capture == nil, let workspace = state.workspace else { return .none }
                state.capture = QuickAddFeature.State(workspace: workspace, now: now, timeZone: timeZone)
                return .none

            case .avenues(.avenue(.presented(.addTaskTapped))):
                guard let scope = state.avenues.avenue?.scope else { return .none }
                return .send(.newTaskInTapped(scope))

            case .avenue(.addTaskTapped):
                guard let scope = state.avenue?.scope else { return .none }
                return .send(.newTaskInTapped(scope))

            case .today(.delegate(.planFromAnytime)):
                if state.platform == .mac { return .send(.selected(.anytime)) }
                state.tab = .avenues
                state.shownShelf = .anytime
                return .none

            case .board(.addTapped):
                guard let scope = state.board?.scope else { return .none }
                return .send(.newTaskInTapped(scope))

            case .shaken, .reportBugTapped, .recordStepsTapped, .stopRecordingTapped, .bugReport:
                return .none

            case .calendar(.delegate(.openProject(let id))):
                guard state.workspace?.projects[id] != nil else { return .none }
                if state.platform == .mac { return .send(.selected(.scope(.project(id)))) }
                state.tab = .avenues
                return .send(.avenues(.opened(.project(id))))

            case .upcomingCalendarShown(let shown):
                state.$upcomingShowsCalendar.withLock { $0 = shown }
                return .none

            case .today(.list(.retryTapped)), .upcoming(.list(.retryTapped)), .calendar(.list(.retryTapped)),
                .procrastinated(.list(.retryTapped)), .anytime(.retryTapped), .later(.retryTapped),
                .logbook(.retryTapped), .board(.list(.retryTapped)):
                // A first load that failed left every screen without the
                // records; one Try Again loads them all.
                return state.workspace == nil ? reload() : .none

            case .remindersScheduled(let failure?):
                guard !state.explainedReminders else { return .none }
                state.explainedReminders = true
                state.notice = AlertState {
                    TextState("Some Reminders Won't Ring Here")
                } actions: {
                    ButtonState(role: .cancel) { TextState("OK") }
                } message: {
                    TextState(failure)
                }
                return .none

            case .binding, .remindersScheduled, .today, .upcoming, .calendar, .trash, .procrastinated, .avenues,
                .search, .savedFilter, .anytime, .later, .logbook,
                .avenue, .board, .conversations, .settings, .quickAdd, .capture, .approval, .notice, .syncReview,
                .lockAlert:
                return .none
            }
        }
        .ifLet(\.avenue, action: \.avenue) { AvenueFeature() }
        .ifLet(\.board, action: \.board) { BoardFeature() }
        .ifLet(\.$savedFilter, action: \.savedFilter) { SearchFeature() }
        .ifLet(\.$quickAdd, action: \.quickAdd) { QuickAddFeature() }
        .ifLet(\.$capture, action: \.capture) { QuickAddFeature() }
        .ifLet(\.$approval, action: \.approval) { ApprovalFeature() }
        .ifLet(\.$syncReview, action: \.syncReview) { SyncReviewFeature() }
        .ifLet(\.$notice, action: \.notice)
        .ifLet(\.$lockAlert, action: \.lockAlert)
        .ifLet(\.$bugReport, action: \.bugReport) { BugReportFeature() }
        .ifLet(\.$goToTask, action: \.goToTask) { GoToTaskFeature() }
        .ifLet(\.$onboarding, action: \.onboarding) { OnboardingFeature() }
    }

    /// The shake menu, recording the steps to a bug, and describing it.
    /// Shows `item` on the Mac, closing what the list left showed.
    private func select(_ item: SidebarItem?, in state: inout State) -> Effect<Action> {
        let left = state.selection
        let leaving = state.currentList.flatMap { state[list: $0]?.detail }
        state.selection = item
        state.avenue = nil
        state.board = nil
        // What the detail column showed belongs to the list left, so
        // it goes with it.
        if left != item {
            state.today.list.detail = nil
            state.upcoming.list.detail = nil
            state.calendar.list.detail = nil
            state.procrastinated.list.detail = nil
            state.search.list.detail = nil
            state.anytime.detail = nil
            state.later.detail = nil
            state.logbook.detail = nil
            state.savedFilter = nil
        }
        var effect: Effect<Action> =
            left == .conversations && item != .conversations ? .send(.conversations(.closed)) : .none
        // Its typed title and description are saved first, as leaving the
        // field does.
        if left != item, let leaving, let edit = leaving.unsavedEdit {
            effect = .merge(
                effect,
                .run { [store] send in
                    await send(.leftTaskSaved(Result { try await store.edit(leaving.id, edit) }))
                })
        }
        if case .savedFilter(let id) = item, let workspace = state.workspace {
            state.savedFilter = workspace.savedFilters[id].map {
                SearchFeature.State(saved: $0, workspace: workspace, now: now, timeZone: timeZone)
            }
        }
        guard case .scope(let scope) = item, let workspace = state.workspace else { return effect }
        let avenue = AvenueFeature.State(scope: scope, workspace: workspace, now: now, timeZone: timeZone)
        state.avenue = avenue
        state.board = BoardFeature.State(
            scope: scope, title: avenue.title, workspace: workspace, now: now, timeZone: timeZone)
        return effect
    }

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

    /// Syncs every so often while the app runs; while it syncs
    /// continuously, more often and also when the network comes back.
    /// Starting it again replaces the one running.
    private func syncTimer(_ state: State) -> Effect<Action> {
        let continuous = state.syncsContinuously
        let interval = continuous ? Self.continuousSyncInterval : Self.syncInterval
        return .merge(
            .run { [sync, clock] _ in
                for await _ in clock.timer(interval: interval) { _ = await sync.sync() }
            },
            .run { [sync] _ in
                guard continuous else { return }
                for await _ in await sync.reconnected() { _ = await sync.sync() }
            }
        )
        .cancellable(id: CancelID.syncTimer, cancelInFlight: true)
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

        case .lockMirrored(true):
            state.isLocked = true
            return .none

        case .appActivated(false):
            if state.awayAt == nil { state.awayAt = now }
            return .none

        case .appActivated(true):
            let away = state.awayAt
            state.awayAt = nil
            // Other windows follow the first one's lock.
            guard !state.isWindow, state.locksApp, !state.isLocked, AppLock.locks(away: away, now: now) else {
                return .none
            }
            state.isLocked = true
            return .send(.unlockTapped)

        case .unlocked(.success), .lockMirrored(false):
            state.isLocked = false
            state.isUnlocking = false
            guard let link = state.pendingLink else { return .none }
            state.pendingLink = nil
            return .send(.linkOpened(link))

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

    /// Shows `snapshot`'s records on every screen, then schedules their
    /// reminders. Saves made at once, and updates from sync, can arrive in
    /// any order: records earlier than ones shown already are ignored.
    /// Loads the records again and shows them on every screen, as of now.
    /// Packs changes too old to undo and deletes tasks trashed too long
    /// ago, adds the Inbox when asked, then loads. Each step runs even when
    /// one before it fails.
    private func tidy(ensuringInbox: Bool) -> Effect<Action> {
        .run { [store, now] send in
            var steps: [@Sendable () async throws -> Void] = [
                { try await store.archiveJournal(now.addingTimeInterval(-JournalEntry.undoWindow)) },
                { try await store.purgeTrash(now: now) },
            ]
            if ensuringInbox { steps.append { try await store.ensureInbox() } }
            for step in steps {
                do {
                    try await step()
                } catch {
                    Logger(subsystem: "lifr", category: "store").error("\(String(describing: error), privacy: .public)")
                }
            }
            await send(.loaded(Result { try await store.snapshot() }))
        }
    }

    private func reload() -> Effect<Action> {
        .run { [store] send in await send(.loaded(Result { try await store.snapshot() })) }
    }

    /// Shows `snapshot` in every list, except `sender`: the list that
    /// saved it shows it already.
    private func show(_ snapshot: Snapshot, from sender: ShownList? = nil, in state: inout State) -> Effect<Action> {
        guard state.saves.isLatest(snapshot) else { return .none }
        let workspace = snapshot.workspace
        state.workspace = workspace
        // The lists show only what the Focus that's on leaves; the records
        // and what's opened by link or search stay whole.
        let full = snapshot
        let snapshot = Snapshot(workspace.focused(on: state.focusAvenues), number: snapshot.number)
        if let picked = workspace.settings.defaultReminder, picked != state.defaultReminder {
            state.$defaultReminder.withLock { $0 = picked }
        }
        var effects: [Effect<Action>] = []
        if sender != .today {
            effects.append(TodayFeature().reduce(into: &state.today, action: .list(.show(snapshot))).map(Action.today))
        }
        if sender != .upcoming {
            effects.append(
                UpcomingFeature().reduce(into: &state.upcoming, action: .list(.show(snapshot))).map(Action.upcoming))
        }
        if sender != .calendar {
            effects.append(
                CalendarFeature().reduce(into: &state.calendar, action: .list(.show(snapshot))).map(Action.calendar))
        }
        if sender != .trash {
            effects.append(TrashFeature().reduce(into: &state.trash, action: .show(full)).map(Action.trash))
        }
        if sender != .procrastinated {
            effects.append(
                ProcrastinatedFeature().reduce(into: &state.procrastinated, action: .list(.show(snapshot)))
                    .map(Action.procrastinated))
        }
        if sender != .anytime {
            effects.append(TaskListFeature().reduce(into: &state.anytime, action: .show(snapshot)).map(Action.anytime))
        }
        if sender != .later {
            effects.append(TaskListFeature().reduce(into: &state.later, action: .show(snapshot)).map(Action.later))
        }
        if sender != .logbook {
            effects.append(TaskListFeature().reduce(into: &state.logbook, action: .show(snapshot)).map(Action.logbook))
        }
        effects.append(AvenuesFeature().reduce(into: &state.avenues, action: .show(snapshot)).map(Action.avenues))
        effects.append(SearchFeature().reduce(into: &state.search, action: .list(.show(snapshot))).map(Action.search))
        var savedFilterShown = Effect<Action>.none
        if let id = state.savedFilter?.savedFilterID, workspace.savedFilters[id] == nil {
            // Deleted here or on another device.
            state.savedFilter = nil
            if state.selection == .savedFilter(id) { state.selection = .search }
        } else if var savedFilter = state.savedFilter {
            savedFilterShown = SearchFeature().reduce(into: &savedFilter, action: .list(.show(snapshot)))
                .map { Action.savedFilter(.presented($0)) }
            state.savedFilter = savedFilter
        }
        state.avenue?.show(snapshot, now: now, timeZone: timeZone)
        state.board?.list.show(snapshot, now: now, timeZone: timeZone)
        state.settings.show(full)
        state.conversations.show(full)
        return .merge(
            effects + [savedFilterShown]
                + (state.isWindow ? [] : [writeGlance(state), indexSpotlight(state), scheduleReminders(state)]))
    }

    /// Another Mac window's start: the records and their changes, and how
    /// sync is going, without the process's own work.
    private func startWindow(_ state: inout State) -> Effect<Action> {
        state.isRunning = true
        state.biometry = lock.biometry()
        if state.place != Place() { state.restoring = state.place }
        return .merge(
            reload(),
            .run { [store] send in
                for await update in await store.updates() { await send(.updated(update)) }
            },
            .run { [sync] send in
                for await state in await sync.states() { await send(.syncChanged(state)) }
            },
            watchFocus(state))
    }

    /// Shows the lists again, with the badge and widgets, when a Focus
    /// comes on or goes off.
    private func watchFocus(_ state: State) -> Effect<Action> {
        .run { [focusAvenues = state.$focusAvenues] send in
            for await _ in focusAvenues.publisher.dropFirst().removeDuplicates().values { await send(.focusChanged) }
        }
    }

    /// Schedules this device's notifications for the reminders in `state`,
    /// without titles while the lock is on.
    private func scheduleReminders(_ state: State) -> Effect<Action> {
        guard let workspace = state.workspace else { return .none }
        let (locked, counted, summary) = (state.locksApp, state.badgeCount, state.dailySummary)
        let focus = state.focusAvenues
        // Worked out in the effect, off the main actor.
        return .run { [reminders, now, timeZone] send in
            guard
                let failure = await Self.schedule(
                    workspace, with: reminders, now: now, timeZone: timeZone, locked: locked, counted: counted,
                    summary: summary, focus: focus)
            else { return }
            await send(.remindersScheduled(failure: failure))
        }
        .cancellable(id: CancelID.reminders, cancelInFlight: true)
    }

    /// Schedules `workspace`'s reminders and sets the badge. Returns nil
    /// when cancelled first, otherwise why scheduling failed, if it did.
    private static func schedule(
        _ workspace: Workspace, with reminders: RemindersClient, now: Date, timeZone: TimeZone, locked: Bool,
        counted: BadgeCount, summary: DailySummary, focus: FocusAvenues
    ) async -> String?? {
        let scheduled = workspace.scheduledReminders(
            now: now, timeZone: timeZone, hidingTitles: locked, summaryAt: summary.time)
        let badge = workspace.focused(on: focus).badgeCount(counted, now: now, timeZone: timeZone)
        guard !Task.isCancelled else { return nil }
        // Notifications that are off leave the reminders saved and
        // ringing elsewhere; the detail screen says so when one is
        // added. Any other failure is told once per launch.
        var failure: String?
        do {
            try await reminders.schedule(scheduled)
        } catch {
            if await reminders.permission() != .denied { failure = String(describing: error) }
        }
        await reminders.setBadge(badge)
        return .some(failure)
    }

    /// Syncs while Lifr is away, then rings this device's reminders and
    /// shows its widgets as the records now are: one closed on another
    /// device stops ringing here before Lifr next opens.
    private func refreshInBackground(_ state: State) -> Effect<Action> {
        let locked = state.locksApp
        let counted = state.badgeCount
        let summary = state.dailySummary
        let focus = state.focusAvenues
        return .run { [sync, store, reminders, glance, now, timeZone] _ in
            _ = await sync.sync()
            let workspace: Workspace
            do {
                workspace = try await store.snapshot().workspace
            } catch {
                Logger(subsystem: "lifr", category: "refresh").error("\(String(describing: error), privacy: .public)")
                return
            }
            _ = await Self.schedule(
                workspace, with: reminders, now: now, timeZone: timeZone, locked: locked, counted: counted,
                summary: summary, focus: focus)
            if let today = Self.glance(of: workspace.focused(on: focus), now: now, timeZone: timeZone, locked: locked)
            {
                await glance.write(today)
            }
        }
    }

    /// Gives the widgets Today as `state` has it, and as it will be on each
    /// of the next two weeks, for Upcoming and for after midnight, and each
    /// Avenue's open tasks and how many are due each day for the calendar:
    /// the titles, or only how many while the lock is on.
    private func writeGlance(_ state: State) -> Effect<Action> {
        guard let workspace = state.workspace else { return .none }
        // Worked out in the effect, off the main actor; a newer save's
        // glance replaces one still being worked out.
        return .run { [glance, now, timeZone, locked = state.locksApp, focus = state.focusAvenues] _ in
            guard
                let today = Self.glance(of: workspace.focused(on: focus), now: now, timeZone: timeZone, locked: locked),
                !Task.isCancelled
            else { return }
            await glance.write(today)
        }
        .cancellable(id: CancelID.glance, cancelInFlight: true)
    }

    /// Saves the tasks marked Done from a widget since Lifr last looked.
    /// One closed or trashed since is left as it is.
    private func saveWidgetMarks() -> Effect<Action> {
        .run { [glance, store] send in
            let marked: Set<UUID>
            do {
                marked = try await glance.takeMarked()
            } catch {
                Logger(subsystem: "lifr", category: "widgets").error("\(String(describing: error), privacy: .public)")
                return
            }
            guard !marked.isEmpty else { return }
            let tasks: [UUID: LifrTask]
            do {
                tasks = try await store.load().tasks
            } catch {
                await send(.widgetMarksSaved(nil, unsaved: marked.count))
                return
            }
            var saved: Saved?
            var unsaved = 0
            for id in marked.sorted(by: { $0.uuidString < $1.uuidString }) {
                guard let task = tasks[id], task.isOpen, task.deletedAt == nil else { continue }
                do {
                    saved = try await store.setStatus(.done, id)
                } catch {
                    unsaved += 1
                }
            }
            await send(.widgetMarksSaved(saved, unsaved: unsaved))
        }
    }

    /// Saves the tasks added from the share sheet since Lifr last looked,
    /// each in the Avenue picked, or the Inbox when that one's gone.
    private func saveSharedTasks() -> Effect<Action> {
        .run { [glance, store] send in
            let shared: [SharedTask]
            do {
                shared = try await glance.takeShared()
            } catch {
                Logger(subsystem: "lifr", category: "share").error("\(String(describing: error), privacy: .public)")
                return
            }
            guard !shared.isEmpty else { return }
            let avenues: [UUID: Avenue]
            do {
                avenues = try await store.load().avenues
            } catch {
                await send(.sharedTasksSaved(nil, unsaved: shared.count))
                return
            }
            let tasks = shared.map { task in
                let avenue = task.avenueID.flatMap { avenues[$0] }.flatMap { $0.archivedAt == nil ? $0.id : nil }
                var new = Workspace.NewTask(title: task.title, avenueID: avenue ?? Avenue.inboxID)
                new.details = task.details
                new.dueDate = task.due.flatMap { try? CalendarDate($0) }
                return new
            }
            do {
                await send(.sharedTasksSaved(try await store.createTasks(tasks), unsaved: 0))
            } catch {
                await send(.sharedTasksSaved(nil, unsaved: tasks.count))
            }
        }
    }

    private static func glance(of workspace: Workspace, now: Date, timeZone: TimeZone, locked: Bool) -> TodayGlance? {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        let start = calendar.startOfDay(for: now)
        let starts = (0...15).compactMap { calendar.date(byAdding: .day, value: $0, to: start) }
        guard starts.count == 16 else { return nil }
        func glanced(_ rows: [TaskRowData]) -> [TodayGlance.Row] {
            rows.map { row in
                let (isOverdue, time): (Bool, TimeOfDay?) =
                    switch row.due {
                    case .overdue(_, let time): (true, time)
                    case .today(let time), .later(_, let time): (false, time)
                    case nil: (false, nil)
                    }
                return TodayGlance.Row(
                    id: row.id, title: row.title, code: row.code, isOverdue: isOverdue,
                    isInProgress: row.status == .inProgress, dueMinutes: time.map { $0.hour * 60 + $0.minute },
                    color: row.avenueColor)
            }
        }
        func rows(at date: Date) -> [TodayGlance.Row] {
            glanced(TaskQuery.today.sections(workspace, date, timeZone).flatMap(\.rows))
        }
        let open = workspace.listedTasks.filter { $0.isOpen && !$0.isSubtask }
        let avenues = workspace.avenueRows.map { avenue in
            let rows = glanced(
                workspace.rows(open.filter { $0.avenueID == avenue.id }, now: now, timeZone: timeZone))
            return AvenueGlance(
                id: avenue.id, name: avenue.name, symbol: avenue.symbol, color: avenue.color, count: rows.count,
                overdue: rows.count(where: \.isOverdue), rows: Array(rows.prefix(8)))
        }
        let projects = workspace.avenueRows.flatMap { avenue in
            workspace.projects.values.filter { $0.avenueID == avenue.id && $0.status == .active }
                .sorted { ($0.sortOrder, $0.createdAt) < ($1.sortOrder, $1.createdAt) }
                .map { project in
                    let rows = glanced(
                        workspace.rows(open.filter { $0.projectID == project.id }, now: now, timeZone: timeZone))
                    return AvenueGlance(
                        id: project.id, name: project.name, symbol: "folder.fill", color: avenue.color,
                        count: rows.count, overdue: rows.count(where: \.isOverdue), rows: Array(rows.prefix(8)))
                }
        }
        let filters = workspace.sortedSavedFilters.map { saved in
            let matching = saved.filter.matching(in: workspace, now: now, timeZone: timeZone).filter {
                $0.isOpen && !$0.isSubtask
            }
            let rows = glanced(workspace.rows(matching, now: now, timeZone: timeZone))
            return AvenueGlance(
                id: saved.id, name: saved.name, symbol: saved.symbol, color: "blue", count: rows.count,
                overdue: rows.count(where: \.isOverdue), rows: Array(rows.prefix(8)))
        }
        let thisMonth = CalendarDate(now, in: timeZone).firstOfMonth
        let months = thisMonth..<thisMonth.firstOfMonth(adding: 2)
        var dueCounts: [String: Int] = [:]
        for task in open {
            guard let due = task.dueDate, months.contains(due) else { continue }
            dueCounts[due.description, default: 0] += 1
        }
        let later = (1..<15).map { day in
            TodayGlance(
                asOf: now, from: starts[day], until: starts[day + 1], rows: rows(at: starts[day]), locked: locked)
        }
        return TodayGlance(
            asOf: now, from: start, until: starts[1], rows: rows(at: now), locked: locked, later: later,
            avenues: avenues, projects: projects, filters: filters, dueCounts: dueCounts)
    }

    /// Lists every task not in the Trash, and each open Avenue and Project,
    /// in the system's search, or nothing while the lock is on.
    private func indexSpotlight(_ state: State) -> Effect<Action> {
        guard let workspace = state.workspace else { return .none }
        // Worked out in the effect, off the main actor.
        return .run { [spotlight, locked = state.locksApp] _ in
            guard !locked else {
                await spotlight.index([])
                return
            }
            let avenues = workspace.avenues.values.filter { $0.archivedAt == nil }.sorted {
                $0.sortOrder < $1.sortOrder
            }
            let projects = workspace.projects.values.filter { $0.status != .archived }.sorted {
                $0.createdAt < $1.createdAt
            }
            let tasks = workspace.tasks.values.filter { $0.deletedAt == nil }.sorted { $0.createdAt < $1.createdAt }
            let items: [SpotlightItem] =
                avenues.map {
                    SpotlightItem(
                        .avenue, id: $0.id, title: $0.name, detail: String(localized: "Avenue in Lifr"), details: "")
                }
                + projects.map { project in
                    let avenue = workspace.avenues[project.avenueID]?.name
                    return SpotlightItem(
                        .project, id: project.id, title: project.name,
                        detail: avenue.map { String(localized: "Project in \($0)") } ?? String(localized: "Project"),
                        details: project.details)
                }
                + tasks.map {
                    let avenue = workspace.avenues[$0.avenueID]?.name
                    let code = workspace.displayCode(of: $0)
                    return SpotlightItem(
                        id: $0.id, title: $0.title, detail: avenue.map { "\(code) · \($0)" } ?? code,
                        details: $0.details)
                }
            guard !Task.isCancelled else { return }
            await spotlight.index(items)
        }
        .cancellable(id: CancelID.spotlight, cancelInFlight: true)
    }

    /// Follows a link: a task from wherever the person is, Today, a day in
    /// the calendar, or Quick Add. Before the records load, a task waits
    /// like a reminder's and anything else waits in `pendingLink`.
    private func open(_ link: DeepLink, in state: inout State) -> Effect<Action> {
        if case .task(let id, let picksStatus) = link {
            if let workspace = state.workspace, workspace.tasks[id].map({ $0.deletedAt != nil }) ?? true {
                state.notice = AlertState {
                    TextState("That task isn't in Lifr")
                } actions: {
                    ButtonState(role: .cancel) { TextState("OK") }
                } message: {
                    TextState(
                        """
                        It was deleted, or it's on another device that hasn't synced yet. Look in Trash, or \
                        sync and open the link again.
                        """)
                }
                return .send(.goTo(.today))
            }
            return .send(picksStatus ? .reminderStatusRequested(id) : .reminderOpened(id))
        }
        guard state.workspace != nil else {
            state.pendingLink = link
            return .none
        }
        switch link {
        case .task:
            return .none
        case .today:
            return .send(.goTo(.today))
        case .upcoming:
            return .send(.goTo(.upcoming))
        case .avenue(let id):
            // An archived or deleted Avenue has no page: Today instead.
            guard state.workspace?.avenues[id].map({ $0.archivedAt == nil }) == true else {
                return .send(.goTo(.today))
            }
            if state.platform == .mac { return .send(.selected(.scope(.avenue(id)))) }
            state.tab = .avenues
            return .send(.avenues(.opened(.avenue(id))))
        case .project(let id):
            // An archived or deleted Project has no page: Today instead.
            guard state.workspace?.projects[id].map({ $0.status != .archived }) == true else {
                return .send(.goTo(.today))
            }
            if state.platform == .mac { return .send(.selected(.scope(.project(id)))) }
            state.tab = .avenues
            return .send(.avenues(.opened(.project(id))))
        case .filter(let id):
            guard state.workspace?.savedFilters[id] != nil else { return .send(.goTo(.today)) }
            if state.platform == .mac { return .send(.selected(.savedFilter(id))) }
            return .send(.savedFilterTapped(id))
        case .calendar(let date):
            return .concatenate(.send(.goTo(.calendar)), .send(.calendar(.daySelected(date))))
        case .add(let title):
            showQuickAdd(.task, in: &state)
            if let title { state.quickAdd?.title = title }
            return .none
        }
    }

    /// Quick Add for `kind`, starting where `adds` says, or in the list on
    /// screen so what's added shows where it was added.
    private func showQuickAdd(_ kind: QuickAddFeature.Kind, _ adds: NewTaskDefaults? = nil, in state: inout State) {
        guard let workspace = state.workspace else { return }
        let adds = adds ?? state.currentList.flatMap { state[list: $0]?.query.adds } ?? NewTaskDefaults()
        var quickAdd = QuickAddFeature.State(workspace: workspace, now: now, timeZone: timeZone)
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
        if let day = adds.dueDate {
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = timeZone
            let components = DateComponents(year: day.year, month: day.month, day: day.day, hour: 12)
            if let noon = calendar.date(from: components) {
                quickAdd.due = .pick
                quickAdd.pickedDate = noon
            }
        }
        state.quickAdd = quickAdd
    }

    private func showNextApproval(_ state: inout State) {
        guard state.approval == nil, !state.queuedApprovals.isEmpty else { return }
        let next = state.queuedApprovals.removeFirst()
        state.approval = ApprovalFeature.State(request: next, workspace: state.workspace ?? Workspace())
    }

    /// `action` for the list in `slot`.
    /// Saves what dropping `ids` on `item` means, as one change, through
    /// the list shown so it shows the result or why it was refused.
    private func dropped(_ ids: [UUID], on item: SidebarItem, in state: State) -> Effect<Action> {
        guard let workspace = state.workspace, let slot = state.currentList else { return .none }
        let tasks = ids.compactMap { workspace.tasks[$0] }
        let save: @Sendable () async throws -> Saved
        switch item {
        case .scope(.avenue(let avenueID)):
            let moving = tasks.filter { $0.avenueID != avenueID || $0.projectID != nil }.map(\.id)
            guard !moving.isEmpty else { return .none }
            save = { [store] in try await store.move(moving, toAvenue: avenueID, project: nil) }
        case .scope(.project(let projectID)):
            guard let project = workspace.projects[projectID] else { return .none }
            let moving = tasks.filter { $0.projectID != projectID }.map(\.id)
            guard !moving.isEmpty else { return .none }
            save = { [store] in try await store.move(moving, toAvenue: project.avenueID, project: projectID) }
        case .today, .upcoming:
            let today = CalendarDate(now, in: timeZone)
            let day = item == .today ? today : today.adding(days: 1)
            let edits = tasks.compactMap { task in Workspace.TaskEdit.due(day, for: task).map { (task.id, $0) } }
            guard !edits.isEmpty else { return .none }
            save = { [store] in try await store.edit(edits) }
        default:
            return .none
        }
        return .run { [feedback] send in
            let saved = await Result { try await save() }
            if case .success = saved { await feedback(.tap) }
            await send(list(slot, .saved(saved)))
        }
    }

    /// Adds a task for each line of `text` where `item` puts it, as one
    /// change, through the list shown.
    private func dropped(_ text: String, on item: SidebarItem, in state: State) -> Effect<Action> {
        guard let workspace = state.workspace, let slot = state.currentList else { return .none }
        let today = CalendarDate(now, in: timeZone)
        var avenueID = Avenue.inboxID
        var projectID: UUID?
        var due: CalendarDate?
        switch item {
        case .scope(.avenue(let id)): avenueID = id
        case .scope(.project(let id)):
            guard let project = workspace.projects[id] else { return .none }
            avenueID = project.avenueID
            projectID = id
        case .today: due = today
        case .upcoming: due = today.adding(days: 1)
        default: return .none
        }
        let tasks = QuickAddFeature.lines(text).map { title in
            var new = Workspace.NewTask(title: title, avenueID: avenueID, projectID: projectID)
            new.dueDate = due
            return new
        }
        guard !tasks.isEmpty else { return .none }
        return .run { [store, feedback] send in
            let saved = await Result { try await store.createTasks(tasks) }
            if case .success = saved { await feedback(.tap) }
            await send(list(slot, .saved(saved)))
        }
    }

    private func list(_ slot: ListSlot, _ action: TaskListFeature.Action) -> Action {
        switch slot {
        case .today: .today(.list(action))
        case .upcoming: .upcoming(.list(action))
        case .calendar: .calendar(.list(action))
        case .procrastinated: .procrastinated(.list(action))
        case .anytime: .anytime(action)
        case .later: .later(action)
        case .logbook: .logbook(action)
        case .search: .search(.list(action))
        case .savedFilter: .savedFilter(.presented(.list(action)))
        case .avenue: .avenue(.list(action))
        case .board: .board(.list(action))
        case .openAvenue: .avenues(.avenue(.presented(.list(action))))
        }
    }
}

extension RootFeature {
    /// Decides, once the first records are in, whether this device needs
    /// onboarding or this release's notes. A device that finds records
    /// already, such as a second one or an update, skips onboarding.
    fileprivate func introduce(_ workspace: Workspace, in state: inout State) {
        guard !state.isWindow, !state.introduced else { return }
        state.introduced = true
        if !state.hasOnboarded {
            guard workspace.hasRecordsOfItsOwn else {
                state.onboarding = OnboardingFeature.State(sync: state.sync)
                return
            }
            state.$hasOnboarded.withLock { $0 = true }
        }
        if state.seenRelease == 0 {
            // Nothing to compare with: the notes describe changes this
            // device never saw before.
            state.$seenRelease.withLock { $0 = WhatsNew.release }
        } else if state.seenRelease < WhatsNew.release {
            state.isWhatsNewShown = true
        }
    }
}

extension Workspace {
    /// Whether there's more than the Inbox every workspace starts with.
    fileprivate var hasRecordsOfItsOwn: Bool {
        !tasks.isEmpty || avenues.keys.contains { $0 != Avenue.inboxID }
    }
}

extension RootFeature.Action {
    /// What a menu, a keyboard shortcut or the menu bar sends.
    fileprivate var isCommand: Bool {
        switch self {
        case .newTaskTapped, .newTaskInTapped, .newAvenueTapped, .newProjectTapped, .newSubtaskTapped,
            .statusShortcut, .undoTapped, .syncReviewTapped, .apiSwitched, .reportBugTapped, .recordStepsTapped,
            .redoTapped, .goTo, .priorityShortcut, .dueShortcut, .goToTaskTapped, .shortcutsTapped, .syncNowTapped,
            .backTapped, .forwardTapped, .listShortcut, .detailShortcut, .captureOpened:
            true
        default:
            false
        }
    }
}

extension SharedKey where Self == AppStorageKey<Bool>.Default {
    /// Whether the iPhone's Upcoming tab shows the calendar. This device
    /// only; the list unless the person picks the calendar.
    package static var upcomingShowsCalendar: Self {
        Self[.appStorage("lifrUpcomingShowsCalendar"), default: false]
    }
}

/// A list that saves and shows its own snapshot, so root needn't show it
/// that snapshot again.
private enum ShownList {
    case today, upcoming, calendar, trash, procrastinated, anytime, later, logbook

    init?(_ action: RootFeature.Action) {
        switch action {
        case .today: self = .today
        case .upcoming: self = .upcoming
        case .calendar: self = .calendar
        case .trash: self = .trash
        case .procrastinated: self = .procrastinated
        case .anytime: self = .anytime
        case .later: self = .later
        case .logbook: self = .logbook
        default: return nil
        }
    }
}

extension RootFeature.State {
    /// Has VoiceOver say `message`, even when it's the same as last time.
    fileprivate mutating func announce(_ message: String) {
        announcement = Announcement(number: (announcement?.number ?? 0) + 1, message: message)
    }
}

/// A message for VoiceOver, numbered so the same words twice are said twice.
package struct Announcement: Equatable, Sendable {
    package var number: Int
    package var message: String

    /// What says that tasks open in `before` were closed or deleted
    /// elsewhere by `after`, or `nil` when none were.
    static func closed(from before: Workspace, to after: Workspace) -> String? {
        let closed = before.tasks.values.filter { task in
            guard task.deletedAt == nil, task.isOpen, let now = after.tasks[task.id] else { return false }
            return now.deletedAt != nil || !now.isOpen
        }
        switch closed.count {
        case 0: return nil
        case 1: return closed.first.map { String(localized: "“\($0.title)” closed elsewhere.") }
        default:
            let text = AttributedString(localized: "^[\(closed.count) task](inflect: true) closed elsewhere.")
            return String(text.characters)
        }
    }
}
