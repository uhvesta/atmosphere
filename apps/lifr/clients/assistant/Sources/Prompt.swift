import Foundation
import LifrAPI
import LifrCore
import LifrModelsClient

/// What the model is told, and how its structured answer becomes a reply.
/// Kept apart from every runtime so it can be tested anywhere.
package enum AssistantPrompt {
    /// The most characters of conversation sent with each message at full
    /// size. Apple's on-device model has about 4,000 tokens for everything,
    /// so older messages are left out; the draft carries what was agreed.
    package static let historyBudget = 1_500

    /// How much of the request goes to the model. Lifr starts with the
    /// largest that fits its estimate, and when the model still says it's
    /// too long, tries the next.
    package enum Size: CaseIterable, Comparable, Sendable {
        /// Every allowed tool, 1,500 characters of conversation, the labels,
        /// and the answer's shape spelled out in the prompt.
        case full
        /// The tools the last message needs, 600 characters of
        /// conversation, no labels, and the answer's shape only enforced.
        case smaller
        /// As `smaller`, with only the last message, cut to 600 characters.
        case smallest

        /// The most characters of conversation.
        package var historyBudget: Int { self == .full ? AssistantPrompt.historyBudget : 600 }
        /// Whether messages before the last are sent.
        package var keepsEarlier: Bool { self != .smallest }
        package var listsLabels: Bool { self == .full }
        /// Whether the answer's schema is written into the prompt as well
        /// as enforced.
        package var schemaInPrompt: Bool { self == .full }
        /// The most characters of each tool's result.
        package var toolResultBudget: Int { self == .full ? 1_200 : 600 }
    }

    /// Each tool's description and parameters, and the answer's schema, as
    /// the model sees them.
    package static let tokensPerTool = 130
    package static let schemaTokens = 550

    /// A cautious count of the tokens in `text`: one per three bytes. The
    /// on-device model's own count needs a newer SDK.
    package static func estimatedTokens(_ text: String) -> Int {
        (text.utf8.count + 2) / 3
    }

    /// The estimated tokens of `request` at `size`, before the answer.
    package static func estimatedTokens(_ request: AssistantRequest, size: Size) -> Int {
        estimatedTokens(instructions(request, size: size)) + estimatedTokens(prompt(request, size: size))
            + tools(request, size: size).count * tokensPerTool + (size.schemaInPrompt ? schemaTokens : 0)
    }

    /// The sizes to try with a model that has `budget`, largest first: from
    /// the largest whose estimate leaves room for the answer. Empty when not
    /// even the smallest fits.
    package static func sizes(_ request: AssistantRequest, budget: ContextBudget) -> [Size] {
        let room = budget.room
        guard let first = Size.allCases.first(where: { estimatedTokens(request, size: $0) <= room }) else {
            return []
        }
        return Size.allCases.filter { $0 >= first }
    }

    /// The tools sent at `size`: every allowed one at full size, or with a
    /// skill; otherwise the reading tools, and the others only when the
    /// last message asks for what they do.
    package static func tools(_ request: AssistantRequest, size: Size) -> Set<AssistantTool> {
        let allowed = request.allowedTools
        guard size != .full, request.skill == nil else { return allowed }
        let words = Set(
            (request.messages.last?.text ?? "").lowercased()
                .split { !$0.isLetter && $0 != "'" }.map(String.init))
        let picked = AssistantTool.allCases.filter { tool in
            switch tool {
            case .findTasks, .getTask, .overview: true
            case .recentActivity, .revert: !words.isDisjoint(with: undoWords)
            case .updateTasks: !words.isDisjoint(with: changeWords)
            case .addComment: !words.isDisjoint(with: commentWords)
            }
        }
        return allowed.intersection(picked)
    }

    private static let undoWords: Set<String> = [
        "undo", "revert", "back", "changed", "change", "changes", "activity", "recent", "happened", "did",
    ]
    private static let changeWords: Set<String> = [
        "mark", "done", "finish", "finished", "complete", "completed", "close", "reopen", "start", "started",
        "move", "set", "change", "rename", "priority", "urgent", "postpone", "reschedule", "push", "delay",
        "won't", "wont", "cancel", "drop", "update",
    ]
    private static let commentWords: Set<String> = ["comment", "note", "notes", "remark"]

    package static func instructions(_ request: AssistantRequest, size: Size = .full) -> String {
        instructions(
            request, tools: tools(request, size: size), draftsPlans: request.draftsPlans,
            listsLabels: size.listsLabels)
    }

    private static func instructions(
        _ request: AssistantRequest, tools: Set<AssistantTool>, draftsPlans: Bool, listsLabels: Bool = true
    ) -> String {
        let lookup = request.lookup
        let today = request.today
        var text = """
            \(request.guidance)

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
        if listsLabels, !lookup.labels.isEmpty {
            text += "Labels: " + lookup.labels.joined(separator: ", ") + "\n"
        }
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
            if draftsPlans {
                text += " New tasks, Projects and Avenues always go in \"plan\" for the person to add."
            }
        }
        if !draftsPlans {
            text += "\n\nDon't propose a plan for this message: leave \"plan\" out."
        }
        if let skill = request.skill {
            text +=
                "\n\nFor this message the person picked the skill \(skill.title). Follow it:\n"
                + skill.instructions
        }
        return text
    }

    /// The instructions for a plain-text answer, used when the model's
    /// structured answer couldn't be read: the same, without tools or a
    /// plan.
    package static func plainInstructions(_ request: AssistantRequest) -> String {
        instructions(request, tools: [], draftsPlans: false, listsLabels: false)
            + "\n\nAnswer in one to three plain sentences, with no lists or formatting. If the person asked for "
            + "new tasks, say what you'd add and ask them to send the message again for a plan."
    }

    /// The conversation, the draft and its problem, as one prompt.
    package static func prompt(_ request: AssistantRequest, size: Size = .full) -> String {
        let messages = size.keepsEarlier ? request.messages : Array(request.messages.suffix(1))
        var text = "Conversation so far:\n"
        text += history(messages, budget: size.historyBudget).map { message in
            switch (message.role, message.skill) {
            case (.person, let skill?): "Person, with the skill \(skill): " + message.text
            case (.person, nil): "Person: " + message.text
            case (.assistant, _): "You: " + message.text
            }
        }.joined(separator: "\n")
        if let draft = request.draft, !draft.isEmpty {
            text += "\n\nThe current plan, with any edits by the person:\n" + PlanFile.write(draft)
        }
        if let problem = request.problem {
            text += "\nIt can't be added yet: \(problem)\nFix that in your plan."
        }
        return text + "\n\nAnswer the person's last message."
    }

    /// The newest messages that fit `budget` characters, oldest first. The
    /// last message is always kept, cut from the front when it's too long
    /// by itself.
    package static func history(_ messages: [ChatMessage], budget: Int = historyBudget) -> [ChatMessage] {
        guard var last = messages.last else { return [] }
        if last.text.count > budget { last.text = String(last.text.suffix(budget)) }
        var kept = [last]
        var used = last.text.count
        for message in messages.dropLast().reversed() {
            used += message.text.count
            guard used <= budget else { break }
            kept.append(message)
        }
        return kept.reversed()
    }

    private static func weekday(_ date: CalendarDate) -> String {
        let names = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"]
        return names[CalendarDate.utcCalendar.component(.weekday, from: date.noonUTC) - 1]
    }

    // MARK: - Writing skills

    /// What the model is told when it writes a skill for the person.
    package static let skillInstructions = """
        You write skills for Lifr's assistant. A skill is instructions the assistant follows for one message, \
        and the tools it may use. From the person's description, write:
        - "name": two or three words.
        - "summary": one short line saying what the skill is for.
        - "instructions": two to four sentences addressed to the assistant, saying what to do and when to ask \
        before changing anything.
        - "tools": only those the skill needs. findTasks searches tasks, getTask reads one, overview shows \
        what's due, overdue or stalled, recentActivity shows recent changes, updateTasks changes tasks, \
        addComment comments on a task, revert undoes changes.
        - "draftsPlans": true only if the skill adds new tasks, Projects or Avenues.
        """

    package static let titleInstructions = """
        You name conversations between a person and the assistant of Lifr, a task app. Answer with only a \
        title of two to five words saying what the conversation is about: no quotes, no punctuation at the \
        end, and no words like "conversation" or "chat".
        """

    /// The start of the conversation, for the model to name.
    package static func titlePrompt(_ messages: [ChatMessage]) -> String {
        messages.prefix(4).map { "\($0.role == .person ? "Person" : "Assistant"): \($0.text.prefix(500))" }
            .joined(separator: "\n")
    }

    /// The model's answer as a title: its first line, without quotes,
    /// a "Title:" label or a final full stop, cut to 40 characters. Nil
    /// when nothing is left.
    package static func title(from answer: String) -> String? {
        var line = answer.split(whereSeparator: \.isNewline).first.map(String.init) ?? ""
        if line.lowercased().hasPrefix("title:") { line = String(line.dropFirst(6)) }
        line = line.trimmingCharacters(in: .whitespaces.union(CharacterSet(charactersIn: "\"'“”‘’*.")))
        guard !line.isEmpty else { return nil }
        return line.count > 40 ? String(line.prefix(39)) + "…" : line
    }

    /// The skill in the model's answer, or nil when it has no instructions.
    package static func skillDraft(from answer: JSONValue) -> SkillDraft? {
        guard let instructions = answer["instructions"]?.text else { return nil }
        return SkillDraft(
            name: answer["name"]?.text ?? "", summary: answer["summary"]?.text ?? "", instructions: instructions,
            tools: Set(answer["tools"]?.items.compactMap { $0.text.flatMap(AssistantTool.init(rawValue:)) } ?? []),
            draftsPlans: answer["draftsPlans"] == .bool(true))
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
