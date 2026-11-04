import ComposableArchitecture
import LifrCore
import LifrSnapshotTesting
import LifrStoreClient
import LifrTaskList
import LifrUpcoming
import SnapshotTesting
import SwiftUI
import XCTest

/// Upcoming with the sample records, as of 2026-11-03 10:00 UTC.
@MainActor
final class UpcomingSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func state(_ workspace: Workspace) -> UpcomingFeature.State {
        UpcomingFeature.State(workspace: workspace, now: now, timeZone: .gmt)
    }

    private func screen(_ state: UpcomingFeature.State) -> some View {
        NavigationStack {
            UpcomingView(store: Store(initialState: state) { EmptyReducer() })
        }
        .environment(\.timeZone, .gmt)
    }

    func testDisplayData() {
        assertSnapshot(
            of: state(.sample(now: now, timeZone: .gmt)).list.sections,
            as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "upcoming")
    }

    func testAccessibilityTree() {
        assertSnapshot(
            of: screen(state(.sample(now: now, timeZone: .gmt))),
            as: .accessibilityTree(size: CGSize(width: 402, height: 874)), named: "upcoming")
    }

    func testImages() {
        assertScreenImages(of: screen(state(.sample(now: now, timeZone: .gmt))), named: "upcoming")
    }

    func testEmptyImage() {
        assertSnapshot(
            of: screen(state(Workspace())), as: .image(layout: .device(config: .iPhoneSe)), named: "empty")
    }
}
