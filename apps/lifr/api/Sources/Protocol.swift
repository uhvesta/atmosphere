import Foundation

/// The local API between `lifr` and the Mac app: JSON-RPC 2.0, one message
/// per line, over a Unix socket in the App Group container.
package enum LifrProtocol {
    /// Bumped on any change a client of the previous version can't handle.
    /// `hello` compares it before anything else.
    package static let version = 1
}

/// What a request needs from the caller's grant.
package enum Scope: String, Hashable, Codable, Sendable, CaseIterable, Comparable {
    case read, write

    package static func < (lhs: Self, rhs: Self) -> Bool { lhs.rawValue < rhs.rawValue }
}

/// Every operation of the local API. The CLI's commands and the MCP tools
/// map one to one onto these.
package enum APIMethod: String, Hashable, Codable, Sendable, CaseIterable {
    /// Starts a connection: compares protocol versions and reports the grant.
    case hello
    /// Opens a Touch ID session for the calling terminal.
    case unlock
    case listAvenues = "list_avenues"
    case listProjects = "list_projects"
    case searchTasks = "search_tasks"
    case getTask = "get_task"
    case createTask = "create_task"
    case updateTask = "update_task"
    case setStatus = "set_status"
    case moveTask = "move_task"
    case addComment = "add_comment"
    /// Always confirmed in the app, even when the grant doesn't confirm writes.
    case trashTask = "trash_task"
    /// Every record as JSON, for backups. Needs Touch ID; never offered to agents.
    case export

    /// The grant scope the method needs. `nil` for methods that come before
    /// a grant is checked or that the app gates with Touch ID itself.
    package var scope: Scope? {
        switch self {
        case .hello, .unlock, .export: nil
        case .listAvenues, .listProjects, .searchTasks, .getTask: .read
        case .createTask, .updateTask, .setStatus, .moveTask, .addComment, .trashTask: .write
        }
    }
}

/// A request as it travels on the wire.
package struct RPCRequest: Hashable, Codable, Sendable {
    package var jsonrpc = "2.0"
    package var id: Int
    package var method: String
    package var params: JSONValue?

    package init(id: Int, method: APIMethod, params: some Encodable) throws {
        self.id = id
        self.method = method.rawValue
        self.params = try JSONValue(params)
    }

    package init(id: Int, method: String, params: JSONValue?) {
        self.id = id
        self.method = method
        self.params = params
    }
}

/// A response: exactly one of `result` and `error`.
package struct RPCResponse: Hashable, Codable, Sendable {
    package var jsonrpc = "2.0"
    /// `nil` only when the request was too broken to read its id.
    package var id: Int?
    package var result: JSONValue?
    package var error: RPCError?

    package static func success(id: Int, _ result: some Encodable) throws -> RPCResponse {
        RPCResponse(id: id, result: try JSONValue(result), error: nil)
    }

    package static func failure(id: Int?, _ error: APIError) -> RPCResponse {
        RPCResponse(id: id, result: nil, error: RPCError(error))
    }
}
