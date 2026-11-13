import ComposableArchitecture
import FlowKit
import LifrCore
import LifrRoot
import SwiftUI
import XCTest

/// The iPad, with the Mac's sidebar, list and task side by side, on an iPad
/// Pro 11-inch on its side.
@MainActor
final class PadFlowTests: PhoneFlowTestCase {
    private let landscape = CGSize(width: 1210, height: 834)

    /// Today in the sidebar's layout, a task opened beside it, then the
    /// month with each day's tasks as chips.
    func testSidebarLayout() async throws {
        let (store, flow) = app("ipad", platform: .pad, size: landscape)
        await flow.step("today")
        let water = try task("Water the plants", in: store)
        store.send(.today(.list(.rowTapped(water))))
        await flow.step("task")
        store.send(.selected(.calendar))
        await flow.step("calendar")
        flow.finish()
    }
}
