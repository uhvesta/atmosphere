import Foundation
import LifrAPI
import LifrCore

/// What the on-device model is told, and how its structured answer becomes
/// a reply. Kept apart from FoundationModels so it can be tested anywhere.
package enum AssistantPrompt {
    /// The most characters of conversation sent with each message. The
    /// on-device model has about 4,000 tokens for everything, so older
    /// messages are left out; the draft carries what was agreed.
    package static let historyBudget = 4_000

    package static func instructions(_ request: AssistantRequest) -> String {
        let lookup = request.lookup
        let today = request.today
        var text = """
            You help the person plan what to add to Lifr, their task manager. Ask short questions when you need \
            to, then propose a plan. Keep "message" to one to three sentences; the plan shows the details.

            Records:
            - Avenues are areas of life, with a KEY such as HEALTH.
            - Projects are goals inside an Avenue, with their own tasks.
            - Tasks can have Sub tasks, one level deep.

            Rules for the plan:
            - Put each top-level task somewhere: "place" is an Avenue KEY or "KEY/Project name" from the list \
            below, or "parent" is the code of an existing task to add it under.
            - Tasks inside a new Project need no place.
            - Only add an Avenue when none below fits and the person agrees.
            - Dates are YYYY-MM-DD, today, tomorrow, a weekday such as friday, next friday, +3d, +2w or +1m.
            - Leave out anything the person didn't ask for. Never invent dates.
            - When the person changes their mind, return the whole updated plan.
            - Leave "plan" out while you are still asking questions.

            Today is \(today), a \(weekday(today)).
            """
        text += "\n\nAvenues and their Projects:\n"
        text +=
            lookup.places.isEmpty
            ? "(none yet)\n"
            : lookup.places.map { place in
                "- \(place.key) (\(place.name))"
                    + (place.projects.isEmpty ? "" : ": " + place.projects.map { "\"\($0)\"" }.joined(separator: ", "))
            }.joined(separator: "\n") + "\n"
        if !lookup.labels.isEmpty {
            text += "Labels: " + lookup.labels.joined(separator: ", ") + "\n"
        }
        let tools = request.allowedTools
        let reads = AssistantTool.allCases.filter { !$0.writes && tools.contains($0) }
        if !reads.isEmpty {
            text +=
                "To answer questions about their tasks, use the tools: "
                + reads.map(\.use).joined(separator: ", ")
                + ". Answer from what the tools return; if they return nothing, say so."
        }
        let writes = AssistantTool.allCases.filter { $0.writes && tools.contains($0) }
        if !writes.isEmpty {
            text +=
                "\n\nTo change existing tasks when the person asks, use "
                + writes.map(\.use).joined(separator: ", ")
                + ". They save at once and the person can undo them. Say in \"message\" what you changed."
            if request.draftsPlans {
                text += " New tasks, Projects and Avenues always go in \"plan\" for the person to add."
            }
        }
        if !request.draftsPlans {
            text += "\n\nDon't propose a plan in this conversation: always leave \"plan\" out."
        }
        if !request.skills.isEmpty {
            text += "\n\nThe person attached these skills. Follow them:"
            for use in request.skills {
                text += "\n- \(use.skill.title): \(use.skill.instructions)"
                let note = use.note.trimmingCharacters(in: .whitespacesAndNewlines)
                if !note.isEmpty { text += "\n  Their note: \(note)" }
            }
        }
        return text
    }

    /// The conversation, the draft and its problem, as one prompt.
    package static func prompt(_ request: AssistantRequest) -> String {
        var text = "Conversation so far:\n"
        text += history(request.messages).map { message in
            (message.role == .person ? "Person: " : "You: ") + message.text
        }.joined(separator: "\n")
        if let draft = request.draft, !draft.isEmpty {
            text += "\n\nThe current plan, with any edits by the person:\n" + PlanFile.write(draft)
        }
        if let problem = request.problem {
            text += "\nIt can't be added yet: \(problem)\nFix that in your plan."
        }
        return text + "\n\nAnswer the person's last message."
    }

    /// The newest messages that fit `historyBudget`, oldest first. The last
    /// message is always kept, cut from the front when it's too long by itself.
    package static func history(_ messages: [ChatMessage]) -> [ChatMessage] {
        guard var last = messages.last else { return [] }
        if last.text.count > historyBudget { last.text = String(last.text.suffix(historyBudget)) }
        var kept = [last]
        var used = last.text.count
        for message in messages.dropLast().reversed() {
            used += message.text.count
            guard used <= historyBudget else { break }
            kept.append(message)
        }
        return kept.reversed()
    }

    private static func weekday(_ date: CalendarDate) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let names = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"]
        let noon = calendar.date(from: DateComponents(year: date.year, month: date.month, day: date.day, hour: 12))!
        return names[calendar.component(.weekday, from: noon) - 1]
    }

    // MARK: - Answers

    /// The structured answer as far as it's written. Fields the model hasn't
    /// reached are missing; tasks without a title yet are left out.
    /// Without `draftsPlans`, any plan the model writes anyway is dropped.
    package static func reply(from answer: JSONValue, isComplete: Bool, draftsPlans: Bool = true) -> AssistantReply {
        AssistantReply(
            text: answer["message"]?.text ?? "", plan: draftsPlans ? answer["plan"].flatMap(plan) : nil,
            isComplete: isComplete)
    }

    private static func plan(_ value: JSONValue) -> Plan? {
        var plan = Plan()
        plan.avenues =
            value["avenues"]?.items.compactMap { avenue in
                guard let key = avenue["key"]?.text, let name = avenue["name"]?.text else { return nil }
                return Plan.AvenueDraft(key: key, name: name)
            } ?? []
        plan.projects =
            value["projects"]?.items.compactMap { project in
                guard let name = project["name"]?.text, let avenue = project["avenue"]?.text else { return nil }
                return Plan.ProjectDraft(
                    name: name, avenue: avenue, details: project["details"]?.text ?? "", due: project["due"]?.text,
                    tasks: project["tasks"]?.items.compactMap { task($0, top: false) } ?? [])
            } ?? []
        plan.tasks = value["tasks"]?.items.compactMap { task($0, top: true) } ?? []
        return plan.isEmpty ? nil : plan
    }

    private static func task(_ value: JSONValue, top: Bool) -> Plan.TaskDraft? {
        guard let title = value["title"]?.text, !title.isEmpty else { return nil }
        var task = Plan.TaskDraft(title: title)
        task.details = value["details"]?.text ?? ""
        task.priority = value["priority"]?.text.flatMap(Priority.init(rawValue:)) ?? .none
        task.due = value["due"]?.text
        task.dueTime = value["dueTime"]?.text
        task.labels = value["labels"]?.items.compactMap(\.text) ?? []
        task.reminders = value["remind"]?.items.compactMap(\.text) ?? []
        task.subtasks = value["subtasks"]?.items.compactMap { self.task($0, top: false) } ?? []
        if top {
            task.place = value["place"]?.text
            task.parent = value["parent"]?.text
        }
        return task
    }
}

extension AssistantTool {
    /// How the instructions describe the tool.
    fileprivate var use: String {
        switch self {
        case .findTasks: "findTasks to search"
        case .getTask: "getTask for one task's details"
        case .overview: "overview for what's due, overdue or stalled"
        case .recentActivity: "recentActivity for what changed lately"
        case .updateTasks: "updateTasks to change tasks"
        case .addComment: "addComment to comment on a task"
        case .revert: "revert to undo changes"
        }
    }
}

extension JSONValue {
    fileprivate subscript(key: String) -> JSONValue? {
        guard case .object(let fields) = self, let value = fields[key], value != .null else { return nil }
        return value
    }

    fileprivate var text: String? {
        guard case .string(let text) = self else { return nil }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    fileprivate var items: [JSONValue] {
        if case .array(let items) = self { items } else { [] }
    }
}
