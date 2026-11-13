import ComposableArchitecture
import Foundation
import HumataAppFeature
import HumataDaemonClient
import HumataDaemonLive
import HumataProjects
import SwiftUI

/// Humata: a window for directing coding agents, over humatad.
@main
struct HumataApp: App {
    @State private var store = Store(initialState: Self.initialState) {
        AppFeature()
    } withDependencies: {
        let version = Self.version
        $0.daemon = .live(programVersion: version)
        $0.daemonService = .live(socket: DaemonClient.defaultSocket, programVersion: version)
        $0.notifications = .live()
        $0.modelServer = .live
        $0.modelFiles = .live
        $0.appActivity = .live()
    }

    private static var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"
    }

    private static var initialState: AppFeature.State {
        var state = AppFeature.State()
        // The build of humatad this app installs on hosts.
        state.settings.bundledVersion = version
        return state
    }

    var body: some Scene {
        WindowGroup("Humata") {
            AppView(store: store)
                .frame(minWidth: 820, minHeight: 520)
                .onOpenURL { store.send(.opened($0)) }
                // A humata:// link goes to this window, not a new one.
                .handlesExternalEvents(preferring: ["*"], allowing: ["*"])
        }
        .handlesExternalEvents(matching: ["*"])

        Settings {
            SettingsView(store: store)
        }
        .commands {
            // One window over one store, so New makes a chat or a project in
            // it rather than another window.
            CommandGroup(replacing: .newItem) {
                Button("New Chat") { store.send(.sidebar(.newChatTapped)) }
                    .keyboardShortcut("n", modifiers: .command)
                Button("Add Project…") { store.send(.sidebar(.addProjectTapped)) }
                    .keyboardShortcut("n", modifiers: [.command, .shift])
            }
            CommandMenu("Go") {
                Button("Inbox") { store.send(.sidebar(.selected(.inbox))) }
                    .keyboardShortcut("0", modifiers: .command)
                Button("Next Waiting Session") { store.send(.sidebar(.nextWaitingTapped)) }
                    .keyboardShortcut("]", modifiers: .command)
                Divider()
                Button("Search Sessions") { store.send(.sidebar(.selected(.search))) }
                    .keyboardShortcut("f", modifiers: [.command, .shift])
                Button("Hosts") { store.send(.sidebar(.selected(.hosts))) }
            }
        }
    }
}
