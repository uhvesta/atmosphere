import LifrAPI
import LifrCLI
import XCTest

/// `lifr mcp` as an MCP client such as Claude Code drives it.
final class MCPTests: XCTestCase {
    private func reply(_ server: MCPServer, _ message: JSONValue) throws -> JSONValue? {
        let line = try XCTUnwrap(String(bytes: try Wire.encoder.encode(message), encoding: .utf8))
        return try server.reply(to: line).map(json)
    }

    func testInitializeAgreesOnAVersionTheClientAskedFor() throws {
        let server = MCPServer { () throws(APIError) in throw .appNotRunning }
        let result = try reply(
            server,
            [
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": [
                    "protocolVersion": "2025-03-26", "capabilities": [:], "clientInfo": ["name": "claude-code"],
                ],
            ])?["result"]

        XCTAssertEqual(result?["protocolVersion"], "2025-03-26")
        XCTAssertEqual(result?["capabilities"], ["tools": [:]])
        XCTAssertNil(try reply(server, ["jsonrpc": "2.0", "method": "notifications/initialized"]))
    }

    func testToolsAreTheGoldenToolsAndListWithoutTheApp() throws {
        let server = MCPServer { () throws(APIError) in throw .appNotRunning }
        let tools = try reply(server, ["jsonrpc": "2.0", "id": 2, "method": "tools/list"])?["result"]?["tools"]

        let expected = try golden("mcp_tools").array.map { tool -> JSONValue in
            [
                "name": try XCTUnwrap(tool["name"]), "description": try XCTUnwrap(tool["description"]),
                "inputSchema": try XCTUnwrap(tool["inputSchema"]),
            ]
        }
        XCTAssertEqual(tools?.array, expected)
    }

    func testAToolCallAnswersWhatTheMethodDoes() throws {
        let connection = try FixtureConnection()
        let server = MCPServer { connection }
        let result = try reply(
            server,
            [
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": ["name": "search_tasks", "arguments": ["query": "run", "avenue": "HEALTH"]],
            ])?["result"]

        XCTAssertNil(result?["isError"])
        guard case .string(let text)? = result?["content"]?.array.first?["text"] else {
            return XCTFail("no text content")
        }
        let search = try golden("methods").array.first { $0["method"] == "search_tasks" }
        XCTAssertEqual(try json(text), search?["response"]?["result"])
    }

    func testARefusedToolCallIsAToolErrorTheModelCanRead() throws {
        let connection = try FixtureConnection { $0.readOnlyAgent }
        let server = MCPServer { connection }
        let result = try reply(
            server,
            [
                "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": ["name": "add_comment", "arguments": ["task": "7004", "body": "Booked"]],
            ])?["result"]

        XCTAssertEqual(result?["isError"], true)
        guard case .string(let text)? = result?["content"]?.array.first?["text"] else {
            return XCTFail("no text content")
        }
        XCTAssertTrue(text.hasPrefix("notAuthorized: "), text)
    }

    func testOnlyToolsCanBeCalled() throws {
        let connection = try FixtureConnection()
        let server = MCPServer { connection }
        let reply = try reply(
            server,
            ["jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": ["name": "export", "arguments": [:]]])

        XCTAssertEqual(reply?["error"]?["code"], -32602)
        XCTAssertEqual(connection.requests, [])
    }
}
