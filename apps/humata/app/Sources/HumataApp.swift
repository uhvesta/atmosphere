import ComposableArchitecture
import Foundation
import HumataAppFeature
import HumataDaemonClient
import HumataDaemonLive
import HumataDiagram
import HumataProjects
import HumataSession
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

    /// The message box of the session open in the main window, for a part
    /// of a diagram.
    private var diagramSender: DiagramSender? {
        guard let session = store.session, !session.session.archived else { return nil }
        let title = session.session.name.isEmpty ? "Session" : session.session.name
        return DiagramSender(title: title) { [store] png, note in
            store.send(.session(.presented(.imagePasted(png))))
            let note = note.trimmingCharacters(in: .whitespacesAndNewlines)
            if !note.isEmpty { store.send(.session(.presented(.quoted(note)))) }
        }
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
            AtLeast(width: 820, height: 520) { AppView(store: store) }
                .onOpenURL { store.send(.opened($0)) }
                // A humata:// link goes to this window, not a new one.
                .handlesExternalEvents(preferring: ["*"], allowing: ["*"])
        }
        .defaultSize(width: 1180, height: 760)
        .handlesExternalEvents(matching: ["*"])

        WindowGroup("Diagram", for: DiagramWindowValue.self) { $value in
            if let value {
                DiagramWindow(source: value.source, sender: diagramSender)
            }
        }
        .defaultSize(width: 900, height: 640)

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
            CommandMenu("Session") {
                sessionCommands
            }
        }
    }

    /// What can be done to the open session, so it's found in the menu bar
    /// and has a shortcut; each is greyed out when it can't be done now.
    @ViewBuilder private var sessionCommands: some View {
        let session = store.session
        let live = session.map { !$0.session.archived } ?? false
        Button("Stop the Turn") { send(.abortTapped) }
            .keyboardShortcut(".", modifiers: .command)
            .disabled(!(live && session?.isWorking == true))
        Button("Try Again") { send(.retryTapped) }
            .keyboardShortcut("r", modifiers: [.command, .shift])
            .disabled(
                !live || session?.retryableFailure == nil || session?.isWorking == true
                    || session?.retrying == true)
        Divider()
        Button("Fork With the Code") { send(.forkTapped(before: nil, chatOnly: false)) }
            .keyboardShortcut("f", modifiers: [.command, .option])
            .disabled(session == nil || session?.forking == true)
        Button("Fork the Chat Only") { send(.forkTapped(before: nil, chatOnly: true)) }
            .disabled(session == nil || session?.forking == true)
        Divider()
        Button("Land…") { send(.landTapped) }
            .disabled(session?.ships != true || session?.isWorking == true)
        Button("Update from the Base Branch") { send(.updateFromBaseTapped) }
            .keyboardShortcut("u", modifiers: [.command, .option])
            .disabled(session?.ships != true || session?.updating == true || session?.isWorking == true)
    }

    private func send(_ action: SessionFeature.Action) {
        store.send(.session(.presented(action)))
    }
}
