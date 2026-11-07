import LifrAssistantClient
import LifrModelsClient

/// The shape of each answer, in the order the model writes it: what it
/// does first, then the message, so the message says what it did.
enum Shapes {
    static func optional(_ name: String, _ description: String, _ schema: ModelSchema = .string) -> ModelProperty {
        ModelProperty(name, description, schema, optional: true)
    }

    static let priority = ModelSchema.choice(name: "Priority", ["none", "low", "medium", "high", "urgent"])

    /// A task's fields in the order the model writes them: what it is and
    /// where it goes first, then what the person may have said about it.
    /// A top-level task may be added under one of the existing tasks with
    /// these `parents` codes.
    static func task(sub: Bool, parents: [String] = []) -> ModelSchema {
        var properties = [ModelProperty("title", "What to do, short.")]
        if !sub {
            properties.append(ModelProperty("place", "An Avenue KEY, or KEY/Project name, from the list."))
            if !parents.isEmpty {
                properties.append(
                    optional(
                        "parent", "Only to add it as a Sub task of one of these existing tasks.",
                        .choice(name: "Parent", parents)))
            }
        }
        properties += [
            optional("priority", "Only if the person said how important it is, such as urgent.", priority),
            optional("due", "Only if the person said when, as they said it: friday, next monday, tomorrow, +3d."),
            optional("dueTime", "HH:MM, 24-hour, only if the person said a time."),
            optional("details", "Notes, only if the person gave some."),
            optional("labels", "Label names, only if the person named some.", .array(.string)),
            optional("remind", "Only if the person asked: 1h before, 1d before, or YYYY-MM-DD HH:MM.", .array(.string)),
        ]
        if !sub {
            properties.append(optional("subtasks", "The steps of this task the person named.", .array(task(sub: true))))
        }
        return .object(name: sub ? "SubTask" : "Task", properties)
    }

    /// A change to one of the existing tasks with these `codes`, which
    /// may move it to one of `places`. Only the fields to change are
    /// given, the most asked for first.
    static func change(codes: [String], places: [String]) -> ModelSchema {
        .object(
            name: "Change",
            [
                ModelProperty("code", "The existing task's code.", .choice(name: "TaskCode", codes)),
                optional(
                    "status", "Only to finish, start or reopen it: done when finished.",
                    .choice(name: "NewStatus", ["todo", "in_progress", "done", "wont_do"])),
                optional(
                    "due", "Only to reschedule, push or postpone it: the new date as the person said it, or none."),
                optional("priority", "Only to change how important it is.", priority),
                optional("moveTo", "Only to move it to another Avenue or Project.", .choice(name: "Place", places)),
                optional("rename", "Only to rename it: the new title."),
            ])
    }

    /// A change to one of the existing Projects named `projects`, as
    /// "KEY/Name". Only the fields to change are given.
    static func projectChange(projects: [String]) -> ModelSchema {
        .object(
            name: "ProjectChange",
            [
                ModelProperty("project", "The existing Project.", .choice(name: "ProjectName", projects)),
                optional(
                    "status", "Only to complete, archive or reopen it: active reopens it.",
                    .choice(name: "ProjectStatus", ["active", "completed", "archived"])),
                optional("due", "Only to change when it's due: the new date as the person said it, or none."),
                optional("rename", "Only to rename it: the new name."),
                optional("details", "Only to change what the goal is: the new description."),
            ])
    }

    /// A change to one of the current plan's `count` tasks, which aren't
    /// saved yet, given by its number: a choice of titles with spaces is
    /// more than the model's grammar can follow. Only the fields to change
    /// are given.
    static func draftChange(count: Int, places: [String]) -> ModelSchema {
        var properties = [
            ModelProperty(
                "task", "The number of the plan's task, from the list.",
                .choice(name: "DraftTask", (0..<count).map { String($0 + 1) })),
            optional("remove", "Only to take it out of the plan: true.", .bool),
            optional("due", "Only to change when it's due: the new date as the person said it, or none."),
            optional("priority", "Only to change how important it is.", priority),
        ]
        if !places.isEmpty {
            properties.append(
                optional("moveTo", "Only to put it in another Avenue or Project.", .choice(name: "Place", places)))
        }
        properties.append(optional("rename", "Only to rename it: the new title."))
        return .object(name: "DraftChange", properties)
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

    /// New things to add, whose top-level tasks may go under the existing
    /// tasks with `parents` codes.
    static func plan(parents: [String] = []) -> ModelSchema {
        .object(
            name: "Plan",
            [
                optional("avenues", "New Avenues, only when the person agreed to one.", .array(avenue)),
                optional("projects", "New Projects, only when the person asked for one.", .array(project)),
                optional("tasks", "New top-level tasks.", .array(task(sub: false, parents: parents))),
            ])
    }

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

    /// The pick of one of `ids`, or none.
    static func pick(_ ids: [String]) -> ModelSchema {
        .object(
            name: "Pick",
            [ModelProperty("skill", "The skill to use.", .choice(name: "SkillID", ids + [AssistantPrompt.noSkill]))])
    }

    /// The answer to a message: the changes made to the existing tasks
    /// with `codes`, the plan of new things, then what to say about them.
    /// A change may move a task to one of `places`. The answer must have
    /// changes with `mustChange`, and a plan with `mustPlan`: left to
    /// choose, a small model leaves them out and only talks. The plan's
    /// tasks may go under the `codes` tasks.
    /// With `draftTitles`, the answer may change the current plan's tasks
    /// with those titles, and must with `mustChangeDraft`; its plan then
    /// has only what's new.
    /// With `projects`, the answer may change those existing Projects, and
    /// must with `mustChangeProject`.
    static func turn(
        changes codes: [String], places: [String] = [], draftTitles: [String] = [], projects: [String] = [],
        mustChange: Bool = false, mustChangeDraft: Bool = false, mustChangeProject: Bool = false,
        mustPlan: Bool = false
    ) -> ModelSchema {
        var properties: [ModelProperty] = []
        if !codes.isEmpty {
            properties.append(
                ModelProperty(
                    "changes", "Changes to these existing tasks the person asked for.",
                    .array(change(codes: codes, places: places)),
                    optional: !mustChange))
        }
        if !projects.isEmpty {
            properties.append(
                ModelProperty(
                    "projectChanges", "Changes to these existing Projects the person asked for.",
                    .array(projectChange(projects: projects)), optional: !mustChangeProject))
        }
        if !draftTitles.isEmpty {
            properties.append(
                ModelProperty(
                    "draftChanges", "Changes to the current plan's tasks the person asked for.",
                    .array(draftChange(count: draftTitles.count, places: places)),
                    optional: !mustChangeDraft))
        }
        let planDescription =
            draftTitles.isEmpty
            ? "The whole plan of new things to add, when there is one."
            : "Only the new things to add to the current plan, when the person asked for more."
        properties += [
            ModelProperty("plan", planDescription, plan(parents: codes), optional: !mustPlan),
            ModelProperty(
                "message", "What you say to the person about the changes and plan, in one to three sentences."),
        ]
        return .object(name: "Turn", properties)
    }
}

extension AssistantLookup.Place {
    /// What a move to this Avenue or one of its Projects is written as.
    var choices: [String] { [key] + projects.map { key + "/" + $0 } }
}
