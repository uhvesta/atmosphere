import ChatUI
import Combine
import ComposableArchitecture
import LifrAvenue
import LifrBoard
import LifrBugReport
import LifrCalendar
import LifrConversations
import LifrCore
import LifrProcrastinated
import LifrQuickAdd
import LifrSearch
import LifrSettings
import LifrSpotlightClient
import LifrSyncReview
import LifrTaskList
import LifrToday
import LifrTrash
import LifrUI
import LifrUpcoming
import SwiftUI
import Synchronization
import UniformTypeIdentifiers

/// The app's window: tabs on iPhone, a sidebar on the Mac.
package struct RootView: View {
    @Bindable package var store: StoreOf<RootFeature>
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    package init(store: StoreOf<RootFeature>) {
        self.store = store
    }

    package var body: some View {
        ZStack {
            if store.isLocked {
                LockView(store: store)
            } else {
                content
            }
            if store.platform == .phone, store.locksApp, !store.isLocked, scenePhase != .active {
                // What the app switcher shows: nothing to read. Locked, the
                // lock screen already shows nothing.
                PrivacyCover()
            }
        }
        .modifier(AccentTint())
        .onChange(of: scenePhase) { _, phase in
            if phase == .active {
                store.send(.conversations(.resumed))
                store.send(.foregrounded)
            } else {
                store.send(.backgrounded)
            }
        }
        .onReceive(Self.dayChanges) { _ in store.send(.dayChanged) }
        .alert($store.scope(state: \.lockAlert, action: \.lockAlert))
        .modifier(ShakeMenu(store: store))
        .task { await store.send(.task).finish() }
    }

    /// Midnight, or a change of the clock's time zone.
    private static var dayChanges: AnyPublisher<Notification, Never> {
        NotificationCenter.default.publisher(for: .NSCalendarDayChanged)
            .merge(with: NotificationCenter.default.publisher(for: .NSSystemTimeZoneDidChange))
            .receive(on: RunLoop.main)
            .eraseToAnyPublisher()
    }

    /// Whether the Undo toast and banners show at the bottom: on iPhone
    /// above the tab bar, except while a chat is open; on the Mac, where
    /// at the top they'd cover the first rows, except in the assistant,
    /// whose composer is at the bottom.
    /// Toasts and banners slide in from their edge, or fade with Reduce
    /// Motion.
    private var edgeTransition: AnyTransition {
        reduceMotion ? .opacity : .move(edge: bannersAtBottom ? .bottom : .top).combined(with: .opacity)
    }

    private var bannersAtBottom: Bool {
        switch store.platform {
        case .phone: !(store.tab == .conversations && store.conversations.openID != nil)
        case .mac: store.selection != .conversations
        }
    }

    /// How far the banners sit from the edge they show at: clear of the
    /// tab bar or the toolbar.
    private var bannerInset: CGFloat {
        guard bannersAtBottom else { return 60 }
        return store.platform == .phone ? 64 : 20
    }

    /// Where the banners show. In the Mac's assistant, over the chat at
    /// the right, clear of the list of conversations.
    private var bannerAlignment: Alignment {
        if bannersAtBottom { return .bottom }
        return store.platform == .mac ? .topTrailing : .top
    }

    private var content: some View {
        Group {
            switch store.platform {
            case .phone: PhoneRootView(store: store)
            case .mac: MacRootView(store: store)
            }
        }
        .overlay(alignment: bannerAlignment) {
            // On iPhone above the tab bar, clear of every screen's own
            // controls, but under the title while a chat is open, whose
            // field is just above the tab bar. On the Mac small enough to
            // leave the sidebar free.
            VStack(spacing: 8) {
                if store.isRecordingSteps {
                    RecordingBar { store.send(.stopRecordingTapped) }
                }
                if let toast = store.toast {
                    UndoToast(message: toast.message) { store.send(.undoTapped(toast.id)) }
                        // A new change starts its own countdown.
                        .id(toast.id)
                        .task(id: toast.id) { await store.send(.toastShown(toast.id)).finish() }
                        .transition(edgeTransition)
                }
                if let banner = store.syncBanner {
                    SyncBannerView(banner: banner) { store.send(.syncBannerDismissed) }
                        .transition(edgeTransition)
                }
            }
            .frame(maxWidth: store.platform == .phone ? .infinity : 440)
            .padding(bannersAtBottom ? .bottom : .top, bannerInset)
            .padding(.trailing, bannerAlignment == .topTrailing ? 24 : 0)
            .animation(reduceMotion ? .easeInOut(duration: 0.2) : .spring, value: store.toast?.id)
            .animation(reduceMotion ? .easeInOut(duration: 0.2) : .spring, value: store.syncBanner)
        }
        .modifier(RegistersUndo(store: store))
        .alert($store.scope(state: \.notice, action: \.notice))
        .onOpenURL { store.send(.urlOpened($0)) }
        .onContinueUserActivity(SpotlightClient.activityType) { activity in
            if let id = SpotlightClient.task(in: activity) { store.send(.linkOpened(.task(id, picksStatus: false))) }
        }
        .sheet(item: $store.scope(state: \.quickAdd, action: \.quickAdd)) { store in
            NavigationStack { QuickAddView(store: store) }
                .sheetLayout()
        }
        .sheet(item: $store.scope(state: \.bugReport, action: \.bugReport)) { store in
            NavigationStack { BugReportView(store: store) }
                .sheetLayout()
                .interactiveDismissDisabled()
        }
        .sheet(item: $store.scope(state: \.syncReview, action: \.syncReview)) { store in
            NavigationStack { SyncReviewView(store: store) }
                .interactiveDismissDisabled()
        }
        .sheet(item: $store.scope(state: \.goToTask, action: \.goToTask)) { store in
            GoToTaskView(store: store)
        }
        .sheet(isPresented: $store.isShortcutsShown) {
            KeyboardShortcutsView()
        }
        .sheet(item: $store.scope(state: \.approval, action: \.approval)) { store in
            NavigationStack { ApprovalView(store: store) }
                .sheetLayout()
                .interactiveDismissDisabled()
        }
        // The Mac's default Form lays out in columns and doesn't scroll, so a
        // long one runs off the window. Grouped scrolls, and is already the
        // iPhone's look.
        .formStyle(.grouped)
    }
}

