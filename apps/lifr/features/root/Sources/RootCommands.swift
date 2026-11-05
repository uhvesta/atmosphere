import ComposableArchitecture
import LifrCore
import LifrSettings
import LifrUI
import SwiftUI

/// The File menu's new items and Sync Now, and the Status menu: a shortcut
/// for each status of the open task.
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
        CommandGroup(after: .saveItem) {
            Button("Sync Now") { store.send(.syncNowTapped) }
                .disabled(store.sync.status == .syncing)
        }
        CommandMenu("Status") {
            ForEach(Array(TaskStatus.shortcuts.enumerated()), id: \.offset) { index, status in
                Button(status.title) { store.send(.statusShortcut(status)) }
                    .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: [.command, .option])
                    .disabled(store.openTaskID == nil)
            }
        }
    }
}

extension TaskStatus {
    /// The statuses in the order of their shortcuts, ⌥⌘1 to ⌥⌘5.
    package static let shortcuts: [TaskStatus] = [.todo, .inProgress, .procrastinated, .done, .wontDo]
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
        Toggle(
            "Local API",
            isOn: Binding(get: { store.isAPIEnabled }, set: { store.send(.apiSwitched($0)) }))
        if let error = store.apiSwitchError {
            Text(error)
        }
    }
}
