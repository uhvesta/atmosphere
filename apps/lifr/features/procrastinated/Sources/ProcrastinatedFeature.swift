import ComposableArchitecture
import Foundation
import LifrCore
import LifrTaskList
import LifrUI

extension TaskQuery {
    /// Stalled tasks, Sub tasks included, oldest activity first. Each row
    /// says why it's here. Swipe right restarts; swipe left reschedules or
    /// drops.
    package static let procrastinated = TaskQuery(
        key: "procrastinated",
        empty: .init(
            "Nothing stalled", systemImage: "tortoise",
            description: "Tasks set to Procrastinated, or In Progress with no activity for a while, show up here."),
        trailingActions: [.reschedule, .status(.wontDo)]
    ) { workspace, now, timeZone in
        let stalled = workspace.listedTasks
            .compactMap { task in task.procrastination(now: now, settings: workspace.settings).map { (task, $0) } }
            .sorted { ($0.0.lastActivityAt, $0.0.id.uuidString) < ($1.0.lastActivityAt, $1.0.id.uuidString) }
        let notes = Dictionary(uniqueKeysWithValues: stalled.map { task, reason in (task.id, reason.note) })
        return [
            TaskSection(
                id: "stalled", title: "Stalled", rows: workspace.rows(stalled.map(\.0), now: now, timeZone: timeZone),
                notes: notes)
        ]
    }

    /// How many tasks Procrastinated shows.
    package static func procrastinatedCount(in workspace: Workspace, now: Date) -> Int {
        workspace.listedTasks.count { $0.procrastination(now: now, settings: workspace.settings) != nil }
    }
}

extension Procrastination {
    /// Why the task is on the list.
    package var note: String {
        switch self {
        case .setByHand: "Set to Procrastinated by hand. It stays until you change its status."
        case .inactive(let days): "In Progress with no activity for \(days) days."
        }
    }
}

/// Procrastinated: everything stalled, to restart, reschedule or drop.
@Reducer
package struct ProcrastinatedFeature {
    @ObservableState
    package struct State: Equatable {
        package var list: TaskListFeature.State

        package init() {
            list = TaskListFeature.State(query: .procrastinated)
        }

        package init(workspace: Workspace, now: Date, timeZone: TimeZone) {
            list = TaskListFeature.State(query: .procrastinated, workspace: workspace, now: now, timeZone: timeZone)
        }
    }

    package enum Action {
        case list(TaskListFeature.Action)
    }

    package init() {}

    package var body: some ReducerOf<Self> {
        Scope(state: \.list, action: \.list) { TaskListFeature() }
    }
}
