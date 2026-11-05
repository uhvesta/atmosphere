import LifrAssistantClient
import LifrModelsClient

/// The shape of each answer, in the order the model writes it: the message
/// first, so it shows while the plan is still coming.
enum Shapes {
    static func optional(_ name: String, _ description: String, _ schema: ModelSchema = .string) -> ModelProperty {
        ModelProperty(name, description, schema, optional: true)
    }

    static let priority = ModelSchema.choice(name: "Priority", ["none", "low", "medium", "high", "urgent"])

    static func task(sub: Bool) -> ModelSchema {
        var properties = [ModelProperty("title", "What to do, short.")]
        if !sub {
            properties += [
                optional("place", "An Avenue KEY, or KEY/Project name, from the list."),
                optional("parent", "The code of an existing task to add this under, from findTasks."),
            ]
        }
        properties += [
            optional("details", "Notes, only if the person gave some."),
            optional("priority", "Only if the person said how important it is.", priority),
            optional("due", "When it's due: YYYY-MM-DD, today, tomorrow, friday, next friday, +3d, +2w or +1m."),
            optional("dueTime", "HH:MM, 24-hour, only with a due date."),
            optional("labels", "Label names.", .array(.string)),
            optional("remind", "Reminders: 1h before, 1d before, or YYYY-MM-DD HH:MM.", .array(.string)),
        ]
        if !sub {
            properties.append(optional("subtasks", "Steps of this task, one level deep.", .array(task(sub: true))))
        }
        return .object(name: sub ? "SubTask" : "Task", properties)
    }

    static let project = ModelSchema.object(
        name: "Project",
        [
            ModelProperty("name", "The goal's name."),
            ModelProperty("avenue", "The KEY of the Avenue it belongs to."),
            optional("details", "What the goal is, if the person said."),
            optional("due", "When the goal is due."),
            optional("tasks", "The Project's tasks; they need no place.", .array(task(sub: true))),
        ])

    static let avenue = ModelSchema.object(
        name: "Avenue",
        [
            ModelProperty("key", "2 to 10 capital letters, such as HEALTH."),
            ModelProperty("name", "Its name, such as Health."),
        ])

    static let plan = ModelSchema.object(
        name: "Plan",
        [
            optional("avenues", "New Avenues, only when the person agreed to one.", .array(avenue)),
            optional("projects", "New Projects.", .array(project)),
            optional("tasks", "New top-level tasks.", .array(task(sub: false))),
        ])

    static let skill = ModelSchema.object(
        name: "Skill",
        [
            ModelProperty("name", "Two or three words."),
            ModelProperty("summary", "One short line: what it's for."),
            ModelProperty("instructions", "Two to four sentences for the assistant."),
            ModelProperty(
                "tools", "The tools it needs.",
                .array(.choice(name: "Tool", AssistantTool.allCases.map(\.rawValue)))),
            ModelProperty("draftsPlans", "Whether it adds new tasks, Projects or Avenues.", .bool),
        ])

    static let turn = ModelSchema.object(
        name: "Turn",
        [
            ModelProperty("message", "What you say to the person, one to three sentences."),
            optional("plan", "The whole plan to add, when there is one.", plan),
        ])
}
