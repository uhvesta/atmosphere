import Foundation

/// What runs a session's agent.
package enum Backend: String, CaseIterable, Hashable, Sendable {
    case scripted
    case local
    case copilot
}

/// What a session may do without asking.
package enum ApprovalMode: String, CaseIterable, Hashable, Sendable {
    case askEveryTime
    /// Reads and file writes go ahead; commands and anything else still ask.
    case allowEdits
    case allowEverything

    /// Whether a request of `kind` goes ahead without asking.
    package func allows(_ kind: PermissionKind) -> Bool {
        switch self {
        case .askEveryTime: false
        case .allowEdits: kind == .read || kind == .write
        case .allowEverything: true
        }
    }
}

/// A git repository a project works in.
package struct Repository: Hashable, Sendable {
    /// Its folder's name in a session's copy of the project; empty when
    /// adding one, for the daemon to name it after its folder.
    package var name: String
    /// Its folder on this Mac.
    package var path: String

    package init(name: String = "", path: String) {
        self.name = name
        self.path = path
    }
}

/// A name, the repositories on this Mac it works in, and instructions for
/// its sessions.
package struct Project: Identifiable, Hashable, Sendable {
    package var id: String
    package var name: String
    /// Each session works in its own copy of these.
    package var repositories: [Repository]
    package var instructions: String
    package var created: Date

    package init(id: String, name: String, repositories: [Repository], instructions: String = "", created: Date) {
        self.id = id
        self.name = name
        self.repositories = repositories
        self.instructions = instructions
        self.created = created
    }
}

package enum SessionState: Hashable, Sendable {
    case idle
    case working
    case waitingForApproval
    case failed
}

package struct Session: Identifiable, Hashable, Sendable {
    package var id: String
    package var name: String
    package var backend: Backend
    /// The folder the agent works in, on its host.
    package var workingDirectory: String
    /// Nil for the backend's default.
    package var model: String?
    package var state: SessionState
    /// The session this was forked from; nil for a root session.
    package var parentID: String?
    /// The event the fork was made before; nil when it kept all of them.
    package var forkedBeforeEventID: String?
    /// The host it runs on, as the daemon names it; nil for this Mac.
    package var host: String?
    package var approvalMode: ApprovalMode
    /// Nil for a session in no project.
    package var projectID: String?
    /// Put away: its copy of the code is saved and removed, and it takes no
    /// messages until unarchived.
    package var archived: Bool
    /// Marked as the session whose changes are the ones to keep.
    package var keeper: Bool
    /// The task or subtask it works on; nil for the project itself.
    package var itemID: String?
    /// How many turns it has finished.
    package var turns: Int
    /// Its host can't be reached, so this is as it last was.
    package var hostOffline: Bool

    package init(
        id: String, name: String, backend: Backend, workingDirectory: String, model: String? = nil,
        state: SessionState = .idle, parentID: String? = nil, forkedBeforeEventID: String? = nil, host: String? = nil,
        approvalMode: ApprovalMode = .askEveryTime, projectID: String? = nil, archived: Bool = false,
        keeper: Bool = false, itemID: String? = nil, turns: Int = 0, hostOffline: Bool = false
    ) {
        self.id = id
        self.name = name
        self.backend = backend
        self.workingDirectory = workingDirectory
        self.model = model
        self.state = state
        self.parentID = parentID
        self.forkedBeforeEventID = forkedBeforeEventID
        self.host = host
        self.approvalMode = approvalMode
        self.projectID = projectID
        self.archived = archived
        self.keeper = keeper
        self.itemID = itemID
        self.turns = turns
        self.hostOffline = hostOffline
    }
}

/// Where on its project's board a session is.
package enum Placement: Hashable, Sendable {
    /// On a task or subtask in the session's project.
    case item(String)
    /// On the project itself, on no item.
    case project
}

