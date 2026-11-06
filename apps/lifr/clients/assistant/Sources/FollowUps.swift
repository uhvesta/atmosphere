import LifrAPI
import LifrCore

/// A change the model makes to a task of the current plan, which isn't
/// saved yet. Fields left nil stay as they are.
package struct DraftChange: Hashable, Sendable {
    /// The title of the plan's task.
    package var task: String
    package var remove: Bool
    package var rename: String?
    /// A date as a plan writes it, or "none" to clear it.
    package var due: String?
    package var priority: Priority?
    /// An Avenue key, or "KEY/Project name".
    package var moveTo: String?

    package init(
        task: String, remove: Bool = false, rename: String? = nil, due: String? = nil, priority: Priority? = nil,
        moveTo: String? = nil
    ) {
        self.task = task
        self.remove = remove
        self.rename = rename
        self.due = due
        self.priority = priority
        self.moveTo = moveTo
    }
}

extension AssistantPrompt {
    /// The titles of the current plan's tasks the model may change, in the
    /// plan's order, each once. Empty without a plan, or when this message
    /// can't change it.
    /// A plan that can't be added is written out whole for the model to fix.
    package static func draftTitles(_ request: AssistantRequest) -> [String] {
        guard request.draftsPlans, request.problem == nil, let draft = request.draft else { return [] }
        var seen: Set<String> = []
        return (draft.tasks + draft.projects.flatMap(\.tasks)).map(\.title)
            .filter { !$0.isEmpty && seen.insert($0.lowercased()).inserted }
    }

    /// How the model changes a plan it wrote earlier in the chat: with one,
    /// it only writes the changes and what's new, as copying the whole
    /// plan back, a small model drops what it was asked to add.
    static func followUpRule(_ request: AssistantRequest) -> String {
        draftTitles(request).isEmpty
            ? "- When the person changes the plan, such as \"also add…\", \"change that to friday\" or \"remove "
                + "the second one\", return the whole plan with that change, and everything else as it was."
            : "- To change or remove the current plan's tasks, such as \"change that to friday\" or \"remove the "
                + "second one\", put each in \"draftChanges\" by its number. Put only new things, such as for "
                + "\"also add…\", in \"plan\": the current plan stays as it is."
    }

    /// The plan without the tasks a small model adds to a Project that only
    /// restate its goal, such as "Run half marathon" in Half marathon, when
    /// the person didn't say them.
    package static func withoutRestatedGoals(_ plan: Plan, _ request: AssistantRequest) -> Plan {
        let said = request.messages.filter { $0.role == .person }.map { $0.text.lowercased() }
        func restates(_ task: Plan.TaskDraft, _ project: String) -> Bool {
            let title = task.title.lowercased()
            return !project.isEmpty && title != project && title.contains(project)
                && !said.contains { $0.contains(title) }
        }
        var plan = plan
        plan.tasks.removeAll { task in
            guard let slash = task.place?.firstIndex(of: "/"), let place = task.place else { return false }
            return restates(task, place[place.index(after: slash)...].lowercased())
        }
        for index in plan.projects.indices {
            let name = plan.projects[index].name.lowercased()
            plan.projects[index].tasks.removeAll { restates($0, name) }
        }
        return plan
    }

    /// The draft's tasks as a numbered list in `draftTitles` order, with
    /// what they have: written as a plan, a small model copies it back.
    static func draftList(_ draft: Plan) -> String {
        var seen: Set<String> = []
        let tasks =
            draft.tasks.map { ($0, $0.place ?? "") }
            + draft.projects.flatMap { project in
                project.tasks.map { ($0, project.avenue + "/" + project.name + " (a new Project)") }
            }
        let avenues = draft.avenues.map { "- New Avenue \($0.key) (\($0.name))" }
        return
            (avenues
            + tasks.filter { !$0.0.title.isEmpty && seen.insert($0.0.title.lowercased()).inserted }
            .enumerated().map { index, item in
                let (task, place) = item
                var parts = ["\(index + 1). \(task.title)"]
                if !place.isEmpty { parts.append("in " + place) }
                if let due = task.due { parts.append("due " + due) }
                if task.priority != .none { parts.append(task.priority.rawValue) }
                if !task.subtasks.isEmpty {
                    parts.append("Sub tasks: " + task.subtasks.map(\.title).joined(separator: "; "))
                }
                return parts.joined(separator: ", ")
            }).joined(separator: "\n")
    }

    /// Whether the last message asks to add something, such as "also add…",
    /// rather than to change what's there.
    package static func asksToAdd(_ request: AssistantRequest) -> Bool {
        !lastWords(request).isDisjoint(with: ["add", "also", "plus", "another", "create"])
    }

    /// The phrases the last message quotes, such as “Call the electrician”,
    /// leaving out the titles of `tasks`: what a rename may be to.
    package static func quoted(_ request: AssistantRequest, besides tasks: [String]) -> [String] {
        let text = request.messages.last?.text ?? ""
        let titles = Set(tasks.map { $0.lowercased() })
        var phrases: [String] = []
        for (open, close) in [("“", "”"), ("\"", "\"")] {
            var rest = Substring(text)
            while let start = rest.range(of: open), let end = rest[start.upperBound...].range(of: close) {
                let phrase = rest[start.upperBound..<end.lowerBound].trimmingCharacters(in: .whitespaces)
                if !phrase.isEmpty, !titles.contains(phrase.lowercased()), !phrases.contains(phrase) {
                    phrases.append(phrase)
                }
                rest = rest[end.upperBound...]
            }
        }
        return phrases
    }

