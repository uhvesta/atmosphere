import ComposableArchitecture
import Foundation
import KhodrAppFeature
import KhodrDaemonClient
import KhodrDaemonLive
import SwiftUI

/// Khodr: a window for directing coding agents, over khodrd.
@main
struct KhodrApp: App {
    @State private var store = Store(initialState: AppFeature.State()) {
        AppFeature()
    } withDependencies: {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"
        $0.daemon = .live(programVersion: version)
        $0.daemonService = .live(socket: DaemonClient.defaultSocket, programVersion: version)
    }

    var body: some Scene {
        WindowGroup("Khodr") {
            AppView(store: store)
                .frame(minWidth: 820, minHeight: 520)
        }
    }
}
