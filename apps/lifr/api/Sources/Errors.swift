import Foundation
import LifrCore

/// Stable error codes. Clients branch on these, never on messages.
package enum APIErrorCode: String, Hashable, Codable, Sendable, CaseIterable {
    case parseError, invalidRequest, methodNotFound, invalidParams
    case versionMismatch
    case apiDisabled
    case appNotRunning
    case notAuthorized
    case notFound
    case openSubtasks
    /// The change needs the person's OK first, such as `cascade: true`.
    case confirmationRequired
    case confirmationDenied
    /// Any other rule from Core, such as a title that's too long.
    case ruleViolation
    case internalError

    /// The JSON-RPC error number. The standard ones keep their numbers;
    /// Lifr's own are in the server range.
    package var number: Int {
        switch self {
        case .parseError: -32700
        case .invalidRequest: -32600
        case .methodNotFound: -32601
        case .invalidParams: -32602
        case .internalError: -32603
        case .versionMismatch: -32000
        case .apiDisabled: -32001
        case .appNotRunning: -32002
        case .notAuthorized: -32003
        case .notFound: -32004
        case .openSubtasks: -32005
        case .confirmationRequired: -32006
        case .confirmationDenied: -32007
        case .ruleViolation: -32008
        }
    }
}

/// Why the API refused a request. `message` says how to fix it.
package struct APIError: Error, Hashable, Codable, Sendable, CustomStringConvertible {
    package var code: APIErrorCode
    package var message: String
    /// Task codes the error is about, such as the open Sub tasks.
    package var tasks: [String]?

    package init(_ code: APIErrorCode, _ message: String, tasks: [String]? = nil) {
        self.code = code
        self.message = message
        self.tasks = tasks
    }

    package var description: String { message }

    package static func versionMismatch(client: Int, app: Int) -> APIError {
        let newer = client > app ? "the Lifr app" : "lifr"
        return APIError(
            .versionMismatch,
            "lifr speaks protocol \(client) and the app speaks \(app). Update \(newer) so both come from the same "
                + "Lifr version; the CLI inside Lifr.app always matches the app."
        )
    }

    package static let appNotRunning = APIError(
        .appNotRunning,
        "The Lifr app didn't start within 10 seconds. Open Lifr on this Mac, or turn on Launch at login in Settings, "
            + "then retry."
    )

    package static let apiDisabled = APIError(
        .apiDisabled,
        "The local API is turned off. Turn it on from Lifr's menu bar item, then retry.")

    package static func notAuthorized(_ reason: String) -> APIError {
        APIError(.notAuthorized, reason)
    }

    package static func methodNotFound(_ method: String) -> APIError {
        APIError(
            .methodNotFound,
            "\"\(method)\" isn't a Lifr method. Update lifr if it's older than the app; `lifr --help` lists the "
                + "commands."
        )
    }

    package static func invalidParams(_ method: String, _ detail: String) -> APIError {
        APIError(.invalidParams, "Invalid params for \(method): \(detail)")
    }

    package static let confirmationDenied = APIError(
        .confirmationDenied,
        "The change was declined in the Lifr app. Nothing changed; ask the person before trying again.")
}

extension RPCError {
    init(_ error: APIError) {
        self.init(code: error.code.number, message: error.message, data: error)
    }
}

/// The `error` member of a JSON-RPC response.
package struct RPCError: Hashable, Codable, Sendable {
    package var code: Int
    package var message: String
    package var data: APIError
}
