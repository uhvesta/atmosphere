import ComposableArchitecture
import LifrCore
import LifrQuickAdd
import LifrSnapshotTesting
import LifrStoreClient
import SnapshotTesting
import SwiftUI
import XCTest

/// Quick Add over the sample records, as of 2026-11-03 10:00 UTC: a new
/// task and Project, and Sub task mode after adding its parent.
@MainActor
final class QuickAddSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func state() -> QuickAddFeature.State {
        withDependencies {
            $0.defaultAppStorage = UserDefaults(suiteName: "QuickAddSnapshotTests-\(UUID())")!
        } operation: {
            var state = QuickAddFeature.State(workspace: .sample(now: now, timeZone: .gmt), now: now)
            state.title = "Buy race gels"
            state.due = .tomorrow
            state.priority = .medium
            return state
        }
    }

    private func subtasks() -> QuickAddFeature.State {
        var state = state()
        state.title = ""
        state.parent = .init(id: UUID(0), name: "Race day")
        state.addedSubtasks = ["Pin the bib", "Charge the watch"]
        return state
    }

    private func project() -> QuickAddFeature.State {
        var state = state()
        state.kind = .project
        state.title = "Garden"
        return state
    }

    private func screen(_ state: QuickAddFeature.State) -> some View {
        NavigationStack {
            QuickAddView(store: Store(initialState: state) { BindingReducer() })
        }
        .environment(\.timeZone, .gmt)
    }

    func testDisplayData() {
        assertSnapshot(
            of: (state().avenues.map(\.name), state().avenueProjects.map(\.name)),
            as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "new")
    }

    func testAccessibilityTree() {
        let size = CGSize(width: 402, height: 874)
        assertSnapshot(of: screen(state()), as: .accessibilityTree(size: size), named: "new")
        assertSnapshot(of: screen(subtasks()), as: .accessibilityTree(size: size), named: "subtasks")
        assertSnapshot(of: screen(project()), as: .accessibilityTree(size: size), named: "project")
    }

    func testImages() {
        assertScreenImages(of: screen(state()), named: "new")
        assertScreenImages(of: screen(subtasks()), named: "subtasks")
        assertScreenImages(of: screen(project()), named: "project")
    }

    func testEmptyImage() {
        let empty = withDependencies {
            $0.defaultAppStorage = UserDefaults(suiteName: "QuickAddSnapshotTests-\(UUID())")!
        } operation: {
            QuickAddFeature.State(workspace: Workspace(), now: now)
        }
        assertSnapshot(of: screen(empty), as: .image(layout: .device(config: .iPhoneSe)), named: "empty")
    }
}
