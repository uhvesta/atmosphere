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
    /// Its folder on this Mac; for one added by its URL, Humata's clone of
    /// it, and empty when adding it.
    package var path: String
    /// Where it's cloned from, for one added by its URL rather than a folder.
    package var url: String

    package init(name: String = "", path: String) {
        self.name = name
        self.path = path
        url = ""
    }

    package init(name: String = "", path: String = "", url: String) {
        self.name = name
        self.path = path
        self.url = url
    }

    /// Whether `text` is a URL to clone, as `https://…` or `git@host:owner/name`,
    /// rather than a folder.
    package static func isURL(_ text: String) -> Bool {
        if text.contains("://") { return true }
        guard !text.hasPrefix("/"), !text.hasPrefix("~"), let colon = text.firstIndex(of: ":") else { return false }
        return text[..<colon].contains("@")
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
    /// Know-how every session in the project reads after its instructions.
    package var skills: [Skill]
    package var created: Date

    package init(
        id: String, name: String, repositories: [Repository], instructions: String = "", skills: [Skill] = [],
        created: Date
    ) {
        self.id = id
        self.name = name
        self.repositories = repositories
        self.instructions = instructions
        self.skills = skills
        self.created = created
    }
}

/// A named piece of know-how, such as how to run the tests.
package struct Skill: Hashable, Sendable {
    package var name: String
    package var text: String

    package init(name: String, text: String) {
        self.name = name
        self.text = text
    }
}

/// What to change about a project; nil leaves it as it is.
package struct ProjectUpdate: Hashable, Sendable {
    package var name: String?
    package var instructions: String?
    /// Replaces every skill; an empty list removes them.
    package var skills: [Skill]?
    /// Replaces the repositories, for sessions started after; sessions
    /// already started keep their copies.
    package var repositories: [Repository]?

    package init(
        name: String? = nil, instructions: String? = nil, skills: [Skill]? = nil, repositories: [Repository]? = nil
    ) {
        self.name = name
        self.instructions = instructions
        self.skills = skills
        self.repositories = repositories
    }
}