/// Today, Upcoming, Avenues and Search, with quick add over each, and a
/// banner over every tab when a synced change needs review.
private struct PhoneRootView: View {
    @Bindable var store: StoreOf<RootFeature>

    var body: some View {
        TabView(selection: $store.tab) {
            Tab("Today", systemImage: "sun.max", value: RootFeature.Tab.today) {
                NavigationStack {
                    TodayView(store: store.scope(state: \.today, action: \.today)) {
                        store.send(.newTaskTapped)
                    }
                    .safeAreaInset(edge: .top) { SyncReviewBanner(store: store) }
                    .toolbar {
                        ToolbarItem(placement: .navigation) {
                            Button {
                                store.isSettingsShown = true
                            } label: {
                                Label("Settings", systemImage: "gear")
                            }
                        }
                        newTask
                    }
                }
            }
            Tab("Upcoming", systemImage: "calendar", value: RootFeature.Tab.upcoming) {
                NavigationStack {
                    Group {
                        if store.upcomingShowsCalendar {
                            CalendarView(store: store.scope(state: \.calendar, action: \.calendar)) {
                                store.send(.newTaskTapped)
                            }
                        } else {
                            UpcomingView(store: store.scope(state: \.upcoming, action: \.upcoming)) {
                                store.send(.upcomingCalendarShown(true))
                            }
                        }
                    }
                    .safeAreaInset(edge: .top) { SyncReviewBanner(store: store) }
                    // The List and Calendar switch is the title: a large
                    // title under it crowds both.
                    #if os(iOS)
                        .navigationBarTitleDisplayMode(.inline)
                    #endif
                    .toolbar {
                        ToolbarItem(placement: .principal) {
                            Picker(
                                "Show as",
                                selection: Binding(
                                    get: { store.upcomingShowsCalendar },
                                    set: { store.send(.upcomingCalendarShown($0)) })
                            ) {
                                Text("List").tag(false)
                                Text("Calendar").tag(true)
                            }
                            .pickerStyle(.segmented)
                        }
                        newTask
                    }
                }
            }
            Tab("Avenues", systemImage: "square.stack.3d.up", value: RootFeature.Tab.avenues) {
                NavigationStack {
                    AvenuesView(store: store.scope(state: \.avenues, action: \.avenues)) {
                        Section {
                            ForEach(RootFeature.Shelf.allCases, id: \.self) { shelf in
                                Button {
                                    store.shownShelf = shelf
                                } label: {
                                    LabeledContent {
                                        if shelf != .logbook {
                                            Text("\(store.state[list: shelf.slot]?.rows.count ?? 0)")
                                        }
                                    } label: {
                                        Label(shelf.title, systemImage: shelf.systemImage)
                                    }
                                }
                            }
                        }
                        let savedFilters = store.workspace?.sortedSavedFilters ?? []
                        if !savedFilters.isEmpty {
                            Section("Filters") {
                                ForEach(savedFilters) { saved in
                                    Button {
                                        store.send(.savedFilterTapped(saved.id))
                                    } label: {
                                        Label(saved.name, systemImage: saved.symbol)
                                    }
                                    .contextMenu {
                                        MoveButtons(
                                            ids: savedFilters.map(\.id), id: saved.id,
                                            move: { store.send(.savedFilterMoved(saved.id, up: $0)) })
                                    }
                                }
                            }
                        }
                        Section {
                            Button {
                                store.isTrashShown = true
                            } label: {
                                LabeledContent {
                                    Text("\(store.trash.rows.count)")
                                } label: {
                                    Label("Trash", systemImage: "trash")
                                }
                            }
                            .accessibilityValue("\(store.trash.rows.count) tasks")
                        }
                    }
                    .safeAreaInset(edge: .top) { SyncReviewBanner(store: store) }
                    .navigationDestination(isPresented: $store.isTrashShown) {
                        TrashView(store: store.scope(state: \.trash, action: \.trash))
                    }
                    .navigationDestination(item: $store.shownShelf) { shelf in
                        ShelfView(store: store, shelf: shelf)
                    }
                    .navigationDestination(item: $store.scope(state: \.savedFilter, action: \.savedFilter)) {
                        SearchView(store: $0)
                    }
                }
            }
            Tab("Assistant", systemImage: "bubble.left.and.text.bubble.right", value: RootFeature.Tab.conversations) {
                NavigationStack {
                    ConversationsView(store: store.scope(state: \.conversations, action: \.conversations))
                        .safeAreaInset(edge: .top) { SyncReviewBanner(store: store) }
                }
            }
            Tab("Search", systemImage: "magnifyingglass", value: RootFeature.Tab.search, role: .search) {
                NavigationStack {
                    SearchView(store: store.scope(state: \.search, action: \.search))
                        .safeAreaInset(edge: .top) { SyncReviewBanner(store: store) }
                }
            }
        }
        .sheet(isPresented: $store.isSettingsShown) {
            NavigationStack {
                SettingsView(store: store.scope(state: \.settings, action: \.settings))
                    .toolbar {
                        Button("Done") { store.isSettingsShown = false }
                    }
            }
        }
    }

