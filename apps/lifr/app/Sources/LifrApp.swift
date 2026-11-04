import ComposableArchitecture
import LifrCore
import LifrStoreClient
import LifrToday
import SwiftUI

@main
struct LifrApp: App {
    /// Sample records in memory until the app stores its own.
    @MainActor static let store = Store(initialState: TodayFeature.State()) {
        TodayFeature()
    } withDependencies: {
        $0.store = .inMemory(.sample(now: Date(), timeZone: .current))
    }

    var body: some Scene {
        WindowGroup {
            NavigationStack {
                TodayView(store: Self.store)
            }
        }
    }
}
