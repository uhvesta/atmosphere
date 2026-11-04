import LifrCore

/// A plan that uses every field, written to the golden plan.toml: a new
/// Avenue and label, a Project with tasks and Sub tasks, and tasks placed in
/// the fixture's HEALTH Avenue and under an existing task.
let examplePlan = Plan(
    avenues: [.init(key: "HOME", name: "Home", color: "green", symbol: "house")],
    labels: [.init(name: "errands", color: "orange")],
    projects: [
        .init(
            name: "Move flat", avenue: "HOME", details: "Before the lease ends.\nKeep receipts.", due: "2026-12-01",
            tasks: [
                .init(
                    title: "Clean up living room", priority: .medium, due: "friday",
                    subtasks: [.init(title: "Vacuum"), .init(title: "Wipe \"shelves\"", status: .inProgress)]),
                .init(
                    title: "Buy new clothes", labels: ["errands"], reminders: ["1 day before due"],
                    subtasks: [.init(title: "Shoes"), .init(title: "Jacket", due: "+2w")]),
            ])
    ],
    tasks: [
        .init(
            title: "Book physio", status: .procrastinated, priority: .high, start: "tomorrow", due: "next monday",
            dueTime: "17:30", reminders: ["tomorrow 08:00"], comments: ["Ask about the knee."], place: "HEALTH"),
        .init(title: "Ice after runs", place: "HEALTH/Half marathon"),
        .init(title: "Stretch", parent: "HEALTH-7001"),
    ])