    private var newTask: some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            AddMenu(store: store)
        }
    }
}

/// The id of one window's toolbar. Windows whose toolbars share an id
/// keep the same items, and AppKit traps when one window's page adds or
/// drops an item the others don't have. So each open window holds its own
/// number: the first window is always "lifr", and each keeps the
/// customisation saved under its number.
private final class ToolbarSlot: Sendable {
    private static let taken = Mutex<Set<Int>>([])
    private let number: Int
    let id: String

    init() {
        number = Self.taken.withLock { taken in
            let free = (0..<taken.count + 1).first { !taken.contains($0) } ?? taken.count
            taken.insert(free)
            return free
        }
        id = number == 0 ? "lifr" : "lifr-\(number + 1)"
    }

    deinit {
        Self.taken.withLock { _ = $0.remove(number) }
    }
}

/// The sidebar, then the list or board, then the task detail.
private struct MacRootView: View {
    @Bindable var store: StoreOf<RootFeature>
    @State private var toolbar = ToolbarSlot()
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        columns
            // The menus act on the window in front.
            .focusedSceneValue(\.lifrWindow, store)
            #if os(macOS)
                .environment(\.opensInNewWindow) { id in
                    openWindow(id: Place.windowID, value: Place(sidebar: store.selection, task: id))
                }
            #endif
    }

    @ViewBuilder private var columns: some View {
        if isBoardShown {
            // A board needs the window's width for its columns, so it takes
            // the detail column's place, and a task opens over it.
            NavigationSplitView {
                sidebar
            } detail: {
                NavigationStack { content.toolbar(id: toolbar.id) { addMenu } }
            }
        } else {
            NavigationSplitView {
                sidebar
            } content: {
                content
                    .navigationSplitViewColumnWidth(min: 320, ideal: 380)
                    .toolbar(id: toolbar.id) { addMenu }
                    #if os(macOS)
                        .onPasteCommand(of: [.utf8PlainText]) { _ in store.send(.pasteCommanded) }
                        // Text from another app lands like ⌘V; a task
                        // dragged within the list is left to the list.
                        .dropDestination(for: String.self) { items, _ in
                            guard !items.allSatisfy({ UUID(uuidString: $0) != nil }) else { return false }
                            store.send(.textPasted(items.joined(separator: "\n")))
                            return true
                        }
                        // Rows take tasks dropped onto them, and hand text on.
                        .environment(\.droppedText) { store.send(.textPasted($0)) }
                    #endif
            } detail: {
                NavigationStack {
                    if store.selection == .conversations {
                        NothingSelected("No conversation selected", systemImage: "bubble.left.and.text.bubble.right")
                    } else {
                        NothingSelected("No task selected", systemImage: "checklist")
                    }
                }
                .navigationSplitViewColumnWidth(min: 360, ideal: 480)
            }
        }
    }

    /// Whether the page shows its own search field, which a second way to
    /// search would squeeze into a button.
    private var hasSearchField: Bool {
        switch store.selection {
        case .search, .savedFilter, .conversations: true
        default: false
        }
    }

    private var isBoardShown: Bool {
        guard case .scope = store.selection else { return false }
        return store.content == .board
    }

    private var sidebar: some View {
        Sidebar(store: store)
            .navigationSplitViewColumnWidth(min: 180, ideal: 220)
    }

    /// Add and Search, which the person can rearrange or remove with
    /// Customize Toolbar.
    @ToolbarContentBuilder private var addMenu: some CustomizableToolbarContent {
        ToolbarItem(id: "add", placement: .primaryAction) {
            // Named, as the Mac otherwise names it after its symbol only
            // once the first window has loaded.
            AddMenu(store: store)
                .help("Add a task or Project")
                .accessibilityIdentifier("add")
        }
        if !hasSearchField {
            ToolbarItem(id: "search", placement: .primaryAction) {
                Button("Search", systemImage: "magnifyingglass") { store.send(.goTo(.search)) }
                    .help("Search titles, details and comments (⌘F)")
                    .accessibilityIdentifier("search")
            }
        }
    }

    @ViewBuilder private var content: some View {
        switch store.selection {
        case .today:
            TodayView(store: store.scope(state: \.today, action: \.today)) { store.send(.newTaskTapped) }
        case .upcoming:
            UpcomingView(store: store.scope(state: \.upcoming, action: \.upcoming)) {
                store.selection = .calendar
            }
        case .calendar:
            CalendarView(store: store.scope(state: \.calendar, action: \.calendar)) {
                store.send(.newTaskTapped)
            }
        case .procrastinated:
            ProcrastinatedView(store: store.scope(state: \.procrastinated, action: \.procrastinated))
        case .search:
            SearchView(store: store.scope(state: \.search, action: \.search))
        case .savedFilter:
            if let savedFilter = store.scope(state: \.savedFilter, action: \.savedFilter.presented) {
                SearchView(store: savedFilter)
            }
        case .trash:
            TrashView(store: store.scope(state: \.trash, action: \.trash))
        case .anytime:
            ShelfView(store: store, shelf: .anytime)
        case .later:
            ShelfView(store: store, shelf: .later)
        case .logbook:
            ShelfView(store: store, shelf: .logbook)
        case .conversations:
            // No stack of its own, so a chat opens in the detail column, as
            // a task does.
            ConversationsView(store: store.scope(state: \.conversations, action: \.conversations))
        case .scope:
            Group {
                if store.content == .board, let board = store.scope(state: \.board, action: \.board) {
                    BoardView(store: board)
                } else if let avenue = store.scope(state: \.avenue, action: \.avenue) {
                    AvenueView(store: avenue)
                }
            }
            .toolbar {
                ToolbarItem(placement: .principal) {
                    Picker("Show as", selection: $store.content) {
                        ForEach(RootFeature.Content.allCases, id: \.self) { Text($0.title) }
                    }
                    .pickerStyle(.segmented)
                }
            }
        case nil:
            ContentUnavailableView("Pick a list", systemImage: "sidebar.left")
        }
    }
}

