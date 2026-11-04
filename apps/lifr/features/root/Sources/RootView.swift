import ComposableArchitecture
import LifrAvenue
import LifrBoard
import LifrCore
import LifrProcrastinated
import LifrQuickAdd
import LifrSearch
import LifrSettings
import LifrToday
import LifrUI
import LifrUpcoming
import SwiftUI

/// The app's window: tabs on iPhone, a sidebar on the Mac.
package struct RootView: View {
    @Bindable package var store: StoreOf<RootFeature>

    package init(store: StoreOf<RootFeature>) {
        self.store = store
    }

    package var body: some View {
        Group {
            switch store.platform {
            case .phone: PhoneRootView(store: store)
            case .mac: MacRootView(store: store)
            }
        }
        .overlay(alignment: .bottom) {
            if let banner = store.syncBanner {
                // Above the tab bar, clear of every screen's own controls.
                SyncBannerView(banner: banner) { store.send(.syncBannerDismissed) }
                    .padding(.bottom, 64)
            }
        }
        .task { await store.send(.task).finish() }
        .sheet(item: $store.scope(state: \.quickAdd, action: \.quickAdd)) { store in
            NavigationStack { QuickAddView(store: store) }
        }
        .sheet(item: $store.scope(state: \.approval, action: \.approval)) { store in
            NavigationStack { ApprovalView(store: store) }
                .interactiveDismissDisabled()
        }
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
            Button {
                store.send(.newTaskTapped)
            } label: {
                Label("New task", systemImage: "plus")
            }
        }
    }
}

/// The sidebar, then the list or board, then the task detail.
private struct MacRootView: View {
    @Bindable var store: StoreOf<RootFeature>

    var body: some View {
        NavigationSplitView {
            Sidebar(store: store)
        } content: {
            content
                .toolbar {
                    ToolbarItem(placement: .primaryAction) {
                        Button {
                            store.send(.newTaskTapped)
                        } label: {
                            Label("New task", systemImage: "plus")
                        }
                        .help("New task (⌘N)")
                    }
                }
        } detail: {
            NavigationStack {
                ContentUnavailableView("No task selected", systemImage: "checklist")
            }
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
                        ForEach(RootFeature.Content.allCases, id: \.self) { Text($0.rawValue) }
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
        }
        .navigationTitle("Lifr")
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
            Text(project.name)
                .padding(.leading)
                .badge(project.openCount)
                .tag(RootFeature.SidebarItem.scope(.project(project.id)))
                .contextMenu {
                    Button("New Task in \(project.name)") { store.send(.newTaskInTapped(.project(project.id))) }
                }
        }
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
