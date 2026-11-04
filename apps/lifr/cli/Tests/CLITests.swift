import InlineSnapshotTesting
import LifrAPI
import LifrCLI
import XCTest

/// What people see: each command's output against the API's fixture.
final class CLITests: XCTestCase {
    func testAvenuesSayWhichAreLimitedForAgents() throws {
        let connection = try FixtureConnection { $0.terminal }
        assertInlineSnapshot(of: describe(connection.run("avenues")), as: .lines) {
            """
            exit 0
            HEALTH   Health
            WORK     Work  (read only for agents)
            FINANCE  Finance  (hidden from agents)

            """
        }
    }

    func testSearchListsTheTasksWithTheirStatusAndDueDate() throws {
        assertInlineSnapshot(of: describe(try FixtureConnection().run("search", "--avenue", "HEALTH")), as: .lines) {
            """
            exit 0
            HEALTH-7001  Procrastinated  Training plan  !high  due 2026-11-10  (2 open Sub tasks)
            HEALTH-7002  To Do           Long run, 12 km  due 2026-11-08
            HEALTH-7003  To Do           Buy shoes
            HEALTH-7004  To Do           Book a checkup

            """
        }
    }

    func testShowGivesTheTaskWithItsSubtasksAndTimeline() throws {
        assertInlineSnapshot(of: describe(try FixtureConnection().run("show", "HEALTH-7001")), as: .lines) {
            """
            exit 0
            HEALTH-7001  Training plan
            Procrastinated · HEALTH · priority high · due 2026-11-10 · #running

            Sub tasks:
            HEALTH-7002  To Do  Long run, 12 km  due 2026-11-08
            HEALTH-7003  To Do  Buy shoes

            Timeline:
            2026-10-20 00:00  Avesta's Mac: Created
            2026-10-20 00:00  Avesta's Mac: Added Sub task HEALTH-7002
            2026-10-20 00:00  Avesta's Mac: Added Sub task HEALTH-7003
            2026-10-20 00:00  Avesta's Mac: Commented
            2026-10-20 00:00  Avesta's Mac: Start with 3 runs a week.

            """
        }
    }

    func testAddingASubtaskSaysWhatWasCreated() throws {
        assertInlineSnapshot(
            of: describe(try FixtureConnection().run("add", "Stretch", "after", "runs", "--parent", "HEALTH-7001")),
            as: .lines
        ) {
            """
            exit 0
            Created HEALTH-7007: Stretch after runs, To Do.
            Also changed:
            HEALTH-7001  In Progress  Training plan  !high  due 2026-11-10  (3 open Sub tasks)

            """
        }
    }

    func testClosingATaskWithOpenSubtasksNamesThem() throws {
        assertInlineSnapshot(of: describe(try FixtureConnection().run("status", "HEALTH-7001", "done")), as: .lines) {
            """
            exit 1
            stderr:
            lifr: This task has 2 open Sub task(s). Close them, or mark the task Won't Do to drop them too.
            Tasks: HEALTH-7002, HEALTH-7003

            """
        }
    }

    // swiftlint:disable line_length
    func testAReadOnlyAgentIsToldHowToGetWriteAccess() throws {
        let connection = try FixtureConnection { $0.readOnlyAgent }
        assertInlineSnapshot(of: describe(connection.run("comment", "7004", "Booked")), as: .lines) {
            """
            exit 1
            stderr:
            lifr: Claude Code has read-only access. To let it make changes, open Settings > Connected apps > Claude Code and choose Read and write.

            """
        }
    }
    // swiftlint:enable line_length

    func testAMistypedCommandLineNeverReachesTheApp() throws {
        let connection = try FixtureConnection()
        assertInlineSnapshot(of: describe(connection.run("status", "HEALTH-7001", "finished")), as: .lines) {
            """
            exit 2
            stderr:
            lifr: "finished" isn't a status. Use todo, in-progress, procrastinated, done or wont-do.
            Run `lifr help` for every command.

            """
        }
        XCTAssertEqual(connection.requests, [])
    }

    // swiftlint:disable line_length
    func testAnAppThatIsntRunningSaysHowToStartIt() {
        var error = ""
        let cli = CLI(
            connect: { () throws(APIError) in throw .appNotRunning }, standardOutput: { _ in },
            standardError: { error += $0 })
        XCTAssertEqual(cli.run(["avenues"]), 1)
        assertInlineSnapshot(of: error, as: .lines) {
            """
            lifr: The Lifr app didn't start within 10 seconds. Open Lifr on this Mac, or turn on Launch at login in Settings, then retry.

            """
        }
    }
    // swiftlint:enable line_length

    func testAddingAPlanListsWhatWasAdded() throws {
        let connection = try FixtureConnection()
        connection.files["move.toml"] = """
            # Lifr plan v1
            [[project]]
            name = "Spring race"
            avenue = "HEALTH"
              [[project.task]]
              title = "Pick a race"
                [[project.task.subtask]]
                title = "Compare dates"
            """
        assertInlineSnapshot(of: describe(connection.run("add", "--plan", "move.toml")), as: .lines) {
            """
            exit 0
            New Project in HEALTH: Spring race.
            Added 2 task(s):
            HEALTH-7007  To Do  Pick a race  (1 open Sub task)
            HEALTH-7008  To Do  Compare dates

            """
        }
    }

    func testAPlanThatCantBeReadIsRefusedWithoutAskingTheApp() throws {
        let connection = try FixtureConnection()
        connection.files["bad.toml"] = "[[task]]\ntitel = \"A\""
        let run = connection.run("add", "--plan", "bad.toml")
        XCTAssertEqual(run.status, 1)
        XCTAssertEqual(connection.requests, [])
    }

    func testAMissingPlanFileIsAUsageError() throws {
        let connection = try FixtureConnection()
        XCTAssertEqual(connection.run("add", "--plan", "nowhere.toml").status, 2)
        XCTAssertEqual(connection.requests, [])
    }

    func testAPlanWithATitleTooIsAUsageError() throws {
        XCTAssertEqual(try FixtureConnection().run("add", "Buy", "--plan", "move.toml").status, 2)
    }

    private func describe(_ run: Run) -> String {
        var text = "exit \(run.status)\n"
        if !run.output.isEmpty { text += run.output }
        if !run.error.isEmpty { text += "stderr:\n" + run.error }
        return text
    }
}
