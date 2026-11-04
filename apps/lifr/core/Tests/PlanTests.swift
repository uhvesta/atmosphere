import Foundation
import LifrCore
import LifrTesting

func planTests() -> [TestCase] {
    let gmt = TimeZone.gmt

    func titles(_ tasks: [LifrTask]) -> [String] { tasks.map(\.title) }

    return [
        TestCase("given two tasks with their own Sub tasks, when applied, then each Sub task is under its own task") {
            let fixture = try Fixture()
            let plan = Plan(tasks: [
                .init(
                    title: "Clean up living room", subtasks: [.init(title: "Vacuum"), .init(title: "Dust")],
                    place: "HEALTH"),
                .init(title: "Buy new clothes", subtasks: [.init(title: "Jacket")], place: "health"),
            ])
            _ = try fixture.workspace.apply(plan, timeZone: gmt, in: fixture.context)
            let room = try fixture.workspace.tasks.values.first { $0.title == "Clean up living room" }.unwrap()
            let clothes = try fixture.workspace.tasks.values.first { $0.title == "Buy new clothes" }.unwrap()
            try expectEqual(titles(fixture.workspace.subtasks(of: room.id)), ["Vacuum", "Dust"])
            try expectEqual(titles(fixture.workspace.subtasks(of: clothes.id)), ["Jacket"])
        },
        TestCase("given a new Avenue and a Project in it, when applied, then the tasks are in that Project") {
            let fixture = try Fixture()
            let plan = Plan(
                avenues: [.init(key: "RENO", name: "Renovation")],
                projects: [.init(name: "Kitchen", avenue: "RENO", tasks: [.init(title: "Get quotes")])])
            _ = try fixture.workspace.apply(plan, timeZone: gmt, in: fixture.context)
            let task = try fixture.workspace.tasks.values.first { $0.title == "Get quotes" }.unwrap()
            try expectEqual(fixture.workspace.projects[task.projectID.unwrap()]?.name, "Kitchen")
            try expectEqual(fixture.workspace.avenues[task.avenueID]?.key.rawValue, "RENO")
        },
        TestCase("given relative dates on a Tuesday, when applied, then Lifr works out the days") {
            let fixture = try Fixture()  // 2026-11-03, a Tuesday
            let plan = Plan(tasks: [
                .init(title: "A", start: "tomorrow", due: "friday", place: "HEALTH"),
                .init(title: "B", start: "next tuesday", due: "+2w", place: "HEALTH"),
            ])
            _ = try fixture.workspace.apply(plan, timeZone: gmt, in: fixture.context)
            let first = try fixture.workspace.tasks.values.first { $0.title == "A" }.unwrap()
            let second = try fixture.workspace.tasks.values.first { $0.title == "B" }.unwrap()
            try expectEqual(first.startDate?.description, "2026-11-04")
            try expectEqual(first.dueDate?.description, "2026-11-06")
            try expectEqual(second.startDate?.description, "2026-11-10")
            try expectEqual(second.dueDate?.description, "2026-11-17")
        },
        TestCase("given an Avenue key with a typo, when applied, then unknownAvenue suggests the key and adds nothing")
        {
            let fixture = try Fixture()
            let before = fixture.workspace
            let plan = Plan(tasks: [.init(title: "Fine", place: "HEALTH"), .init(title: "Typo", place: "HELTH")])
            try expectError(
                .plan(at: "task 2 “Typo”", .unknownAvenue(key: "HELTH", suggestion: "HEALTH"))
            ) {
                try fixture.workspace.apply(plan, timeZone: gmt, in: fixture.context)
            }
            try expectEqual(fixture.workspace, before)
        },
        TestCase("given a plan with a wrong key, when it's checked, then the problem is the one adding would give") {
            let fixture = try Fixture()
            let typo = Plan(tasks: [.init(title: "Typo", place: "HELTH")])
            try expectEqual(
                fixture.workspace.problem(with: typo, timeZone: gmt, now: fixture.now),
                .plan(at: "task 1 “Typo”", .unknownAvenue(key: "HELTH", suggestion: "HEALTH")))
            let fine = Plan(tasks: [.init(title: "Fine", place: "HEALTH")])
            try expectEqual(fixture.workspace.problem(with: fine, timeZone: gmt, now: fixture.now), nil)
            try expectEqual(
                fixture.workspace.problem(with: Plan(), timeZone: gmt, now: fixture.now), .plan(at: "the plan", .empty))
        },
        TestCase("given an existing task's code as parent, when applied, then the drafts become its Sub tasks") {
            let fixture = try Fixture()
            let run = try fixture.task("Run")
            let code = fixture.workspace.displayCode(of: run)
            _ = try fixture.workspace.apply(
                Plan(tasks: [.init(title: "Buy gels", parent: code)]), timeZone: gmt, in: fixture.context)
            try expectEqual(titles(fixture.workspace.subtasks(of: run.id)), ["Buy gels"])
        },
        TestCase("given Sub tasks under a task whose parent is given, when applied, then subtaskTooDeep") {
            let fixture = try Fixture()
            let run = try fixture.task("Run")
            let code = fixture.workspace.displayCode(of: run)
            let plan = Plan(tasks: [.init(title: "Gels", subtasks: [.init(title: "Lemon")], parent: code)])
            try expectError(.plan(at: "task 1 “Gels”", .subtaskTooDeep)) {
                try fixture.workspace.apply(plan, timeZone: gmt, in: fixture.context)
            }
        },
        TestCase("given a task with no place, when applied, then noPlace") {
            let fixture = try Fixture()
            try expectError(.plan(at: "task 1 “Lost”", .noPlace)) {
                try fixture.workspace.apply(Plan(tasks: [.init(title: "Lost")]), timeZone: gmt, in: fixture.context)
            }
        },
        TestCase("given a new Project named like an existing one, when applied, then projectExists") {
            let fixture = try Fixture()
            let plan = Plan(projects: [.init(name: "half marathon", avenue: "HEALTH")])
            try expectError(
                .plan(at: "Project 1 “half marathon”", .projectExists(name: "half marathon", avenue: "HEALTH"))
            ) {
                try fixture.workspace.apply(plan, timeZone: gmt, in: fixture.context)
            }
        },
        TestCase("given an existing Project by name, when applied, then the task goes in it") {
            let fixture = try Fixture()
            _ = try fixture.workspace.apply(
                Plan(tasks: [.init(title: "Long run", place: "HEALTH/Half Marathon")]), timeZone: gmt,
                in: fixture.context)
            let task = try fixture.workspace.tasks.values.first { $0.title == "Long run" }.unwrap()
            try expectEqual(task.projectID, fixture.marathon.id)
        },
        TestCase("given a Done task with an open Sub task, when applied, then closedWithOpenSubtasks") {
            let fixture = try Fixture()
            let plan = Plan(tasks: [
                .init(title: "Pack", status: .done, subtasks: [.init(title: "Socks")], place: "HEALTH")
            ])
            try expectError(.plan(at: "task 1 “Pack”", .closedWithOpenSubtasks)) {
                try fixture.workspace.apply(plan, timeZone: gmt, in: fixture.context)
            }
        },
        TestCase("given label names, when applied, then existing labels match in any case and new ones are made") {
            let fixture = try Fixture()
            let errand = fixture.workspace.createLabel(name: "Errand", in: fixture.context)
            _ = try fixture.workspace.apply(
                Plan(tasks: [.init(title: "Socks", labels: ["errand", "calls"], place: "HEALTH")]), timeZone: gmt,
                in: fixture.context)
            let task = try fixture.workspace.tasks.values.first { $0.title == "Socks" }.unwrap()
            try expectEqual(task.labelIDs.contains(errand.id), true)
            try expectEqual(
                Set(task.labelIDs.compactMap { fixture.workspace.labels[$0]?.name }), ["Errand", "calls"])
        },
        TestCase("given reminders, when applied, then before-due and dated ones are set") {
            let fixture = try Fixture()
            _ = try fixture.workspace.apply(
                Plan(tasks: [
                    .init(
                        title: "Race", due: "+3d", reminders: ["1 day before due", "tomorrow 17:30"],
                        place: "HEALTH")
                ]), timeZone: gmt, in: fixture.context)
            let triggers = Set(fixture.workspace.reminders.values.map(\.trigger))
            try expectEqual(
                triggers, [.beforeDue(seconds: 86400), .at(fixture.now + 24 * 3600 + 17.5 * 3600)])
        },
        TestCase("given an unreadable date, when applied, then invalidDate") {
            let fixture = try Fixture()
            try expectError(.plan(at: "task 1 “Run”", .invalidDate("someday"))) {
                try fixture.workspace.apply(
                    Plan(tasks: [.init(title: "Run", due: "someday", place: "HEALTH")]), timeZone: gmt,
                    in: fixture.context)
            }
        },
        TestCase("given an applied plan, when its change is undone, then everything it added is gone") {
            let fixture = try Fixture()
            let before = fixture.workspace
            let plan = Plan(
                avenues: [.init(key: "RENO", name: "Renovation")], labels: [.init(name: "calls", color: "blue")],
                projects: [
                    .init(
                        name: "Kitchen", avenue: "RENO",
                        tasks: [
                            .init(
                                title: "Quotes", due: "+1w", labels: ["calls"], reminders: ["1 day before due"],
                                comments: ["Ask Mira"], subtasks: [.init(title: "Call Smith")])
                        ])
                ])
            let change = try fixture.workspace.apply(plan, timeZone: gmt, in: fixture.context)
            _ = fixture.workspace.undo(change, in: fixture.context)
            try expectEqual(fixture.workspace.avenues, before.avenues)
            try expectEqual(fixture.workspace.projects, before.projects)
            try expectEqual(fixture.workspace.tasks, before.tasks)
            try expectEqual(fixture.workspace.labels, before.labels)
            try expectEqual(fixture.workspace.comments, before.comments)
            try expectEqual(fixture.workspace.reminders, before.reminders)
        },
    ]
}
