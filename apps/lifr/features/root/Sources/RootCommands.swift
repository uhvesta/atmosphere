import ComposableArchitecture
import LifrConversations
import LifrCore
import LifrLockClient
import LifrSettings
import LifrUI
import SwiftUI

/// The File menu's new items, Find, the View menu's Go to items, the Task
/// menu for the open task, the Chat menu's plan prompt, the Sync menu, and
/// Help's shortcuts and bug reports. Items that don't apply are disabled,
/// not hidden.
package struct RootCommands: Commands {
    /// The first window's store, for when no Lifr window is in front.
    private let main: StoreOf<RootFeature>
    @FocusedValue(\.lifrWindow) private var focused
    @Environment(\.openWindow) private var openWindow

    package init(store: StoreOf<RootFeature>) {
        main = store
    }

    /// The window in front's store.
    private var store: StoreOf<RootFeature> { focused ?? main }

    package var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("New Task") { store.send(.newTaskTapped) }
                .keyboardShortcut("n")
            Button("New Sub task") { store.send(.newSubtaskTapped) }
                .keyboardShortcut("n", modifiers: [.command, .shift])
            Button("New Project") { store.send(.newProjectTapped(nil)) }
                .keyboardShortcut("n", modifiers: [.command, .option])
            Button("New Avenue") { store.send(.newAvenueTapped) }
            #if os(macOS)
                Divider()
                Button("New Window") {
                    openWindow(id: WindowPlace.sceneID, value: WindowPlace(Place(sidebar: store.selection)))
                }
                .keyboardShortcut("n", modifiers: [.command, .control])
                .disabled(store.isLocked)
                Button("New Tab") { WindowTabs.open(Place(sidebar: store.selection), with: openWindow) }
                    .keyboardShortcut("t", modifiers: [.command, .option])
                    .disabled(store.isLocked)
                Button("Open Task in New Window") {
                    let place = Place(sidebar: store.selection, task: store.openTaskID)
                    openWindow(id: WindowPlace.sceneID, value: WindowPlace(place))
                }
                .disabled(store.isLocked || store.openTaskID == nil)
            #endif
        }
        #if os(macOS)
            CommandGroup(replacing: .printItem) {
                Button("Print…") {
                    guard let (title, list) = store.state.printable else { return }
                    ListPrinter.print(title: title, markdown: list.markdown)
                }
                .keyboardShortcut("p")
                .disabled(store.state.printable == nil)
            }
        #endif
        CommandGroup(after: .pasteboard) {
            Divider()
            Button("Duplicate") { store.send(.listShortcut(.duplicate)) }
                .keyboardShortcut("d")
                .disabled(store.statusTaskID == nil)
            // ⌫ in the list; a menu key would take it from text fields.
            Button("Move to Trash") { store.send(.listShortcut(.trash)) }
                .disabled(store.statusTaskID == nil)
            Button("Select All Tasks") { store.send(.listShortcut(.selectAll)) }
                .disabled(store.currentList == nil)
        }
        CommandGroup(after: .textEditing) {
            Divider()
            Button("Find") { store.send(.goTo(.search)) }
                .keyboardShortcut("f")
            Button("Go to Task…") { store.send(.goToTaskTapped) }
                .keyboardShortcut("k")
                .disabled(store.workspace == nil)
        }
        CommandGroup(before: .sidebar) {
            Button("Today") { store.send(.goTo(.today)) }
                .keyboardShortcut("1")
            Button("Upcoming") { store.send(.goTo(.upcoming)) }
                .keyboardShortcut("2")
            Button("Calendar") { store.send(.goTo(.calendar)) }
                .keyboardShortcut("3")
            ForEach(RootFeature.Shelf.allCases, id: \.self) { shelf in
                Button(shelf.title) { store.send(.goTo(shelf.sidebarItem)) }
            }
            Button("Trash") { store.send(.goTo(.trash)) }
            Divider()
            Toggle(
                "Show Closed",
                isOn: Binding(
                    get: { store.avenue?.showsCompleted ?? true },
                    set: { store.send(.avenue(.showsCompletedToggled($0))) })
            )
            // Only status groups show closed tasks, and the board always
            // has a column for each status.
            .disabled(store.avenue?.grouping != .status || store.content == .board)
            Picker(
                "Show As",
                selection: Binding(get: { store.content }, set: { store.send(.binding(.set(\.content, $0))) })
            ) {
                ForEach(RootFeature.Content.allCases, id: \.self) { Text($0.title) }
            }
            .disabled(!store.state.showsScope)
            Divider()
            Button("Back") { store.send(.backTapped) }
                .keyboardShortcut("[")
                .disabled(store.back.isEmpty)
            Button("Forward") { store.send(.forwardTapped) }
                .keyboardShortcut("]")
                .disabled(store.forward.isEmpty)
            Divider()
        }
        CommandMenu("Task") {
            Menu("Status") {
                ForEach(Array(TaskStatus.shortcuts.enumerated()), id: \.offset) { index, status in
                    Button(status.title) { store.send(.statusShortcut(status)) }
                        .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: [.command, .option])
                }
            }
            .disabled(store.statusTaskID == nil)
            Menu("Priority") {
                ForEach(Array(Priority.shortcuts.enumerated()), id: \.offset) { index, priority in
                    Button(priority.title ?? String(localized: "None")) { store.send(.priorityShortcut(priority)) }
                        .keyboardShortcut(KeyEquivalent(Character("\(index)")), modifiers: .control)
                }
            }
            .disabled(store.openTaskID == nil)
            Divider()
            Button("Due Today") { store.send(.dueShortcut(.today)) }
                .keyboardShortcut("t")
                .disabled(store.openTaskID == nil)
            Button("Due Tomorrow") { store.send(.dueShortcut(.tomorrow)) }
                .keyboardShortcut("t", modifiers: [.command, .shift])
                .disabled(store.openTaskID == nil)
            Button("Due Next Week") { store.send(.dueShortcut(.nextWeek)) }
                .disabled(store.openTaskID == nil)
            Button("No Due Date") { store.send(.dueShortcut(.none)) }
                .disabled(store.openTaskID == nil)
            Divider()
            // ⌘↑ and ⌘↓ in the list; menu keys would take them from text
            // fields.
            Button("Move Up") { store.send(.listShortcut(.moveUp)) }
                .disabled(!store.state.canMoveCursorRow)
            Button("Move Down") { store.send(.listShortcut(.moveDown)) }
                .disabled(!store.state.canMoveCursorRow)
        }
        CommandMenu("Assistant") {
            Button("Export Plan Prompt") { store.send(.chatCommandChosen(.export)) }
                .keyboardShortcut("e", modifiers: [.command, .shift])
                .disabled(store.state.openChatID == nil)
            Button("Paste Plan Reply") { store.send(.chatCommandChosen(.paste)) }
                .keyboardShortcut("v", modifiers: [.command, .shift])
                .disabled(store.state.openChatID == nil)
        }
        CommandMenu("Sync") {
            Button("Sync Now") { store.send(.syncNowTapped) }
                .keyboardShortcut("r")
                .disabled(store.sync.status == .syncing || store.sync.status == .failed(.notSetUp))
            Button("Review Changes…") { store.send(.syncReviewTapped) }
                .disabled(store.sync.review.isEmpty)
        }
        CommandGroup(replacing: .help) {
            Button("Keyboard Shortcuts") { store.send(.shortcutsTapped) }
                .keyboardShortcut("/", modifiers: [.command, .shift])
            Divider()
            Button("Report a Bug…") { store.send(.reportBugTapped) }
            if store.isRecordingSteps {
                Button("Stop Recording") { store.send(.stopRecordingTapped) }
            } else {
                Button("Record Steps") { store.send(.recordStepsTapped) }
            }
        }
    }
}

