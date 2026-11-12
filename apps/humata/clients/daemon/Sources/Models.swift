import Foundation

/// What runs a session's agent.
package enum Backend: String, CaseIterable, Hashable, Sendable {
    case scripted
    case local
    case copilot
    case claude
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
    /// How many of its finished turns the person has seen.
    package var seenTurns: Int
    /// The requests for permission its running turn waits on, oldest first.
    package var pending: [PendingPermission]
    /// Its host can't be reached, so this is as it last was.
    package var hostOffline: Bool
    /// When anything last happened in it; nil when nothing has yet.
    package var lastActive: Date?

    package init(
        id: String, name: String, backend: Backend, workingDirectory: String, model: String? = nil,
        state: SessionState = .idle, parentID: String? = nil, forkedBeforeEventID: String? = nil, host: String? = nil,
        approvalMode: ApprovalMode = .askEveryTime, projectID: String? = nil, archived: Bool = false,
        keeper: Bool = false, itemID: String? = nil, turns: Int = 0, seenTurns: Int = 0,
        pending: [PendingPermission] = [], hostOffline: Bool = false, lastActive: Date? = nil
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
        self.seenTurns = seenTurns
        self.pending = pending
        self.hostOffline = hostOffline
        self.lastActive = lastActive
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
    /// The patch's files, read into lines.
    package var files: [FileDiff]

    package init(name: String, patch: String, files: [FileDiff] = []) {
        self.name = name
        self.patch = patch
        self.files = files
    }
}

/// Which of a session's changes to read.
package enum ChangeRange: Hashable, Sendable {
    /// Everything since its copy was made.
    case session
    /// In the copy, not staged yet.
    case unstaged
    /// Staged in the copy, not committed yet.
    case staged
    /// From where the copy's work and the branch meet to the files in the copy.
    case branch(String)
    /// What one turn changed; the first is 1.
    case turn(Int)
}

/// A file, by its repository's name in the session and its path there.
package struct FileRef: Hashable, Sendable {
    package var repository: String
    package var path: String

    package init(repository: String, path: String) {
        self.repository = repository
        self.path = path
    }
}

package struct ChangesQuery: Hashable, Sendable {
    package var range: ChangeRange
    /// Unchanged lines shown around each change.
    package var context: Int
    /// Only this file.
    package var file: FileRef?
    /// Every line of `file`, even when it didn't change.
    package var wholeFile: Bool

    package init(range: ChangeRange = .session, context: Int = 3, file: FileRef? = nil, wholeFile: Bool = false) {
        self.range = range
        self.context = context
        self.file = file
        self.wholeFile = wholeFile
    }
}

package enum FileStatus: Hashable, Sendable {
    case modified
    case added
    case deleted
    case renamed
    /// Read as a whole file, and the same at both ends of the range.
    case unchanged
}

package struct FileDiff: Hashable, Sendable {
    package var path: String
    /// Where a renamed file was.
    package var oldPath: String?
    package var status: FileStatus
    package var binary: Bool
    /// Names its contents at the end of the range; empty for a deleted file.
    package var blob: String
    package var hunks: [Hunk]
    /// Marked viewed with these contents.
    package var viewed: Bool

    package init(
        path: String,
        oldPath: String? = nil,
        status: FileStatus,
        binary: Bool = false,
        blob: String = "",
        hunks: [Hunk],
        viewed: Bool = false
    ) {
        self.path = path
        self.oldPath = oldPath
        self.status = status
        self.binary = binary
        self.blob = blob
        self.hunks = hunks
        self.viewed = viewed
    }
}

package struct Hunk: Hashable, Sendable {
    package var oldStart: Int
    package var oldCount: Int
    package var newStart: Int
    package var newCount: Int
    /// What git shows after the hunk's range, such as the function it's in.
    package var header: String
    package var lines: [DiffLine]

    package init(oldStart: Int, oldCount: Int, newStart: Int, newCount: Int, header: String = "", lines: [DiffLine]) {
        self.oldStart = oldStart
        self.oldCount = oldCount
        self.newStart = newStart
        self.newCount = newCount
        self.header = header
        self.lines = lines
    }
}

package struct DiffLine: Hashable, Sendable {
    package enum Kind: Hashable, Sendable {
        case context
        case added
        case removed
    }

    package var kind: Kind
    /// Its number before the change; nil for an added line.
    package var old: Int?
    /// Its number after; nil for a removed line.
    package var new: Int?
    package var text: String

    package init(kind: Kind, old: Int?, new: Int?, text: String) {
        self.kind = kind
        self.old = old
        self.new = new
        self.text = text
    }
}

package enum ChangeAction: Hashable, Sendable {
    /// Stages the copy's unstaged changes.
    case stage
    /// Unstages staged changes, keeping them in the copy.
    case unstage
    /// Throws away the copy's unstaged changes; a new file is removed.
    case revert
}

/// A hunk, by where it starts in the diff the action works on, read with
/// `context` lines.
package struct HunkAt: Hashable, Sendable {
    package var oldStart: Int
    package var newStart: Int
    package var context: Int

    package init(oldStart: Int, newStart: Int, context: Int) {
        self.oldStart = oldStart
        self.newStart = newStart
        self.context = context
    }
}

/// Stages, unstages or reverts a file or one of its hunks.
package struct ChangeEdit: Hashable, Sendable {
    package var file: FileRef
    package var action: ChangeAction
    package var hunk: HunkAt?

    package init(file: FileRef, action: ChangeAction, hunk: HunkAt? = nil) {
        self.file = file
        self.action = action
        self.hunk = hunk
    }
}

/// Comments on lines of a file, kept with those lines as the code changes.
package struct ReviewThread: Identifiable, Hashable, Sendable {
    package var id: String
    package var file: FileRef
    /// Where it is in the file as it is now, from 1.
    package var startLine: Int
    package var endLine: Int
    /// The lines it was left on have changed, so it's where they were.
    package var outdated: Bool
    /// The lines as they were when it was left.
    package var code: [String]
    /// In rendered Markdown, the text it was left on.
    package var quote: String
    package var resolved: Bool
    package var comments: [ReviewComment]

    package init(
        id: String,
        file: FileRef,
        startLine: Int,
        endLine: Int,
        outdated: Bool = false,
        code: [String] = [],
        quote: String = "",
        resolved: Bool = false,
        comments: [ReviewComment]
    ) {
        self.id = id
        self.file = file
        self.startLine = startLine
        self.endLine = endLine
        self.outdated = outdated
        self.code = code
        self.quote = quote
        self.resolved = resolved
        self.comments = comments
    }

    /// Comments not sent to the agent yet.
    package var pending: Int { comments.filter { $0.author == .you && !$0.sent }.count }
}

package struct ReviewComment: Identifiable, Hashable, Sendable {
    package enum Author: Hashable, Sendable {
        case you
        case agent
    }

    package var id: String
    package var author: Author
    package var text: String
    /// Sent to the agent; only a comment not sent yet can be deleted.
    package var sent: Bool
    package var time: Date
    /// The agent's: the turn it answered in, whose changes are its fix.
    package var turn: Int?

    package init(id: String, author: Author, text: String, sent: Bool, time: Date, turn: Int? = nil) {
        self.id = id
        self.author = author
        self.text = text
        self.sent = sent
        self.time = time
        self.turn = turn
    }
}

/// A comment to leave on a file's lines.
package struct NewThread: Hashable, Sendable {
    package var file: FileRef
    package var startLine: Int
    package var endLine: Int
    package var quote: String
    package var text: String

    package init(file: FileRef, startLine: Int, endLine: Int, quote: String = "", text: String) {
        self.file = file
        self.startLine = startLine
        self.endLine = endLine
        self.quote = quote
        self.text = text
    }
}

package enum PermissionKind: Hashable, Sendable {
    case shell
    case write
    case read
    case other
}

/// A request for permission a session's running turn waits on.
package struct PendingPermission: Hashable, Sendable {
    package var requestID: String
    package var kind: PermissionKind
    /// What the agent wants to do, for the person deciding.
    package var summary: String

    package init(requestID: String, kind: PermissionKind, summary: String) {
        self.requestID = requestID
        self.kind = kind
        self.summary = summary
    }
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
        /// A message sent while a turn ran, waiting for its own turn.
        case messageQueued(queueID: String, text: String)
        /// A queued message left the queue: its turn started, or it was
        /// taken out.
        case dequeued(queueID: String)
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
    /// Nil for the session's own copy of the project's repositories, or,
    /// with no project, an empty folder of its own: a chat in no folder.
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
    package var copilot: Result<String, ProgramProblem>
    /// Whether its agents have a GitHub token; without one, Copilot needs
    /// `copilot login` there.
    package var githubToken: Bool
    /// Nil when it has no setup script.
    package var setup: SetupResult?
    /// Claude Code's version, or why it couldn't be run.
    package var claude: Result<String, ProgramProblem>
    /// Whether its agents have ANTHROPIC_API_KEY or CLAUDE_CODE_OAUTH_TOKEN;
    /// without one, Claude Code needs `claude` signed in there.
    package var anthropicKey: Bool

    package init(
        programVersion: String, platform: String, freeDiskBytes: UInt64, copilot: Result<String, ProgramProblem>,
        githubToken: Bool, setup: SetupResult? = nil,
        claude: Result<String, ProgramProblem> = .failure(ProgramProblem("Not checked")), anthropicKey: Bool = false
    ) {
        self.programVersion = programVersion
        self.platform = platform
        self.freeDiskBytes = freeDiskBytes
        self.copilot = copilot
        self.githubToken = githubToken
        self.setup = setup
        self.claude = claude
        self.anthropicKey = anthropicKey
    }
}

/// Why an agent's program couldn't be run on a host.
package struct ProgramProblem: Error, Hashable, Sendable {
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
    case unknownThread
    /// What the request worked on changed since it was read.
    case changed
    /// The queued message already started or was taken out.
    case unknownQueuedMessage
    /// The session to import isn't one the backend has.
    case unknownImport
    /// The session was imported before.
    case alreadyImported
    case unknown
}

/// A session a backend ran on a host outside Humata, which can be imported
/// with its history and carried on.
package struct ImportableSession: Equatable, Identifiable, Sendable {
    package var backend: Backend
    /// The backend's own id for it.
    package var id: String
    /// The folder it worked in, on the host.
    package var workingDirectory: String
    /// Its first message, on one line.
    package var summary: String
    package var updated: Date

    package init(backend: Backend, id: String, workingDirectory: String, summary: String, updated: Date) {
        self.backend = backend
        self.id = id
        self.workingDirectory = workingDirectory
        self.summary = summary
        self.updated = updated
    }
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

/// A file in a session's folder.
package struct ChatFile: Hashable, Sendable, Identifiable {
    /// From the session's folder, with "/" between folders.
    package var path: String
    package var size: UInt64
    package var modified: Date

    package var id: String { path }

    package init(path: String, size: UInt64 = 0, modified: Date = Date(timeIntervalSince1970: 0)) {
        self.path = path
        self.size = size
        self.modified = modified
    }

    /// Whether it's Markdown, shown rendered.
    package var isMarkdown: Bool {
        ["md", "markdown"].contains((path as NSString).pathExtension.lowercased())
    }
}

/// The files in a session's folder, hidden ones left out.
package struct ChatFiles: Hashable, Sendable {
    /// By path.
    package var files: [ChatFile]
    /// There were more than are listed.
    package var truncated: Bool

    package init(files: [ChatFile] = [], truncated: Bool = false) {
        self.files = files
        self.truncated = truncated
    }
}

/// A file's contents; nil when it isn't text.
package struct FileContents: Hashable, Sendable {
    package var path: String
    package var text: String?

    package init(path: String, text: String?) {
        self.path = path
        self.text = text
    }
}
