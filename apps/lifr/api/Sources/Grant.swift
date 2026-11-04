import Foundation
import LifrCore

/// What a client the person approved may do. Approval, Touch ID and the
/// Keychain token live in the app; this is the part every request checks.
package struct Grant: Hashable, Codable, Sendable {
    /// The approved client's name, from its code signature, such as "Claude Code".
    package var client: String
    package var scopes: Set<Scope>
    /// The Avenues it may see, or `nil` for all of them. Hidden Avenues are
    /// never visible, whatever this says.
    package var avenueIDs: Set<UUID>?
    package var expiresAt: Date
    /// Ask the person before each write.
    package var confirmWrites: Bool
    /// A Touch ID terminal session (`lifr unlock`) acts as the person, not as
    /// an agent.
    package var isTerminalSession: Bool

    package init(
        client: String, scopes: Set<Scope>, avenueIDs: Set<UUID>? = nil, expiresAt: Date,
        confirmWrites: Bool = false, isTerminalSession: Bool = false
    ) {
        self.client = client
        self.scopes = scopes
        self.avenueIDs = avenueIDs
        self.expiresAt = expiresAt
        self.confirmWrites = confirmWrites
        self.isTerminalSession = isTerminalSession
    }

    /// Who history and comments say made the change.
    package func actor(device: String) -> Actor {
        isTerminalSession ? .user(device: device) : .agent(client, device: device)
    }
}
