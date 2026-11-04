import ComposableArchitecture
import LifrCore
import LifrSnapshotTesting
import LifrStoreClient
import LifrToday
import LifrUI
import SnapshotTesting
import SwiftUI
import XCTest

/// Today with the sample records, as of 2026-11-03 10:00 UTC.
@MainActor
final class TodaySnapshotTests: XCTestCase {
    private struct Display {
        var overdue: [TaskRowData]
        var dueToday: [TaskRowData]
    }

    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private var state: TodayFeature.State {
        TodayFeature.State(workspace: .sample(now: now, timeZone: .gmt), now: now, timeZone: .gmt)
    }

    private func screen(_ state: TodayFeature.State) -> some View {
        NavigationStack {
            TodayView(store: Store(initialState: state) { EmptyReducer() })
        }
        .environment(\.timeZone, .gmt)
    }

    func testDisplayData() {
        let state = state
        assertSnapshot(
            of: Display(overdue: state.overdue, dueToday: state.dueToday),
            as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "today")
    }

    func testAccessibilityTree() {
        assertSnapshot(of: screen(state), as: .accessibilityTree(size: CGSize(width: 402, height: 874)), named: "today")
    }

    func testImages() {
        assertScreenImages(of: screen(state), named: "today")
    }

    func testEmptyImage() {
        let empty = TodayFeature.State(workspace: Workspace(), now: now, timeZone: .gmt)
        assertSnapshot(
            of: screen(empty), as: .image(layout: .device(config: .iPhoneSe)),
            named: "empty")
    }
}