    /// What a task titled `old` is renamed to, from the model's `rename`:
    /// a small model often writes the old title back, which means the one
    /// phrase the person quoted besides it. Nil when it stays as it is.
    package static func renamed(_ old: String, to rename: String?, _ request: AssistantRequest) -> String? {
        guard let rename, !rename.isEmpty else { return nil }
        guard rename.caseInsensitiveCompare(old) == .orderedSame else { return rename }
        let phrases = quoted(request, besides: [old])
        return phrases.count == 1 ? phrases.first : nil
    }

    /// The date a change is to, from the model's `due`: a small model
    /// often works out a date the person named as a day, such as next
    /// Monday, and gets it wrong, so when it writes a date the last message
    /// doesn't have, the one day that message names is used instead.
    package static func dated(_ due: String?, _ request: AssistantRequest) -> String? {
        guard let due, (try? CalendarDate(due)) != nil else { return due }
        let text = (request.messages.last?.text ?? "").lowercased()
        guard !text.contains(due) else { return due }
        let words = text.split { !$0.isLetter }.map(String.init)
        let today = CalendarDate(request.now, in: request.timeZone)
        var named: [String] = []
        for (index, word) in words.enumerated() {
            let phrase = index > 0 && words[index - 1] == "next" ? "next " + word : word
            guard dayWords.contains(word), CalendarDate.resolving(phrase, today: today) != nil,
                !named.contains(phrase)
            else { continue }
            named.append(phrase)
        }
        return named.count == 1 ? named.first : due
    }

    /// The words that name a day by themselves.
    private static let dayWords: Set<String> = [
        "today", "tomorrow", "sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday",
    ]

    /// The changes to the current plan's tasks in the model's whole answer,
    /// whose tasks are given by their number in `titles`, counting from 1,
    /// or by title. Changes that name none of them are left out.
    package static func draftChanges(
        from answer: JSONValue, titles: [String], _ request: AssistantRequest
    ) -> [DraftChange] {
        let items = answer["draftChanges"]?.items ?? []
        let latest = items.count == 1 && meansTheLatest(request, titles: titles) ? titles.last : nil
        return items.compactMap { change in
            guard let given = change["task"]?.text else { return nil }
            let task: String
            if let latest {
                task = latest
            } else if let number = Int(given), titles.indices.contains(number - 1) {
                task = titles[number - 1]
            } else if let title = titles.first(where: { $0.caseInsensitiveCompare(given) == .orderedSame }) {
                task = title
            } else {
                return nil
            }
            return DraftChange(
                task: task, remove: change["remove"]?.bool ?? false,
                rename: renamed(task, to: change["rename"]?.text, request),
                due: dated(change["due"]?.text, request),
                // The model writes none for every change it makes, so none
                // changes nothing.
                priority: change["priority"]?.text.flatMap(Priority.init(rawValue:)).flatMap { $0 == .none ? nil : $0 },
                moveTo: change["moveTo"]?.text)
        }
    }

    /// Whether the last message changes "that" or "it" without saying which
    /// task, such as "change that to friday": the one added last. A small
    /// model often changes the first instead.
    static func meansTheLatest(_ request: AssistantRequest, titles: [String]) -> Bool {
        let words = lastWords(request)
        let text = (request.messages.last?.text ?? "").lowercased()
        // In a draft with Projects, the last title needn't be the newest.
        return titles.count > 1 && request.draft?.projects.isEmpty == true && !words.isDisjoint(with: ["that", "it"])
            && words.isDisjoint(with: whichTask) && !titles.contains { text.contains($0.lowercased()) }
    }

    /// Words that say which tasks a change is to.
    private static let whichTask: Set<String> = [
        "first", "second", "third", "fourth", "fifth", "last", "one", "ones", "all", "both", "them", "those",
        "these", "each", "every",
    ]

    /// The whole plan after a follow-up: the `draft` with `changes` made to
    /// its tasks, and the new things the model's `added` plan has. Nil when
    /// the answer neither adds nor changes anything, so the draft stays.
    /// Tasks of `added` that the draft has already are its copy of the
    /// draft, and left for `Plan.repaired` to make one; a removed task is
    /// left out even when `added` has it again.
    package static func followUp(_ added: Plan?, changes: [DraftChange], to draft: Plan) -> Plan? {
        guard added != nil || !changes.isEmpty else { return nil }
        var plan = draft
        if let added {
            plan.avenues += added.avenues.filter { new in
                !plan.avenues.contains { $0.key.caseInsensitiveCompare(new.key) == .orderedSame }
            }
            plan.projects += added.projects.filter { new in
                !plan.projects.contains { $0.name.caseInsensitiveCompare(new.name) == .orderedSame }
            }
            plan.labels += added.labels.filter { !plan.labels.contains($0) }
            plan.tasks += added.tasks.filter { new in
                !plan.tasks.contains { $0.title.caseInsensitiveCompare(new.title) == .orderedSame }
            }
        }
        for change in changes {
            let matches = { (task: Plan.TaskDraft) in task.title.caseInsensitiveCompare(change.task) == .orderedSame }
            if change.remove {
                plan.tasks.removeAll(where: matches)
                for index in plan.projects.indices { plan.projects[index].tasks.removeAll(where: matches) }
                continue
            }
            func apply(_ task: inout Plan.TaskDraft, top: Bool) {
                guard matches(task) else { return }
                if let rename = change.rename { task.title = rename }
                if let due = change.due { task.due = due.lowercased() == "none" ? nil : due }
                if let priority = change.priority { task.priority = priority }
                if top, let moveTo = change.moveTo { task.place = moveTo }
            }
            for index in plan.tasks.indices { apply(&plan.tasks[index], top: true) }
            for project in plan.projects.indices {
                for index in plan.projects[project].tasks.indices {
                    apply(&plan.projects[project].tasks[index], top: false)
                }
            }
        }
        return plan
    }
}
