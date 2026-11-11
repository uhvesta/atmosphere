import Foundation

/// What runs a session's agent.
package enum Backend: String, CaseIterable, Hashable, Sendable {
    case scripted
    case local
    case copilot
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

    package init(
        id: String, name: String, backend: Backend, workingDirectory: String, model: String? = nil,
        state: SessionState = .idle, parentID: String? = nil, forkedBeforeEventID: String? = nil, host: String? = nil
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
    }
}

package enum PermissionKind: Hashable, Sendable {
    case shell
    case write
    case read
    case other
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
        case permissionCompleted(requestID: String, approved: Bool)
        case turnEnded(aborted: Bool)
        case idle
        case failed(String)
        case usage(model: String, inputTokens: UInt64, outputTokens: UInt64, premiumRequests: Double)
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
    package var workingDirectory: String
    package var name: String
    /// Nil for this Mac.
    package var host: String?
    package var model: String?
    package var instructions: String?
    /// Settings only the backend reads, as JSON; the scripted backend's script.
    package var backendConfig: String?

    package init(
        backend: Backend, workingDirectory: String, name: String = "", host: String? = nil, model: String? = nil,
        instructions: String? = nil, backendConfig: String? = nil
    ) {
        self.backend = backend
        self.workingDirectory = workingDirectory
        self.name = name
        self.host = host
        self.model = model
        self.instructions = instructions
        self.backendConfig = backendConfig
    }
}

package enum SendMode: Hashable, Sendable {
    /// Start a turn now, or after the running one.
    case enqueue
    /// Steer the running turn.
    case steer
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
    case unknown
}

package enum DaemonError: Error, Hashable, CustomStringConvertible {
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

    package var description: String {
        switch self {
        case .refused(_, let message):
            message
        case .notRunning(let socket, let reason):
            "Khodr's daemon isn't running at \(socket) (\(reason)). Start it with `khodrd daemon`, then try again."
        case .disconnected:
            "The connection to Khodr's daemon closed before it answered. Check that it is still running, then "
                + "retry; the request may or may not have been carried out."
        case .versionMismatch(let daemon):
            "Khodr's daemon is version \(daemon) and speaks another protocol. Update Khodr so both are the "
                + "same version."
        case .unexpected(let what):
            "Khodr's daemon sent \(what), which this app doesn't expect. Update Khodr so both are the same version."
        }
    }
}
