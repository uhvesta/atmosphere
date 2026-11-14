import Foundation
import LifrAPI
import LifrCore
import TestingKit

func accessTests() -> [TestCase] {
    [
        TestCase("given no grant, then reads are refused but hello answers") {
            let fixture = try Fixture()
            try expectEqual(fixture.call("list_avenues", grant: nil).response.code, .notAuthorized)
            let hello = try fixture.call("hello", ["protocolVersion": 1, "client": "lifr"], grant: nil).response
                .value(Hello.Result.self)
            try expectEqual(hello.scopes, [])
        },
        TestCase("given a read-only grant, then writes are refused and nothing changes") {
            let fixture = try Fixture()
            let before = fixture.workspace
            let response = fixture.call("add_comment", ["task": "7004", "body": "Hi"], grant: fixture.readOnlyAgent)
                .response
            try expectEqual(response.code, .notAuthorized)
            try expectEqual(fixture.workspace, before)
        },
        TestCase("given an expired grant, then every scoped request is refused") {
            let fixture = try Fixture()
            var grant = fixture.agent
            grant.expiresAt = fixture.now
            try expectEqual(fixture.call("list_avenues", grant: grant).response.code, .notAuthorized)
        },
        TestCase("given a hidden Avenue, then agents can't list, search or get its tasks") {
            let fixture = try Fixture()
            let avenues = try fixture.call("list_avenues", grant: fixture.agent).response.value([AvenueSummary].self)
            try expectEqual(avenues.map(\.key), ["HEALTH", "WORK"])
            let found = try fixture.call("search_tasks", ["query": "taxes"], grant: fixture.agent).response
                .value([TaskSummary].self)
            try expectEqual(found, [])
            try expectEqual(
                fixture.call("get_task", ["task": "FINANCE-7006"], grant: fixture.agent).response.code,
                .notFound)
            try expectEqual(
                fixture.call("list_projects", ["avenue": "FINANCE"], grant: fixture.agent).response.code,
                .notFound)
        },
        TestCase("given a hidden Avenue, then a terminal session still sees it") {
            let fixture = try Fixture()
            let task = try fixture.call("get_task", ["task": "FINANCE-7006"], grant: fixture.terminal).response
                .value(TaskDetail.self)
            try expectEqual(task.task.title, "File taxes")
        },
        TestCase("given a grant for some Avenues, then the others are out of sight") {
            let fixture = try Fixture()
            var grant = fixture.agent
            grant.avenueIDs = [try fixture.task("WORK-7005").avenueID]
            let avenues = try fixture.call("list_avenues", grant: grant).response.value([AvenueSummary].self)
            try expectEqual(avenues.map(\.key), ["WORK"])
            try expectEqual(fixture.call("get_task", ["task": "HEALTH-7004"], grant: grant).response.code, .notFound)
        },
        TestCase("given a read-only Avenue, then agents read its tasks but can't change them") {
            let fixture = try Fixture()
            _ = try fixture.call("get_task", ["task": "WORK-7005"], grant: fixture.agent).response
                .value(TaskDetail.self)
            let response = fixture.call("set_status", ["task": "WORK-7005", "status": "done"], grant: fixture.agent)
                .response
            try expectEqual(response.code, .notAuthorized)
            try expectEqual(try fixture.task("WORK-7005").status, .todo)
        },
        TestCase("given a grant that confirms writes, then a declined write changes nothing") {
            let fixture = try Fixture()
            var grant = fixture.agent
            grant.confirmWrites = true
            var asked: [Confirmation] = []
            let response = fixture.call("set_status", ["task": "7004", "status": "done"], grant: grant) {
                asked.append($0)
                return false
            }.response
            try expectEqual(response.code, .confirmationDenied)
            try expectEqual(asked, [.write(.setStatus, task: "7004")])
            try expectEqual(try fixture.task("7004").status, .todo)
        },
        TestCase("given a grant that doesn't confirm writes, then trash still asks first") {
            let fixture = try Fixture()
            var asked: [Confirmation] = []
            _ = try fixture.call("trash_task", ["task": "7001"], grant: fixture.agent) {
                asked.append($0)
                return true
            }.response.value(TrashTask.Result.self)
            try expectEqual(asked, [.trash(["HEALTH-7001", "HEALTH-7002", "HEALTH-7003"])])
            try expect(try fixture.task("7002").deletedAt != nil, "Sub tasks go to the trash with their parent")
        },
        TestCase("given a task in the trash, then an agent can't empty the trash and the task stays") {
            let fixture = try Fixture()
            _ = try fixture.call("trash_task", ["task": "7004"], grant: fixture.agent).response
                .value(TrashTask.Result.self)
            for method in ["empty_trash", "purge_trash", "delete_task"] {
                try expectEqual(
                    fixture.call(method, ["task": "7004"], grant: fixture.agent).response.code, .methodNotFound)
            }
            try expect(try fixture.task("7004").deletedAt != nil, "the task is still in the trash")
        },
        TestCase("given an agent grant, then changes are recorded as the agent") {
            let fixture = try Fixture()
            _ = try fixture.call("add_comment", ["task": "7004", "body": "Booked."], grant: fixture.agent).response
                .value(AddComment.Result.self)
            let author = fixture.workspace.comments.values.first { $0.body == "Booked." }?.author
            try expectEqual(author, .agent("Claude Code", device: fixture.device, surface: .cli))
        },
        TestCase("given a terminal session, then changes are recorded as the person") {
            let fixture = try Fixture()
            _ = try fixture.call("add_comment", ["task": "7004", "body": "Booked."], grant: fixture.terminal).response
                .value(AddComment.Result.self)
            let author = fixture.workspace.comments.values.first { $0.body == "Booked." }?.author
            try expectEqual(author, .user(device: fixture.device, surface: .cli))
        },
        TestCase("given an agent, then export is refused; given a terminal session, it needs Touch ID") {
            let fixture = try Fixture()
            try expectEqual(fixture.call("export", grant: fixture.agent).response.code, .notAuthorized)
            try expectEqual(
                fixture.call("export", grant: fixture.terminal) { _ in false }.response.code,
                .confirmationDenied)
            let export = try fixture.call("export", grant: fixture.terminal).response.value(Export.self)
            try expectEqual(export.tasks.count, 6)
        },
    ]
}
