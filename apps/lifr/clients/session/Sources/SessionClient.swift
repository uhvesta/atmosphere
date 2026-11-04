import ComposableArchitecture
import Foundation
import LifrAPI

/// An app that asked to use Lifr through the CLI, and what you allowed it.
package struct ConnectedApp: Identifiable, Hashable, Sendable {
    /// The launcher's code-signing identity, such as `com.anthropic.claude-code`.
    package var id: String
    package var grant: Grant
    package var lastUsedAt: Date?
    /// Whether it has a CLI connected right now. The menu bar shows a dot.
    package var isConnected: Bool

    package init(id: String, grant: Grant, lastUsedAt: Date? = nil, isConnected: Bool = false) {
        self.id = id
        self.grant = grant
        self.lastUsedAt = lastUsedAt
        self.isConnected = isConnected
    }

    package var name: String { grant.client }
}

/// An app the app doesn't know yet, waiting for you to approve it.
package struct ApprovalRequest: Identifiable, Hashable, Sendable {
    package var id: UUID
    /// The launcher's code-signing identity.
    package var identity: String
    /// Its name, from its code signature.
    package var client: String
    /// What its first request needs, so the sheet can say why it asked.
    package var method: APIMethod
    package var requestedAt: Date

    package init(id: UUID, identity: String, client: String, method: APIMethod, requestedAt: Date) {
        self.id = id
        self.identity = identity
        self.client = client
        self.method = method
        self.requestedAt = requestedAt
    }
}

/// Where `lifr` is linked on the PATH, if it is.
package enum CLIInstallation: Hashable, Sendable {
    case notInstalled
    case installed(path: String)
}

/// Something that changed outside the app's own actions.
package enum SessionEvent: Hashable, Sendable {
    /// An unknown app launched the CLI. Its request waits for an answer.
    case approvalRequested(ApprovalRequest)
    /// It gave up waiting, such as when the CLI exited.
    case approvalWithdrawn(UUID)
    /// Grants were used, connected or disconnected.
    case appsChanged([ConnectedApp])
}

/// Why a session change failed.
package enum SessionError: Error, Equatable, CustomStringConvertible {
    case unknownApp(String)
    case unknownRequest(UUID)
    /// Touch ID or the password was cancelled or failed.
    case notAuthenticated
    case installFailed(path: String, reason: String)

    package var description: String {
        switch self {
        case .unknownApp(let identity):
            "\(identity) isn't a connected app anymore. It may have been revoked on another window; reopen Settings."
        case .unknownRequest:
            "That request is gone, because the app that asked stopped waiting. Run its command again to ask anew."
        case .notAuthenticated:
            "Approving needs Touch ID or your password. Nothing was granted; try again."
        case .installFailed(let path, let reason):
            "Couldn't link lifr at \(path): \(reason). Check that the folder exists and you can write to it, "
                + "or add the app's bundled lifr to your PATH yourself."
        }
    }
}

/// Approvals, grants and the CLI install. Approving and changing grants ask
/// for Touch ID in the live version.
@DependencyClient
package struct SessionClient: Sendable {
    /// Every app you approved, most recently used first.
    package var apps: @Sendable () async throws -> [ConnectedApp]
    /// Requests and changes as they happen. Ends when the calling task is
    /// cancelled.
    package var events: @Sendable () async -> AsyncStream<SessionEvent> = { AsyncStream { $0.finish() } }
    /// Answers a request with what you allowed, after Touch ID.
    package var approve: @Sendable (_ request: UUID, _ grant: Grant) async throws -> Void
    package var deny: @Sendable (_ request: UUID) async throws -> Void
    /// Replaces an app's grant, such as Read and write or a renewed expiry,
    /// after Touch ID.
    package var update: @Sendable (_ app: String, _ grant: Grant) async throws -> Void
    /// Revokes an app's grant now. Its next request is refused.
    package var revoke: @Sendable (_ app: String) async throws -> Void
    package var cliInstallation: @Sendable () async -> CLIInstallation = { .notInstalled }
    /// Links the bundled `lifr` onto the PATH.
    package var installCLI: @Sendable () async throws -> CLIInstallation
}

extension Grant {
    /// What a new agent gets unless you change it: read-only, every Avenue,
    /// for 30 days.
    package static func agentDefault(client: String, now: Date) -> Grant {
        Grant(client: client, scopes: [.read], expiresAt: now + 30 * 24 * 60 * 60)
    }
}

extension SessionClient: TestDependencyKey {
    package static let testValue = SessionClient()
    package static let previewValue = InMemorySession(apps: [
        ConnectedApp(
            id: "com.anthropic.claude-code",
            grant: .agentDefault(client: "Claude Code", now: Date()),
            lastUsedAt: Date() - 3600, isConnected: true)
    ]).client
}

extension DependencyValues {
    package var session: SessionClient {
        get { self[SessionClient.self] }
        set { self[SessionClient.self] = newValue }
    }
}
