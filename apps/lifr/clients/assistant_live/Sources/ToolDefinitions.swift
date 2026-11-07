import LifrAPI
import LifrAssistantClient
import LifrModelsClient

extension AssistantTool {
    /// The tool as a model sees it, running on `tools`. Nil for updateTasks
    /// and updateProjects, whose changes come in the answer.
    func definition(_ tools: AssistantTools) -> ModelTool? {
        switch self {
        case .findTasks:
            ModelTool(
                name: "findTasks",
                description:
                    "Search the person's tasks. Returns one line per task: code, title, status, place, due date.",
                parameters: [
                    ModelProperty("words", "Words in the title or details; empty for all."),
                    Shapes.optional("avenue", "Only in this Avenue KEY."),
                    Shapes.optional(
                        "status", "Only with this status. Without one, open tasks.",
                        .choice(name: "Status", ["todo", "in_progress", "procrastinated", "done", "wont_do"])),
                    Shapes.optional("dueBy", "Only due on or before this date, such as today or +7d."),
                ]
            ) { arguments in
                tools.capped(
                    tools.findTasks(
                        words: arguments.string("words") ?? "", avenue: arguments.string("avenue"),
                        status: arguments.string("status"), dueBy: arguments.string("dueBy")))
            }
        case .getTask:
            ModelTool(
                name: "getTask", description: "One task by its code, with details, Sub tasks and comments.",
                parameters: [ModelProperty("code", "Such as HOME-0003.")]
            ) { arguments in
                tools.capped(tools.getTask(code: arguments.string("code") ?? ""))
            }
        case .overview:
            ModelTool(
                name: "overview",
                description: "Open tasks per Avenue, and what's overdue, due today, due this week and procrastinated.",
                parameters: []
            ) { _ in
                tools.capped(tools.overview())
            }
        case .recentActivity:
            ModelTool(
                name: "recentActivity", description: "The newest changes: when, who, what. Each has a ref for revert.",
                parameters: []
            ) { _ in
                tools.capped(tools.recentActivity())
            }
        case .updateTasks, .updateProjects:
            // Changes come in the answer's `changes` and `projectChanges`.
            nil
        case .addComment:
            ModelTool(
                name: "addComment", description: "Add a comment to a task now. The person can undo it.",
                parameters: [
                    ModelProperty("code", "Such as HOME-0003."),
                    ModelProperty("text", "The comment."),
                ]
            ) { arguments in
                await tools.addComment(code: arguments.string("code") ?? "", text: arguments.string("text") ?? "")
            }
        case .revert:
            ModelTool(
                name: "revert",
                description:
                    "Undo changes: a ref from recentActivity, last for the newest, or conversation for all yours.",
                parameters: [ModelProperty("ref", "A ref, last, or conversation.")]
            ) { arguments in
                await tools.revert(ref: arguments.string("ref") ?? "")
            }
        }
    }
}

extension JSONValue {
    /// The string field `key` of an object.
    fileprivate func string(_ key: String) -> String? {
        guard case .object(let fields) = self, case .string(let text)? = fields[key] else { return nil }
        return text
    }

    /// The strings in the array field `key` of an object.
}
