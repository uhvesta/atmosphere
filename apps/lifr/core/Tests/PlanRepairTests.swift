import Foundation
import LifrCore
import TestingKit

func planRepairTests() -> [TestCase] {
    let gmt = TimeZone.gmt

    return [
        TestCase(
            "given places by name, in another case, a Project typo or only a Project, when repaired, then they add"
        ) {
            let fixture = try Fixture()
            let plan = Plan(tasks: [
                .init(title: "A", place: "Health"),
                .init(title: "C", place: "health/half MARATHON"),
                .init(title: "D", place: "HEALTH/Half marathn"),
                .init(title: "E", place: "Half marathon"),
            ])
            let repaired = plan.repaired(for: fixture.workspace)
            try expectEqual(
                repaired.tasks.map(\.place),
                ["HEALTH", "HEALTH/Half marathon", "HEALTH/Half marathon", "HEALTH/Half marathon"])
            _ = try fixture.workspace.apply(repaired, timeZone: gmt, in: fixture.context)
        },
        TestCase("given a key a letter from HEALTH that the plan doesn't add, when repaired, then it's refused") {
            let fixture = try Fixture()
            // WEALTH may be a new Avenue the plan left out, or HEALTH
            // mistyped: a guess could put it all in the wrong Avenue.
            let plan = Plan(
                projects: [.init(name: "Emergency fund", avenue: "WEALTH", tasks: [.init(title: "Open an account")])],
                tasks: [.init(title: "Budget", place: "wealth")])
            let repaired = plan.repaired(for: fixture.workspace)
            try expectEqual(repaired.projects.map(\.avenue), ["WEALTH"])
            try expectEqual(repaired.tasks.map(\.place), ["wealth"])
            let problem = fixture.workspace.problem(with: repaired, timeZone: gmt, now: fixture.now)
            var refused = false
            if case .plan(_, .unknownAvenue(key: "WEALTH", _))? = problem { refused = true }
            try expect(refused, "WEALTH wasn't refused: \(String(describing: problem))")
        },
        TestCase("given a place nothing is close to, when repaired, then it stays for the person to fix") {
            let fixture = try Fixture()
            let plan = Plan(tasks: [.init(title: "A", place: "GARDEN")])
            try expectEqual(plan.repaired(for: fixture.workspace), plan)
        },
        TestCase("given a Project of a new Avenue the plan lacks, when repaired, then the task is in the Avenue") {
            let fixture = try Fixture()
            let plan = Plan(
                avenues: [.init(key: "GARDEN", name: "Garden")],
                projects: [.init(name: "Roses", avenue: "GARDEN")],
                tasks: [
                    .init(title: "Prune the roses", place: "Garden/prune_roses"),
                    .init(title: "Feed the roses", place: "GARDEN/roses"),
                ])
            let repaired = plan.repaired(for: fixture.workspace)
            try expectEqual(repaired.tasks.map(\.place), ["GARDEN", "GARDEN/roses"])
            _ = try fixture.workspace.apply(repaired, timeZone: gmt, in: fixture.context)
        },
        TestCase("given a Project named like the task, when repaired, then the task is in the Avenue") {
            let fixture = try Fixture()
            let plan = Plan(tasks: [
                .init(title: "Vacuum the living room", place: "HEALTH/vacuum the living room"),
                .init(title: "Add an urgent task to file my taxes", place: "HEALTH/file my taxes"),
            ])
            let repaired = plan.repaired(for: fixture.workspace)
            try expectEqual(repaired.tasks.map(\.place), ["HEALTH", "HEALTH"])
            _ = try fixture.workspace.apply(repaired, timeZone: gmt, in: fixture.context)
        },
        TestCase("given steps under another new task by title, when repaired, then they're its Sub tasks") {
            let fixture = try Fixture()
            let plan = Plan(tasks: [
                .init(title: "Plan the offsite", place: "HEALTH"),
                .init(
                    title: "Book a venue", subtasks: [.init(title: "Compare")], place: "HEALTH/plan the Offsite"),
                .init(title: "Send the invites", parent: "Plan the offsite"),
            ])
            let repaired = plan.repaired(for: fixture.workspace)
            try expectEqual(repaired.tasks.map(\.title), ["Plan the offsite"])
            try expectEqual(
                repaired.tasks.first?.subtasks.map(\.title), ["Book a venue", "Compare", "Send the invites"])
            _ = try fixture.workspace.apply(repaired, timeZone: gmt, in: fixture.context)
        },
        TestCase("given a new Project named like one there, when repaired, then its tasks go in that one") {
            let fixture = try Fixture()
            let plan = Plan(projects: [.init(name: "half marathon", avenue: "HEALTH", tasks: [.init(title: "Stretch")])]
            )
            let repaired = plan.repaired(for: fixture.workspace)
            try expectEqual(repaired.projects, [])
            try expectEqual(repaired.tasks.map(\.place), ["HEALTH/Half marathon"])
            _ = try fixture.workspace.apply(repaired, timeZone: gmt, in: fixture.context)
        },
        TestCase("given no dates written as none and a Sub task twice, when repaired, then they're left out") {
            let fixture = try Fixture()
            let plan = Plan(tasks: [
                .init(
                    title: "Plan", due: "none", dueTime: "",
                    subtasks: [.init(title: "Book", due: "None"), .init(title: "book")], place: "HEALTH")
            ])
            let repaired = plan.repaired(for: fixture.workspace)
            try expectEqual(repaired.tasks.first?.due, nil)
            try expectEqual(repaired.tasks.first?.dueTime, nil)
            try expectEqual(repaired.tasks.first?.subtasks, [.init(title: "Book")])
            _ = try fixture.workspace.apply(repaired, timeZone: gmt, in: fixture.context)
        },
        TestCase("given a new Avenue named like one that's there, when repaired, then its tasks go in that one") {
            let fixture = try Fixture()
            let plan = Plan(
                avenues: [.init(key: "FIT", name: "health")],
                projects: [.init(name: "Swimming", avenue: "FIT")],
                tasks: [.init(title: "Stretch", place: "fit")])
            let repaired = plan.repaired(for: fixture.workspace)
            try expectEqual(repaired.avenues, [])
            try expectEqual(repaired.projects.map(\.avenue), ["HEALTH"])
            try expectEqual(repaired.tasks.map(\.place), ["HEALTH"])
            _ = try fixture.workspace.apply(repaired, timeZone: gmt, in: fixture.context)
        },
        TestCase("given a task placed by a new Avenue's name, when repaired, then it goes in the new Avenue") {
            let fixture = try Fixture()
            let plan = Plan(
                avenues: [.init(key: "GARDEN", name: "Garden")], tasks: [.init(title: "Plant", place: "garden")])
            let repaired = plan.repaired(for: fixture.workspace)
            try expectEqual(repaired.tasks.map(\.place), ["GARDEN"])
            _ = try fixture.workspace.apply(repaired, timeZone: gmt, in: fixture.context)
        },
        TestCase("given a task with a place and an existing parent, when repaired, then it goes under the parent") {
            let fixture = try Fixture()
            let parent = try fixture.task("Run")
            let code = fixture.workspace.displayCode(of: parent)
            let plan = Plan(tasks: [.init(title: "Warm up", place: "HEALTH", parent: code.lowercased())])
            let repaired = plan.repaired(for: fixture.workspace)
            _ = try fixture.workspace.apply(repaired, timeZone: gmt, in: fixture.context)
            let added = try fixture.workspace.tasks.values.first { $0.title == "Warm up" }.unwrap()
            try expectEqual(added.parentID, parent.id)
        },
        TestCase("given a task with a place and a parent that isn't there, when repaired, then it goes in the place") {
            let fixture = try Fixture()
            let plan = Plan(tasks: [.init(title: "Warm up", place: "HEALTH", parent: "HEALTH-ZZZZ")])
            let repaired = plan.repaired(for: fixture.workspace)
            try expectEqual(repaired.tasks.map(\.parent), [nil])
            _ = try fixture.workspace.apply(repaired, timeZone: gmt, in: fixture.context)
        },
        TestCase("given places on Sub tasks and a new Project's tasks, when repaired, then they're dropped") {
            let fixture = try Fixture()
            let plan = Plan(
                projects: [
                    .init(name: "Swimming", avenue: "health", tasks: [.init(title: "Goggles", place: "HEALTH")])
                ],
                tasks: [.init(title: "Offsite", subtasks: [.init(title: "Venue", place: "HEALTH")], place: "HEALTH")])
            let repaired = plan.repaired(for: fixture.workspace)
            _ = try fixture.workspace.apply(repaired, timeZone: gmt, in: fixture.context)
            try expectEqual(fixture.workspace.tasks.count, 3)
        },
        TestCase("given the same task twice in one place, when repaired, then it's one with what both say") {
            let fixture = try Fixture()
            let plan = Plan(tasks: [
                .init(title: "Buy milk", place: "HEALTH"),
                .init(title: "buy milk", priority: .high, due: "friday", place: "health"),
                .init(title: "Buy milk", place: "HEALTH/Half marathon"),
                .init(title: "Buy eggs", place: "HEALTH"),
            ])
            let repaired = plan.repaired(for: fixture.workspace)
            try expectEqual(repaired.tasks.map(\.title), ["Buy milk", "Buy milk", "Buy eggs"])
            try expectEqual(repaired.tasks.first?.due, "friday")
            try expectEqual(repaired.tasks.first?.priority, .high)
        },
        TestCase("given no Avenues yet and a plan that names one it doesn't add, when repaired, then it's added") {
            var workspace = Workspace()
            let context = Context(actor: .user(device: "Mac", surface: .mac), now: Date(timeIntervalSince1970: 0))
            let plan = Plan(
                projects: [.init(name: "Gym", avenue: "Fitness", tasks: [.init(title: "Cardio")])],
                tasks: [.init(title: "Legs", place: "HEALTH"), .init(title: "Stretch", place: "fitness/Gym")])
            let repaired = plan.repaired(for: workspace)
            try expectEqual(repaired.avenues.map(\.key), ["FITNESS", "HEALTH"])
            try expectEqual(repaired.avenues.map(\.name), ["Fitness", "Health"])
            _ = try workspace.apply(repaired, timeZone: gmt, in: context)
            try expectEqual(workspace.avenues.count, 2)
            try expectEqual(workspace.tasks.count, 3)
        },
        TestCase("given a Sub task with its task's title, when repaired, then it's left out") {
            let fixture = try Fixture()
            let plan = Plan(tasks: [
                .init(title: "Cardio", subtasks: [.init(title: "cardio"), .init(title: "Warm up")], place: "HEALTH")
            ])
            let repaired = plan.repaired(for: fixture.workspace)
            try expectEqual(repaired.tasks.first?.subtasks.map(\.title), ["Warm up"])
        },
    ]
}