/// What to change about a session; nil leaves it as it is.
package struct SessionUpdate: Hashable, Sendable {
    package var name: String?
    package var archived: Bool?
    package var keeper: Bool?
    /// Moves it to another item in its project.
    package var placement: Placement?

    package init(name: String? = nil, archived: Bool? = nil, keeper: Bool? = nil, placement: Placement? = nil) {
        self.name = name
        self.archived = archived
        self.keeper = keeper
        self.placement = placement
    }
}

/// A column of the board.
package enum ItemStatus: String, CaseIterable, Hashable, Sendable {
    case todo
    case inProgress
    case waitingOnYou
    case inReview
    case done
    case wontDo

    /// Done and Won't do: set by hand, and sessions don't move an item out.
    package var closed: Bool { self == .done || self == .wontDo }
}

package enum Priority: Int, CaseIterable, Comparable, Hashable, Sendable {
    case none
    case low
    case medium
    case high
    case urgent

    package static func < (lhs: Self, rhs: Self) -> Bool { lhs.rawValue < rhs.rawValue }
}

/// What the board's changes record of an item.
package struct ItemFields: Hashable, Sendable {
    /// The task a subtask is under; nil for a task.
    package var parentID: String?
    package var title: String
    /// Markdown.
    package var spec: String
    package var status: ItemStatus
    package var priority: Priority

    package init(
        parentID: String? = nil, title: String, spec: String = "", status: ItemStatus = .todo,
        priority: Priority = .none
    ) {
        self.parentID = parentID
        self.title = title
        self.spec = spec
        self.status = status
        self.priority = priority
    }
}

/// A task, or a subtask of one, on a project's board.
package struct Item: Identifiable, Hashable, Sendable {
    package var id: String
    package var projectID: String
    package var fields: ItemFields
    /// Where it shows: what its sessions, and a task's subtasks' sessions,
    /// say while they work, wait or have finished a turn; otherwise, and
    /// always once closed, its given status.
    package var status: ItemStatus
    package var created: Date
    package var updated: Date
    /// The sessions on it, archived ones too.
    package var sessionIDs: [String]

    package init(
        id: String, projectID: String, fields: ItemFields, status: ItemStatus? = nil, created: Date,
        updated: Date? = nil, sessionIDs: [String] = []
    ) {
        self.id = id
        self.projectID = projectID
        self.fields = fields
        self.status = status ?? fields.status
        self.created = created
        self.updated = updated ?? created
        self.sessionIDs = sessionIDs
    }

    package var parentID: String? { fields.parentID }
    package var title: String { fields.title }
    package var spec: String { fields.spec }
    package var priority: Priority { fields.priority }
}

/// What to change in an item; nil leaves a field as it is.
package struct ItemUpdate: Hashable, Sendable {
    package var title: String?
    package var spec: String?
    package var status: ItemStatus?
    package var priority: Priority?

    package init(title: String? = nil, spec: String? = nil, status: ItemStatus? = nil, priority: Priority? = nil) {
        self.title = title
        self.spec = spec
        self.status = status
        self.priority = priority
    }
}

/// One change to a project's board.
package struct BoardChange: Identifiable, Hashable, Sendable {
    package var id: UInt64
    package var itemID: String
    package var time: Date
    /// Nil when it added the item.
    package var before: ItemFields?
    /// Nil when it removed it.
    package var after: ItemFields?
    /// The change it reverted, if it did.
    package var reverts: UInt64?

    package init(
        id: UInt64, itemID: String, time: Date, before: ItemFields?, after: ItemFields?, reverts: UInt64? = nil
    ) {
        self.id = id
        self.itemID = itemID
        self.time = time
        self.before = before
        self.after = after
        self.reverts = reverts
    }
}

/// Why a session waits on you.
package enum InboxReason: Hashable, Sendable {
    /// It asked to run a command, write or read.
    case approval
    /// It finished a turn: a question to answer or work to review.
    case reply
    /// Its turn failed.
    case failed
}

package struct InboxEntry: Identifiable, Hashable, Sendable {
    package var session: Session
    package var reason: InboxReason
    package var projectName: String?
    package var itemTitle: String?

    package init(session: Session, reason: InboxReason, projectName: String? = nil, itemTitle: String? = nil) {
        self.session = session
        self.reason = reason
        self.projectName = projectName
        self.itemTitle = itemTitle
    }

    package var id: String { session.id }
}

