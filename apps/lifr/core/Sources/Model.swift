import Foundation

/// Who made a change, recorded on comments and history.
package struct Actor: Hashable, Codable, Sendable {
    package enum Kind: String, Hashable, Codable, Sendable {
        case user, agent, system
    }

    package var kind: Kind
    /// The device the change was made on.
    package var device: String
    /// For agents, the approved client's name, such as "Claude Code".
    package var client: String?
    /// Where the change came from.
    package var surface: Surface

    /// Where a change was made.
    package enum Surface: String, Hashable, Codable, Sendable, CaseIterable {
        case iPhone = "iphone"
        case iPad = "ipad"
        case mac
        case cli
        case mcp
        /// Apple Intelligence, in a conversation.
        case assistant
    }

    package static func user(device: String, surface: Surface) -> Actor {
        Actor(kind: .user, device: device, client: nil, surface: surface)
    }

    package static func agent(_ client: String, device: String, surface: Surface) -> Actor {
        Actor(kind: .agent, device: device, client: client, surface: surface)
    }

    /// Changes the rules make on their own, such as reopening a parent.
    package static func system(device: String, surface: Surface) -> Actor {
        Actor(kind: .system, device: device, client: nil, surface: surface)
    }

    /// The on-device assistant, acting in a conversation.
    package static func assistant(device: String) -> Actor {
        .agent("Apple Intelligence", device: device, surface: .assistant)
    }
}

/// How much an agent may see of an Avenue.
package enum AgentAccess: String, Hashable, Codable, Sendable, CaseIterable {
    case full, readOnly, hidden
}

/// An ongoing area of life. Avenues never end; they are only archived.
package struct Avenue: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    package var key: AvenueKey
    package var name: String
    package var color: String
    /// An SF Symbol name.
    package var symbol: String
    package var sortOrder: String
    package var agentAccess: AgentAccess
    package var createdAt: Date
    package var archivedAt: Date?

    package init(
        id: UUID, key: AvenueKey, name: String, color: String, symbol: String, sortOrder: String,
        agentAccess: AgentAccess, createdAt: Date, archivedAt: Date?
    ) {
        self.id = id
        self.key = key
        self.name = name
        self.color = color
        self.symbol = symbol
        self.sortOrder = sortOrder
        self.agentAccess = agentAccess
        self.createdAt = createdAt
        self.archivedAt = archivedAt
    }
}

package enum ProjectStatus: String, Hashable, Codable, Sendable, CaseIterable {
    case active, completed, archived
}

/// A goal with a finish line, inside one Avenue.
package struct Project: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    package var avenueID: UUID
    package var name: String
    package var details: String
    package var status: ProjectStatus
    package var dueDate: CalendarDate?
    package var sortOrder: String
    package var createdAt: Date
    package var completedAt: Date?

    package init(
        id: UUID, avenueID: UUID, name: String, details: String, status: ProjectStatus, dueDate: CalendarDate?,
        sortOrder: String, createdAt: Date, completedAt: Date?
    ) {
        self.id = id
        self.avenueID = avenueID
        self.name = name
        self.details = details
        self.status = status
        self.dueDate = dueDate
        self.sortOrder = sortOrder
        self.createdAt = createdAt
        self.completedAt = completedAt
    }
}

/// A task's stored status. Procrastinated is also derived; see
/// `LifrTask.procrastination(now:settings:)`.
package enum TaskStatus: String, Hashable, Codable, Sendable, CaseIterable {
    case todo, inProgress, procrastinated, done, wontDo

    /// Done and Won't Do both count as closed.
    package var isClosed: Bool { self == .done || self == .wontDo }
}

package enum Priority: String, Hashable, Codable, Sendable, CaseIterable, Comparable {
    case none, low, medium, high, urgent

    var rank: Int { Self.allCases.firstIndex(of: self)! }

    package static func < (lhs: Self, rhs: Self) -> Bool { lhs.rank < rhs.rank }
}

/// A Task, or a Sub task when `parentID` is set. Named `LifrTask` so it
/// doesn't shadow Swift's `Task`.
package struct LifrTask: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    package var code: TaskCode
    package var title: String
    /// Markdown.
    package var details: String
    package var status: TaskStatus
    package var priority: Priority
    package var startDate: CalendarDate?
    package var dueDate: CalendarDate?
    package var dueTime: TimeOfDay?
    package var avenueID: UUID
    package var projectID: UUID?
    package var parentID: UUID?
    package var labelIDs: Set<UUID>
    package var sortOrder: String
    package var createdAt: Date
    package var updatedAt: Date
    /// Any edit, comment or status change. Drives automatic Procrastinated.
    package var lastActivityAt: Date
    /// Set when the status becomes Done or Won't Do.
    package var closedAt: Date?
    /// Soft delete. Purged after `Workspace.trashRetention`.
    package var deletedAt: Date?

    package var isSubtask: Bool { parentID != nil }
    package var isOpen: Bool { !status.isClosed }

    package init(
        id: UUID, code: TaskCode, title: String, details: String, status: TaskStatus, priority: Priority,
        startDate: CalendarDate?, dueDate: CalendarDate?, dueTime: TimeOfDay?, avenueID: UUID, projectID: UUID?,
        parentID: UUID?, labelIDs: Set<UUID>, sortOrder: String, createdAt: Date, updatedAt: Date,
        lastActivityAt: Date, closedAt: Date?, deletedAt: Date?
    ) {
        self.id = id
        self.code = code
        self.title = title
        self.details = details
        self.status = status
        self.priority = priority
        self.startDate = startDate
        self.dueDate = dueDate
        self.dueTime = dueTime
        self.avenueID = avenueID
        self.projectID = projectID
        self.parentID = parentID
        self.labelIDs = labelIDs
        self.sortOrder = sortOrder
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.lastActivityAt = lastActivityAt
        self.closedAt = closedAt
        self.deletedAt = deletedAt
    }
}

