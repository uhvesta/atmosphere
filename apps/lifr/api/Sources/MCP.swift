import Foundation
import LifrCore

/// A tool `lifr mcp` offers, with the JSON Schema of its arguments. The
/// arguments are the method's params as they are.
package struct MCPTool: Hashable, Codable, Sendable {
    package var name: String
    package var description: String
    package var inputSchema: JSONValue
    /// Not part of MCP: the scope the tool needs, for `lifr mcp --list`.
    package var scope: Scope

    /// Every tool, in `Method` order.
    package static var all: [MCPTool] {
        APIMethod.allCases.compactMap { method in
            guard let scope = method.scope, let (description, schema) = Self.describe(method) else { return nil }
            return MCPTool(name: method.rawValue, description: description, inputSchema: schema, scope: scope)
        }
    }

    private static func describe(_ method: APIMethod) -> (String, JSONValue)? {
        switch method {
        case .hello, .unlock, .export:
            return nil
        case .listAvenues:
            return ("List the Avenues (areas of life, such as HEALTH) you can see, with their keys.", object([:]))
        case .listProjects:
            return (
                "List Projects, the goals inside an Avenue. Active ones only unless includeClosed is true.",
                object(["avenue": avenueKey, "includeClosed": boolean("Include completed and archived Projects.")])
            )
        case .searchTasks:
            return (
                "Find tasks. Open tasks only unless includeClosed is true or statuses names closed ones.",
                object([
                    "query": string("Words that must all appear in the title or details, ignoring case."),
                    "avenue": avenueKey,
                    "project": projectID,
                    "statuses": .object([
                        "type": "array", "items": status,
                        "description": "Match any of these. procrastinated includes tasks idle too long.",
                    ]),
                    "includeClosed": boolean("Include Done and Won't Do tasks."),
                    "limit": .object([
                        "type": "integer", "minimum": 1, "maximum": .number(Double(SearchTasks.maxLimit)),
                        "description": .string("At most this many tasks. Defaults to \(SearchTasks.defaultLimit)."),
                    ]),
                ])
            )
        case .getTask:
            return (
                "Get a task with its Sub tasks, comments and history.", object(["task": taskCode], required: ["task"])
            )
        case .createTask:
            return (
                "Create a task in an Avenue, or a Sub task under a parent task.",
                object(
                    taskFields.merging([
                        "title": title,
                        "avenue": avenueKey,
                        "project": projectID,
                        "parent": string(
                            "Code of the parent task. Makes this a Sub task in the parent's Avenue and Project."),
                        "status": status,
                    ]) { $1 }, required: ["title"])
            )
        case .updateTask:
            return (
                "Change a task's title, details, priority, dates or labels.",
                object(
                    taskFields.merging([
                        "task": taskCode,
                        "title": title,
                        "clear": .object([
                            "type": "array",
                            "items": .object([
                                "enum": .array(UpdateTask.ClearableField.allCases.map { .string($0.rawValue) })
                            ]),
                            "description": "Fields to unset.",
                        ]),
                    ]) { $1 }, required: ["task"])
            )
        case .setStatus:
            return (
                "Set a task's status. Done needs every Sub task closed; Won't Do with open Sub tasks needs cascade: "
                    + "true and closes them too.",
                object(
                    [
                        "task": taskCode, "status": status,
                        "cascade": boolean("Also mark open Sub tasks Won't Do. Ask the person first."),
                    ],
                    required: ["task", "status"])
            )
        case .moveTask:
            return (
                "Move a task to another Avenue or Project, under a parent task, or out to the top level. Give "
                    + "exactly one of avenue, parent or topLevel.",
                object(
                    [
                        "task": taskCode, "avenue": avenueKey, "project": projectID,
                        "parent": string("Code of the new parent task."),
                        "topLevel": boolean("Promote a Sub task to a top-level task."),
                    ],
                    required: ["task"])
            )
        case .addComment:
            return (
                "Add a Markdown comment to a task.",
                object(["task": taskCode, "body": string("The comment, in Markdown.")], required: ["task", "body"])
            )
        case .addPlan:
            return (
                "Add several records at once from a plan, all or nothing: new Avenues, labels, Projects with "
                    + "their tasks, and tasks with Sub tasks. Show the plan to the person before adding it. Give "
                    + "the plan as a JSON object, or as TOML text, where a table belongs to the nearest parent "
                    + "table above it:\n\n" + PlanFile.guide,
                object(
                    [
                        "plan": .object([
                            "anyOf": [
                                plan,
                                string("The plan as TOML, starting with \"\(PlanFile.firstLine)\"."),
                            ]
                        ]),
                        "timeZone": string("IANA time zone for relative dates, such as Europe/London."),
                    ],
                    required: ["plan"])
            )
        case .trashTask:
            return (
                "Move a task and its Sub tasks to the trash. The person always confirms this in the Lifr app.",
                object(["task": taskCode], required: ["task"])
            )
        }
    }

    // MARK: - Schema pieces

    private static func object(_ properties: [String: JSONValue], required: [String] = []) -> JSONValue {
        var schema: [String: JSONValue] = [
            "type": "object", "properties": .object(properties), "additionalProperties": false,
        ]
        if !required.isEmpty { schema["required"] = .array(required.map { .string($0) }) }
        return .object(schema)
    }

    private static func string(_ description: String, pattern: String? = nil, format: String? = nil) -> JSONValue {
        var schema: [String: JSONValue] = ["type": "string", "description": .string(description)]
        if let pattern { schema["pattern"] = .string(pattern) }
        if let format { schema["format"] = .string(format) }
        return .object(schema)
    }

    private static func boolean(_ description: String) -> JSONValue {
        .object(["type": "boolean", "description": .string(description)])
    }

    private static func cases<T: RawRepresentable & CaseIterable>(_ type: T.Type, _ description: String) -> JSONValue
    where T.RawValue == String {
        .object([
            "type": "string", "enum": .array(T.allCases.map { .string($0.rawValue) }),
            "description": .string(description),
        ])
    }

    private static let taskCode = string("A task code, such as HEALTH-7K2Q or 7K2Q.")
    private static let avenueKey = string("An Avenue key, such as HEALTH.")
    private static let projectID = string("A Project id from list_projects.", format: "uuid")
    private static let title = string("1 to 200 characters.")
    private static let status = cases(TaskStatus.self, "A task status.")

    // MARK: - Plan

    private static func array(_ items: JSONValue, _ description: String) -> JSONValue {
        .object(["type": "array", "items": items, "description": .string(description)])
    }

    private static let planDate = string(
        "YYYY-MM-DD, today, tomorrow, a weekday, next friday, +3d, +2w or +1m.")

    private static func planTask(subtasks: JSONValue?, placed: Bool) -> JSONValue {
        var properties: [String: JSONValue] = [
            "title": title,
            "details": string("Markdown notes."),
            "status": status,
            "priority": cases(Priority.self, "How urgent the task is."),
            "start": planDate,
            "due": planDate,
            "dueTime": string("24-hour HH:MM. Needs a due date.", pattern: "^[0-2][0-9]:[0-5][0-9]$"),
            "labels": array(["type": "string"], "Label names. Ones that don't exist are created."),
            "reminders": array(
                ["type": "string"],
                "\"1 day before due\", \"2 hours before due\", or a date with a time, such as \"tomorrow 09:00\"."),
            "comments": array(["type": "string"], "Markdown comments."),
        ]
        if let subtasks { properties["subtasks"] = array(subtasks, "Sub tasks.") }
        if placed {
            properties["place"] = string("An Avenue key, or \"KEY/Project name\" for an existing Project.")
            properties["parent"] = string("Code of an existing task to put this under, such as HEALTH-7K2Q.")
        }
        return object(properties, required: ["title"])
    }

    private static let plan: JSONValue = {
        let subtask = planTask(subtasks: nil, placed: false)
        return object([
            "avenues": array(
                object(
                    [
                        "key": string("2 to 10 capital letters, such as HEALTH."),
                        "name": string("Its name, such as Health."),
                        "color": string("A color name. Defaults to blue."),
                        "symbol": string("An SF Symbol name. Defaults to circle."),
                    ], required: ["key", "name"]),
                "New Avenues."),
            "labels": array(
                object(
                    ["name": string("The label."), "color": string("A color name. Defaults to gray.")],
                    required: ["name"]),
                "New labels with a color."),
            "projects": array(
                object(
                    [
                        "name": string("The goal's name."),
                        "avenue": string("The key of an existing Avenue or one of the plan's new ones."),
                        "details": string("Markdown notes."),
                        "due": planDate,
                        "tasks": array(planTask(subtasks: subtask, placed: false), "Its tasks."),
                    ], required: ["name", "avenue"]),
                "New Projects with their tasks."),
            "tasks": array(
                planTask(subtasks: subtask, placed: true),
                "Tasks outside the new Projects. Each needs place or parent."),
        ])
    }()

    private static let taskFields: [String: JSONValue] = [
        "details": string("Markdown notes."),
        "priority": cases(Priority.self, "How urgent the task is."),
        "startDate": string("YYYY-MM-DD.", format: "date"),
        "dueDate": string("YYYY-MM-DD.", format: "date"),
        "dueTime": string("24-hour HH:MM. Needs a due date.", pattern: "^[0-2][0-9]:[0-5][0-9]$"),
        "labels": .object([
            "type": "array", "items": ["type": "string"],
            "description": "Label names. Replaces every label on update.",
        ]),
    ]
}
