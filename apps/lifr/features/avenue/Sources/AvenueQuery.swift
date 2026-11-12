import Foundation
import LifrCore
import LifrTaskList
import LifrUI

/// How an Avenue or Project view groups its tasks. Every grouping but
/// Status shows open tasks only.
package enum Grouping: String, CaseIterable, Hashable, Sendable {
    /// Every task, one section per status, open ones first.
    case status
    /// One section per Project, then those in none.
    case project
    /// Most urgent first, then those with no priority.
    case priority
    /// Overdue, today, the next 7 days, later, then those with no due date.
    case due
    /// One section per label, by name: a task with several goes under the
    /// first. Then those with none.
    case label
    /// One section of every open task.
    case none

    package var title: String {
        switch self {
        case .status: String(localized: "Status")
        case .project: String(localized: "Project")
        case .priority: String(localized: "Priority")
        case .due: String(localized: "Due Date")
        case .label: String(localized: "Label")
        case .none: String(localized: "None")
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
    package static func scope(_ scope: AvenueScope, by grouping: Grouping, showsCompleted: Bool = true) -> TaskQuery {
        TaskQuery(
            key: "\(scope)-\(grouping.rawValue)-\(showsCompleted)", listID: "\(scope)",
            empty: .init(
                "No tasks yet", systemImage: "tray",
                description: String(localized: "Tasks you add here, or move here, show up in this list.")),
            adds: NewTaskDefaults(scope: scope.newTaskScope), reorders: true
        ) { workspace, now, timeZone in
            let all = workspace.listedTasks.filter { task in
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
            // Open tasks that can't start yet go in a section of their own.
            let today = CalendarDate(now, in: timeZone)
            let waiting = all.filter { task in task.isOpen && task.startDate.map { $0 > today } ?? false }
            let later = TaskSection(id: "later", title: String(localized: "Later"), rows: rows(waiting))
            // Only grouping by status shows closed tasks, unless Show
            // Completed is off.
            let tasks = all.filter { !waiting.contains($0) && (showsCompleted || $0.isOpen) }
            let open = tasks.filter(\.isOpen)
            switch grouping {
            case .status:
                let order: [TaskStatus] = [.inProgress, .procrastinated, .todo, .done, .wontDo]
                return order.map { status in
                    TaskSection(
                        id: status.rawValue, title: status.title,
                        rows: rows(
                            tasks.filter { $0.effectiveStatus(now: now, settings: workspace.settings) == status }))
                } + [later]
            case .project:
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
                        rows: rows(open.filter { $0.projectID == nil })),
                    later,
                ]
            case .priority:
                return Priority.allCases.reversed().map { priority in
                    TaskSection(
                        id: priority.rawValue, title: priority.title ?? String(localized: "No Priority"),
                        rows: rows(open.filter { $0.priority == priority }))
                } + [later]
            case .due:
                return DueGroup.allCases.map { group in
                    TaskSection(
                        id: group.rawValue, title: group.title,
                        rows: rows(open.filter { DueGroup($0.dueDate, today: today) == group }))
                } + [later]
            case .label:
                let names = workspace.labels.values.sorted {
                    ($0.name.localizedLowercase, $0.id.uuidString) < ($1.name.localizedLowercase, $1.id.uuidString)
                }
                func first(_ task: LifrTask) -> UUID? { names.first { task.labelIDs.contains($0.id) }?.id }
                return names.map { label in
                    TaskSection(
                        id: label.id.uuidString, title: "#\(label.name)",
                        rows: rows(open.filter { first($0) == label.id }))
                } + [
                    TaskSection(
                        id: "none", title: String(localized: "No Label"), rows: rows(open.filter { first($0) == nil })),
                    later,
                ]
            case .none:
                return [TaskSection(id: "open", title: String(localized: "Open"), rows: rows(open)), later]
            }
        }
    }
}

/// Where a due date falls, for grouping by it.
enum DueGroup: String, CaseIterable {
    case overdue, today, week, later, none

    init(_ due: CalendarDate?, today: CalendarDate) {
        guard let due else {
            self = .none
            return
        }
        self =
            due < today ? .overdue : due == today ? .today : due <= today.adding(days: 7) ? .week : .later
    }

    var title: String {
        switch self {
        case .overdue: String(localized: "Overdue")
        case .today: String(localized: "Today")
        case .week: String(localized: "Next 7 Days")
        case .later: String(localized: "Later")
        case .none: String(localized: "No Due Date")
        }
    }
}