/// A task or spec edit an agent proposed; nothing is saved until the
/// person applies it.
package struct Proposal: Identifiable, Hashable, Sendable {
    package enum Kind: Hashable, Sendable {
        case task
        case specEdit
    }

    package enum State: Hashable, Sendable {
        case pending
        /// Applied, making or changing this item.
        case applied(itemID: String)
        case declined
    }

    /// The tool call the agent proposed it with.
    package var id: String
    package var kind: Kind
    /// For a task, its title; for a spec edit, the item's.
    package var title: String
    package var spec: String
    /// The task a proposed subtask goes under.
    package var parentID: String?
    /// The item a spec edit changes.
    package var itemID: String?
    /// Why, in the agent's words.
    package var note: String
    package var state: State
    /// For a pending spec edit, the item's spec as it is now.
    package var currentSpec: String?

    package init(
        id: String, kind: Kind = .task, title: String, spec: String = "", parentID: String? = nil,
        itemID: String? = nil, note: String = "", state: State = .pending, currentSpec: String? = nil
    ) {
        self.id = id
        self.kind = kind
        self.title = title
        self.spec = spec
        self.parentID = parentID
        self.itemID = itemID
        self.note = note
        self.state = state
        self.currentSpec = currentSpec
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
    /// That task's or subtask's title; nil for none.
    package var itemTitle: String?
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
    /// It plans in its project, in no copy of the code.
    package var planning: Bool
    /// It waits on the person to answer its question or its move request.
    package var asking: Bool
    /// What it last said or why it failed, cut short; nil when it hasn't yet.
    package var lastMessage: String?

    package init(
        id: String, name: String, backend: Backend, workingDirectory: String, model: String? = nil,
        state: SessionState = .idle, parentID: String? = nil, forkedBeforeEventID: String? = nil, host: String? = nil,
        approvalMode: ApprovalMode = .askEveryTime, projectID: String? = nil, archived: Bool = false,
        keeper: Bool = false, itemID: String? = nil, turns: Int = 0, seenTurns: Int = 0,
        pending: [PendingPermission] = [], hostOffline: Bool = false, lastActive: Date? = nil, planning: Bool = false,
        asking: Bool = false, lastMessage: String? = nil, itemTitle: String? = nil
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
        self.planning = planning
        self.asking = asking
        self.lastMessage = lastMessage
        self.itemTitle = itemTitle
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

extension ItemStatus {
    /// The board column's name.
    package var title: String {
        switch self {
        case .todo: String(localized: "To Do")
        case .inProgress: String(localized: "In Progress")
        case .waitingOnYou: String(localized: "Waiting on You")
        case .inReview: String(localized: "In Review")
        case .done: String(localized: "Done")
        case .wontDo: String(localized: "Won't Do")
        }
    }
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

/// Who made a board change.
package enum BoardAuthor: Hashable, Sendable {
    /// The person, in this app.
    case app
    /// The person, with the humata command.
    case commandLine
    /// An outside agent, through humata mcp.
    case mcp
    /// Humata itself, such as a task done when its pull request merged.
    case humata
    /// A session, whose proposal or request the person approved.
    case session(String)
    /// Made before Humata recorded who made changes.
    case unknown
}

/// A field of an item, as a board change names it.
package enum ItemField: Hashable, Sendable, CaseIterable {
    case title, spec, status, priority
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
    package var author: BoardAuthor
    /// The admin approval it ran under, if another client asked for it.
    package var approvalID: String?
    /// The grant it ran under, if one covered it rather than the person.
    package var grantID: String?

    package init(
        id: UInt64, itemID: String, time: Date, before: ItemFields?, after: ItemFields?, reverts: UInt64? = nil,
        author: BoardAuthor = .unknown, approvalID: String? = nil, grantID: String? = nil
    ) {
        self.id = id
        self.itemID = itemID
        self.time = time
        self.before = before
        self.after = after
        self.reverts = reverts
        self.author = author
        self.approvalID = approvalID
        self.grantID = grantID
    }

    /// The fields it changed, for an edit; none for adding or removing.
    package var fields: [ItemField] {
        guard let before, let after else { return [] }
        return ItemField.allCases.filter { before.differs(from: after, in: $0) }
    }
}

extension ItemFields {
    /// Whether `other` has another value for `field`.
    package func differs(from other: ItemFields, in field: ItemField) -> Bool {
        switch field {
        case .title: title != other.title
        case .spec: spec != other.spec
        case .status: status != other.status
        case .priority: priority != other.priority
        }
    }
}

/// What reverting a board change did.
package struct Reverted: Hashable, Sendable {
    /// The item as it is now; nil when reverting removed it.
    package var item: Item?
    /// The fields the change set that were kept, as they changed again
    /// since.
    package var kept: [ItemField]

    package init(item: Item?, kept: [ItemField] = []) {
        self.item = item
        self.kept = kept
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
    /// It asked you a question, or for your opinion.
    case question
    /// It finished and asks you to move its item.
    case move

    /// Whether the session can't go on until the person acts; otherwise it
    /// only has something to look over.
    package var blocks: Bool { self != .reply }
}

/// A question an agent asked, or a move it asked for, waiting on you.
package struct AgentRequest: Identifiable, Hashable, Sendable {
    package enum Kind: Hashable, Sendable {
        case question
        /// Move `item` to `status`.
        case move(item: String, status: ItemStatus)
    }

    package var id: String
    package var sessionID: String
    package var kind: Kind
    /// The question, or what it did and what to check.
    package var text: String
    package var created: Date

    package init(id: String, sessionID: String, kind: Kind, text: String, created: Date) {
        self.id = id
        self.sessionID = sessionID
        self.kind = kind
        self.text = text
        self.created = created
    }
}

/// How the person answers an agent's request.
package enum RequestAnswer: Hashable, Sendable {
    /// Sent to the session as a message.
    case reply(String)
    /// Makes the move asked for.
    case approve
    /// Moves the item somewhere else.
    case move(ItemStatus)
    case dismiss
}

package struct InboxEntry: Identifiable, Hashable, Sendable {
    package var session: Session
    package var reason: InboxReason
    package var projectName: String?
    package var itemTitle: String?
    /// The request, for a question or a move.
    package var request: AgentRequest?

    package init(
        session: Session, reason: InboxReason, projectName: String? = nil, itemTitle: String? = nil,
        request: AgentRequest? = nil
    ) {
        self.session = session
        self.reason = reason
        self.projectName = projectName
        self.itemTitle = itemTitle
        self.request = request
    }

    package var id: String { request?.id ?? session.id }
}

/// What's waiting on the person: sessions, and admin actions other clients
/// asked for.
package struct Inbox: Hashable, Sendable {
    package var entries: [InboxEntry]
    /// Oldest first.
    package var approvals: [AdminApproval]

    package init(entries: [InboxEntry] = [], approvals: [AdminApproval] = []) {
        self.entries = entries
        self.approvals = approvals
    }
}

/// What an admin action may do.
package enum ApprovalScope: Hashable, Sendable, CaseIterable {
    /// Applying and declining proposals.
    case proposals
    /// Adding and changing items and their specs, and importing issues.
    case board
    /// Deleting items and reverting board changes.
    case destructive

    package var title: String {
        switch self {
        case .proposals: String(localized: "Proposals")
        case .board: String(localized: "Board Changes")
        case .destructive: String(localized: "Deletions and Reverts")
        }
    }
}

package enum ApprovalState: Hashable, Sendable {
    case pending, approved, denied
}

/// An admin action a client other than the app asked for, as the `humata`
/// command or an agent's `humata mcp`, and what became of it.
package struct AdminApproval: Identifiable, Hashable, Sendable {
    package var id: String
    /// The program that asked, with its arguments.
    package var client: String
    /// The program that started it, such as `claude`; empty if unknown.
    package var parent: String
    package var scope: ApprovalScope
    package var summary: String
    /// For a title or spec it changes: the text now and the text it sets.
    package var before: String
    package var after: String
    package var state: ApprovalState
    package var asked: Date
    package var decided: Date?
    /// The grant it ran under; nil when the person approved it.
    package var grantID: String?
    /// Once approved, what went wrong running it, if anything.
    package var failure: String?
    package var projectID: String?

    package init(
        id: String, client: String, parent: String = "", scope: ApprovalScope, summary: String,
        before: String = "", after: String = "", state: ApprovalState = .pending, asked: Date,
        decided: Date? = nil, grantID: String? = nil, failure: String? = nil, projectID: String? = nil
    ) {
        self.id = id
        self.client = client
        self.parent = parent
        self.scope = scope
        self.summary = summary
        self.before = before
        self.after = after
        self.state = state
        self.asked = asked
        self.decided = decided
        self.grantID = grantID
        self.failure = failure
        self.projectID = projectID
    }

    /// Who asked, as the person would name it: its parent program, then the client.
    package var asker: String {
        parent.isEmpty ? client : "\(parent) › \(client)"
    }
}

/// Leave the person gave for admin actions of `scopes` to run without asking.
package struct ApprovalGrant: Identifiable, Hashable, Sendable {
    package var id: String
    package var scopes: [ApprovalScope]
    package var granted: Date
    package var expires: Date
    /// How many actions ran under it.
    package var used: Int

    package init(id: String, scopes: [ApprovalScope], granted: Date, expires: Date, used: Int = 0) {
        self.id = id
        self.scopes = scopes
        self.granted = granted
        self.expires = expires
        self.used = used
    }
}

/// The approvals the daemon keeps, newest first, and the grants in force.
package struct Approvals: Hashable, Sendable {
    package var approvals: [AdminApproval]
    package var grants: [ApprovalGrant]

    package init(approvals: [AdminApproval] = [], grants: [ApprovalGrant] = []) {
        self.approvals = approvals
        self.grants = grants
    }
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
    package var pending: Int { comments.filter { $0.author != .agent && !$0.sent }.count }
}

package struct ReviewComment: Identifiable, Hashable, Sendable {
    package enum Author: Hashable, Sendable {
        case you
        case agent
        /// A reviewer on the pull request, by their GitHub login.
        case github(String)
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
        /// The turn can't go on yet, and says why; any later event means it
        /// went on.
        case waiting(String)
        /// Something the person should know that isn't from the agent, such
        /// as that it started over without its earlier conversation.
        case notice(String)
        case usage(model: String, inputTokens: UInt64, outputTokens: UInt64, premiumRequests: Double)
        /// The session's code was saved after a turn; a fork with code made
        /// after this starts from it.
        case checkpoint
        /// A message sent while a turn ran, waiting for its own turn.
        case messageQueued(queueID: String, text: String)
        /// A queued message left the queue: its turn started, or it was
        /// taken out, or, `dropped`, Humata stopped before it ran.
        case dequeued(queueID: String, dropped: Bool = false)
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
    /// Plans in the project, proposing tasks and spec edits, with no copy
    /// of the code, so it opens at once.
    package var planning: Bool

    package init(
        backend: Backend, workingDirectory: String? = nil, name: String = "", host: String? = nil,
        model: String? = nil, instructions: String? = nil, backendConfig: String? = nil,
        approvalMode: ApprovalMode = .askEveryTime, projectID: String? = nil, itemID: String? = nil,
        specs: SpecScope = .item, planning: Bool = false
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
        self.planning = planning
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

    /// The coordinator, the Mac the daemon runs on: it has no name.
    package static func coordinator(envNames: [String] = []) -> Self {
        Self(name: "", sshDestination: nil, envNames: envNames)
    }

    /// Whether it's the coordinator rather than a remote host.
    package var isCoordinator: Bool { name.isEmpty }

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
    /// Why its login shell's variables couldn't be read when it started, so
    /// its agents run without them; nil when they were.
    package var environmentProblem: String?

    package init(
        programVersion: String, platform: String, freeDiskBytes: UInt64, copilot: Result<String, ProgramProblem>,
        githubToken: Bool, setup: SetupResult? = nil,
        claude: Result<String, ProgramProblem> = .failure(ProgramProblem("Not checked")), anthropicKey: Bool = false,
        environmentProblem: String? = nil
    ) {
        self.environmentProblem = environmentProblem
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

/// This Mac's `humata` and `humatad`, linked into a folder a terminal
/// looks in, or not.
package struct CommandLinks: Hashable, Sendable {
    package struct Link: Hashable, Sendable {
        package enum State: Hashable, Sendable {
            case missing
            case linked
            /// Linked to another copy of Humata, as an older one.
            case stale
            /// Something else of the name is there, which stays.
            case other
        }

        package var name: String
        package var state: State

        package init(name: String, state: State) {
            self.name = name
            self.state = state
        }
    }

    /// `~/.local/bin`, in full.
    package var folder: String
    package var links: [Link]
    /// Whether the login shell's PATH has `folder`.
    package var onPath: Bool
    /// Why the login shell's variables couldn't be read when the daemon
    /// started, so agents here run without them; nil when they were.
    package var environmentProblem: String?

    package init(folder: String, links: [Link], onPath: Bool, environmentProblem: String? = nil) {
        self.folder = folder
        self.links = links
        self.onPath = onPath
        self.environmentProblem = environmentProblem
    }

    /// Whether linking would change anything.
    package var linkable: Bool {
        links.contains { $0.state == .missing || $0.state == .stale }
    }
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
    /// The request was answered already.
    case unknownRequest
    case unknownTerminal
    /// The browser's connection to a forwarded port closed already.
    case unknownConnection
    /// The forward was stopped already.
    case unknownForward
    /// The session proposed nothing by that id, or it was decided already.
    case unknownProposal
    /// GitHub refused, or there's no token to reach it with.
    case github
    /// Only the app may ask it, as it decides approvals.
    case appOnly
    /// The approval or grant isn't one the daemon keeps.
    case unknownApproval
    case unknown
}

/// The Mac's copy of a session on a host, kept after each turn, to set it
/// up again if its host loses it, or on another host.
package struct Recovery: Equatable, Identifiable, Sendable {
    /// The session's id.
    package var id: String
    package var name: String
    package var host: String
    /// The turns the copy has, of the session's `turns`.
    package var backedUpTurns: Int
    package var turns: Int
    package var backedUp: Date
    /// Whether its host still has the session; if not, restoring it sets
    /// it up there again.
    package var onHost: Bool
    /// Why its last backup failed; empty when it worked.
    package var problem: String

    package init(
        id: String, name: String, host: String, backedUpTurns: Int, turns: Int, backedUp: Date, onHost: Bool,
        problem: String = ""
    ) {
        self.id = id
        self.name = name
        self.host = host
        self.backedUpTurns = backedUpTurns
        self.turns = turns
        self.backedUp = backedUp
        self.onHost = onHost
        self.problem = problem
    }

    /// Whether there is a copy at all: a session whose first backup failed
    /// has none.
    package var hasCopy: Bool { backedUp.timeIntervalSince1970 > 0 }
}

/// What restoring a host's lost sessions did: those set up again, and why
/// each other one wasn't.
package struct HostRestore: Equatable, Sendable {
    package var restored: [Session]
    package var failed: [RestoreFailure]

    package init(restored: [Session], failed: [RestoreFailure]) {
        self.restored = restored
        self.failed = failed
    }
}

package struct RestoreFailure: Equatable, Identifiable, Sendable {
    /// The session's id.
    package var id: String
    /// What went wrong, and how to fix it.
    package var message: String

    package init(id: String, message: String) {
        self.id = id
        self.message = message
    }
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

/// A session on a host whose files couldn't be read, so it was left as it
/// is. Removing it deletes its folder, with its history.
package struct SkippedSession: Equatable, Identifiable, Sendable {
    package var backend: Backend
    /// Its folder's name, which is its id.
    package var id: String
    /// Its folder, on the host.
    package var folder: String
    /// Why it couldn't be opened, and what fixing it would take.
    package var reason: String

    package init(backend: Backend, id: String, folder: String, reason: String) {
        self.backend = backend
        self.id = id
        self.folder = folder
        self.reason = reason
    }
}

/// A model a backend offers, to start a session with.
package struct ModelChoice: Equatable, Identifiable, Sendable {
    /// What a session starts with as its model.
    package var id: String
    package var name: String
    /// Such as what a turn costs; nil when there is nothing to say.
    package var detail: String?

    package init(id: String, name: String, detail: String? = nil) {
        self.id = id
        self.name = name
        self.detail = detail
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
    /// nil when it isn't text.
    package var text: String?
    /// An image's bytes, when it was small enough to send.
    package var data: Data?
    package var size: UInt64
    /// Only the start of a large text file was read.
    package var truncated: Bool

    package init(path: String, text: String?, data: Data? = nil, size: UInt64? = nil, truncated: Bool = false) {
        self.path = path
        self.text = text
        self.data = data
        self.size = size ?? UInt64(text?.utf8.count ?? data?.count ?? 0)
        self.truncated = truncated
    }
}

/// One level of a folder on a host: folders first, then files.
package struct Folder: Equatable, Sendable {
    /// The absolute folder browsed from.
    package var root: String
    /// Under `root`, with "/" between folders; empty for the root itself.
    package var directory: String
    package var entries: [FolderEntry]
    /// There were more than are listed.
    package var truncated: Bool

    package init(root: String, directory: String, entries: [FolderEntry], truncated: Bool = false) {
        self.root = root
        self.directory = directory
        self.entries = entries
        self.truncated = truncated
    }
}

package struct FolderEntry: Equatable, Sendable {
    package var name: String
    package var isDirectory: Bool
    package var size: UInt64
    package var modified: Date

    package init(name: String, isDirectory: Bool, size: UInt64 = 0, modified: Date = Date(timeIntervalSince1970: 0)) {
        self.name = name
        self.isDirectory = isDirectory
        self.size = size
        self.modified = modified
    }
}

/// A port on a host, such as a dev server's, that the daemon listens for
/// on 127.0.0.1 here and carries over the host's connection.
package struct Forward: Equatable, Identifiable, Sendable {
    package var id: String
    /// Nil for this Mac.
    package var host: String?
    /// The port on the host.
    package var port: Int
    /// The port here to open, which is `port` when that was free.
    package var localPort: Int
    /// The session the server belongs to, if any.
    package var sessionID: String?

    package init(id: String, host: String?, port: Int, localPort: Int, sessionID: String? = nil) {
        self.id = id
        self.host = host
        self.port = port
        self.localPort = localPort
        self.sessionID = sessionID
    }

    /// Where the forwarded server opens here.
    package var url: URL? { URL(string: "http://127.0.0.1:\(localPort)") }
}

/// A shell on a host, in a session's folder or any of the host's. It keeps
/// running while nothing shows it, until it exits or is closed.
package struct Terminal: Equatable, Identifiable, Sendable {
    package var id: String
    /// Nil for this Mac.
    package var host: String?
    /// The session it was opened for, if any.
    package var sessionID: String?
    /// Where it started, on the host.
    package var folder: String
    package var shell: String
    package var opened: Date
    /// How the shell exited, once it has; -1 when a signal ended it.
    package var exitCode: Int?

    package init(
        id: String, host: String?, sessionID: String?, folder: String, shell: String, opened: Date,
        exitCode: Int? = nil
    ) {
        self.id = id
        self.host = host
        self.sessionID = sessionID
        self.folder = folder
        self.shell = shell
        self.opened = opened
        self.exitCode = exitCode
    }

    /// The folder's last part, to name the terminal by.
    package var title: String {
        let name = (folder as NSString).lastPathComponent
        return name.isEmpty ? folder : name
    }
}

/// A folder on a host to open a terminal in or browse: a session's folder
/// on its host, else `folder` on `host`, else the host's home.
package struct HostPlace: Equatable, Sendable {
    package var host: String?
    package var sessionID: String?
    package var folder: String?

    package init(host: String? = nil, sessionID: String? = nil, folder: String? = nil) {
        self.host = host
        self.sessionID = sessionID
        self.folder = folder
    }
}

package struct TerminalSize: Equatable, Sendable {
    package var rows: Int
    package var columns: Int

    package init(rows: Int, columns: Int) {
        self.rows = rows
        self.columns = columns
    }

    package static let standard = TerminalSize(rows: 24, columns: 80)
}

/// What a terminal printed.
package struct TerminalOutput: Equatable, Sendable {
    package var data: Data
    /// The data is the terminal's recent output from the start: what was
    /// shown goes first.
    package var replay: Bool
    /// Set once the shell has exited; nothing follows.
    package var exitCode: Int?

    package init(data: Data, replay: Bool = false, exitCode: Int? = nil) {
        self.data = data
        self.replay = replay
        self.exitCode = exitCode
    }
}
