import ComposableArchitecture
import FlowKit
import LifrCore
import LifrRoot
import SwiftUI
import XCTest

/// The iPad, with the Mac's sidebar, list and task side by side, on an iPad
/// Pro on its side.
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

    /// The App Store's iPad screenshots, each with a task open beside its
    /// list: an iPad Pro 13-inch on its side is
    /// 1376 by 1032 points, which renders at the 2752 by 2064 pixels App
    /// Store Connect takes. Record with --config=record_snapshots and upload
    /// the PNGs from __Snapshots__/PadFlowTests.
    func testAppStoreScreenshots() async throws {
        let (store, flow) = app("ipad-13", platform: .pad, size: CGSize(width: 1376, height: 1032))
        // Today opens with no task beside it, which isn't worth a screenshot.
        await flow.step("open", image: false)
        let water = try task("Water the plants", in: store)
        store.send(.today(.list(.rowTapped(water))))
        await flow.step("today")
        store.send(.selected(.upcoming))
        await flow.step("upcoming")
        store.send(.selected(.calendar))
        await flow.step("calendar")
        flow.finish()
    }
}
