import Foundation
import LifrAPI
import LifrCore
import TestingKit

private func prompt(goal: String = "Plan my week", scope: PlanPrompt.Scope = .names) throws -> String {
    let fixture = try Fixture()
    return PlanPrompt.make(
        goal: goal, scope: scope, workspace: fixture.workspace, today: try CalendarDate("2026-11-03"),
        timeZone: TimeZone(identifier: "Europe/London") ?? .gmt)
}

/// The text between the prompt's first ```toml fence and the fence after it.
private func example(in prompt: String) throws -> String {
    let parts = prompt.components(separatedBy: "```toml\n")
    let body = try parts.dropFirst().first.unwrap()
    return try body.components(separatedBy: "```").first.unwrap()
}

func planPromptTests() -> [TestCase] {
    let bare = """
        # Lifr plan v1

        [[task]]
        title = "Book the van"
        in = "HEALTH"
        """
    return [
        TestCase("given a workspace, then the prompt names the Avenues an assistant may add to and no others") {
            let text = try prompt()
            try expect(text.contains("HEALTH: Health"))
            try expect(text.contains("Half marathon"))
            try expect(text.contains("running"))
            try expect(!text.contains("WORK"))
            try expect(!text.contains("FINANCE"))
        },
        TestCase("given a workspace, then the prompt holds no task") {
            let text = try prompt()
            try expect(!text.contains("Training plan"))
            try expect(!text.contains("Book a checkup"))
        },
        TestCase("given everything in scope, then the prompt lists the open tasks an assistant may add to") {
            let text = try prompt(scope: .everything)
            try expect(text.contains("HEALTH: Training plan"))
            try expect(text.contains("HEALTH: Book a checkup"))
            try expect(!text.contains("Quarterly review"))
            try expect(!text.contains("File taxes"))
            try expect(!text.contains("Ask about knee pain."))
        },
        TestCase("given one Avenue in scope, then the prompt lists its open tasks only") {
            let fixture = try Fixture()
            let health = try fixture.workspace.avenues.values.first { $0.key.rawValue == "HEALTH" }.unwrap()
            let text = PlanPrompt.make(
                goal: "", scope: .avenue(health.id), workspace: fixture.workspace,
                today: try CalendarDate("2026-11-03"), timeZone: .gmt)
            try expect(text.contains("HEALTH: Book a checkup"))
            try expect(!text.contains("Quarterly review"))
        },
        TestCase("given one Project in scope, then the prompt lists its tasks only") {
            let fixture = try Fixture()
            let marathon = try fixture.workspace.projects.values.first { $0.name == "Half marathon" }.unwrap()
            let text = PlanPrompt.make(
                goal: "", scope: .project(marathon.id), workspace: fixture.workspace,
                today: try CalendarDate("2026-11-03"), timeZone: .gmt)
            try expect(text.contains("HEALTH: Training plan"))
            try expect(!text.contains("Book a checkup"))
        },
        TestCase("given descriptions asked for, then the prompt lists each task's description") {
            let fixture = try Fixture()
            let text = PlanPrompt.make(
                goal: "", scope: .everything, listing: PlanPrompt.Listing(descriptions: true),
                workspace: fixture.workspace, today: try CalendarDate("2026-11-03"), timeZone: .gmt)
            try expect(text.contains("Ask about knee pain."))
        },
        TestCase("given an Avenue hidden from assistants in scope, then the prompt lists none of its tasks") {
            let fixture = try Fixture()
            let finance = try fixture.workspace.avenues.values.first { $0.key.rawValue == "FINANCE" }.unwrap()
            let text = PlanPrompt.make(
                goal: "", scope: .avenue(finance.id), workspace: fixture.workspace,
                today: try CalendarDate("2026-11-03"), timeZone: .gmt)
            try expect(!text.contains("File taxes"))
        },
        TestCase("given extra instructions, then the prompt carries them") {
            let fixture = try Fixture()
            let text = PlanPrompt.make(
                goal: "Errands", instructions: "Use the Errands label.", workspace: fixture.workspace,
                today: try CalendarDate("2026-11-03"), timeZone: .gmt)
            try expect(text.contains("Use the Errands label."))
        },
        TestCase("given a workspace, then the prompt has the date, the time zone and the goal") {
            let text = try prompt(goal: "Get ready for the race")
            try expect(text.contains("2026-11-03"))
            try expect(text.contains("Europe/London"))
            try expect(text.contains("Get ready for the race"))
        },
        TestCase("given the prompt's example, then it reads as a plan in the Avenue listed") {
            let plan = try PlanFile.read(try example(in: try prompt()))
            try expect(plan.avenues.isEmpty)
            try expectEqual(plan.projects.map(\.avenue), ["HEALTH"])
            try expectEqual(plan.tasks.map(\.place), ["HEALTH"])
        },
        TestCase("given no Avenue to add to, then the example adds one and reads back") {
            let text = PlanPrompt.make(
                goal: "", workspace: Workspace(), today: try CalendarDate("2026-11-03"), timeZone: .gmt)
            let plan = try PlanFile.read(try example(in: text))
            try expectEqual(plan.avenues.map(\.key), ["HOME"])
        },
        TestCase("given a reply with words around a fence, then it reads the same as the bare plan") {
            let reply = "Here's your plan:\n\n```toml\n\(bare)\n```\n\nWant me to add reminders?"
            try expectEqual(try PlanPrompt.read(reply), try PlanFile.read(bare))
        },
        TestCase("given a reply with words before a bare plan, then it reads the same as the bare plan") {
            try expectEqual(try PlanPrompt.read("Sure!\n\n\(bare)"), try PlanFile.read(bare))
        },
        TestCase("given a fence without tables before the plan's, then the plan's fence is read") {
            let reply = "Run this:\n```\nopen lifr\n```\n```toml\n\(bare)\n```"
            try expectEqual(try PlanPrompt.read(reply), try PlanFile.read(bare))
        },
        TestCase("given a malformed reply, then the error names its line in the plan") {
            let reply = "```toml\n# Lifr plan v1\n[[task]]\ntitel = \"Van\"\n```"
            try expectError(
                PlanFileError(line: 3, problem: .unknownKey("titel", table: "task", suggestion: "title"))
            ) { try PlanPrompt.read(reply) }
        },
    ]
}
