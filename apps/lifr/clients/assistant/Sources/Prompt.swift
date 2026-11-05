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
            + tools(request, size: size).count * tokensPerTool
            + (size.schemaInPrompt ? schemaTokens + (request.changesTasks ? changesSchemaTokens : 0) : 0)
            + attachedTokens(request)
    }

    /// The schema of `changes`, when the answer has them.
    package static let changesSchemaTokens = 150

    /// What the model reads for the photos and recordings sent with the new
    /// message: none when none of their files are on this device.
    package static func attachedTokens(_ request: AssistantRequest) -> Int {
        guard !request.attachments.isEmpty else { return 0 }
        return (request.messages.last?.attachments ?? []).reduce(0) { $0 + $1.estimatedTokens }
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
    /// last message asks for what they do. A message that asks to add
    /// something or change a task gets no reading tools at any size: a
    /// small model calls them anyway, and loses its way.
    /// updateTasks isn't a tool the model calls: its changes come in the
    /// answer, as `changes`.
    package static func tools(_ request: AssistantRequest, size: Size) -> Set<AssistantTool> {
        let allowed = request.allowedTools.subtracting([.updateTasks])
        guard request.skill == nil else { return allowed }
        let reads = !asksForChange(request) && !asksForPlan(request)
        guard size != .full || !reads else { return allowed }
        let words = lastWords(request)
        let picked = AssistantTool.allCases.filter { tool in
            switch tool {
            case .findTasks, .getTask, .overview: reads
            case .recentActivity, .revert: !words.isDisjoint(with: undoWords)
            case .updateTasks: false
            case .addComment: !words.isDisjoint(with: commentWords)
            }
        }
        return allowed.intersection(picked)
    }

    private static let undoWords: Set<String> = [
        "undo", "revert", "back", "changed", "change", "changes", "activity", "recent", "happened", "did",
    ]
    private static let commentWords: Set<String> = ["comment", "note", "notes", "remark"]

    package static func instructions(_ request: AssistantRequest, size: Size = .full) -> String {
        instructions(
            request, tools: tools(request, size: size), draftsPlans: request.draftsPlans,
            changesTasks: !changeable(request).isEmpty, listsLabels: size.listsLabels)
    }

    /// The existing tasks the model may change in this answer: those the
    /// last message may mean, when the person lets it change tasks. With
    /// none, the answer has no changes, so the model can't change a task
    /// by a code it made up.
    package static func changeable(_ request: AssistantRequest) -> [AssistantLookup.OpenTask] {
        request.changesTasks ? meantTasks(request) : []
    }

    /// Whether the last message asks to change a task the prompt lists,
    /// such as "mark it done" or "move … to WORK". Then the answer must
    /// have a change, and the reading tools aren't sent: the list has the
    /// codes, and a search only takes the model off course.
    package static func asksForChange(_ request: AssistantRequest) -> Bool {
        !changeable(request).isEmpty && !lastWords(request).isDisjoint(with: changeWords)
    }

    private static let changeWords: Set<String> = [
        "rename", "call", "move", "put", "mark", "done", "finish", "finished", "complete", "completed", "tick",
        "push", "reschedule", "postpone", "delay", "change", "due", "priority", "urgent", "reopen", "won't",
    ]

    /// Whether the last message asks to add something or change the plan,
    /// such as "add…", "also…" or "remove the second one", rather than
    /// asking a question.
    package static func asksForPlan(_ request: AssistantRequest) -> Bool {
        let text = request.messages.last?.text ?? ""
        guard !text.contains("?") else { return false }
        let words = text.lowercased().split { !$0.isLetter && $0 != "'" }.map(String.init)
        return words.first.map(planWords.contains) == true || words.contains("add")
    }

    private static let planWords: Set<String> = [
        "add", "also", "make", "create", "plus", "remove", "delete", "drop", "change", "set", "and", "then",
        "actually", "instead", "new",
    ]

    /// `plan` with each new Project the person didn't ask for made a task,
    /// its tasks the Sub tasks: a small model makes a Project of a task
    /// with steps. Projects are kept with a skill, when a message of the
    /// person's says project or goal, or when the draft has them already.
    package static func withAskedProjects(_ plan: Plan, _ request: AssistantRequest) -> Plan {
        guard request.skill == nil else { return plan }
        let asked = request.messages.contains { message in
            message.role == .person && !Set(words(message.text)).isDisjoint(with: ["project", "goal"])
        }
        guard !asked else { return plan }
        let drafted = Set((request.draft?.projects ?? []).map { $0.name.lowercased() })
        var plan = plan
        let unasked = plan.projects.filter { !drafted.contains($0.name.lowercased()) }
        plan.projects.removeAll { !drafted.contains($0.name.lowercased()) }
        plan.tasks += unasked.map { project in
            var task = Plan.TaskDraft(title: project.name)
            task.place = project.avenue
            task.details = project.details
            task.due = project.due
            task.subtasks = project.tasks.map { subtask in
                var subtask = subtask
                subtask.subtasks = []
                return subtask
            }
            return task
        }
        return plan
    }

    /// The last message's words, in lower case.
    static func lastWords(_ request: AssistantRequest) -> Set<String> {
        Set((request.messages.last?.text ?? "").lowercased().split { !$0.isLetter && $0 != "'" }.map(String.init))
    }

    private static func instructions(
        _ request: AssistantRequest, tools: Set<AssistantTool>, draftsPlans: Bool, changesTasks: Bool,
        listsLabels: Bool = true
    ) -> String {
        let lookup = request.lookup
        let today = request.today
        var text = """
            \(request.guidance)

            Records:
            - Avenues are areas of life, with a KEY such as HEALTH.
            - Projects are goals inside an Avenue, with their own tasks.
            - Tasks can have Sub tasks, one level deep.

            Rules for the plan, which only adds new things:
            - Give each top-level task a "place": an Avenue KEY or "KEY/Project name" from the list below. Use \
            the one the person names; when they name none, the one that fits best. To add a Sub task to an \
            existing task the prompt lists, give "parent", its code.
            - Steps of a new task go in its "subtasks". Only make a Project when the person asks for one.
            - Only add an Avenue when none below fits and the person agrees.
            - Only fill in what the person said: no dates, priorities, reminders, labels, details or extra tasks \
            they didn't ask for.
            - For dates, write what the person said, such as friday, next monday, tomorrow, today, +3d or \
            +2w. Only write YYYY-MM-DD for a date the person gave as one.
            \(followUpRule(request))
            - Leave out the Avenues, Projects and labels listed below, such as those the person added earlier \
            in this chat.
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
        if changesTasks {
            text +=
                "\n\nTo change tasks that already exist, such as to rename, move, finish, reopen or reschedule "
                + "them, put each in \"changes\" with its code, from the existing tasks the prompt lists. Never put "
                + "an existing task in \"plan\": that would add it a second time. Tasks in the current plan "
                + "aren't saved yet: change those in \"\(draftTitles(request).isEmpty ? "plan" : "draftChanges")\". "
                + "Changes save at once and the person can undo them."
        }
        let writes = AssistantTool.allCases.filter { $0.writes && tools.contains($0) }
        if !writes.isEmpty {
            text +=
                "\n\nAlso, when the person asks, use " + writes.map(\.use).joined(separator: ", ")
                + ". They save at once and the person can undo them."
        }
        if changesTasks || !writes.isEmpty {
            text += " Say in \"message\" what you changed."
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
        instructions(request, tools: [], draftsPlans: false, changesTasks: false, listsLabels: false)
            + "\n\nAnswer in one to three plain sentences, with no lists or formatting. If the person asked for "
            + "new tasks, say what you'd add and ask them to send the message again for a plan."
    }

    /// The conversation, the draft and its problem, as one prompt.
    package static func prompt(_ request: AssistantRequest, size: Size = .full) -> String {
        let messages = size.keepsEarlier ? request.messages : Array(request.messages.suffix(1))
        var text = "Conversation so far:\n"
        let kept = history(messages, budget: size.historyBudget)
        text += kept.enumerated().map { index, message in
            let said = said(message, attached: index == kept.count - 1 && !request.attachments.isEmpty)
            return switch (message.role, message.skill) {
            case (.person, let skill?): "Person, with the skill \(skill): " + said
            case (.person, nil): "Person: " + said
            case (.assistant, _): "You: " + said
            }
        }.joined(separator: "\n")
        let meant = meantTasks(request)
        if !meant.isEmpty {
            text +=
                "\n\nExisting tasks the last message may mean:\n"
                + meant.map { "- \($0.code) \"\($0.title)\" in \($0.place)" }.joined(separator: "\n")
            if asksForChange(request) {
                text += "\nIt asks to change one of them: put that in \"changes\"."
            }
        }
        if let draft = request.draft, !draft.isEmpty {
            text += "\n\nThe current plan, with any edits by the person:\n"
            text +=
                draftTitles(request).isEmpty
                ? PlanFile.write(draft)
                : draftList(draft)
                    + "\nThese are in the plan already: never write them in \"plan\" again. Put only what the last "
                    + "message adds in \"plan\", and changes to these in \"draftChanges\" by number."
        }
        if let problem = request.problem {
            text += "\nIt can't be added yet: \(problem)\nFix that in your plan."
        }
        // Repeated after the plan, so a small model answers it and not the
        // message the plan came from.
        if !draftTitles(request).isEmpty, let last = request.messages.last?.text {
            return text + "\n\nAnswer the person's last message: “\(last)”"
        }
        return text + "\n\nAnswer the person's last message."
    }

    /// The most tasks `meantTasks` lists.
    package static let meantTasksLimit = 8

    /// The open top-level tasks the last message may be about: those more
    /// than half of whose words it has, most words first. So the model has
    /// their codes to change them without looking them up.
    package static func meantTasks(_ request: AssistantRequest) -> [AssistantLookup.OpenTask] {
        let said = Set(words(request.messages.last?.text ?? ""))
        guard !said.isEmpty else { return [] }
        let scored = request.lookup.tasks.compactMap { task -> (AssistantLookup.OpenTask, Int)? in
            let own = Set(words(task.title))
            let shared = own.intersection(said).count
            return !own.isEmpty && shared * 2 > own.count ? (task, shared) : nil
        }
        return scored.enumerated()
            .sorted { ($0.element.1, -$0.offset) > ($1.element.1, -$1.offset) }
            .prefix(meantTasksLimit).map(\.element.0)
    }

    /// The words of `text` that say what it's about, each without an
    /// ending such as -s, -ed or -ing, so "watered the plants" and "Water
    /// the plant" share theirs.
    private static func words(_ text: String) -> [String] {
        text.lowercased().split { !$0.isLetter && !$0.isNumber }.compactMap { word in
            var word = String(word)
            guard word.count > 2, !stopWords.contains(word) else { return nil }
            for ending in ["ing", "ed", "s"] where word.hasSuffix(ending) && word.count > ending.count + 2 {
                word.removeLast(ending.count)
                break
            }
            return word
        }
    }

    private static let stopWords: Set<String> = [
        "the", "and", "for", "with", "from", "into", "onto", "that", "this", "these", "those", "task", "tasks",
        "add", "please", "can", "you", "my", "our", "your", "its", "it's", "mark", "set", "change", "move",
        "rename", "push", "next", "all", "one", "new", "now", "today", "tomorrow",
    ]

    /// A message's text, with what was sent with it: the new message's
    /// photos and recordings are `attached` for the model to read; earlier
    /// ones are only named.
    private static func said(_ message: ChatMessage, attached: Bool) -> String {
        guard !message.attachments.isEmpty else { return message.text }
        let photos = message.attachments.count { $0.kind == .image }
        let recordings = message.attachments.count { $0.kind == .audio }
        let sent = [
            photos == 0 ? nil : photos == 1 ? "a photo" : "\(photos) photos",
            recordings == 0 ? nil : recordings == 1 ? "a recording" : "\(recordings) recordings",
        ].compactMap(\.self).joined(separator: " and ")
        let note = attached ? "[sent with \(sent), attached]" : "[sent with \(sent), no longer shown]"
        return message.text.isEmpty ? note : message.text + " " + note
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

    /// The changes to existing tasks in the model's whole answer, without
    /// those with no code. Only the first change to each task is kept: a
    /// later one restates the task as it was.
    package static func changes(from answer: JSONValue) -> [TaskChange] {
        var seen: Set<String> = []
        return (answer["changes"]?.items ?? []).compactMap { change in
            guard let code = change["code"]?.text, seen.insert(code.uppercased()).inserted else { return nil }
            return TaskChange(
                code: code, title: change["rename"]?.text, place: change["moveTo"]?.text,
                status: change["status"]?.text, priority: change["priority"]?.text, due: change["due"]?.text)
        }
    }

    /// `reply` after its changes were made: tasks the plan would add a
    /// second time, with the title of a task just `changed`, left out;
    /// and what was `refused` said after the model's message.
    package static func reply(
        _ reply: AssistantReply, changed: [LifrTask], refused: [TaskChangeError]
    ) -> AssistantReply {
        var reply = reply
        if var plan = reply.plan, !changed.isEmpty {
            let titles = changed.map { $0.title.lowercased() }
            plan.tasks.removeAll { titles.contains($0.title.lowercased()) }
            reply.plan = plan.isEmpty ? nil : plan
        }
        if !refused.isEmpty {
            let notes = refused.map(\.description).joined(separator: " ")
            reply.text = reply.text.isEmpty ? notes : reply.text + "\n\n" + notes
        }
        return reply
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

/// Reading the model's answers.
extension JSONValue {
    subscript(key: String) -> JSONValue? {
        guard case .object(let fields) = self, let value = fields[key], value != .null else { return nil }
        return value
    }

    /// The string, trimmed; nil when it's blank or not a string.
    var text: String? {
        guard case .string(let text) = self else { return nil }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    var items: [JSONValue] {
        if case .array(let items) = self { items } else { [] }
    }

    var bool: Bool? {
        if case .bool(let value) = self { value } else { nil }
    }
}
