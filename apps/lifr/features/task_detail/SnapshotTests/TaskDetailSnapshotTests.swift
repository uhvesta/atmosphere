import ComposableArchitecture
import LifrCore
import LifrSnapshotTesting
import LifrStoreClient
import LifrTaskDetail
import LifrUI
import SnapshotTesting
import SwiftUI
import XCTest

/// The sample's "Review Q4 roadmap", which has Sub tasks, as of 2026-11-03
/// 10:00 UTC.
@MainActor
final class TaskDetailSnapshotTests: XCTestCase {
    private struct Display {
        var row: TaskRowData
        var details: String
        var project: String?
        var subtasks: [TaskRowData]
    }

    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private var state: TaskDetailFeature.State {
        let workspace = Workspace.sample(now: now, timeZone: .gmt)
        let review = workspace.tasks.values.first { $0.title == "Review Q4 roadmap" }!
        return TaskDetailFeature.State(id: review.id, workspace: workspace, now: now, timeZone: .gmt)!
    }

    private func screen() -> some View {
        NavigationStack {
            TaskDetailView(store: Store(initialState: state) { EmptyReducer() })
        }
    }

    func testDisplayData() {
        let state = state
        assertSnapshot(
            of: Display(row: state.row, details: state.details, project: state.project, subtasks: state.subtasks),
            as: Snapshotting<Any, String>.dump.pullback { $0 }, named: "review")
    }

    func testAccessibilityTree() {
        assertSnapshot(of: screen(), as: .accessibilityTree(size: CGSize(width: 402, height: 874)), named: "review")
    }

    func testImages() {
        assertScreenImages(of: screen(), named: "review")
    }
}
