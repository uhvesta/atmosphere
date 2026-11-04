import Foundation
import LifrCore

// Params and results for each `Method`. Tasks are named by code (HEALTH-7K2Q
// or 7K2Q), Avenues by key and Projects by id, so agents can use what they
// see in a task list.

package enum Hello {
    package struct Params: Hashable, Codable, Sendable {
        package var protocolVersion: Int
        /// `lifr`'s version, for the access log.
        package var client: String

        package init(protocolVersion: Int = LifrProtocol.version, client: String) {
            self.protocolVersion = protocolVersion
            self.client = client
        }
    }

    package struct Result: Hashable, Codable, Sendable {
        package var protocolVersion: Int
        /// Empty until the person approves this client.
        package var scopes: [Scope]
        package var expiresAt: Date?
    }
}

package enum Unlock {
    /// A terminal session ends after this long without a request.
    package static let idleTimeout: TimeInterval = 15 * 60

    package struct Result: Hashable, Codable, Sendable {
        package var expiresAt: Date
    }
}

package struct AvenueSummary: Hashable, Codable, Sendable {
    package var key: String
    package var name: String
    package var agentAccess: AgentAccess
    package var needsRename: Bool
}

package enum ListProjects {
    package struct Params: Hashable, Codable, Sendable {
        /// Only this Avenue's Projects.
        package var avenue: String?
        package var includeClosed: Bool?

        package init(avenue: String? = nil, includeClosed: Bool? = nil) {
            self.avenue = avenue
            self.includeClosed = includeClosed
        }
    }
}

package struct ProjectSummary: Hashable, Codable, Sendable {
    package var id: UUID
    package var avenue: String
    package var name: String
    package var status: ProjectStatus
    package var dueDate: CalendarDate?
}

package enum SearchTasks {
    package struct Params: Hashable, Codable, Sendable {
        /// Words to find in titles and details, ignoring case.
        package var query: String?
        package var avenue: String?
        package var project: UUID?
        /// Effective statuses, so `procrastinated` includes derived ones.
        package var statuses: [TaskStatus]?
        /// Closed tasks are left out unless asked for.
        package var includeClosed: Bool?
        package var limit: Int?

        package init(
            query: String? = nil, avenue: String? = nil, project: UUID? = nil,
            statuses: [TaskStatus]? = nil, includeClosed: Bool? = nil, limit: Int? = nil
        ) {
            self.query = query
            self.avenue = avenue
            self.project = project
            self.statuses = statuses
            self.includeClosed = includeClosed
            self.limit = limit
        }
    }

    package static let defaultLimit = 50
    package static let maxLimit = 500
}

package struct TaskSummary: Hashable, Codable, Sendable {
    package var code: String
    package var title: String
    /// The stored status, or `procrastinated` when it's derived.
    package var status: TaskStatus
    /// Set when the status is Procrastinated: "setByHand" or the idle days.
    package var procrastination: String?
    package var priority: Priority
    package var avenue: String
    package var project: UUID?
    package var parent: String?
    package var startDate: CalendarDate?
    package var dueDate: CalendarDate?
    package var dueTime: TimeOfDay?
    package var labels: [String]
    package var openSubtasks: Int
}

package enum TaskRef {
    package struct Params: Hashable, Codable, Sendable {
        package var task: String

        package init(task: String) { self.task = task }
    }
}

package struct TaskDetail: Hashable, Codable, Sendable {
    package var task: TaskSummary
    package var details: String
    package var createdAt: Date
    package var closedAt: Date?
    package var subtasks: [TaskSummary]
    /// Comments and history, oldest first, as the detail screen shows them.
    package var timeline: [TimelineEntry]
}

package struct ActorSummary: Hashable, Codable, Sendable {
    package var kind: Actor.Kind
    package var device: String
    package var client: String?
}

package struct TimelineEntry: Hashable, Codable, Sendable {
    package enum Kind: String, Hashable, Codable, Sendable { case comment, history }

    package var kind: Kind
    package var at: Date
    package var actor: ActorSummary
    /// For comments: the Markdown body, or nil once deleted.
    package var body: String?
    package var commentID: UUID?
    /// For history: what happened.
    package var event: HistoryEvent.Kind?
    package var field: HistoryEvent.Field?
    package var oldValue: String?
    package var newValue: String?
    package var reason: HistoryEvent.Reason?
}

package enum CreateTask {
    package struct Params: Hashable, Codable, Sendable {
        package var title: String
        /// Required unless `parent` is set; a Sub task takes its parent's.
        package var avenue: String?
        package var project: UUID?
        package var parent: String?
        package var details: String?
        package var status: TaskStatus?
        package var priority: Priority?
        package var startDate: CalendarDate?
        package var dueDate: CalendarDate?
        package var dueTime: TimeOfDay?
        /// Label names. Unknown names are an error, not created.
        package var labels: [String]?

        package init(title: String, avenue: String? = nil, project: UUID? = nil, parent: String? = nil) {
            self.title = title
            self.avenue = avenue
            self.project = project
            self.parent = parent
        }
    }
}

