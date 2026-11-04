import ComposableArchitecture
import LifrCore
import LifrRemindersClient
import LifrRemindersLive
import LifrRoot
import LifrSessionClient
import LifrStoreClient
import LifrStoreLive
import SwiftUI

#if canImport(UIKit)
    import UIKit
#endif

@main
struct LifrApp: App {
    var body: some Scene {
        #if os(macOS)
            window
                .commands { RootCommands(store: Storage.root) }
            MenuBarExtra {
                MenuBarContent(store: Storage.root)
            } label: {
                MenuBarLabel(store: Storage.root)
            }
        #else
            window
        #endif
    }

    private var window: some Scene {
        WindowGroup {
            switch Storage.opened {
            case .success(let store):
                RootView(store: Storage.root)
                    .task {
                        if ProcessInfo.processInfo.arguments.contains(Storage.sampleDataArgument) {
                            try? await store.insert(.sample(now: Date(), timeZone: .current))
                        }
                    }
            case .failure(let error):
                ContentUnavailableView(
                    "Lifr can't open its data", systemImage: "exclamationmark.triangle",
                    description: Text(error.description))
            }
        }
    }
}

@MainActor
private enum Storage {
    /// Launch with this argument to add sample records, for trying out the
    /// app without typing them in.
    static let sampleDataArgument = "-LifrSampleData"

    static let opened = Result { () throws(LiveStoreError) in try LiveStore(modelContainer: LiveStore.onDisk()) }

    static let actor = Actor.user(device: deviceName)

    /// One store for every screen, so each change shows everywhere.
    static let root = Store(initialState: RootFeature.State(platform: platform)) {
        RootFeature()
    } withDependencies: {
        $0.store = client
        $0.reminders = .live()
        // Until the socket server, no app can connect.
        $0.session = InMemorySession().client
    }

    static let client: StoreClient =
        if case .success(let store) = opened { .live(store, actor: actor) } else { StoreClient() }

    private static var platform: RootFeature.Platform {
        #if os(macOS)
            .mac
        #else
            .phone
        #endif
    }

    private static var deviceName: String {
        #if os(macOS)
            Host.current().localizedName ?? "Mac"
        #else
            UIDevice.current.name
        #endif
    }
}
