import ComposableArchitecture
import LifrAvenue
import LifrCore
import LifrSnapshotTesting
import LifrStoreClient
import SnapshotTesting
import SwiftUI
import XCTest

/// Avenues and the Health Avenue with the sample records, as of 2026-11-03
/// 10:00 UTC.
@MainActor
final class AvenueSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var sample = Workspace.sample(now: now, timeZone: .gmt)
    private var healthID: UUID { sample.avenues.values.first { $0.key.rawValue == "HEALTH" }!.id }

    private func avenues(_ workspace: Workspace) -> some View {
        NavigationStack {
            AvenuesView(store: Store(initialState: AvenuesFeature.State(workspace: workspace)) { EmptyReducer() })
        }
        .environment(\.timeZone, .gmt)
    }

    private func health(_ grouping: Grouping) -> AvenueFeature.State {
        AvenueFeature.State(scope: .avenue(healthID), workspace: sample, grouping: grouping, now: now, timeZone: .gmt)
    }

    private func avenue(_ state: AvenueFeature.State) -> some View {
        NavigationStack {
            AvenueView(store: Store(initialState: state) { EmptyReducer() })
        }
        .environment(\.timeZone, .gmt)
    }

    func testDisplayData() {
        assertSnapshot(
            of: AvenuesFeature.State(workspace: sample).avenues,
            as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "avenues")
        for grouping in Grouping.allCases {
            assertSnapshot(
                of: health(grouping).list.sections,
                as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "health-\(grouping.rawValue)")
        }
    }

    func testAccessibilityTree() {
        let size = CGSize(width: 402, height: 874)
        assertSnapshot(of: avenues(sample), as: .accessibilityTree(size: size), named: "avenues")
        assertSnapshot(of: avenue(health(.status)), as: .accessibilityTree(size: size), named: "health")
    }

    func testImages() {
        assertScreenImages(of: avenues(sample), named: "avenues")
        assertScreenImages(of: avenue(health(.status)), named: "health-status")
        assertScreenImages(of: avenue(health(.project)), named: "health-project")
    }

    func testEmptyImage() {
        assertSnapshot(
            of: avenues(Workspace()), as: .image(layout: .device(config: .iPhoneSe)), named: "empty")
    }
}
