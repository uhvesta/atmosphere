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

    package static func user(device: String) -> Actor {
        Actor(kind: .user, device: device, client: nil)
    }

    package static func agent(_ client: String, device: String) -> Actor {
        Actor(kind: .agent, device: device, client: client)
    }

    /// Changes the rules make on their own, such as reopening a parent.
    package static func system(device: String) -> Actor {
        Actor(kind: .system, device: device, client: nil)
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
}

/// A free-form tag shared by every Avenue.
package struct Label: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    package var name: String
    package var color: String
    package var createdAt: Date
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
}
