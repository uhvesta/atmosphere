import Foundation
import LifrAPI
import LifrCore
import LifrTesting

private func plan(_ body: String) -> JSONValue {
    ["plan": .string("# Lifr plan v1\n" + body), "timeZone": "UTC"]
}

func addPlanTests() -> [TestCase] {
    [
        TestCase("given a plan with a task and Sub task, then both are added under it") {
            let fixture = try Fixture()
            let result = try fixture.call(
                "add_plan", plan("[[task]]\ntitle = \"A\"\nin = \"HEALTH\"\n  [[task.subtask]]\n  title = \"B\""),
                grant: fixture.agent
            ).response.value(AddPlan.Result.self)
            try expectEqual(result.tasks.map(\.title), ["A", "B"])
            try expectEqual(result.tasks[1].parent, result.tasks[0].code)
        },
        TestCase("given TOML that can't be read, then invalidParams and nothing changes") {
            let fixture = try Fixture()
            let before = fixture.workspace
            let response = fixture.call("add_plan", plan("[[task]]\ntitle = \"A"), grant: fixture.agent).response
            try expectEqual(response.code, .invalidParams)
            try expectEqual(fixture.workspace, before)
        },
        TestCase("given a plan where a later task breaks a rule, then nothing is added") {
            let fixture = try Fixture()
            let before = fixture.workspace
            let response = fixture.call(
                "add_plan",
                plan("[[task]]\ntitle = \"A\"\nin = \"HEALTH\"\n[[task]]\ntitle = \"B\"\nin = \"HEALTH/Nope\""),
                grant: fixture.agent
            ).response
            try expectEqual(response.code, .ruleViolation)
            try expectEqual(fixture.workspace, before)
        },
        TestCase("given an agent placing tasks in a hidden Avenue, then notFound, as if it didn't exist") {
            let fixture = try Fixture()
            let hidden = fixture.call(
                "add_plan", plan("[[task]]\ntitle = \"A\"\nin = \"FINANCE\""), grant: fixture.agent)
            let missing = fixture.call("add_plan", plan("[[task]]\ntitle = \"A\"\nin = \"NOPE\""), grant: fixture.agent)
            try expectEqual(hidden.response.code, .notFound)
            try expectEqual(missing.response.code, .notFound)
        },
        TestCase("given an agent adding a Sub task under a hidden task, then notFound") {
            let fixture = try Fixture()
            let response = fixture.call(
                "add_plan", plan("[[task]]\ntitle = \"A\"\nparent = \"FINANCE-7006\""), grant: fixture.agent
            ).response
            try expectEqual(response.code, .notFound)
        },
        TestCase("given an agent adding to a read-only Avenue, then notAuthorized") {
            let fixture = try Fixture()
            let response = fixture.call(
                "add_plan", plan("[[project]]\nname = \"P\"\navenue = \"WORK\""), grant: fixture.agent
            ).response
            try expectEqual(response.code, .notAuthorized)
        },
        TestCase("given an agent creating an Avenue and placing tasks in it, then both are added") {
            let fixture = try Fixture()
            let result = try fixture.call(
                "add_plan",
                plan("[[avenue]]\nkey = \"HOME\"\nname = \"Home\"\n[[task]]\ntitle = \"A\"\nin = \"home\""),
                grant: fixture.agent
            ).response.value(AddPlan.Result.self)
            try expectEqual(result.avenues, ["HOME"])
            try expectEqual(result.tasks.map(\.avenue), ["HOME"])
        },
        TestCase("given a grant limited to some Avenues, then it can't create new ones") {
            let fixture = try Fixture()
            var grant = fixture.agent
            grant.avenueIDs = Set(fixture.workspace.avenues.values.filter { $0.key.rawValue == "HEALTH" }.map(\.id))
            let response = fixture.call(
                "add_plan", plan("[[avenue]]\nkey = \"HOME\"\nname = \"Home\""), grant: grant
            ).response
            try expectEqual(response.code, .notAuthorized)
        },
        TestCase("given a time zone that doesn't exist, then invalidParams") {
            let fixture = try Fixture()
            let response = fixture.call(
                "add_plan", ["plan": "[[task]]\ntitle = \"A\"\nin = \"HEALTH\"", "timeZone": "Mars/Base"],
                grant: fixture.agent
            ).response
            try expectEqual(response.code, .invalidParams)
        },
        TestCase("given a grant that confirms writes and a denial, then nothing is added") {
            let fixture = try Fixture()
            var grant = fixture.agent
            grant.confirmWrites = true
            let before = fixture.workspace
            let response = fixture.call(
                "add_plan", plan("[[task]]\ntitle = \"A\"\nin = \"HEALTH\""), grant: grant, confirm: { _ in false }
            ).response
            try expectEqual(response.code, .confirmationDenied)
            try expectEqual(fixture.workspace, before)
        },
    ]
}
