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
    /// Answers a permission request.
    package var approve: @Sendable (_ session: String, _ request: String, _ approved: Bool) async throws -> Void
    /// Sets what the session may do without asking; waiting requests the
    /// new mode allows go ahead.
    package var setApprovalMode: @Sendable (_ session: String, _ mode: ApprovalMode) async throws -> Void
    /// Stops the running turn.
    package var abort: @Sendable (_ session: String) async throws -> Void
    /// Takes a queued message out before its turn starts.
    package var unqueue: @Sendable (_ session: String, _ queueID: String) async throws -> Void
    /// A new session with the history before `before`, or all of it if nil.
    /// It gets its own copy of the code as it was there, unless `chatOnly`,
    /// when it works in the same copy as the session.
    package var fork:
        @Sendable (_ session: String, _ before: String?, _ name: String?, _ chatOnly: Bool) async throws -> Session
    /// Renames, archives or unarchives, or marks the session as the keeper.
    package var update: @Sendable (_ session: String, _ update: SessionUpdate) async throws -> Session
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
    /// The project's board, its items' statuses as shown: tasks by
    /// priority, then by when they were added, each followed by its subtasks.
    package var items: @Sendable (_ project: String) async throws -> [Item]
    /// Adds a task to `project`, or a subtask under `fields.parentID`.
    package var createItem: @Sendable (_ project: String, _ fields: ItemFields) async throws -> Item
    package var updateItem: @Sendable (_ item: String, _ update: ItemUpdate) async throws -> Item
    /// The project's board changes, newest first.
    package var boardLog: @Sendable (_ project: String) async throws -> [BoardChange]
    /// Puts an item back as it was before the change; nil when that removed it.
    package var revert: @Sendable (_ change: UInt64) async throws -> Item?
    /// Every session waiting on you: approvals, then failures, then replies.
    package var inbox: @Sendable () async throws -> [InboxEntry]
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
    /// The hosts named in ~/.ssh/config, to add one by.
    package var sshHosts: @Sendable () async throws -> [String]
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
