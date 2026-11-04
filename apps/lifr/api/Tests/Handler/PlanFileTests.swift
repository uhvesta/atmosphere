import LifrAPI
import LifrCore
import LifrTesting

private func problem(_ text: String) -> PlanFileError? {
    do {
        _ = try PlanFile.read(text)
        return nil
    } catch {
        return error
    }
}

func planFileTests() -> [TestCase] {
    [
        TestCase("given a plan using every field, then reading what was written gives it back") {
            try expectEqual(try PlanFile.read(PlanFile.write(examplePlan)), examplePlan)
        },
        TestCase("given two tasks each followed by Sub tasks, then each Sub task belongs to the task above it") {
            let plan = try PlanFile.read(
                """
                # Lifr plan v1
                [[task]]
                title = "Clean up living room"
                in = "HOME"
                  [[task.subtask]]
                  title = "Vacuum"
                  [[task.subtask]]
                  title = "Dust shelves"

                [[task]]
                title = "Buy new clothes"
                in = "HOME"
                  [[task.subtask]]
                  title = "Shoes"
                """)
            try expectEqual(plan.tasks.map(\.title), ["Clean up living room", "Buy new clothes"])
            try expectEqual(plan.tasks[0].subtasks.map(\.title), ["Vacuum", "Dust shelves"])
            try expectEqual(plan.tasks[1].subtasks.map(\.title), ["Shoes"])
        },
        TestCase("given literal, multi-line and escaped strings, bare dates and comments, then they read as text") {
            let plan = try PlanFile.read(
                #"""
                [[task]] # a comment
                title = 'C:\path'
                in = "HOME"
                due = 2026-11-20
                details = """
                Line one
                "quoted" \u00e9\
                   joined"""
                labels = [
                  "a", # first
                  'b',
                ]
                """#)
            try expectEqual(plan.tasks[0].title, #"C:\path"#)
            try expectEqual(plan.tasks[0].due, "2026-11-20")
            try expectEqual(plan.tasks[0].details, "Line one\n\"quoted\" éjoined")
            try expectEqual(plan.tasks[0].labels, ["a", "b"])
        },
        TestCase("given details ending in quotes, then they survive a round trip") {
            let plan = Plan(tasks: [.init(title: "T", details: "say \"hi\"\nthen \"\"\" three\n\"", place: "HOME")])
            try expectEqual(try PlanFile.read(PlanFile.write(plan)), plan)
        },
        TestCase("given a misspelt field, then unknownKey on its line suggests the right one") {
            try expectEqual(
                problem("[[task]]\ntitle = \"T\"\nprioirty = \"high\""),
                PlanFileError(line: 3, problem: .unknownKey("prioirty", table: "task", suggestion: "priority")))
        },
        TestCase("given a misspelt table, then unknownTable suggests the right one") {
            try expectEqual(
                problem("[[task.subtasks]]\ntitle = \"T\""),
                PlanFileError(line: 1, problem: .unknownTable("task.subtasks", suggestion: "task.subtask")))
        },
        TestCase("given a Sub task with no task above it, then orphanTable") {
            try expectEqual(
                problem("[[project]]\nname = \"P\"\navenue = \"HOME\"\n[[task.subtask]]\ntitle = \"T\""),
                PlanFileError(line: 4, problem: .orphanTable("task.subtask", parent: "task")))
        },
        TestCase("given a task with no title, then missingKey on its header") {
            try expectEqual(
                problem("\n[[task]]\nin = \"HOME\""),
                PlanFileError(line: 2, problem: .missingKey("title", table: "task")))
        },
        TestCase("given a status Lifr doesn't have, then badChoice lists the ones it does") {
            let found = problem("[[task]]\ntitle = \"T\"\nstatus = \"later\"")?.problem
            guard case .badChoice(let key, _, let choices)? = found else {
                return try expect(false, "expected badChoice, got \(String(describing: found))")
            }
            try expectEqual(key, "status")
            try expect(choices.contains("in_progress"), "lists in_progress")
        },
        TestCase("given a field set twice, then duplicateKey on the second") {
            try expectEqual(
                problem("[[task]]\ntitle = \"A\"\ntitle = \"B\""),
                PlanFileError(line: 3, problem: .duplicateKey("title")))
        },
        TestCase("given wrong value shapes, then notAString or notAList") {
            try expectEqual(problem("[[task]]\ntitle = [\"A\"]")?.problem, .notAString(key: "title"))
            try expectEqual(problem("[[task]]\ntitle = \"A\"\nlabels = \"x\"")?.problem, .notAList(key: "labels"))
        },
        TestCase("given broken syntax, then a syntax error on the line it's on") {
            for (text, line) in [
                ("[[task]]\ntitle = \"open", 2), ("[task]", 1), ("[[task]]\ntitle \"T\"", 2),
                ("[[task]]\ntitle = \"T\" extra", 2), ("title = \"T\"", 1), ("[[task]]\ntitle = 12", 2),
            ] {
                guard case .syntax? = problem(text)?.problem else {
                    return try expect(false, "expected a syntax error for \(text)")
                }
                try expectEqual(problem(text)?.line, line)
            }
        },
    ]
}
