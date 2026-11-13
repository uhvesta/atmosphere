import ComposableArchitecture
import Foundation
import LifrCalendar
import LifrCore
import LifrStoreClient
import SwiftUI

/// The calendar alone over sample tasks, on a fixed day, for UI tests to tap
/// with a finger.
@main
struct CalendarTouchHost: App {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private static let now = Date(timeIntervalSince1970: 1_793_700_000)

    @MainActor private let store = Store(initialState: CalendarFeature.State()) {
        CalendarFeature()
    } withDependencies: {
        $0.store = .inMemory(.sample(now: Self.now, timeZone: .gmt))
        $0.date.now = Self.now
        $0.timeZone = .gmt
    }

    var body: some Scene {
        WindowGroup {
            NavigationStack {
                CalendarView(store: store) {}
            }
            .environment(\.calendar, Self.sundayFirst)
            .task { store.send(.list(.task)) }
        }
    }

    private static var sundayFirst: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.firstWeekday = 1
        calendar.timeZone = .gmt
        return calendar
    }
}