/// The result of a change: the task as it is now, and anything the rules
/// changed with it.
package struct ChangeResult: Hashable, Codable, Sendable {
    package var task: TaskSummary
    /// Other tasks the change touched, such as Sub tasks closed by a cascade.
    package var alsoChanged: [TaskSummary]
    /// A closed parent that moved back to In Progress.
    package var reopenedParent: String?
}

package enum UpdateTask {
    /// Fields that `clear` can unset.
    package enum ClearableField: String, Hashable, Codable, Sendable, CaseIterable {
        case startDate, dueDate, dueTime
    }

    package struct Params: Hashable, Codable, Sendable {
        package var task: String
        package var title: String?
        package var details: String?
        package var priority: Priority?
        package var startDate: CalendarDate?
        package var dueDate: CalendarDate?
        package var dueTime: TimeOfDay?
        /// Replaces every label.
        package var labels: [String]?
        package var clear: [ClearableField]?

        package init(task: String) { self.task = task }
    }
}

package enum SetStatus {
    package struct Params: Hashable, Codable, Sendable {
        package var task: String
        package var status: TaskStatus
        /// Required to mark a task with open Sub tasks Won't Do: they close too.
        package var cascade: Bool?

        package init(task: String, status: TaskStatus, cascade: Bool? = nil) {
            self.task = task
            self.status = status
            self.cascade = cascade
        }
    }
}

package enum MoveTask {
    /// Exactly one of: `avenue` (with an optional `project`), `parent`, or
    /// `topLevel: true`.
    package struct Params: Hashable, Codable, Sendable {
        package var task: String
        package var avenue: String?
        package var project: UUID?
        /// Make it a Sub task of this task.
        package var parent: String?
        /// Promote a Sub task to a top-level task.
        package var topLevel: Bool?

        package init(
            task: String, avenue: String? = nil, project: UUID? = nil, parent: String? = nil,
            topLevel: Bool? = nil
        ) {
            self.task = task
            self.avenue = avenue
            self.project = project
            self.parent = parent
            self.topLevel = topLevel
        }
    }
}

package enum AddComment {
    package struct Params: Hashable, Codable, Sendable {
        package var task: String
        /// Markdown.
        package var body: String

        package init(task: String, body: String) {
            self.task = task
            self.body = body
        }
    }

    package struct Result: Hashable, Codable, Sendable {
        package var id: UUID
        package var task: String
        package var createdAt: Date
    }
}

package enum AddPlan {
    package struct Params: Hashable, Codable, Sendable {
        package var plan: PlanInput
        /// An IANA time zone, such as Europe/London, for relative dates like
        /// "tomorrow". Defaults to the app's.
        package var timeZone: String?

        package init(plan: PlanInput, timeZone: String? = nil) {
            self.plan = plan
            self.timeZone = timeZone
        }
    }

    /// A plan as TOML text, starting with "# Lifr plan v1", or as a JSON
    /// object with `Plan`'s fields.
    package enum PlanInput: Hashable, Codable, Sendable {
        case toml(String)
        case json(Plan)

        package init(from decoder: any Decoder) throws {
            let container = try decoder.singleValueContainer()
            if let text = try? container.decode(String.self) {
                self = .toml(text)
            } else {
                self = .json(try container.decode(Plan.self))
            }
        }

        package func encode(to encoder: any Encoder) throws {
            var container = encoder.singleValueContainer()
            switch self {
            case .toml(let text): try container.encode(text)
            case .json(let plan): try container.encode(plan)
            }
        }
    }

    /// What the plan added, each in the plan's order.
    package struct Result: Hashable, Codable, Sendable {
        /// Avenue keys.
        package var avenues: [String]
        /// Label names, only for labels that didn't exist yet.
        package var labels: [String]
        package var projects: [ProjectSummary]
        /// Tasks and Sub tasks.
        package var tasks: [TaskSummary]
    }
}

package enum TrashTask {
    package struct Result: Hashable, Codable, Sendable {
        /// The task and the Sub tasks trashed with it.
        package var trashed: [String]
    }
}

/// Every record, for `lifr export`.
package struct Export: Hashable, Codable, Sendable {
    package var protocolVersion: Int
    package var avenues: [Avenue]
    package var projects: [Project]
    package var tasks: [LifrTask]
    package var labels: [Label]
    package var comments: [Comment]
    package var reminders: [Reminder]
    package var history: [HistoryEvent]
    package var settings: Settings
}
