// Writes the API's golden files: the MCP tool list, an example request and
// response for every method, and an example of every error code. Each example
// runs against a fresh `Fixture`, so the output only changes when the
// protocol or the rules do.
//
// Usage: generate <mcp_tools.json> <methods.json> <errors.json> <plan.toml>

import Foundation
import LifrAPI
import LifrCore

func check(_ condition: Bool, _ message: @autoclosure () -> String) {
    if !condition {
        FileHandle.standardError.write(Data("generate: \(message())\n".utf8))
        exit(1)
    }
}

struct Example: Encodable {
    var request: JSONValue?
    var response: RPCResponse
}

struct MethodExample: Encodable {
    var method: String
    var scope: Scope?
    var request: JSONValue?
    var response: RPCResponse
}

/// One request against a fresh fixture, as an agent unless `grant` says otherwise.
func example(
    _ method: String, _ params: JSONValue = [:], grant: (Fixture) -> Grant? = { $0.agent },
    confirm: @escaping (Confirmation) -> Bool = { _ in true }
) throws -> Example {
    let fixture = try Fixture()
    let (request, response) = fixture.call(method, params, grant: grant(fixture), confirm: confirm)
    return Example(request: try JSONValue(request), response: response)
}

func example(line: String) throws -> Example {
    let fixture = try Fixture()
    return Example(request: .string(line), response: fixture.call(line: line, grant: fixture.agent))
}

let methodExamples: [APIMethod: Example] = [
    .hello: try example("hello", ["protocolVersion": 1, "client": "lifr 0.1.0"]),
    .unlock: try example("unlock", grant: { _ in nil }),
    .listAvenues: try example("list_avenues"),
    .listProjects: try example("list_projects", ["avenue": "HEALTH"]),
    .searchTasks: try example("search_tasks", ["query": "run", "avenue": "HEALTH"]),
    .getTask: try example("get_task", ["task": "HEALTH-7001"]),
    .createTask: try example(
        "create_task",
        [
            "title": "Stretch after runs", "parent": "HEALTH-7001",
            "priority": "low", "labels": ["running"],
        ]),
    .updateTask: try example("update_task", ["task": "HEALTH-7004", "dueDate": "2026-11-12", "dueTime": "09:30"]),
    .setStatus: try example("set_status", ["task": "HEALTH-7001", "status": "wontDo", "cascade": true]),
    .moveTask: try example("move_task", ["task": "HEALTH-7004", "parent": "HEALTH-7001"]),
    .addComment: try example("add_comment", ["task": "7002", "body": "Done in 1:12, felt **good**."]),
    .trashTask: try example("trash_task", ["task": "HEALTH-7001"]),
    .export: try example("export", grant: { $0.terminal }),
]
check(Set(methodExamples.keys) == Set(APIMethod.allCases), "every method needs an example")
let methods = APIMethod.allCases.map { method in
    let example = methodExamples[method]!
    check(example.response.error == nil, "\(method.rawValue)'s example failed: \(example.response.error!.message)")
    return MethodExample(
        method: method.rawValue, scope: method.scope, request: example.request,
        response: example.response)
}

let errorExamples: [APIErrorCode: Example] = [
    .parseError: try example(line: "list_avenues"),
    .invalidRequest: try example(line: #"{"id": 7, "method": "list_avenues"}"#),
    .methodNotFound: try example("delete_everything"),
    .invalidParams: try example("get_task", [:]),
    .versionMismatch: try example("hello", ["protocolVersion": 2, "client": "lifr 0.2.0"]),
    // The CLI answers these two itself, before reaching the app.
    .apiDisabled: Example(response: .failure(id: 1, .apiDisabled)),
    .appNotRunning: Example(response: .failure(id: 1, .appNotRunning)),
    .notAuthorized: try example(
        "add_comment", ["task": "7004", "body": "Booked for Friday."],
        grant: { $0.readOnlyAgent }),
    .notFound: try example("get_task", ["task": "FINANCE-7006"]),
    .openSubtasks: try example("set_status", ["task": "HEALTH-7001", "status": "done"]),
    .confirmationRequired: try example("set_status", ["task": "HEALTH-7001", "status": "wontDo"]),
    .confirmationDenied: try example("trash_task", ["task": "7004"], confirm: { _ in false }),
    .ruleViolation: try example("update_task", ["task": "7004", "dueTime": "09:30"]),
    .internalError: Example(
        response: .failure(
            id: 1,
            APIError(
                .internalError,
                "Lifr hit an unexpected error: (the error). Nothing changed. Retry, and report it if it keeps "
                    + "happening."
            ))),
]
check(Set(errorExamples.keys) == Set(APIErrorCode.allCases), "every error code needs an example")
var errors: [String: Example] = [:]
for (code, example) in errorExamples {
    check(
        example.response.error?.data.code == code,
        "the \(code.rawValue) example answered \(example.response.error?.data.code.rawValue ?? "success")")
    errors[code.rawValue] = example
}

let paths = Array(CommandLine.arguments.dropFirst())
check(paths.count == 4, "usage: generate <mcp_tools.json> <methods.json> <errors.json> <plan.toml>")
let encoder = Wire.encoder
encoder.outputFormatting.insert(.prettyPrinted)
func write(_ value: some Encodable, to path: String) throws {
    var data = try encoder.encode(value)
    data.append(UInt8(ascii: "\n"))
    try data.write(to: URL(fileURLWithPath: path))
}
try write(MCPTool.all, to: paths[0])
try write(methods, to: paths[1])
try write(errors, to: paths[2])
try Data(PlanFile.write(examplePlan).utf8).write(to: URL(fileURLWithPath: paths[3]))
