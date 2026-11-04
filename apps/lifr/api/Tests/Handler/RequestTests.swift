import Foundation
import LifrAPI
import LifrCore
import LifrTesting

func requestTests() -> [TestCase] {
    [
        TestCase("given another protocol version, then hello refuses before anything else") {
            let fixture = try Fixture()
            let error = fixture.call("hello", ["protocolVersion": 2, "client": "lifr"], grant: fixture.agent)
                .response.error?.data
            try expectEqual(error?.code, .versionMismatch)
        },
        TestCase("given an unknown method, then methodNotFound") {
            let fixture = try Fixture()
            try expectEqual(fixture.call("delete_everything", grant: fixture.agent).response.code, .methodNotFound)
        },
        TestCase("given missing or malformed params, then invalidParams and nothing changes") {
            let fixture = try Fixture()
            let before = fixture.workspace
            try expectEqual(fixture.call("get_task", grant: fixture.agent).response.code, .invalidParams)
            try expectEqual(fixture.call("get_task", ["task": "NOT A CODE"], grant: fixture.agent).response.code,
                            .invalidParams)
            try expectEqual(fixture.call("update_task", ["task": "7004", "dueDate": "2026-02-30"],
                                         grant: fixture.agent).response.code, .invalidParams)
            try expectEqual(fixture.call("set_status", ["task": "7004", "status": "finished"],
                                         grant: fixture.agent).response.code, .invalidParams)
            try expectEqual(fixture.call("create_task", ["title": "No home"], grant: fixture.agent).response.code,
                            .invalidParams)
            try expectEqual(fixture.workspace, before)
        },
        TestCase("given an unknown label, then the task isn't created") {
            let fixture = try Fixture()
            try expectEqual(fixture.call("create_task", ["title": "Swim", "avenue": "HEALTH", "labels": ["pool"]],
                                         grant: fixture.agent).response.code, .notFound)
        },
        TestCase("given move_task, then exactly one destination is required") {
            let fixture = try Fixture()
            for params: JSONValue in [["task": "7004"], ["task": "7004", "avenue": "HEALTH", "topLevel": true]] {
                try expectEqual(fixture.call("move_task", params, grant: fixture.agent).response.code, .invalidParams)
            }
            let moved = try fixture.call("move_task", ["task": "7002", "topLevel": true], grant: fixture.agent)
                .response.value(ChangeResult.self)
            try expectEqual(moved.task.parent, nil)
        },
        TestCase("given update_task with clear, then those fields are unset") {
            let fixture = try Fixture()
            let result = try fixture.call("update_task", ["task": "7001", "clear": ["dueDate"]], grant: fixture.agent)
                .response.value(ChangeResult.self)
            try expectEqual(result.task.dueDate, nil)
        },
        TestCase("given a search limit, then at most that many tasks come back, closed ones only when asked") {
            let fixture = try Fixture()
            let limited = try fixture.call("search_tasks", ["limit": 1], grant: fixture.agent).response
                .value([TaskSummary].self)
            try expectEqual(limited.count, 1)
            _ = try fixture.call("set_status", ["task": "7004", "status": "done"], grant: fixture.agent).response
                .value(ChangeResult.self)
            let open = try fixture.call("search_tasks", ["query": "checkup"], grant: fixture.agent).response
                .value([TaskSummary].self)
            try expectEqual(open, [])
            let all = try fixture.call("search_tasks", ["query": "checkup", "includeClosed": true],
                                       grant: fixture.agent).response.value([TaskSummary].self)
            try expectEqual(all.map(\.code), ["HEALTH-7004"])
        },
        TestCase("given get_task, then the timeline has comments and history, oldest first") {
            let fixture = try Fixture()
            let detail = try fixture.call("get_task", ["task": "7001"], grant: fixture.agent).response
                .value(TaskDetail.self)
            try expectEqual(detail.subtasks.map(\.code), ["HEALTH-7002", "HEALTH-7003"])
            try expectEqual(detail.timeline.map(\.at), detail.timeline.map(\.at).sorted())
            try expect(detail.timeline.contains { $0.kind == .comment && $0.body == "Start with 3 runs a week." },
                       "the comment is in the timeline")
        },
        TestCase("given the MCP tool list, then it offers every scoped method and nothing else") {
            let tools = MCPTool.all.map(\.name)
            try expectEqual(Set(tools), Set(APIMethod.allCases.filter { $0.scope != nil }.map(\.rawValue)))
        },
    ]
}
