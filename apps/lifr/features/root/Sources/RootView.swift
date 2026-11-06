import ComposableArchitecture
import LifrAvenue
import LifrBoard
import LifrBugReport
import LifrConversations
import LifrCore
import LifrProcrastinated
import LifrQuickAdd
import LifrSearch
import LifrSettings
import LifrSyncReview
import LifrToday
import LifrUI
import LifrUpcoming
import SwiftUI

/// The app's window: tabs on iPhone, a sidebar on the Mac.
package struct RootView: View {
    @Bindable package var store: StoreOf<RootFeature>
    @Environment(\.scenePhase) private var scenePhase

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
        .onChange(of: scenePhase) { _, phase in
            if phase == .active {
                store.send(.conversations(.resumed))
                store.send(.foregrounded)
            } else {
                store.send(.backgrounded)
            }
        }
        .alert($store.scope(state: \.lockAlert, action: \.lockAlert))
        .modifier(ShakeMenu(store: store))
        .task { await store.send(.task).finish() }
    }

    private var content: some View {
        Group {
            switch store.platform {
            case .phone: PhoneRootView(store: store)
            case .mac: MacRootView(store: store)
            }
        }
        .overlay(alignment: store.platform == .phone ? .bottom : .top) {
            // On iPhone above the tab bar, clear of every screen's own
            // controls. On the Mac under the toolbar, small enough to leave
            // the sidebar and the chat's composer free.
            VStack(spacing: 8) {
                if store.isRecordingSteps {
                    RecordingBar { store.send(.stopRecordingTapped) }
                }
                if let toast = store.toast {
                    UndoToast(message: toast.message) { store.send(.undoTapped(toast.id)) }
                        .task(id: toast.id) {
                            try? await Task.sleep(for: .seconds(5))
                            store.send(.toastExpired(toast.id))
                        }
                }
                if let banner = store.syncBanner {
                    SyncBannerView(banner: banner) { store.send(.syncBannerDismissed) }
                }
            }
            .frame(maxWidth: store.platform == .phone ? .infinity : 440)
            .padding(store.platform == .phone ? .bottom : .top, store.platform == .phone ? 64 : 60)
            .animation(.default, value: store.toast?.id)
        }
        .modifier(RegistersUndo(store: store))
        .alert($store.scope(state: \.undoAlert, action: \.undoAlert))
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

/// Today, Upcoming, Avenues and Search, with quick add over each.
private struct PhoneRootView: View {
    @Bindable var store: StoreOf<RootFeature>

    var body: some View {
        TabView(selection: $store.tab) {
            Tab("Today", systemImage: "sun.max", value: RootFeature.Tab.today) {
                NavigationStack {
                    TodayView(store: store.scope(state: \.today, action: \.today))
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
                    UpcomingView(store: store.scope(state: \.upcoming, action: \.upcoming))
                        .toolbar { newTask }
                }
            }
            Tab("Avenues", systemImage: "square.stack.3d.up", value: RootFeature.Tab.avenues) {
                NavigationStack {
                    AvenuesView(store: store.scope(state: \.avenues, action: \.avenues))
                }
            }
            Tab("Assistant", systemImage: "bubble.left.and.text.bubble.right", value: RootFeature.Tab.conversations) {
                NavigationStack {
                    ConversationsView(store: store.scope(state: \.conversations, action: \.conversations))
                }
            }
            Tab("Search", systemImage: "magnifyingglass", value: RootFeature.Tab.search, role: .search) {
                NavigationStack {
                    SearchView(store: store.scope(state: \.search, action: \.search))
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

/// The sidebar, then the list or board, then the task detail.
private struct MacRootView: View {
    @Bindable var store: StoreOf<RootFeature>

    var body: some View {
        if isBoardShown {
            // A board needs the window's width for its columns, so it takes
            // the detail column's place, and a task opens over it.
            NavigationSplitView {
                sidebar
            } detail: {
                NavigationStack { content.toolbar { addMenu } }
            }
        } else {
            NavigationSplitView {
                sidebar
            } content: {
                content
                    .navigationSplitViewColumnWidth(min: 320, ideal: 380)
                    .toolbar { addMenu }
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

    private var isBoardShown: Bool {
        guard case .scope = store.selection else { return false }
        return store.content == .board
    }

    private var sidebar: some View {
        Sidebar(store: store)
            .navigationSplitViewColumnWidth(min: 180, ideal: 220)
    }

    private var addMenu: some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            AddMenu(store: store)
                .help("Add a task or Project")
        }
    }

    @ViewBuilder private var content: some View {
        switch store.selection {
        case .today:
            TodayView(store: store.scope(state: \.today, action: \.today))
        case .upcoming:
            UpcomingView(store: store.scope(state: \.upcoming, action: \.upcoming))
        case .procrastinated:
            ProcrastinatedView(store: store.scope(state: \.procrastinated, action: \.procrastinated))
        case .search:
            SearchView(store: store.scope(state: \.search, action: \.search))
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
            Button {
                store.send(.newAvenueTapped)
            } label: {
                Label("New Avenue", systemImage: "plus")
            }
            .buttonStyle(.borderless)
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
            .tag(RootFeature.SidebarItem.today)
        Label("Upcoming", systemImage: "calendar")
            .badge(store.upcoming.list.rows.count)
            .tag(RootFeature.SidebarItem.upcoming)
        Label("Procrastinated", systemImage: "tortoise")
            .badge(store.procrastinated.list.rows.count)
            .tag(RootFeature.SidebarItem.procrastinated)
        Label("Search", systemImage: "magnifyingglass")
            .tag(RootFeature.SidebarItem.search)
        Label("Assistant", systemImage: "bubble.left.and.text.bubble.right")
            .tag(RootFeature.SidebarItem.conversations)
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
        .contextMenu {
            Button("New Task in \(avenue.name)") { store.send(.newTaskInTapped(.avenue(avenue.id))) }
            Button("New Project…") { store.send(.newProjectTapped(avenue.id)) }
        }
        ForEach(avenue.projects) { project in
            Label(project.name, systemImage: "folder")
                .padding(.leading)
                .badge(project.openCount)
                .tag(RootFeature.SidebarItem.scope(.project(project.id)))
                .contextMenu {
                    Button("New Task in \(project.name)") { store.send(.newTaskInTapped(.project(project.id))) }
                }
        }
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

    var body: some View {
        HStack {
            Text(message).lineLimit(2)
            Spacer()
            Button("Undo", action: undo).bold()
        }
        .font(.callout)
        .padding()
        .background(.thinMaterial, in: .rect(cornerRadius: 12))
        .padding(.horizontal)
        .transition(.move(edge: .bottom).combined(with: .opacity))
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
        .accessibilityLabel("Recording steps")
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
        content.onChange(of: store.lastChange?.id, initial: true) { _, id in
            guard let undoManager else { return }
            undoManager.removeAllActions(withTarget: UndoTarget.shared)
            guard let id, let message = store.lastChange?.message else { return }
            undoManager.registerUndo(withTarget: UndoTarget.shared) { _ in
                MainActor.assumeIsolated { _ = store.send(.undoTapped(id)) }
            }
            undoManager.setActionName(message)
        }
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
