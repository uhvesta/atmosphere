import ComposableArchitecture
import Foundation

/// Humata's daemon: sessions on this Mac and on the hosts it reaches.
@DependencyClient
package struct DaemonClient: Sendable {
    package var projects: @Sendable () async throws -> [Project]
    /// Adds a project that works in `repositories`, folders on this Mac.
    package var createProject:
        @Sendable (_ name: String, _ repositories: [Repository], _ instructions: String) async throws -> Project
    /// Removes a project once every session in it is archived; its sessions
    /// are kept, in no project.
    package var deleteProject: @Sendable (_ project: String) async throws -> Void
    /// Renames the project, or changes its instructions or skills, for the
    /// turns its sessions start after.
    package var updateProject: @Sendable (_ project: String, _ update: ProjectUpdate) async throws -> Project
    /// Every host's sessions, or only the project's.
    package var sessions: @Sendable (_ project: String?) async throws -> [Session]
    /// Starts a session, idle until the first message.
    package var start: @Sendable (_ request: StartRequest) async throws -> Session
    /// The Claude Code and Copilot sessions on `host`, or this Mac if nil,
    /// that were started outside Humata, newest first.
    package var importable: @Sendable (_ host: String?) async throws -> [ImportableSession]
    /// A new session carrying on `session`, with its history, in its folder.
    package var importSession:
        @Sendable (_ session: ImportableSession, _ host: String?, _ projectID: String?) async throws -> Session
    /// The models `backend` offers on `host`, or this Mac if nil; with a
    /// model server in `backendConfig`, that server's. Throws when they
    /// can't be listed, and a typed model name still works then.
    package var models:
        @Sendable (_ host: String?, _ backend: Backend, _ backendConfig: String?) async throws -> [ModelChoice]
    /// The sessions on `host`, or this Mac if nil, whose files couldn't be
    /// read, so they were left as they are.
    package var skipped: @Sendable (_ host: String?) async throws -> [SkippedSession]
    /// Deletes a skipped session's folder, with its history.
    package var removeSkipped: @Sendable (_ session: SkippedSession, _ host: String?) async throws -> Void
    package var send: @Sendable (_ session: String, _ text: String, _ mode: SendMode) async throws -> Void
    /// Sends `text` with `images`. A steer with images goes next in the
    /// queue instead, as a running turn can't take them.
    package var sendImages:
        @Sendable (_ session: String, _ text: String, _ images: [ImageAttachment], _ mode: SendMode) async throws
            -> Void
    /// The image `name` a user message names.
    package var readImage: @Sendable (_ session: String, _ name: String) async throws -> ImageAttachment
    /// Answers a permission request.
    package var approve: @Sendable (_ session: String, _ request: String, _ approved: Bool) async throws -> Void
    /// Sets what the session may do without asking; waiting requests the
    /// new mode allows go ahead.
    package var setApprovalMode: @Sendable (_ session: String, _ mode: ApprovalMode) async throws -> Void
    /// Stops the running turn.
    package var abort: @Sendable (_ session: String) async throws -> Void
    /// Stops the turn, drops what's queued and cuts the chat back to
    /// before the person's message `before`; with `files`, the code too.
    package var rewind: @Sendable (_ session: String, _ before: String, _ files: Bool) async throws -> Void
    /// Undoes the last rewind, unless a message was sent since.
    package var undoRewind: @Sendable (_ session: String) async throws -> Void
    /// Takes a queued message out before its turn starts.
    package var unqueue: @Sendable (_ session: String, _ queueID: String) async throws -> Void
    /// Changes a queued message's text before its turn starts; refused
    /// with `.unknownQueuedMessage` once it has started or was taken out.
    package var editQueued: @Sendable (_ session: String, _ queueID: String, _ text: String) async throws -> Void
    /// A new session with the history before `before`, or all of it if nil.
    /// It gets its own copy of the code as it was there, unless `chatOnly`,
    /// when it works in the same copy as the session.
    package var fork:
        @Sendable (_ session: String, _ before: String?, _ name: String?, _ chatOnly: Bool) async throws -> Session
    /// A working session on `item` carrying on the planning session
    /// `session`, with all of its history and its own copy of the code.
    package var startWork: @Sendable (_ session: String, _ item: String) async throws -> Session
    /// What the session's agent proposed, oldest first, with what the
    /// person did about each.
    package var proposals: @Sendable (_ session: String) async throws -> [Proposal]
    /// What agents outside Humata proposed for the project's board through
    /// `humata mcp`, oldest first; apply or decline them with no session.
    package var outsideProposals: @Sendable (_ project: String) async throws -> [Proposal]
    /// Adds the proposed task, or changes the item's spec, with `title` or
    /// `spec` instead of the proposed ones when given. `session` is empty
    /// for an outside agent's proposal.
    package var applyProposal:
        @Sendable (_ session: String, _ proposal: String, _ title: String?, _ spec: String?) async throws -> Item
    package var declineProposal: @Sendable (_ session: String, _ proposal: String) async throws -> Void
    /// Renames, archives or unarchives, or marks the session as the keeper.
    package var update: @Sendable (_ session: String, _ update: SessionUpdate) async throws -> Session
    /// Deletes an archived session for good, with its folders, refs and
    /// branch. Refused unless it's archived, while a fork of it exists, or
    /// while its host can't be reached.
    package var delete: @Sendable (_ session: String) async throws -> Void
    /// Records that the person has seen the session's first `turns` finished
    /// turns, so its reply leaves the inbox until another turn ends.
    package var markSeen: @Sendable (_ session: String, _ turns: Int) async throws -> Void
    /// What the session changed in each repository over the query's range.
    package var changes: @Sendable (_ session: String, _ query: ChangesQuery) async throws -> [RepositoryChanges]
    /// Stages, unstages or reverts a file or hunk in the session's copy.
    package var editChanges: @Sendable (_ session: String, _ edit: ChangeEdit) async throws -> Void
    /// Marks a file viewed with the contents `blob` names, or not viewed.
    package var markViewed:
        @Sendable (_ session: String, _ file: FileRef, _ blob: String, _ viewed: Bool) async throws -> Void
    package var createThread: @Sendable (_ session: String, _ thread: NewThread) async throws -> ReviewThread
    /// Adds a comment, sent with the next review.
    package var replyToThread: @Sendable (_ thread: String, _ text: String) async throws -> ReviewThread
    package var resolveThread: @Sendable (_ thread: String, _ resolved: Bool) async throws -> ReviewThread
    /// Deletes a comment not sent yet; a thread left without comments goes too.
    package var deleteComment: @Sendable (_ comment: String) async throws -> Void
    /// The session's threads, where their lines are now, with the agent's replies.
    package var threads: @Sendable (_ session: String) async throws -> [ReviewThread]
    /// Sends every comment not sent yet as one message; returns how many.
    package var sendReview: @Sendable (_ session: String) async throws -> Int
    /// The files in the session's folder.
    package var files: @Sendable (_ session: String) async throws -> ChatFiles
    /// A file in the session's folder, by its path from `files`.
    package var readFile: @Sendable (_ session: String, _ path: String) async throws -> FileContents
    /// The versions of a file in the session's folder that its turns left,
    /// oldest first; none for a file outside its repositories.
    package var fileVersions: @Sendable (_ session: String, _ path: String) async throws -> [FileVersion]
    /// A file in the session's folder as checkpoint `turn` had it.
    package var readFileVersion: @Sendable (_ session: String, _ path: String, _ turn: Int) async throws -> FileContents
    /// Writes a version over the file, as a change of its own; returns the
    /// file as it is now.
    package var restoreFileVersion:
        @Sendable (_ session: String, _ path: String, _ turn: Int) async throws -> FileContents
    /// One level of `directory`, under the folder `place` names.
    package var listFolder: @Sendable (_ place: HostPlace, _ directory: String) async throws -> Folder
    /// A file under the folder `place` names, by its path from `listFolder`.
    package var readFileAt: @Sendable (_ place: HostPlace, _ path: String) async throws -> FileContents
    /// The project's board, its items' statuses as shown: tasks by
    /// priority, then by when they were added, each followed by its subtasks.
    package var items: @Sendable (_ project: String) async throws -> [Item]
    /// Adds a task to `project`, or a subtask under `fields.parentID`.
    package var createItem: @Sendable (_ project: String, _ fields: ItemFields) async throws -> Item
    package var updateItem: @Sendable (_ item: String, _ update: ItemUpdate) async throws -> Item
    /// Removes a task or subtask; Board History brings it back. Refused while
    /// it has subtasks or unarchived sessions.
    package var deleteItem: @Sendable (_ item: String) async throws -> Void
    /// The project's board changes, newest first.
    package var boardLog: @Sendable (_ project: String) async throws -> [BoardChange]
    /// Puts an item back as it was before the change, keeping each field
    /// changed again since.
    package var revert: @Sendable (_ change: UInt64) async throws -> Reverted
    /// Every session waiting on you: approvals, then failures, then replies;
    /// and the admin actions other clients asked for.
    package var inbox: @Sendable () async throws -> Inbox
    /// The admin approvals kept, newest first, and the grants in force.
    package var approvals: @Sendable () async throws -> Approvals
    /// Approves or denies pending admin actions; approving can grant
    /// `scopes` for `seconds` too, to the actions after. Returns those decided.
    package var decideApprovals:
        @Sendable (_ ids: [String], _ approve: Bool, _ scopes: [ApprovalScope], _ seconds: UInt32) async throws ->
            [AdminApproval]
    package var revokeGrant: @Sendable (_ grant: String) async throws -> Void
    /// What humatad's files take on this Mac, and the cleanup settings.
    package var storage: @Sendable () async throws -> Storage
    /// Turns the daily cleanup on or off, keeping archived sessions `days`.
    package var setStorageSettings: @Sendable (_ autoDelete: Bool, _ days: Int) async throws -> Storage
    /// Starts a cleanup in the background, doing the parts of `cleanup`
    /// asked for. `storage` shows how far it is.
    package var cleanUpStorage: @Sendable (_ cleanup: Storage.Cleanup) async throws -> Storage
    /// Every skill on the Mac and each host that answers, with the clashes
    /// among those one session would get.
    package var skills: @Sendable () async throws -> SkillList
    /// Picks which of the skills named `name` wins, for every project and
    /// host, while the files are `candidates`, by hash.
    package var pickSkill: @Sendable (_ name: String, _ candidates: [String], _ winner: String) async throws -> Void
    /// Every Claude Code and Copilot plugin on the Mac and each host that
    /// answers.
    package var plugins: @Sendable () async throws -> PluginList
    /// Turns a plugin on or off for a project's sessions; nil goes back to
    /// how its CLI has it.
    package var setProjectPlugin:
        @Sendable (_ projectID: String, _ agent: Plugin.Agent, _ id: String, _ enabled: Bool?) async throws -> Project
    /// What New Session starts from in every project.
    package var sessionDefaults: @Sendable () async throws -> SessionDefaults
    /// Sets what New Session starts from in every project, with `projectID`
    /// nil, or in one, over every project's; nil takes a project's out.
    /// Returns the project, for one.
    package var setSessionDefaults:
        @Sendable (_ projectID: String?, _ defaults: SessionDefaults?) async throws -> Project?
    /// Answers an agent's question or move; it leaves the inbox.
    package var answerRequest: @Sendable (_ request: String, _ answer: RequestAnswer) async throws -> Void
    package var history: @Sendable (_ session: String) async throws -> [SessionEvent]
    package var hosts: @Sendable () async throws -> [RemoteHost]
    /// The coordinator, the Mac the daemon runs on, as a host with no name:
    /// the variables for its agents. `updateHost` with "" changes them; it
    /// takes no setup script.
    package var coordinator: @Sendable () async throws -> RemoteHost
    /// Adds a host Humata reaches with `ssh destination` and installs itself on.
    package var addHost: @Sendable (_ name: String, _ destination: String) async throws -> RemoteHost
    package var updateHost: @Sendable (_ host: String, _ update: HostUpdate) async throws -> RemoteHost
    /// Forgets the host and its sessions; nothing on it is removed.
    package var removeHost: @Sendable (_ host: String) async throws -> Void
    /// Connects to the host, updating Humata's program there first if it
    /// is of this Mac's kind, and reports how it is.
    package var checkHost: @Sendable (_ host: String) async throws -> HostHealth
    /// Copies Humata's program to the host; returns its version.
    package var installHost: @Sendable (_ host: String) async throws -> String
    /// Links this Mac's commands into `~/.local/bin`, or with `check` only
    /// says how they're linked. Never replaces what isn't such a link.
    package var linkCommands: @Sendable (_ check: Bool) async throws -> CommandLinks
    /// Points local-model sessions, on this Mac and its hosts, at the app's
    /// model server on 127.0.0.1:`port`.
    package var setModelServer: @Sendable (_ port: UInt16) async throws -> Void
    /// The hosts named in ~/.ssh/config, to add one by.
    package var sshHosts: @Sendable () async throws -> [String]
    /// The Mac's copies of the sessions on hosts, kept after each turn.
    package var recoveries: @Sendable () async throws -> [Recovery]
    /// Sets the session up again from its copy on `host`, or this Mac if
    /// nil: its own host if that lost it, or another, which it runs on
    /// from then on.
    package var restoreSession: @Sendable (_ session: String, _ host: String?) async throws -> Session
    /// Sets up again every session the host lost.
    package var restoreHost: @Sendable (_ host: String) async throws -> HostRestore
    /// Opens a shell at `place`, `size` big.
    package var openTerminal: @Sendable (_ place: HostPlace, _ size: TerminalSize) async throws -> Terminal
    /// The terminals on `host`, or this Mac if nil, oldest first.
    package var terminals: @Sendable (_ host: String?) async throws -> [Terminal]
    /// Types `data` into the terminal: keys as the terminal sends them.
    package var typeInTerminal: @Sendable (_ terminal: Terminal, _ data: Data) async throws -> Void
    package var resizeTerminal: @Sendable (_ terminal: Terminal, _ size: TerminalSize) async throws -> Void
    /// Ends the terminal's shell, if it still runs, and forgets it.
    package var closeTerminal: @Sendable (_ terminal: Terminal) async throws -> Void
    /// The terminal's recent output, then what it prints, until it exits.
    /// Ends when the calling task is cancelled; throws if the connection
    /// drops, after which attaching again replays what was missed.
    package var attachTerminal:
        @Sendable (_ terminal: Terminal) async -> AsyncThrowingStream<TerminalOutput, any Error> = { _ in
            AsyncThrowingStream { $0.finish() }
        }
    /// Listens on 127.0.0.1 here for `port` on `host`, or this Mac if nil,
    /// once something answers on it there.
    package var forwardPort: @Sendable (_ host: String?, _ port: Int, _ session: String?) async throws -> Forward
    /// The ports being forwarded.
    package var forwards: @Sendable () async throws -> [Forward]
    /// Stops listening; connections already open run until they close.
    package var stopForward: @Sendable (_ forward: String) async throws -> Void
    /// What every session said and did, on every host, that has the
    /// query's words; best first, then those found by related words.
    package var search: @Sendable (_ query: SearchQuery) async throws -> SearchResults
    /// Commits the session's work onto a new branch in each repository it
    /// changed; pushes them, and opens or finds their pull requests, if asked.
    package var land: @Sendable (_ session: String, _ request: LandRequest) async throws -> Landed
    /// The pull requests of a session or a task, read again from GitHub
    /// first with `refresh`.
    package var pullRequests: @Sendable (_ of: PullRequestsOf, _ refresh: Bool) async throws -> [PullRequest]
    /// Merges each repository's base into the session's copy; on conflicts,
    /// the agent is asked to resolve them.
    package var updateFromBase: @Sendable (_ session: String) async throws -> BaseUpdate
    /// Makes a task of each open GitHub issue of the project's repositories
    /// not imported before.
    package var importIssues: @Sendable (_ project: String) async throws -> ImportedIssues
    /// The session's history, then each new event. Ends when the calling task
    /// is cancelled; throws if the connection drops.
    package var watch: @Sendable (_ session: String) async -> AsyncThrowingStream<SessionEvent, any Error> = { _ in
        AsyncThrowingStream { $0.finish() }
    }
}

extension DaemonClient: TestDependencyKey {
    package static let testValue = DaemonClient()
    package static let previewValue = InMemoryDaemon().client
}

extension DependencyValues {
    package var daemon: DaemonClient {
        get { self[DaemonClient.self] }
        set { self[DaemonClient.self] = newValue }
    }
}
