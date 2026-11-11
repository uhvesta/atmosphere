import ComposableArchitecture
import LifrCore
import LifrLockClient
import LifrSettings
import LifrUI
import SwiftUI

/// The File menu's new items, Find, the View menu's Go to items, the Task
/// menu for the open task, the Sync menu, and Help's shortcuts and bug
/// reports. Items that don't apply are disabled, not hidden.
package struct RootCommands: Commands {
    package var store: StoreOf<RootFeature>

    package init(store: StoreOf<RootFeature>) {
        self.store = store
    }

    package var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("New Task") { store.send(.newTaskTapped) }
                .keyboardShortcut("n")
            Button("New Sub Task") { store.send(.newSubtaskTapped) }
                .keyboardShortcut("n", modifiers: [.command, .shift])
            Button("New Project") { store.send(.newProjectTapped(nil)) }
                .keyboardShortcut("n", modifiers: [.command, .option])
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
        }
        CommandMenu("Task") {
            Menu("Status") {
                ForEach(Array(TaskStatus.shortcuts.enumerated()), id: \.offset) { index, status in
                    Button(status.title) { store.send(.statusShortcut(status)) }
                        .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: [.command, .option])
                }
            }
            .disabled(store.openTaskID == nil)
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
            Button("No Due Date") { store.send(.dueShortcut(.none)) }
                .disabled(store.openTaskID == nil)
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

/// The menu bar item: quick add, and the apps connected through the CLI. The
/// icon gets a dot while one is connected.
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
    }

    @ViewBuilder private var unlocked: some View {
        Button("New Task") { store.send(.newTaskTapped) }
            .keyboardShortcut("n")
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
