import LifrAPI
import LifrCLI
import XCTest

/// `--json` prints what the API's golden files say each method answers, and
/// each command sends the golden request's params.
final class GoldenTests: XCTestCase {
    private func grant(for method: String) -> (Fixture) -> Grant? {
        switch method {
        case "unlock": { _ in nil }
        case "export": { $0.terminal }
        default: { $0.agent }
        }
    }

    func testEachCommandAnswersItsMethodsGoldenResult() throws {
        // hello is part of connecting, not a command.
        let commands: [String: [String]] = [
            "unlock": ["unlock"],
            "list_avenues": ["avenues"],
            "list_projects": ["projects", "HEALTH"],
            "search_tasks": ["search", "run", "--avenue", "HEALTH"],
            "get_task": ["show", "HEALTH-7001"],
            "create_task": [
                "add", "Stretch after runs", "--parent", "HEALTH-7001", "--priority", "low", "--label", "running",
            ],
            "update_task": ["edit", "HEALTH-7004", "--due", "2026-11-12", "--time", "09:30"],
            "set_status": ["status", "HEALTH-7001", "wont-do", "--cascade"],
            "move_task": ["move", "HEALTH-7004", "--parent", "HEALTH-7001"],
            "add_comment": ["comment", "7002", "Done in 1:12, felt **good**."],
            "trash_task": ["trash", "HEALTH-7001"],
            "add_plan": ["add", "--plan", "plan.toml"],
            "export": ["export"],
        ]
        let examples = try golden("methods").array
        XCTAssertEqual(
            Set(examples.compactMap { if case .string(let method)? = $0["method"] { method } else { nil } }),
            Set(commands.keys).union(["hello"]))

        for example in examples {
            guard case .string(let method)? = example["method"], let arguments = commands[method] else { continue }
            let connection = try FixtureConnection(grant: grant(for: method))
            if case .string(let plan)? = example["request"]?["params"]?["plan"] { connection.files["plan.toml"] = plan }
            let run = connection.run(arguments + ["--json"])

            XCTAssertEqual(run.status, 0, "\(method): \(run.output)")
            XCTAssertEqual(connection.requests, [example["request"]?["params"] ?? [:]], method)
            XCTAssertEqual(try json(run.output), example["response"]?["result"], method)
        }
    }

    func testErrorsPrintTheGoldenError() throws {
        let cases: [(code: String, arguments: [String], grant: (Fixture) -> Grant?)] = [
            ("notAuthorized", ["comment", "7004", "Booked for Friday."], { $0.readOnlyAgent }),
            ("notFound", ["show", "FINANCE-7006"], { $0.agent }),
            ("openSubtasks", ["status", "HEALTH-7001", "done"], { $0.agent }),
            ("confirmationRequired", ["status", "HEALTH-7001", "wont-do"], { $0.agent }),
            ("ruleViolation", ["edit", "7004", "--time", "09:30"], { $0.agent }),
        ]
        let errors = try golden("errors")
        for (code, arguments, grant) in cases {
            let run = try FixtureConnection(grant: grant).run(arguments + ["--json"])

            XCTAssertEqual(run.status, 1, code)
            XCTAssertEqual(try json(run.output), ["error": errors[code]?["response"]?["error"] ?? .null], code)
        }
    }
}