/// Which specs a session on an item reads ahead of its instructions.
package enum SpecScope: Hashable, Sendable {
    /// Its item's alone.
    case item
    /// Its item's, and the task's above a subtask.
    case withParents
}

/// What a session changed in one of its repositories.
package struct RepositoryChanges: Hashable, Sendable {
    package var name: String
    /// A unified diff; empty if nothing changed.
    package var patch: String

    package init(name: String, patch: String) {
        self.name = name
        self.patch = patch
    }
}

package enum PermissionKind: Hashable, Sendable {
    case shell
    case write
    case read
    case other
}

/// One thing that happened in a session. Its id stays the same for the life
/// of the session, and a fork's copies keep it.
package struct SessionEvent: Identifiable, Hashable, Sendable {
    package enum Kind: Hashable, Sendable {
        case userMessage(String)
        case turnStarted
        /// Part of the reply as it streams; the assistantMessage after it
        /// holds all of it.
        case assistantDelta(String)
        case assistantMessage(String)
        case reasoning(String)
        case toolStarted(callID: String, tool: String, argumentsJSON: String)
        case toolCompleted(callID: String, success: Bool, output: String)
        case permissionRequested(requestID: String, kind: PermissionKind, summary: String)
        /// `automatic` when the session's approval mode answered it, not a
        /// person.
        case permissionCompleted(requestID: String, approved: Bool, automatic: Bool)
        case turnEnded(aborted: Bool)
        case idle
        case failed(String)
        case usage(model: String, inputTokens: UInt64, outputTokens: UInt64, premiumRequests: Double)
        /// The session's code was saved after a turn; a fork with code made
        /// after this starts from it.
        case checkpoint
    }

    package var id: String
    package var time: Date
    package var kind: Kind

    package init(id: String, time: Date, kind: Kind) {
        self.id = id
        self.time = time
        self.kind = kind
    }
}

package struct StartRequest: Hashable, Sendable {
    package var backend: Backend
    /// Nil for the session's own copy of the project's repositories.
    package var workingDirectory: String?
    package var name: String
    /// Nil for this Mac.
    package var host: String?
    package var model: String?
    package var instructions: String?
    /// Settings only the backend reads, as JSON; the scripted backend's script.
    package var backendConfig: String?
    package var approvalMode: ApprovalMode
    /// The project it belongs to. The session works in its own copy of the
    /// project's repositories, and its instructions are the default.
    package var projectID: String?
    /// The task or subtask it works on, whose project it is in.
    package var itemID: String?
    package var specs: SpecScope

    package init(
        backend: Backend, workingDirectory: String? = nil, name: String = "", host: String? = nil,
        model: String? = nil, instructions: String? = nil, backendConfig: String? = nil,
        approvalMode: ApprovalMode = .askEveryTime, projectID: String? = nil, itemID: String? = nil,
        specs: SpecScope = .item
    ) {
        self.backend = backend
        self.workingDirectory = workingDirectory
        self.name = name
        self.host = host
        self.model = model
        self.instructions = instructions
        self.backendConfig = backendConfig
        self.approvalMode = approvalMode
        self.projectID = projectID
        self.itemID = itemID
        self.specs = specs
    }
}

package enum SendMode: Hashable, Sendable {
    /// Start a turn now, or after the running one.
    case enqueue
    /// Steer the running turn.
    case steer
}

/// A machine sessions run on, reached over SSH.
package struct RemoteHost: Identifiable, Hashable, Sendable {
    package var name: String
    /// What `ssh` connects to, such as a Host in ~/.ssh/config; nil for a
    /// host reached by a command of its own, which Humata can't install on.
    package var sshDestination: String?
    /// The variables set for its agents. Their values never come back from
    /// the daemon.
    package var envNames: [String]
    /// Run on the host when it changes, or until it succeeds.
    package var setupScript: String

    package init(name: String, sshDestination: String?, envNames: [String] = [], setupScript: String = "") {
        self.name = name
        self.sshDestination = sshDestination
        self.envNames = envNames
        self.setupScript = setupScript
    }

    package var id: String { name }
}

