import ComposableArchitecture
import FlowKit
import LifrAvenue
import LifrBoard
import LifrCore
import LifrStoreClient
import SnapshotTesting
import SwiftUI
import XCTest

/// The Health Avenue of the sample records as a board, as of 2026-11-03
/// 10:00 UTC. The board is the Mac's, so the images are window sized.
@MainActor
final class BoardSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private var state: BoardFeature.State {
        let workspace = Workspace.sample(now: now, timeZone: .gmt)
        let health = workspace.avenues.values.first { $0.key.description == "HEALTH" }!
        return BoardFeature.State(
            scope: .avenue(health.id), title: health.name, workspace: workspace, now: now, timeZone: .gmt)
    }

    private func screen(_ colorScheme: ColorScheme = .light) -> some View {
        NavigationStack {
            BoardView(store: Store(initialState: state) { EmptyReducer() })
        }
        .environment(\.timeZone, .gmt)
        .environment(\.colorScheme, colorScheme)
    }

    func testDisplayData() {
        assertSnapshot(
            of: state.columns, as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "health")
    }

    func testAccessibilityTree() {
        assertSnapshot(
            of: screen(), as: .accessibilityTree(size: CGSize(width: 1100, height: 600)), named: "health")
    }

    func testImages() {
        let size = CGSize(width: 1100, height: 600)
        assertSnapshot(of: screen(), as: .image(layout: .fixed(width: size.width, height: size.height)), named: "light")
        assertSnapshot(
            of: screen(.dark), as: .image(layout: .fixed(width: size.width, height: size.height)), named: "dark")
        // The largest text size: cards grow taller and the columns scroll.
        assertSnapshot(
            of: screen().dynamicTypeSize(.accessibility5),
            as: .image(layout: .fixed(width: size.width, height: size.height)), named: "largest-text")
    }
}
