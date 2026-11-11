import ComposableArchitecture
import Foundation

/// Khodr's daemon: sessions on this Mac and on the hosts it reaches.
@DependencyClient
package struct DaemonClient: Sendable {
    package var projects: @Sendable () async throws -> [Project]
    /// Adds a project whose repository is the folder `repository` on this Mac.
    package var createProject:
        @Sendable (_ name: String, _ repository: String, _ instructions: String) async throws -> Project
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
    package var fork: @Sendable (_ session: String, _ before: String?, _ name: String?) async throws -> Session
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
