import ComposableArchitecture
import LifrCore
import LifrSearch
import LifrSnapshotTesting
import LifrStoreClient
import SnapshotTesting
import SwiftUI
import XCTest

/// Search for "run" in the sample records, as of 2026-11-03 10:00 UTC.
@MainActor
final class SearchSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func state(_ text: String, _ workspace: Workspace) -> SearchFeature.State {
        var filter = SearchFilter()
        filter.text = text
        return SearchFeature.State(filter: filter, workspace: workspace, now: now, timeZone: .gmt)
    }

    private func screen(_ state: SearchFeature.State) -> some View {
        NavigationStack {
            SearchView(store: Store(initialState: state) { EmptyReducer() })
        }
        .environment(\.timeZone, .gmt)
    }

    func testDisplayData() {
        assertSnapshot(
            of: state("run", .sample(now: now, timeZone: .gmt)).list.sections,
            as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "run")
    }

    func testAccessibilityTree() {
        assertSnapshot(
            of: screen(state("run", .sample(now: now, timeZone: .gmt))),
            as: .accessibilityTree(size: CGSize(width: 402, height: 874)), named: "run")
    }

    func testImages() {
        assertScreenImages(of: screen(state("run", .sample(now: now, timeZone: .gmt))), named: "run")
    }

    func testEmptyImage() {
        assertSnapshot(
            of: screen(state("", .sample(now: now, timeZone: .gmt))),
            as: .image(layout: .device(config: .iPhoneSe)), named: "empty")
    }
}
