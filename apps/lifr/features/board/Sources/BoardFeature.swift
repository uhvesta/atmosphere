import ComposableArchitecture
import Foundation
import LifrAvenue
import LifrCore
import LifrTaskList
import LifrUI

extension TaskQuery {
    /// The Mac's board: one column per status, in the order work moves.
    /// Procrastinated cards say whether they're idle or set by hand.
    package static func board(_ scope: AvenueScope) -> TaskQuery {
        TaskQuery(
            key: "board-\(scope)",
            empty: .init(
                "No tasks yet", systemImage: "rectangle.split.3x1",
                description: String(localized: "Tasks you add here, or move here, show up in their status's column.")),
            adds: NewTaskDefaults(scope: scope.newTaskScope)
        ) { workspace, now, timeZone in
            let tasks = workspace.listedTasks.filter { task in
                guard !task.isSubtask else { return false }
                switch scope {
                case .avenue(let id): return task.avenueID == id
                case .project(let id): return task.projectID == id
                }
            }
            return BoardFeature.columns.map { status in
                let column = tasks.filter { $0.effectiveStatus(now: now, settings: workspace.settings) == status }
                var notes: [UUID: String] = [:]
                for task in column {
                    switch task.procrastination(now: now, settings: workspace.settings) {
                    case .setByHand: notes[task.id] = "set by hand"
                    case .inactive(let days): notes[task.id] = "idle \(days) days"
                    case nil: break
                    }
                }
                // Codes drop the Avenue's key, since every card shares it.
                let rows = column.map { task in
                    var row = TaskRowData(task, in: workspace, now: now, timeZone: timeZone)
                    row.code = task.code.rawValue
                    return row
                }
                return TaskSection(id: status.rawValue, title: status.title, rows: rows, notes: notes)
            }
        }
    }
}

/// One Avenue or Project as a board. Dragging a card to another column
/// sets its status, with the same rules and prompts as the list.
@Reducer
package struct BoardFeature {
    /// The columns, left to right.
    package static let columns: [TaskStatus] = [.todo, .inProgress, .procrastinated, .done, .wontDo]

    @ObservableState
    package struct State: Equatable {
        package var scope: AvenueScope
        package var title: String
        package var list: TaskListFeature.State

        package init(scope: AvenueScope, title: String) {
            self.scope = scope
            self.title = title
            list = TaskListFeature.State(query: .board(scope))
        }

        package init(scope: AvenueScope, title: String, workspace: Workspace, now: Date, timeZone: TimeZone) {
            self.scope = scope
            self.title = title
            list = TaskListFeature.State(query: .board(scope), workspace: workspace, now: now, timeZone: timeZone)
        }

        /// Every column, empty ones too.
        package var columns: [TaskSection] {
            BoardFeature.columns.map { status in
                list.sections.first { $0.id == status.rawValue }
                    ?? TaskSection(id: status.rawValue, title: status.title, rows: [])
            }
        }
    }

    package enum Action {
        case list(TaskListFeature.Action)
        /// A card was dropped on a column.
        case dropped(UUID, on: TaskStatus)
    }

    package init() {}

    package var body: some ReducerOf<Self> {
        Scope(state: \.list, action: \.list) { TaskListFeature() }
        Reduce { state, action in
            switch action {
            case .dropped(let id, let status):
                let column = state.columns.first { $0.rows.contains { $0.id == id } }
                guard column?.id != status.rawValue else { return .none }
                return .send(.list(.rowAction(id, .status(status))))

            case .list:
                return .none
            }
        }
    }
}
