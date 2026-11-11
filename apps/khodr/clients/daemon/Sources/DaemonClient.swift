import ComposableArchitecture
import Foundation

/// Khodr's daemon: sessions on this Mac and on the hosts it reaches.
@DependencyClient
package struct DaemonClient: Sendable {
    /// Every host's sessions.
    package var sessions: @Sendable () async throws -> [Session]
    /// Starts a session, idle until the first message.
    package var start: @Sendable (_ request: StartRequest) async throws -> Session
    package var send: @Sendable (_ session: String, _ text: String, _ mode: SendMode) async throws -> Void
    /// Answers a permission request.
    package var approve: @Sendable (_ session: String, _ request: String, _ approved: Bool) async throws -> Void
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
