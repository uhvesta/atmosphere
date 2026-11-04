import Foundation
import LifrAPI
import LifrCore

/// `lifr mcp`: an MCP server over stdio, one JSON-RPC message per line. Each
/// tool call is one request to the app, so MCP clients get exactly what the
/// CLI gets.
package final class MCPServer {
    /// The MCP versions this server speaks, newest first.
    static let protocolVersions = ["2025-06-18", "2025-03-26", "2024-11-05"]

    private let connect: () throws(APIError) -> any Connection
    private var connection: (any Connection)?

    /// `connect` runs on the first tool call, so a client can list the
    /// tools while the app isn't running.
    package init(connect: @escaping () throws(APIError) -> any Connection) {
        self.connect = connect
    }

    /// The reply to one line from the client, or `nil` for a notification.
    package func reply(to line: String) -> String? {
        guard let message = try? Wire.decoder.decode(JSONValue.self, from: Data(line.utf8)),
            case .object(let fields) = message
        else {
            return encode(["jsonrpc": "2.0", "id": .null, "error": error(-32700, "Send one JSON-RPC object per line.")])
        }
        // Notifications, such as notifications/initialized, have no id and
        // get no reply.
        guard let id = fields["id"] else { return nil }
        guard case .string(let method)? = fields["method"] else {
            return encode(["jsonrpc": "2.0", "id": id, "error": error(-32600, "The request has no method.")])
        }
        let params: [String: JSONValue] =
            if case .object(let params)? = fields["params"] { params } else { [:] }
        let result: JSONValue
        switch method {
        case "initialize":
            result = initialize(params)
        case "ping":
            result = [:]
        case "tools/list":
            result = [
                "tools": .array(
                    MCPTool.all.map { tool in
                        [
                            "name": .string(tool.name), "description": .string(tool.description),
                            "inputSchema": tool.inputSchema,
                        ]
                    })
            ]
        case "tools/call":
            guard case .string(let name)? = params["name"], let tool = APIMethod(rawValue: name), tool.scope != nil
            else {
                return encode([
                    "jsonrpc": "2.0", "id": id,
                    "error": error(-32602, "There's no such tool. tools/list names every tool."),
                ])
            }
            result = call(tool, params["arguments"] ?? [:])
        default:
            return encode([
                "jsonrpc": "2.0", "id": id, "error": error(-32601, "lifr mcp doesn't handle \(method)."),
            ])
        }
        return encode(["jsonrpc": "2.0", "id": id, "result": result])
    }

    private func initialize(_ params: [String: JSONValue]) -> JSONValue {
        var version = Self.protocolVersions[0]
        if case .string(let asked)? = params["protocolVersion"], Self.protocolVersions.contains(asked) {
            version = asked
        }
        return [
            "protocolVersion": .string(version),
            "capabilities": ["tools": [:]],
            "serverInfo": ["name": "lifr", "version": .string(CLI.version)],
            "instructions": """
            Lifr is a task manager. Tasks live in Avenues (areas of life, by key such as HEALTH) and may have \
            Sub tasks. Name tasks by their code, such as HEALTH-7K2Q. Some Avenues are read only or hidden for \
            you; the person chooses that in the Lifr app.
            """,
        ]
    }

    /// A tool's result, or its error as a tool error the model can read.
    private func call(_ method: APIMethod, _ arguments: JSONValue) -> JSONValue {
        do throws(APIError) {
            if connection == nil {
                let new = try connect()
                new.surface = .mcp
                connection = new
            }
            let result = try connection!.call(method, arguments)
            return ["content": [["type": "text", "text": .string((try? Output.json(result)) ?? "null")]]]
        } catch {
            if error.code == .appNotRunning { connection = nil }
            var text = "\(error.code.rawValue): \(error.message)"
            if let tasks = error.tasks { text += " Tasks: \(tasks.joined(separator: ", "))." }
            return ["content": [["type": "text", "text": .string(text)]], "isError": true]
        }
    }

    private func error(_ code: Int, _ message: String) -> JSONValue {
        ["code": .number(Double(code)), "message": .string(message)]
    }

    private func encode(_ value: JSONValue) -> String {
        (try? Wire.encoder.encode(value)).flatMap { String(bytes: $0, encoding: .utf8) } ?? "{}"
    }
}
