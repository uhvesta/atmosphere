import ComposableArchitecture
import Foundation
import LifrAPI
import LifrCore

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
    /// The launcher runs scripts, such as `python3`: every script it runs
    /// shares its identity, so the sheet warns and suggests a short expiry.
    package var isInterpreter: Bool

    package init(
        id: UUID, identity: String, client: String, method: APIMethod, requestedAt: Date, isInterpreter: Bool = false
    ) {
        self.id = id
        self.identity = identity
        self.client = client
        self.method = method
        self.requestedAt = requestedAt
        self.isInterpreter = isInterpreter
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
    /// A connected app changed the records. The store's own updates only
    /// cover other devices. `number` is where its save comes among the
    /// store's saves and updates, as the store's `Saved.number`.
    case recordsChanged(Workspace, number: Int)
    /// The local API was turned on or off, here or from another window.
    case apiEnabledChanged(Bool)
}

/// Why a session change failed.
package enum SessionError: Error, Equatable, CustomStringConvertible {
    case unknownApp(String)
    case unknownRequest(UUID)
    /// Touch ID or the password was cancelled or failed.
    case notAuthenticated
    case installFailed(path: String, reason: String)
    /// The Keychain refused to keep the grants.
    case grantsNotSaved(reason: String)
    /// Turning the local API on or off failed.
    case apiSwitchFailed(reason: String)
    /// macOS refused to add or remove the login item.
    case loginItemFailed(reason: String)

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
        case .grantsNotSaved(let reason):
            "Couldn't keep the grant in the Keychain: \(reason). Nothing changed. Unlock the login keychain in "
                + "Keychain Access, then try again."
        case .apiSwitchFailed(let reason):
            "Couldn't switch the local API: \(reason). Quit and reopen Lifr, then try again."
        case .loginItemFailed(let reason):
            "macOS didn't change Launch at login: \(reason). Check System Settings > General > Login Items, "
                + "where Lifr may need to be allowed."
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
    /// Whether the CLI can connect at all.
    package var isAPIEnabled: @Sendable () async -> Bool = { true }
    /// Turning it off disconnects every app and tells the CLI it's off.
    package var setAPIEnabled: @Sendable (_ enabled: Bool) async throws -> Void
    /// Whether macOS opens Lifr at login, in the menu bar with no window.
    package var launchesAtLogin: @Sendable () async -> Bool = { false }
    package var setLaunchesAtLogin: @Sendable (_ enabled: Bool) async throws -> Void
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

/// Its description is written for the person, so alerts show it.
extension SessionError: LocalizedError {
    package var errorDescription: String? { description }
}
