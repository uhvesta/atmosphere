import ChatKit
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

    /// The history a model with `budget` gets: Apple's 1,500 characters,
    /// and three quarters of whatever room it has past Apple's, at three
    /// characters a token. The rest goes to tools' results and the draft.
    package static func historyBudget(for budget: ContextBudget) -> Int {
        let more = max(0, budget.room - ContextBudget.appleOnDevice.room)
        return historyBudget + more * 3 * 3 / 4
    }

    /// How much of the request goes to the model. Lifr starts with the
    /// largest that fits its estimate, and when the model still says it's
    /// too long, tries the next.
    package enum Size: CaseIterable, Comparable, Sendable {
        /// Every allowed tool, the request's history budget of
        /// conversation, the labels, and the answer's shape spelled out in
        /// the prompt.
        case full
        /// The tools the last message needs, two fifths of the history
        /// budget and at least 600 characters of conversation, no labels,
        /// and the answer's shape only enforced.
        case smaller
        /// As `smaller`, with only the last message, cut to fit.
        case smallest

        /// The most characters of conversation, out of `full` at full size.
        package func historyBudget(_ full: Int) -> Int { self == .full ? full : max(600, full * 2 / 5) }
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

    /// How much of the window of a model with `budget` the turn took: the
    /// request at `size`, what the model thought, and its answer.
    package static func contextUse(
        _ request: AssistantRequest, size: Size, answer: String, thinking: ModelThinking?, budget: ContextBudget
    ) -> ContextUse {
        let answered = estimatedTokens(answer) + estimatedTokens(thinking?.text ?? "")
        let used = estimatedTokens(request, size: size) + answered
        return ContextUse(used: used, window: budget.window)
    }

    /// The estimated tokens of `request` at `size`, before the answer.
    package static func estimatedTokens(_ request: AssistantRequest, size: Size) -> Int {
        estimatedTokens(instructions(request, size: size)) + estimatedTokens(prompt(request, size: size))
            + tools(request, size: size).count * tokensPerTool
            + (size.schemaInPrompt && request.skill != nil
                ? schemaTokens + (request.changesTasks ? changesSchemaTokens : 0) : 0)
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

    /// The tools sent: those the skill allows, none without one. A skill
    /// that must add something or change a task gets no reading tools: a
    /// small model calls them anyway, and loses its way; the prompt lists
    /// the tasks it may mean. updateTasks and updateProjects aren't tools
    /// the model calls: their changes come in the answer, as `changes` and
    /// `projectChanges`.
    package static func tools(_ request: AssistantRequest, size: Size) -> Set<AssistantTool> {
        let allowed = request.allowedTools.subtracting([.updateTasks, .updateProjects])
        guard let skill = request.skill, skill.requiresPlan || skill.requiresChange else { return allowed }
        return allowed.filter(\.writes)
    }

    package static func instructions(_ request: AssistantRequest, size: Size = .full) -> String {
        instructions(
            request, tools: tools(request, size: size), draftsPlans: request.draftsPlans,
            changesTasks: !changeable(request).isEmpty, changesProjects: !changeableProjects(request).isEmpty,
            listsLabels: size.listsLabels)
    }

    /// The existing tasks the model may change in this answer: those the
    /// last message may mean, when the person lets it change tasks. With
    /// none, the answer has no changes, so the model can't change a task
    /// by a code it made up.
    package static func changeable(_ request: AssistantRequest) -> [AssistantLookup.OpenTask] {
        request.changesTasks ? meantTasks(request) : []
    }

    /// Whether the answer must change a task the prompt lists: the skill
    /// is Change and the last message may mean one. Then the reading tools
    /// aren't sent: the list has the codes, and a search only takes the
    /// model off course.
    package static func asksForChange(_ request: AssistantRequest) -> Bool {
        request.skill?.requiresChange == true && !changeable(request).isEmpty
    }

    /// The existing Projects the model may change in this answer: those
    /// the last message may mean, when the person lets it change Projects.
    package static func changeableProjects(_ request: AssistantRequest) -> [AssistantLookup.KnownProject] {
        request.changesProjects ? meantProjects(request) : []
    }

    /// Whether the answer may change a Project the prompt lists: the skill
    /// is Change and the last message may mean one. Then the answer must
    /// have a Project change, unless it may mean a task too.
    package static func asksForProjectChange(_ request: AssistantRequest) -> Bool {
        request.skill?.requiresChange == true && !changeableProjects(request).isEmpty
    }

    /// Whether the answer must hold a plan, or changes to the current one:
    /// the skill is Add.
    package static func asksForPlan(_ request: AssistantRequest) -> Bool {
        request.draftsPlans && request.skill?.requiresPlan == true
    }

    /// `plan` with each new Project the person didn't ask for made a task,
    /// its tasks the Sub tasks: a small model makes a Project of a task
    /// with steps. Projects are kept with Break down or a skill of the
    /// person's, when a message of the person's says project or goal, or
    /// when the draft has them already.
    package static func withAskedProjects(_ plan: Plan, _ request: AssistantRequest) -> Plan {
        if let skill = request.skill, skill.suggestsProjects || skill.isUser { return plan }
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
        changesProjects: Bool = false, listsLabels: Bool = true
    ) -> String {
        let lookup = request.lookup
        let today = request.today
        var text = """
            \(request.guidance)

            Keep "message" to one to three sentences; a plan or changes show the details.

            Records:
            - Avenues are areas of life, with a KEY such as HEALTH.
            - Projects are goals inside an Avenue, with their own tasks.
            - Tasks can have Sub tasks, one level deep.

            Rules for the plan, which only adds new things:
            - Give each top-level task a "place": an Avenue KEY or "KEY/Project name" from the list below. Use \
            the one the person names; when they name none, the one that fits best. To add a Sub task to an \
            existing task the prompt lists, give "parent", its code.
            - Steps of a new task go in its "subtasks". \(projectRule(request))
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
                + "Each change waits for the person to apply it."
        }
        if changesProjects {
            text +=
                "\n\nTo change Projects that already exist, such as to rename, describe, reschedule, complete, "
                + "reopen or archive them, put each in \"projectChanges\" with its name from the existing Projects "
                + "the prompt lists. Archiving a Project moves its open tasks out to its Avenue. Each change waits "
                + "for the person to apply it."
        }
        let writes = AssistantTool.allCases.filter { $0.writes && tools.contains($0) }
        if !writes.isEmpty {
            text +=
                "\n\nAlso, when the person asks, use " + writes.map(\.use).joined(separator: ", ")
                + ". What they change waits for the person to apply it."
        }
        if changesTasks || changesProjects || !writes.isEmpty {
            text +=
                " Say in \"message\" what you'd change: nothing is changed until the person applies it below "
                + "your message."
            if draftsPlans {
                text += " New tasks, Projects and Avenues always go in \"plan\" for the person to add."
            }
        }
        if !draftsPlans {
            text += "\n\nDon't propose a plan for this message: leave \"plan\" out."
        }
        if let skill = request.skill {
            text +=
                "\n\nFor this message, use the skill \(skill.title):\n"
                + skill.instructions
        }
        return text
    }

    /// When the plan may have a Project.
    private static func projectRule(_ request: AssistantRequest) -> String {
        request.skill?.suggestsProjects == true
            ? "Make a Project where several tasks serve one goal."
            : "Only make a Project when the person asks for one."
    }

    /// The instructions for talking without a skill: the person's guidance
    /// and today's date, so the model answers as it would anywhere.
    package static func chatInstructions(_ request: AssistantRequest) -> String {
        "\(request.guidance)\n\nToday is \(request.today), a \(weekday(request.today))."
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
        var text = ""
        if size.keepsEarlier, let summary = request.summary {
            text += "Earlier in the conversation, in short: \(summary)\n\n"
        }
        text += "Conversation so far:\n"
        let kept = kept(request, size: size)
        text += kept.enumerated().map { index, message in
            let said = said(message, attached: index == kept.count - 1 && !request.attachments.isEmpty)
            return switch (message.role, message.skill) {
            case (.person, let skill?): "Person, with the skill \(skill): " + said
            case (.person, nil): "Person: " + said
            case (.assistant, _): "You: " + said
            }
        }.joined(separator: "\n")
        guard request.skill != nil else { return text + "\n\nAnswer the person's last message." }
        let meant = meantTasks(request)
        if !meant.isEmpty {
            text +=
                "\n\nExisting tasks the last message may mean:\n"
                + meant.map { "- \($0.code) \"\($0.title)\" in \($0.place)" }.joined(separator: "\n")
            if asksForChange(request) {
                text += "\nIt asks to change one of them: put that in \"changes\"."
            }
        }
        let projects = meantProjects(request)
        if !projects.isEmpty {
            text +=
                "\n\nExisting Projects the last message may mean:\n"
                + projects.map { project in
                    "- \"\(project.place)\", \(project.status.rawValue)"
                        + (project.due.map { ", due \($0)" } ?? "")
                }.joined(separator: "\n")
            if asksForProjectChange(request) {
                text += "\nIt may ask to change one of them: put that in \"projectChanges\"."
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

    /// The messages sent at `size`: the newest that fit what's left of the
    /// history budget after the summary.
    private static func kept(_ request: AssistantRequest, size: Size) -> [ChatMessage] {
        guard size.keepsEarlier else {
            return history(Array(request.messages.suffix(1)), budget: size.historyBudget(request.historyBudget))
        }
        let budget = size.historyBudget(request.historyBudget) - (request.summary?.count ?? 0)
        return history(request.messages, budget: max(minimumHistory, budget))
    }

    /// The fewest characters of conversation sent beside a summary.
    private static let minimumHistory = 400

    /// How many of the request's messages aren't sent at `size`.
    package static func leftOut(_ request: AssistantRequest, size: Size) -> Int {
        request.messages.count - kept(request, size: size).count
    }

    // MARK: - Compacting

    /// The longest summary kept: it's sent with every message after it, so
    /// it takes room from the conversation.
    package static let summaryLimit = 600

    package static let summaryInstructions = """
        You summarize a conversation between a person and the assistant of Lifr, a task app, so the assistant \
        can go on with it later. In at most five short sentences, say what the person asked for, what was \
        decided or changed, and what's still open. Keep task names, dates and Avenue keys exactly as written. \
        Answer with only the summary, in plain sentences with no lists or formatting.
        """

    /// The characters of conversation the model with `budget` reads to
    /// summarize it: most of its room, as the answer is short and there
    /// are no tools.
    package static func summaryInputBudget(for budget: ContextBudget) -> Int {
        max(historyBudget, (budget.room - estimatedTokens(summaryInstructions)) * 3 * 3 / 4)
    }

    /// The summary before `messages`, and the newest of them that fit
    /// `budget` characters.
    package static func summaryPrompt(_ messages: [ChatMessage], earlier: String?, budget: Int) -> String {
        var text = ""
        if let earlier { text += "Summary so far: \(earlier)\n\nWhat came after:\n" }
        let kept = history(messages, budget: max(minimumHistory, budget - text.count))
        text += kept.map { "\($0.role == .person ? "Person" : "Assistant"): \($0.text)" }.joined(separator: "\n")
        return text + "\n\nSummarize the conversation" + (earlier == nil ? "." : ", the summary so far included.")
    }

    /// The model's answer as a summary: trimmed, and cut at a sentence's end
    /// to `summaryLimit`. Nil when it wrote nothing.
    package static func summary(from answer: String) -> String? {
        ChatSummary.text(from: answer, limit: summaryLimit)
    }

    /// What the scripted assistant summarizes any conversation as.
    package static let exampleSummary =
        "The person planned the week: cleaning the living room on Saturday and buying new clothes. Both are drafted."

    /// The most Projects `meantProjects` lists.
    package static let meantProjectsLimit = 4

    /// The Projects the last message may be about, of any status: those
    /// more than half of whose name's words it has, most words first.
    package static func meantProjects(_ request: AssistantRequest) -> [AssistantLookup.KnownProject] {
        let said = Set(words(request.messages.last?.text ?? ""))
        guard !said.isEmpty else { return [] }
        let scored = request.lookup.projects.compactMap { project -> (AssistantLookup.KnownProject, Int)? in
            let own = Set(words(project.name))
            let shared = own.intersection(said).count
            return !own.isEmpty && shared * 2 > own.count ? (project, shared) : nil
        }
        return scored.enumerated()
            .sorted { ($0.element.1, -$0.offset) > ($1.element.1, -$1.offset) }
            .prefix(meantProjectsLimit).map(\.element.0)
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
        messages.newest(fitting: budget)
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
        updateProjects changes Projects, addComment comments on a task, revert undoes changes.
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
        Conversation.name(from: answer)
    }

    /// What the model is told to pick a skill for the last message with:
    /// each of `skills` by ID, and when to use it.
    package static func pickInstructions(_ skills: [Skill]) -> String {
        "Pick the skill that fits the person's last message, or \"\(noSkill)\" when none does, such as for "
            + "small talk or a question that isn't about their tasks. When the last message answers or follows "
            + "up on the one before, keep that message's skill.\nSkills:\n"
            + skills.map { "- \($0.id): use when \($0.useWhen.isEmpty ? $0.summary : $0.useWhen)" }
            .joined(separator: "\n")
    }

    /// The pick for no skill.
    package static let noSkill = "none"

    /// The last few messages, newest last, for the model to pick a skill by.
    /// With `drafting`, says a plan is being drafted with that skill, which
    /// a message about the plan keeps.
    package static func pickPrompt(_ messages: [ChatMessage], drafting: String? = nil) -> String {
        let conversation = messages.suffix(3).map { message in
            let who =
                switch (message.role, message.skill) {
                case (.person, let skill?): "Person, with the skill \(skill)"
                case (.person, nil): "Person"
                case (.assistant, _): "Assistant"
                }
            return "\(who): \(message.text.prefix(600))"
        }
        .joined(separator: "\n")
        guard let drafting else { return conversation }
        return conversation + "\n\nA plan is being drafted with the skill \(drafting). Pick \(drafting) when the "
            + "last message adds to that plan or changes or removes something in it, such as \"also add…\", "
            + "\"change that to Friday\" or \"remove the second one\"."
    }

    /// The skill the model picked, if it's one of `skills`.
    package static func pickedSkill(from answer: JSONValue, in skills: [Skill]) -> String? {
        answer["skill"]?.text.flatMap { Skill.named($0, in: skills)?.id }
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

    /// The changes to existing Projects in the model's whole answer, the
    /// first to each Project.
    package static func projectChanges(from answer: JSONValue) -> [ProjectChange] {
        var seen: Set<String> = []
        return (answer["projectChanges"]?.items ?? []).compactMap { change in
            guard let project = change["project"]?.text, seen.insert(project.lowercased()).inserted else {
                return nil
            }
            return ProjectChange(
                project: project, name: change["rename"]?.text, details: change["details"]?.text,
                status: change["status"]?.text, due: change["due"]?.text)
        }
    }

    /// `reply` after its changes were made: tasks the plan would add a
    /// second time, with the title of a task just `changed`, left out;
    /// and what was `refused`, of tasks then Projects, said after the
    /// model's message.
    package static func reply(
        _ reply: AssistantReply, changed: [LifrTask], refused: [TaskChangeError],
        refusedProjects: [ProjectChangeError] = []
    ) -> AssistantReply {
        var reply = reply
        if var plan = reply.plan, !changed.isEmpty {
            let titles = changed.map { $0.title.lowercased() }
            plan.tasks.removeAll { titles.contains($0.title.lowercased()) }
            reply.plan = plan.isEmpty ? nil : plan
        }
        let notes = (refused.map(\.description) + refusedProjects.map(\.description)).joined(separator: " ")
        if !notes.isEmpty {
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
        case .updateProjects: "updateProjects to change Projects"
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
