import Foundation
import LifrCore
import LifrTaskList
import LifrUI

/// How an Avenue or Project view groups its tasks.
package enum Grouping: String, CaseIterable, Hashable, Sendable {
    /// Every task, one section per status, open ones first.
    case status
    /// Open tasks, one section per Project, then those in none.
    case project

    package var title: String {
        switch self {
        case .status: String(localized: "Status")
        case .project: String(localized: "Project")
        }
    }
}

/// What an Avenue view shows: a whole Avenue, or one of its Projects.
package enum AvenueScope: Hashable, Codable, Sendable {
    case avenue(UUID)
    case project(UUID)

    package var newTaskScope: NewTaskDefaults.Scope {
        switch self {
        case .avenue(let id): .avenue(id)
        case .project(let id): .project(id)
        }
    }
}

extension TaskQuery {
    /// The tasks in `scope`, Sub tasks inside their parents. Codes drop the
    /// Avenue's key, since every row shares it.
    package static func scope(_ scope: AvenueScope, by grouping: Grouping) -> TaskQuery {
        TaskQuery(
            key: "\(scope)-\(grouping.rawValue)", listID: "\(scope)",
            empty: .init(
                "No tasks yet", systemImage: "tray",
                description: String(localized: "Tasks you add here, or move here, show up in this list.")),
            adds: NewTaskDefaults(scope: scope.newTaskScope), reorders: true
        ) { workspace, now, timeZone in
            let tasks = workspace.listedTasks.filter { task in
                guard !task.isSubtask else { return false }
                switch scope {
                case .avenue(let id): return task.avenueID == id
                case .project(let id): return task.projectID == id
                }
            }
            func rows(_ tasks: [LifrTask]) -> [TaskRowData] {
                tasks.map { task in
                    var row = TaskRowData(task, in: workspace, now: now, timeZone: timeZone)
                    row.code = task.code.rawValue
                    return row
                }
            }
            switch grouping {
            case .status:
                let order: [TaskStatus] = [.inProgress, .procrastinated, .todo, .done, .wontDo]
                return order.map { status in
                    TaskSection(
                        id: status.rawValue, title: status.title,
                        rows: rows(
                            tasks.filter { $0.effectiveStatus(now: now, settings: workspace.settings) == status }))
                }
            case .project:
                let open = tasks.filter(\.isOpen)
                let projects = workspace.projects.values
                    .filter { project in open.contains { $0.projectID == project.id } }
                    .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                return projects.map { project in
                    TaskSection(
                        id: project.id.uuidString, title: project.name,
                        rows: rows(open.filter { $0.projectID == project.id }))
                } + [
                    TaskSection(
                        id: "none", title: String(localized: "No Project"),
                        rows: rows(open.filter { $0.projectID == nil }))
                ]
            }
        }
    }
}