/// What to change about a host; nil and empty leave it as it is.
package struct HostUpdate: Hashable, Sendable {
    /// Variables to set or replace, by name.
    package var setEnv: [String: String]
    package var unsetEnv: [String]
    package var setupScript: String?

    package init(setEnv: [String: String] = [:], unsetEnv: [String] = [], setupScript: String? = nil) {
        self.setEnv = setEnv
        self.unsetEnv = unsetEnv
        self.setupScript = setupScript
    }
}

/// How a host's setup script last ran.
package struct SetupResult: Hashable, Sendable {
    package var succeeded: Bool
    /// What it printed, cut to the end if it was long.
    package var output: String
    package var time: Date

    package init(succeeded: Bool, output: String, time: Date) {
        self.succeeded = succeeded
        self.output = output
        self.time = time
    }
}

/// What a reachable host says about itself.
package struct HostStatus: Hashable, Sendable {
    package var programVersion: String
    /// Its system and architecture, as `uname -sm` prints them.
    package var platform: String
    package var freeDiskBytes: UInt64
    /// Copilot's version, or why it couldn't be run.
    package var copilot: Result<String, CopilotProblem>
    /// Whether its agents have a GitHub token; without one, Copilot needs
    /// `copilot login` there.
    package var githubToken: Bool
    /// Nil when it has no setup script.
    package var setup: SetupResult?

    package init(
        programVersion: String, platform: String, freeDiskBytes: UInt64, copilot: Result<String, CopilotProblem>,
        githubToken: Bool, setup: SetupResult? = nil
    ) {
        self.programVersion = programVersion
        self.platform = platform
        self.freeDiskBytes = freeDiskBytes
        self.copilot = copilot
        self.githubToken = githubToken
        self.setup = setup
    }
}

/// Why Copilot couldn't be run on a host.
package struct CopilotProblem: Error, Hashable, Sendable {
    package var message: String

    package init(_ message: String) {
        self.message = message
    }
}

/// Whether a host answered, and how it is.
package enum HostHealth: Hashable, Sendable {
    case reachable(HostStatus)
    /// Why it couldn't be reached, for the person who has to fix it.
    case unreachable(String)
}

/// What the daemon said went wrong.
package enum ErrorCode: Hashable, Sendable {
    case unknownSession
    case unknownEvent
    case unknownPermissionRequest
    case unknownBackend
    case busy
    case invalidRequest
    case backendFailed
    case storageFailed
    case hostUnavailable
    case unknownProject
    case archived
    case unknownItem
    case unknownBoardChange
    case unknownHost
    case unknown
}

package enum DaemonError: LocalizedError, Hashable, CustomStringConvertible {
    /// The daemon answered with an error; its message says how to fix it.
    case refused(ErrorCode, message: String)
    /// Nothing answered on the socket.
    case notRunning(socket: String, reason: String)
    /// The connection closed before the answer came.
    case disconnected
    /// The daemon speaks another protocol version.
    case versionMismatch(daemon: String)
    /// The daemon sent something this app doesn't expect.
    case unexpected(String)

    package var errorDescription: String? { description }

    package var description: String {
        switch self {
        case .refused(_, let message):
            message
        case .notRunning(let socket, let reason):
            "Humata's daemon isn't running at \(socket) (\(reason)). Start it with `humatad daemon`, then try again."
        case .disconnected:
            "The connection to Humata's daemon closed before it answered. Check that it is still running, then "
                + "retry; the request may or may not have been carried out."
        case .versionMismatch(let daemon):
            "Humata's daemon is version \(daemon) and speaks another protocol. Update Humata so both are the "
                + "same version."
        case .unexpected(let what):
            "Humata's daemon sent \(what), which this app doesn't expect. Update Humata so both are the same version."
        }
    }
}
