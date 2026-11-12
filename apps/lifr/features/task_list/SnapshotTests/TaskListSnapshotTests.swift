import ComposableArchitecture
import FlowKit
import LifrCore
import LifrStoreClient
import LifrTaskList
import LifrUI
import SnapshotTesting
import SwiftUI
import XCTest

/// The sample's open tasks by status, with a note on Procrastinated rows, as
/// of 2026-11-03 10:00 UTC.
@MainActor
final class TaskListSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private let byStatus = TaskQuery(
        key: "byStatus", empty: .init("Nothing open", systemImage: "checkmark", description: "All done.")
    ) { workspace, now, timeZone in
        [TaskStatus.inProgress, .procrastinated, .todo].map { status in
            let tasks = workspace.listedTasks.filter {
                !$0.isSubtask && $0.effectiveStatus(now: now, settings: workspace.settings) == status
            }
            return TaskSection(
                id: status.rawValue, title: status.title, rows: workspace.rows(tasks, now: now, timeZone: timeZone),
                notes: status == .procrastinated
                    ? Dictionary(uniqueKeysWithValues: tasks.map { ($0.id, "Stalled") }) : [:])
        }
    }

    private func state(_ workspace: Workspace) -> TaskListFeature.State {
        TaskListFeature.State(query: byStatus, workspace: workspace, now: now, timeZone: .gmt)
    }

    private func screen(_ state: TaskListFeature.State) -> some View {
        NavigationStack {
            TaskListView(store: Store(initialState: state) { EmptyReducer() })
                .navigationTitle("Open")
        }
        .environment(\.timeZone, .gmt)
    }

    func testDisplayData() {
        assertSnapshot(
            of: state(.sample(now: now, timeZone: .gmt)).sections, as: Snapshotting<Any, String>.dump.pullback { $0 },
            named: "open")
    }

    func testAccessibilityTree() {
        assertSnapshot(
            of: screen(state(.sample(now: now, timeZone: .gmt))),
            as: .accessibilityTree(size: CGSize(width: 402, height: 874)), named: "open")
    }

    func testImages() {
        assertScreenImages(of: screen(state(.sample(now: now, timeZone: .gmt))), named: "open")
    }

    /// Rows with labels show them as chips, as many as fit.
    func testLabelImages() throws {
        var workspace = Workspace.sample(now: now, timeZone: .gmt)
        let context = Context(actor: .user(device: "iPhone", surface: .iPhone), now: now)
        let names = ["Errands", "Outdoors", "Quick", "Weekend", "Waiting on someone"]
        let colors = ["orange", "green", "blue", "purple", "gray"]
        let labels = zip(names, colors).map { workspace.createLabel(name: $0, color: $1, in: context) }
        let open = workspace.tasks.values.filter { $0.deletedAt == nil && $0.isOpen && !$0.isSubtask }
            .sorted { $0.title < $1.title }
        for (index, task) in open.prefix(3).enumerated() {
            let picked = index == 0 ? labels : Array(labels.prefix(index))
            _ = try workspace.edit(task.id, Workspace.TaskEdit(labelIDs: Set(picked.map(\.id))), in: context)
        }
        assertScreenImages(of: screen(state(workspace)), named: "labels")
    }

    func testEmptyImage() {
        assertSnapshot(
            of: screen(state(Workspace())), as: .image(layout: .device(config: .iPhoneSe)), named: "empty")
    }
}