/// A free-form tag shared by every Avenue.
package struct Label: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    package var name: String
    package var color: String
    package var createdAt: Date

    package init(id: UUID, name: String, color: String, createdAt: Date) {
        self.id = id
        self.name = name
        self.color = color
        self.createdAt = createdAt
    }
}

package struct Comment: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    package var taskID: UUID
    /// Markdown.
    package var body: String
    package var author: Actor
    package var createdAt: Date
    package var editedAt: Date?
    /// Deleted comments stay as a "comment deleted" marker.
    package var deletedAt: Date?

    package init(
        id: UUID, taskID: UUID, body: String, author: Actor, createdAt: Date, editedAt: Date?, deletedAt: Date?
    ) {
        self.id = id
        self.taskID = taskID
        self.body = body
        self.author = author
        self.createdAt = createdAt
        self.editedAt = editedAt
        self.deletedAt = deletedAt
    }
}

/// A local notification. Each device schedules its own from these records.
package struct Reminder: Identifiable, Hashable, Codable, Sendable {
    package enum Trigger: Hashable, Codable, Sendable {
        case at(Date)
        /// This long before the due date (and time, if set).
        case beforeDue(seconds: Int)
    }

    package var id: UUID
    package var taskID: UUID
    package var trigger: Trigger

    package init(id: UUID, taskID: UUID, trigger: Trigger) {
        self.id = id
        self.taskID = taskID
        self.trigger = trigger
    }
}

extension Reminder {
    /// When the reminder fires in `timeZone`: its set time, or this long
    /// before the due date and time (9:00 when there's no time). `nil` when
    /// it's before the due date and the task has none.
    package func fireDate(for task: LifrTask, in timeZone: TimeZone) -> Date? {
        switch trigger {
        case .at(let date):
            return date
        case .beforeDue(let seconds):
            guard let due = task.dueDate else { return nil }
            let (hour, minute) = task.dueTime.map { ($0.hour, $0.minute) } ?? (9, 0)
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = timeZone
            let components = DateComponents(
                year: due.year, month: due.month, day: due.day, hour: hour, minute: minute)
            return calendar.date(from: components).map { $0 - TimeInterval(seconds) }
        }
    }
}

/// One synced record of preferences.
package struct Settings: Hashable, Codable, Sendable {
    /// In Progress tasks with no activity for longer than this are
    /// Procrastinated.
    package var procrastinatedAfterDays: Int = 7

    package init(procrastinatedAfterDays: Int = 7) {
        self.procrastinatedAfterDays = procrastinatedAfterDays
    }
}

/// One change to a task. Never edited once written.
package struct HistoryEvent: Identifiable, Hashable, Codable, Sendable {
    package enum Kind: String, Hashable, Codable, Sendable {
        case created, fieldChanged, subtaskAdded, subtaskRemoved
        case commentAdded, commentEdited, commentDeleted
        case movedToTrash, restored
    }

    package enum Field: String, Hashable, Codable, Sendable {
        case title, details, status, priority, startDate, dueDate, dueTime
        case avenue, project, parent, labels, code
    }

    /// Why the rules changed something on their own (`actor.kind == .system`)
    /// or on someone's behalf.
    package enum Reason: String, Hashable, Codable, Sendable {
        /// A Sub task was added, moved in or reopened under a closed parent.
        case subtaskOpenedUnderClosedParent
        /// The parent was marked Won't Do with `cascade: true`.
        case closedWithParent
        /// The parent was moved, so its Sub tasks moved with it.
        case movedWithParent
        case undo
        case reconciledClosedParent
        case reconciledDuplicateCode
        case reconciledNestedSubtask
        case reconciledParentPlacement
    }

    package var id: UUID
    package var taskID: UUID
    package var actor: Actor
    package var kind: Kind
    package var field: Field?
    package var oldValue: String?
    package var newValue: String?
    package var reason: Reason?
    package var createdAt: Date

    package init(
        id: UUID, taskID: UUID, actor: Actor, kind: Kind, field: Field?, oldValue: String?, newValue: String?,
        reason: Reason?, createdAt: Date
    ) {
        self.id = id
        self.taskID = taskID
        self.actor = actor
        self.kind = kind
        self.field = field
        self.oldValue = oldValue
        self.newValue = newValue
        self.reason = reason
        self.createdAt = createdAt
    }
}