extension TaskStatus {
    /// The statuses in the order of their shortcuts, ⌥⌘1 to ⌥⌘5.
    package static let shortcuts: [TaskStatus] = [.todo, .inProgress, .procrastinated, .done, .wontDo]
}

extension Priority {
    /// The priorities in the order of their shortcuts, ⌃0 to ⌃4.
    package static let shortcuts: [Priority] = [.none, .low, .medium, .high, .urgent]
}

/// The menu bar item: quick add, Today's first tasks, the apps connected
/// through the CLI, sync, and the way back to the window. The icon gets a
/// dot while an app is connected.
package struct MenuBarLabel: View {
    package var store: StoreOf<RootFeature>

    package init(store: StoreOf<RootFeature>) {
        self.store = store
    }

    package var body: some View {
        Image(systemName: store.hasConnectedApp ? "checklist.checked" : "checklist")
            .overlay(alignment: .topTrailing) {
                if store.hasConnectedApp {
                    Circle().frame(width: 5, height: 5)
                }
            }
            .accessibilityLabel(store.hasConnectedApp ? "Lifr, an app is connected" : "Lifr")
    }
}

package struct MenuBarContent: View {
    package var store: StoreOf<RootFeature>
    @Environment(\.openWindow) private var openWindow

    /// The main window's id, so the menu bar can open it again once closed.
    package static let windowID = "main"
    /// How many of Today's tasks the menu shows.
    static let todayLimit = 5

    package init(store: StoreOf<RootFeature>) {
        self.store = store
    }

    package var body: some View {
        if store.isLocked {
            Text("Lifr is locked")
            Button(LockView.unlockTitle(store.biometry)) { store.send(.unlockTapped) }
        } else {
            unlocked
        }
        Divider()
        Button("Open Lifr") { showWindow() }
        #if os(macOS)
            SettingsLink { Text("Settings…") }
                .keyboardShortcut(",")
            Button("Quit Lifr") { NSApplication.shared.terminate(nil) }
                .keyboardShortcut("q")
        #endif
    }

    /// Today's first tasks, each with Open and the statuses it can move to.
    /// A status that needs a question, such as closing a task with open
    /// subtasks, asks in the window.
    @ViewBuilder private var today: some View {
        let rows = store.today.list.rows.prefix(Self.todayLimit)
        Section("Today") {
            if rows.isEmpty {
                Text("Nothing due today")
            }
            ForEach(rows) { row in
                Menu(row.title) {
                    Button("Open") {
                        store.send(.reminderOpened(row.id))
                        showWindow()
                    }
                    Divider()
                    ForEach(TaskStatus.shortcuts.filter { $0 != row.status }, id: \.self) { status in
                        Button(status.title) {
                            store.send(.today(.list(.rowAction(row.id, .status(status)))))
                            if store.today.list.prompt != nil { showWindow() }
                        }
                    }
                }
            }
        }
    }

    /// Brings the window forward, or opens it again once closed.
    private func showWindow() {
        #if os(macOS)
            let windows = NSApplication.shared.windows
            if let window = windows.first(where: { $0.identifier?.rawValue.hasPrefix(Self.windowID) == true }) {
                window.makeKeyAndOrderFront(nil)
            } else {
                openWindow(id: Self.windowID)
            }
            NSApplication.shared.activate()
        #endif
    }

    @ViewBuilder private var unlocked: some View {
        Button("New Task") { store.send(.newTaskTapped) }
            .keyboardShortcut("n")
        Divider()
        today
        Divider()
        if store.apps.isEmpty {
            Text("No connected apps")
        } else {
            Section("Connected apps") {
                ForEach(store.apps) { app in
                    Text(app.isConnected ? "● \(app.name)" : app.name)
                }
            }
        }
        Divider()
        Text("Sync: \(SyncStatusText.status(store.sync))")
        Button("Sync Now") { store.send(.syncNowTapped) }
            .disabled(store.sync.status == .syncing || store.sync.status == .failed(.notSetUp))
        Divider()
        Toggle(
            "Local API",
            isOn: Binding(get: { store.isAPIEnabled }, set: { store.send(.apiSwitched($0)) }))
        if let error = store.apiSwitchError {
            Text(error)
        }
    }
}

extension FocusedValues {
    /// The store of the Lifr window in front, which the menus act on.
    @Entry var lifrWindow: StoreOf<RootFeature>?
}
