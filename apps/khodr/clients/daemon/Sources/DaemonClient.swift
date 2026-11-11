import ComposableArchitecture
import Foundation

/// Khodr's daemon: sessions on this Mac and on the hosts it reaches.
@DependencyClient
package struct DaemonClient: Sendable {
    package var projects: @Sendable () async throws -> [Project]
    /// Adds a project that works in `repositories`, folders on this Mac.
    package var createProject:
        @Sendable (_ name: String, _ repositories: [Repository], _ instructions: String) async throws -> Project
    /// Every host's sessions, or only the project's.
    package var sessions: @Sendable (_ project: String?) async throws -> [Session]
    /// Starts a session, idle until the first message.
    package var start: @Sendable (_ request: StartRequest) async throws -> Session
    package var send: @Sendable (_ session: String, _ text: String, _ mode: SendMode) async throws -> Void
    /// Answers a permission request.
    package var approve: @Sendable (_ session: String, _ request: String, _ approved: Bool) async throws -> Void
    /// Sets what the session may do without asking; waiting requests the
    /// new mode allows go ahead.
    package var setApprovalMode: @Sendable (_ session: String, _ mode: ApprovalMode) async throws -> Void
    /// Stops the running turn.
    package var abort: @Sendable (_ session: String) async throws -> Void
    /// A new session with the history before `before`, or all of it if nil.
    /// It gets its own copy of the code as it was there, unless `chatOnly`,
    /// when it works in the same copy as the session.
    package var fork:
        @Sendable (_ session: String, _ before: String?, _ name: String?, _ chatOnly: Bool) async throws -> Session
    /// Renames, archives or unarchives, or marks the session as the keeper.
    package var update: @Sendable (_ session: String, _ update: SessionUpdate) async throws -> Session
    /// What the session changed in each repository since it began.
    package var changes: @Sendable (_ session: String) async throws -> [RepositoryChanges]
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
    package var history: @Sendable (_ session: String) async throws -> [SessionEvent]
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
