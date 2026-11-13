import ComposableArchitecture
import FlowKit
import LifrCore
import LifrRoot
import LifrStoreClient
import SnapshotTesting
import SwiftUI
import XCTest

/// The iPhone's app on the sample records as of 2026-11-03 10:00 UTC, just
/// after a sync reopened "Review Q4 roadmap".
@MainActor
final class RootSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private var state: RootFeature.State {
        let workspace = Workspace.sample(now: now, timeZone: .gmt)
        let review = workspace.tasks.values.first { $0.title == "Review Q4 roadmap" }!
        var state = RootFeature.State(platform: .phone)
        withDependencies {
            $0.date.now = now
            $0.timeZone = .gmt
        } operation: {
            _ = RootFeature().reduce(
                into: &state, action: .updated(Update(workspace: workspace, fixes: [.reopenedParent(task: review.id)])))
        }
        return state
    }

    private func screen() -> some View {
        RootView(store: Store(initialState: state) { EmptyReducer() })
            .environment(\.timeZone, .gmt)
    }

    func testDisplayData() {
        let state = state
        assertSnapshot(
            of: (
                tab: state.tab,
                today: state.today.list.rows.map(\.title),
                procrastinated: state.today.procrastinatedCount,
                upcoming: state.upcoming.list.rows.map(\.title),
                avenues: state.avenues.avenues.map { ($0.name, $0.openCount) },
                banner: state.syncBanner?.messages ?? []
            ),
            as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "sample")
    }

    func testAccessibilityTree() {
        assertSnapshot(of: screen(), as: .accessibilityTree(size: CGSize(width: 402, height: 874)), named: "today")
    }

    func testImages() {
        // Outside a window, which these tests don't have, the tab bar leaves
        // its tab empty on the SE's size. Each tab's screen has its own SE
        // images in its package.
        assertScreenImages(of: screen(), named: "today", images: [.proMaxLight, .proMaxDark])
    }
}