private struct Sidebar: View {
    @Bindable var store: StoreOf<RootFeature>

    var body: some View {
        List(selection: $store.selection.sending(\.selected)) {
            lists
            let savedFilters = store.workspace?.sortedSavedFilters ?? []
            if !savedFilters.isEmpty {
                Section("Filters") {
                    ForEach(savedFilters) { saved in
                        Label(saved.name, systemImage: saved.symbol)
                            .tag(RootFeature.SidebarItem.savedFilter(saved.id))
                            .contextMenu {
                                Button("Edit Filter…", systemImage: "pencil") {
                                    store.send(.selected(.savedFilter(saved.id)))
                                    store.send(.savedFilter(.presented(.editSavedFilterTapped)))
                                }
                                Divider()
                                MoveButtons(
                                    ids: savedFilters.map(\.id), id: saved.id,
                                    move: { store.send(.savedFilterMoved(saved.id, up: $0)) })
                            }
                    }
                }
            }
            Section("Avenues") {
                ForEach(store.avenues.avenues) { avenue in
                    AvenueItems(avenue: avenue, store: store)
                }
            }
            if !store.avenues.archived.isEmpty {
                Section("Archived") {
                    ForEach(store.avenues.archived) { row in
                        Label {
                            Text(row.name)
                        } icon: {
                            Image(systemName: "archivebox")
                        }
                        .badge(Text(row.detail))
                        .tag(RootFeature.SidebarItem.scope(row.scope))
                    }
                }
            }
        }
        .navigationTitle("Lifr")
        .safeAreaInset(edge: .top) { SyncReviewBanner(store: store) }
        .safeAreaInset(edge: .bottom) {
            VStack(alignment: .leading, spacing: 6) {
                Button {
                    store.send(.newAvenueTapped)
                } label: {
                    Label("New Avenue", systemImage: "plus")
                }
                .buttonStyle(.borderless)
                Text(SyncStatusText.status(store.sync))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel(Text("Sync: \(SyncStatusText.status(store.sync))"))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
        .sheet(item: $store.scope(state: \.avenues.newAvenue, action: \.avenues.newAvenue)) { store in
            NavigationStack { NewAvenueView(store: store) }
                .sheetLayout(height: 260)
        }
    }

    @ViewBuilder private var lists: some View {
        Label("Today", systemImage: "sun.max")
            .badge(store.today.list.rows.count)
            .tag(RootFeature.SidebarItem.today)
            .modifier(TaskDrop(item: .today, store: store))
        Label("Upcoming", systemImage: "calendar")
            .badge(store.upcoming.list.rows.count)
            .tag(RootFeature.SidebarItem.upcoming)
            .modifier(TaskDrop(item: .upcoming, store: store))
        Label("Calendar", systemImage: "square.grid.3x3")
            .tag(RootFeature.SidebarItem.calendar)
        shelf(.anytime)
        shelf(.later)
        Label("Procrastinated", systemImage: "tortoise")
            .badge(store.procrastinated.list.rows.count)
            .tag(RootFeature.SidebarItem.procrastinated)
        shelf(.logbook)
        Label("Search", systemImage: "magnifyingglass")
            .tag(RootFeature.SidebarItem.search)
        Label("Assistant", systemImage: "bubble.left.and.text.bubble.right")
            .tag(RootFeature.SidebarItem.conversations)
        Label("Trash", systemImage: "trash")
            .tag(RootFeature.SidebarItem.trash)
    }

    private func shelf(_ shelf: RootFeature.Shelf) -> some View {
        Label(shelf.title, systemImage: shelf.systemImage)
            .badge(shelf == .logbook ? 0 : store.state[list: shelf.slot]?.rows.count ?? 0)
            .tag(shelf.sidebarItem)
    }
}

/// Anytime, Later or the Logbook, as a list titled with its name.
private struct ShelfView: View {
    let store: StoreOf<RootFeature>
    let shelf: RootFeature.Shelf

    var body: some View {
        list.navigationTitle(shelf.title)
    }

    @ViewBuilder private var list: some View {
        switch shelf {
        case .anytime: TaskListView(store: store.scope(state: \.anytime, action: \.anytime))
        case .later: TaskListView(store: store.scope(state: \.later, action: \.later))
        case .logbook: TaskListView(store: store.scope(state: \.logbook, action: \.logbook))
        }
    }
}

/// An Avenue in the sidebar, then its Projects, each with a menu to add
/// to it.
private struct AvenueItems: View {
    var avenue: AvenueRow
    var store: StoreOf<RootFeature>

    var body: some View {
        Label {
            Text(avenue.name)
        } icon: {
            Image(systemName: avenue.symbol).foregroundStyle(Color(avenue: avenue.color))
        }
        .badge(avenue.openCount)
        .tag(RootFeature.SidebarItem.scope(.avenue(avenue.id)))
        .modifier(TaskDrop(item: .scope(.avenue(avenue.id)), store: store))
        .contextMenu {
            Button("New Task in \(avenue.name)") { store.send(.newTaskInTapped(.avenue(avenue.id))) }
            Button("New Project…") { store.send(.newProjectTapped(avenue.id)) }
            Divider()
            MoveButtons(
                ids: store.avenues.avenues.map(\.id), id: avenue.id,
                move: { store.send(.avenues(.moveAvenue(avenue.id, up: $0))) })
        }
        ForEach(avenue.projects) { project in
            Label(project.name, systemImage: "folder")
                .padding(.leading)
                .badge(project.openCount)
                .tag(RootFeature.SidebarItem.scope(.project(project.id)))
                .modifier(TaskDrop(item: .scope(.project(project.id)), store: store))
                .contextMenu {
                    Button("New Task in \(project.name)") { store.send(.newTaskInTapped(.project(project.id))) }
                    Divider()
                    MoveButtons(
                        ids: avenue.projects.map(\.id), id: project.id,
                        move: { store.send(.avenues(.moveProject(project.id, up: $0))) })
                }
        }
    }
}

/// Takes tasks dragged from a list, or text dragged in from another app,
/// onto a sidebar item, highlighting the item while a drag is over it.
private struct TaskDrop: ViewModifier {
    var item: RootFeature.SidebarItem
    var store: StoreOf<RootFeature>
    @State private var isTargeted = false

    func body(content: Content) -> some View {
        content
            .dropDestination(for: String.self) { items, _ in
                let ids = items.compactMap(UUID.init(uuidString:))
                if ids.count == items.count {
                    store.send(.tasksDropped(ids, on: item))
                } else {
                    store.send(.textDropped(items.joined(separator: "\n"), on: item))
                }
                return !items.isEmpty
            } isTargeted: {
                isTargeted = $0
            }
            .listRowBackground(
                isTargeted ? RoundedRectangle(cornerRadius: 6).fill(Color.accentColor.opacity(0.2)) : nil)
    }
}

/// Records that couldn't merge, waiting for review, with a button that
/// opens it. Shown until they're resolved.
private struct SyncReviewBanner: View {
    let store: StoreOf<RootFeature>

    var body: some View {
        if !store.sync.review.isEmpty {
            Button {
                store.send(.syncReviewTapped)
            } label: {
                HStack(alignment: .firstTextBaseline) {
                    Image(systemName: "arrow.triangle.2.circlepath").accessibilityHidden(true)
                    Text(message).frame(maxWidth: .infinity, alignment: .leading)
                    Text("Review").bold()
                }
                .font(.callout)
                .padding(12)
                .background(.thinMaterial, in: .rect(cornerRadius: 12))
            }
            .buttonStyle(.plain)
            .padding(.horizontal)
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
        }
    }

    private var message: String {
        let count = store.sync.review.count
        return count == 1
            ? String(localized: "1 change needs review")
            : String(localized: "\(count) changes need review")
    }
}

/// What the last sync fixed, with a way to dismiss it.
private struct SyncBannerView: View {
    var banner: SyncBanner
    var dismiss: () -> Void

    var body: some View {
        HStack(alignment: .top) {
            Image(systemName: "arrow.triangle.2.circlepath")
            VStack(alignment: .leading, spacing: 4) {
                ForEach(banner.messages, id: \.self) { Text($0) }
            }
            Spacer()
            Button("Dismiss", systemImage: "xmark", action: dismiss)
                .labelStyle(.iconOnly)
        }
        .font(.callout)
        .padding()
        .background(.thinMaterial, in: .rect(cornerRadius: 12))
        .padding(.horizontal)
        .accessibilityElement(children: .combine)
    }
}

/// The last change with Undo, for a few seconds after it's made.
private struct UndoToast: View {
    var message: String
    var undo: () -> Void
    /// How much of the time to undo is left, drawn as a line that runs down.
    @State private var remaining = 1.0

    var body: some View {
        HStack {
            Text(message).lineLimit(2)
            Spacer()
            Button("Undo", action: undo).bold()
        }
        .font(.callout)
        .padding()
        .overlay(alignment: .bottomLeading) {
            GeometryReader { proxy in
                Capsule()
                    .fill(.secondary.opacity(0.5))
                    .frame(width: proxy.size.width * remaining, height: 2)
            }
            .frame(height: 2)
            .padding(.horizontal, 12)
            .padding(.bottom, 5)
            .accessibilityHidden(true)
        }
        .background(.thinMaterial, in: .rect(cornerRadius: 12))
        .padding(.horizontal)
        .onAppear {
            withAnimation(.linear(duration: RootFeature.toastDuration / .seconds(1))) { remaining = 0 }
        }
    }
}

/// Shown while the steps to a bug are recorded, with the way to stop.
private struct RecordingBar: View {
    var stop: () -> Void

    var body: some View {
        Button(action: stop) {
            HStack(spacing: 6) {
                Circle().fill(.red).frame(width: 8, height: 8).accessibilityHidden(true)
                Text("Recording")
                Text("·").accessibilityHidden(true)
                Text("Stop").bold()
            }
            .font(.callout)
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(.thinMaterial, in: .capsule)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Stop Recording Steps")
        .accessibilityHint("Stops recording and describes the bug")
    }
}

/// Shaking the iPhone: Report Bug, Record Steps, the Undo a shake used to
/// give, or Cancel. While steps are recorded, a shake stops them.
private struct ShakeMenu: ViewModifier {
    @Bindable var store: StoreOf<RootFeature>

    func body(content: Content) -> some View {
        #if os(iOS)
            content
                .onReceive(NotificationCenter.default.publisher(for: .lifrShaken)) { _ in
                    store.send(.shaken)
                }
                .onAppear {
                    // The shake is this menu's, not the system's Undo alert.
                    UIApplication.shared.applicationSupportsShakeToEdit = false
                }
                .confirmationDialog("Report a Bug", isPresented: $store.isBugMenuShown, titleVisibility: .hidden) {
                    Button("Report Bug") { store.send(.reportBugTapped) }
                    Button("Record Steps") { store.send(.recordStepsTapped) }
                    if let last = store.lastChange {
                        Button("Undo \(last.message)") { store.send(.undoTapped(last.id)) }
                    }
                    Button("Cancel", role: .cancel) {}
                }
        #else
            content
        #endif
    }
}

#if os(iOS)
    extension Notification.Name {
        /// The iPhone was shaken.
        static let lifrShaken = Notification.Name("LifrShaken")
    }

    extension UIWindow {
        // Every window passes shakes on, so a shake reaches the menu
        // whatever is on screen.
        override open func motionEnded(_ motion: UIEvent.EventSubtype, with event: UIEvent?) {
            if motion == .motionShake { NotificationCenter.default.post(name: .lifrShaken, object: nil) }
            super.motionEnded(motion, with: event)
        }
    }
#endif

/// Puts the last change in the window's undo manager, so Edit › Undo and ⌘Z
/// revert it. Text fields keep their own undo while they're being edited.
private struct RegistersUndo: ViewModifier {
    var store: StoreOf<RootFeature>
    @Environment(\.undoManager) private var undoManager

    func body(content: Content) -> some View {
        content
            .onChange(of: store.lastChange?.id, initial: true) { _, id in
                guard let undoManager else { return }
                guard let id, let message = store.lastChange?.message else {
                    // While Undo is on its way, its Redo stays.
                    if store.undoing == nil { undoManager.removeAllActions(withTarget: UndoTarget.shared) }
                    return
                }
                undoManager.removeAllActions(withTarget: UndoTarget.shared)
                register(undo: id, message, on: undoManager)
            }
            .onChange(of: store.redo?.id) { _, id in
                guard let undoManager else { return }
                // Undo from the toast leaves no Redo in the Edit menu, and
                // a failed or overtaken Undo none at all.
                if id == nil || !undoManager.canRedo {
                    if store.lastChange == nil { undoManager.removeAllActions(withTarget: UndoTarget.shared) }
                }
            }
    }

    /// Undo for the change `id`, which registers its Redo as it runs.
    private func register(undo id: UUID, _ message: String, on undoManager: UndoManager) {
        undoManager.registerUndo(withTarget: UndoTarget.shared) { _ in
            MainActor.assumeIsolated {
                _ = store.send(.undoTapped(id))
                undoManager.registerUndo(withTarget: UndoTarget.shared) { _ in
                    MainActor.assumeIsolated {
                        if let redo = store.redo { _ = store.send(.redoTapped(redo.id)) }
                    }
                }
                undoManager.setActionName(message)
            }
        }
        undoManager.setActionName(message)
    }
}

/// The object Lifr's undo actions are registered on.
private final class UndoTarget: Sendable {
    static let shared = UndoTarget()
}

/// The + button: a task or a Project, starting in what's on screen. Sub
/// tasks are added from their task, and Avenues have their own screen.
private struct AddMenu: View {
    let store: StoreOf<RootFeature>

    var body: some View {
        Menu {
            Button("Task", systemImage: "checklist") { store.send(.newTaskTapped) }
            Button("Project", systemImage: "folder") { store.send(.newProjectTapped(nil)) }
        } label: {
            Label("Add", systemImage: "plus")
        }
    }
}

/// The empty detail column on the Mac: quieter than a list with nothing in
/// it, since there's nothing wrong, only nothing picked yet.
private struct NothingSelected: View {
    let title: LocalizedStringKey
    let systemImage: String

    init(_ title: LocalizedStringKey, systemImage: String) {
        self.title = title
        self.systemImage = systemImage
    }

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: systemImage).font(.title).accessibilityHidden(true)
            Text(title).font(.title3)
        }
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
