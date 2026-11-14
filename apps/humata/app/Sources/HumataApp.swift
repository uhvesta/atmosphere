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
                Divider()
                projectCommands
                Button("Import Session…") { store.send(.sidebar(.importTapped)) }
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

    /// What the sidebar offers for a project, for the project of the board
    /// that's open or of the session that is; greyed out with neither.
    @ViewBuilder private var projectCommands: some View {
        let id = store.sidebar.commandProject
        let name = id.flatMap { store.sidebar.projects[id: $0]?.name } ?? ""
        Button(name.isEmpty ? "New Session…" : "New Session in \(name)…") {
            if let id { store.send(.sidebar(.newSessionTapped(projectID: id))) }
        }
        .disabled(id == nil)
        Button("Plan With an Agent…") {
            if let id { store.send(.sidebar(.planTapped(projectID: id))) }
        }
        .disabled(id == nil)
        Button("Project Settings…") {
            if let id { store.send(.sidebar(.projectSettingsTapped(id))) }
        }
        .disabled(id == nil)
    }

    /// What can be done to the session of the column the person is in, so
    /// it's found in the menu bar and has a shortcut; each is greyed out
    /// when it can't be done now, and says so when it acts on the side chat.
    @ViewBuilder private var sessionCommands: some View {
        let session = store.commandSession
        let side = store.focusedColumn == .side && store.side != nil
        let live = session.map { !$0.session.archived } ?? false
        // A side chat, in either column, only reads, and so does what's
        // made from it: it gets no copy of the code it could never write.
        let readOnly = session?.session.readOnly == true
        Button(side ? "Stop the Side Chat’s Turn" : "Stop the Turn") { send(.abortTapped) }
            .keyboardShortcut(".", modifiers: .command)
            .disabled(session?.canStop != true)
        Button(side ? "Try Again in the Side Chat" : "Try Again") { send(.retryTapped) }
            .keyboardShortcut("r", modifiers: [.command, .shift])
            .disabled(
                !live || session?.retryableFailure == nil || session?.isWorking == true
                    || session?.retrying == true)
        Divider()
        // The chat's own commands, for those who don't type them. Compact
        // is greyed out unless the session's host has said its agent
        // takes it, and Undo Rewind unless the last rewind can be undone.
        Button(side ? "Rewind the Side Chat…" : "Rewind…") { send(.rewindPickerAsked) }
            .disabled(!live || session?.rewinding == true)
        Button(side ? "Undo the Side Chat’s Rewind" : "Undo Rewind") { send(.undoRewindTapped) }
            .disabled(!live || session?.canUndoRewind != true || session?.rewinding == true)
        Button(side ? "Compact the Side Chat" : "Compact") { send(.compactAsked(focus: "")) }
            .disabled(!live || session?.chatCommands?.compact != true || session?.compactStarting == true)
        Divider()
        if readOnly {
            Button(side ? "Fork the Side Chat, Read-Only" : "Fork, Read-Only") {
                send(.forkTapped(before: nil, chatOnly: true))
            }
            .keyboardShortcut("f", modifiers: [.command, .option])
            .disabled(session?.forking == true)
            Button(side ? "New Read-Only Chat Under the Side Chat" : "New Read-Only Chat Here") {
                send(.childTapped(.empty(ownCopy: false), message: ""))
            }
            .keyboardShortcut("n", modifiers: [.command, .option])
            .disabled(!live || session?.forking == true)
        } else {
            Button(side ? "Fork the Side Chat With the Code" : "Fork With the Code") {
                send(.forkTapped(before: nil, chatOnly: false))
            }
            .keyboardShortcut("f", modifiers: [.command, .option])
            .disabled(session == nil || session?.forking == true)
            Button(side ? "Fork the Side Chat’s Chat Only" : "Fork the Chat Only") {
                send(.forkTapped(before: nil, chatOnly: true))
            }
            .disabled(session == nil || session?.forking == true)
            Button(side ? "New Chat Under the Side Chat" : "New Chat Here") {
                send(.childTapped(.empty(ownCopy: false), message: ""))
            }
            .keyboardShortcut("n", modifiers: [.command, .option])
            .disabled(!live || session?.forking == true)
            Button(side ? "New Chat Under the Side Chat With Its Own Copy" : "New Chat Here With Its Own Copy") {
                send(.childTapped(.empty(ownCopy: true), message: ""))
            }
            .disabled(!live || session?.forking == true)
        }
        // Greyed out unless the session's host has said it holds a side
        // chat to reading only.
        Button(side ? "Side Chat of the Side Chat, in Its Place" : "Side Chat") {
            send(.childTapped(.side, message: ""))
        }
        .keyboardShortcut("s", modifiers: [.command, .option])
        .disabled(!live || session?.forking == true || session?.chatCommands?.side != true)
        Button("Close Side Chat") { store.send(.side(.dismiss)) }
            .keyboardShortcut("w", modifiers: [.command, .option])
            .disabled(store.side == nil)
        Divider()
        // A side chat has no code of its own to land or update.
        Button("Land…") { send(.landTapped) }
            .disabled(session?.ships != true || session?.isWorking == true)
        Button("Update from the Base Branch") { send(.updateFromBaseTapped) }
            .keyboardShortcut("u", modifiers: [.command, .option])
            .disabled(session?.ships != true || session?.updating == true || session?.isWorking == true)
        Divider()
        // The session's views, in the picker's order, and another
        // terminal. Here rather than in the view, so a side chat beside a
        // session doesn't put each key in the window twice.
        let tabs = session?.tabs ?? [.chat, .review, .files, .spec, .terminal]
        ForEach(Array(tabs.enumerated()), id: \.element) { index, tab in
            Button(tab.menuTitle) { send(.tabPicked(tab)) }
                .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: .command)
                .disabled(session == nil)
        }
        if tabs.contains(.terminal) {
            Button("New Terminal") { send(.newTerminalTapped) }
                .keyboardShortcut("t", modifiers: .command)
                .disabled(session == nil)
        }
        Divider()
        sidebarCommands(for: session?.id)
    }

    /// What the sidebar's row offers for the session, as the sidebar has
    /// it; greyed out for one the sidebar doesn't list, and while it's
    /// being moved, archived or deleted.
    @ViewBuilder
    private func sidebarCommands(for id: Session.ID?) -> some View {
        let menu = id.flatMap { store.sidebar.menuState(of: $0) }
        Button("Rename…") {
            if let id { store.send(.sidebar(.renameTapped(id))) }
        }
        .disabled(menu == nil)
        if menu?.archived == true {
            Button("Unarchive") {
                if let id { store.send(.sidebar(.archiveTapped(id, archived: false))) }
            }
            .disabled(menu?.busy != false)
            Button("Delete…") {
                if let id { store.send(.sidebar(.deleteSessionTapped(id))) }
            }
            .disabled(menu?.busy != false)
        } else {
            Button("Move to Host…") {
                if let id { store.send(.sidebar(.moveToHostTapped(id))) }
            }
            .disabled(menu?.held != false)
            Button("Archive") {
                if let id { store.send(.sidebar(.archiveTapped(id, archived: true))) }
            }
            .disabled(menu?.held != false)
        }
    }

    private func send(_ action: SessionFeature.Action) {
        store.send(.sessionCommand(action))
    }
}
