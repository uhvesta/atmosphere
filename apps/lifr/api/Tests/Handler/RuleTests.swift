import Foundation
import LifrAPI
import LifrCore
import LifrTesting

func ruleTests() -> [TestCase] {
    [
        TestCase("given open Sub tasks, then Done is refused with their codes") {
            let fixture = try Fixture()
            let error = fixture.call("set_status", ["task": "7001", "status": "done"], grant: fixture.agent)
                .response.error?.data
            try expectEqual(error?.code, .openSubtasks)
            try expectEqual(error?.tasks, ["HEALTH-7002", "HEALTH-7003"])
        },
        TestCase("given open Sub tasks, then Won't Do needs cascade, and with it closes them too") {
            let fixture = try Fixture()
            try expectEqual(
                fixture.call("set_status", ["task": "7001", "status": "wontDo"], grant: fixture.agent)
                    .response.code, .confirmationRequired)
            let result = try fixture.call(
                "set_status", ["task": "7001", "status": "wontDo", "cascade": true],
                grant: fixture.agent
            ).response.value(ChangeResult.self)
            try expectEqual(result.task.status, .wontDo)
            try expectEqual(result.alsoChanged.map(\.code).sorted(), ["HEALTH-7002", "HEALTH-7003"])
            try expectEqual(try fixture.task("7002").status, .wontDo)
        },
        TestCase("given a closed parent, then a new Sub task reopens it and the result says so") {
            let fixture = try Fixture()
            for code in ["7002", "7003", "7001"] {
                _ = try fixture.call("set_status", ["task": .string(code), "status": "done"], grant: fixture.agent)
                    .response.value(ChangeResult.self)
            }
            let result = try fixture.call(
                "create_task", ["title": "Cool down", "parent": "7001"],
                grant: fixture.agent
            ).response.value(ChangeResult.self)
            try expectEqual(result.reopenedParent, "HEALTH-7001")
            try expectEqual(try fixture.task("7001").status, .inProgress)
        },
        TestCase("given an In Progress task idle for two weeks, then searches see it as Procrastinated") {
            let fixture = try Fixture()
            let found = try fixture.call("search_tasks", ["statuses": ["procrastinated"]], grant: fixture.agent)
                .response.value([TaskSummary].self)
            try expectEqual(found.map(\.code), ["HEALTH-7001"])
            try expectEqual(found.first?.procrastination, "inactive 14 days")
        },
        TestCase("given a rule from Core, then the error keeps Core's fix-it message") {
            let fixture = try Fixture()
            let error = fixture.call("update_task", ["task": "7004", "dueTime": "09:30"], grant: fixture.agent)
                .response.error
            try expectEqual(error?.data.code, .ruleViolation)
            try expectEqual(error?.message, LifrError.dueTimeWithoutDueDate.description)
        },
        TestCase("given a task, then a Sub task can't move to another Avenue on its own") {
            let fixture = try Fixture()
            try expectEqual(
                fixture.call("move_task", ["task": "7002", "avenue": "HEALTH"], grant: fixture.agent)
                    .response.code, .ruleViolation)
        },
    ]
}
