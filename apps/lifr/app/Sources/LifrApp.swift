import ComposableArchitecture
import LifrCore
import LifrStoreClient
import LifrStoreLive
import LifrToday
import SwiftUI

#if canImport(UIKit)
    import UIKit
#endif

@main
struct LifrApp: App {
    var body: some Scene {
        WindowGroup {
            switch Storage.opened {
            case .success(let store):
                NavigationStack {
                    TodayView(store: Storage.today)
                }
                .task {
                    if ProcessInfo.processInfo.arguments.contains(Storage.sampleDataArgument) {
                        try? await store.insert(.sample(now: Date(), timeZone: .current))
                    }
                    await store.reconcileAfterEachImport(as: Storage.actor)
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
    /// Launch with this argument to add sample records, for trying out sync
    /// before Lifr can create tasks.
    static let sampleDataArgument = "-LifrSampleData"

    static let opened = Result { () throws(LiveStoreError) in try LiveStore(modelContainer: LiveStore.onDisk()) }

    static let actor = Actor.user(device: deviceName)

    static let today = Store(initialState: TodayFeature.State()) {
        TodayFeature()
    } withDependencies: {
        if case .success(let store) = opened {
            $0.store = .live(store, actor: actor)
        }
    }

    private static var deviceName: String {
        #if os(macOS)
            Host.current().localizedName ?? "Mac"
        #else
            UIDevice.current.name
        #endif
    }
}
