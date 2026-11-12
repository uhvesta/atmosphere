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
    @State private var store = Store(initialState: AppFeature.State()) {
        AppFeature()
    } withDependencies: {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"
        $0.daemon = .live(programVersion: version)
        $0.daemonService = .live(socket: DaemonClient.defaultSocket, programVersion: version)
        $0.notifications = .live()
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
        .commands {
            // One window over one store; ⌘N starts a chat in it instead.
            CommandGroup(replacing: .newItem) {}
        }
    }
}
