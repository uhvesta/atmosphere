import Foundation

/// Records to add in one go, as a person or an assistant described them:
/// new Avenues, labels and Projects, and tasks with their Sub tasks. Names,
/// keys, codes and dates are kept as written; `Workspace.apply` looks them
/// up and works them out, so a plan reads the same however it was made.
package struct Plan: Hashable, Codable, Sendable {
    package var avenues: [AvenueDraft]
    package var labels: [LabelDraft]
    package var projects: [ProjectDraft]
    /// Tasks outside the plan's new Projects. Each says where it goes.
    package var tasks: [TaskDraft]

    package init(
        avenues: [AvenueDraft] = [], labels: [LabelDraft] = [], projects: [ProjectDraft] = [],
        tasks: [TaskDraft] = []
    ) {
        self.avenues = avenues
        self.labels = labels
        self.projects = projects
        self.tasks = tasks
    }

    package struct AvenueDraft: Hashable, Codable, Sendable {
        package var key: String
        package var name: String
        package var color: String
        package var symbol: String

        package init(key: String, name: String, color: String = "blue", symbol: String = "circle") {
            self.key = key
            self.name = name
            self.color = color
            self.symbol = symbol
        }
    }

    /// A label to create with a color. Tasks may also name labels that
    /// aren't listed here; those are created gray when nothing matches.
    package struct LabelDraft: Hashable, Codable, Sendable {
        package var name: String
        package var color: String

        package init(name: String, color: String = "gray") {
            self.name = name
            self.color = color
        }
    }

    package struct ProjectDraft: Hashable, Codable, Sendable {
        package var name: String
        /// The key of an existing Avenue or one of the plan's new ones.
        package var avenue: String
        package var details: String
        /// A date as `CalendarDate.resolving` reads it.
        package var due: String?
        package var tasks: [TaskDraft]

        package init(name: String, avenue: String, details: String = "", due: String? = nil, tasks: [TaskDraft] = []) {
            self.name = name
            self.avenue = avenue
            self.details = details
            self.due = due
            self.tasks = tasks
        }
    }

    /// A Task, or a Sub task when it's in another draft's `subtasks` or has
    /// a `parent`.
    package struct TaskDraft: Hashable, Codable, Sendable {
        package var title: String
        package var details: String
        package var status: TaskStatus
        package var priority: Priority
        /// Dates as `CalendarDate.resolving` reads them.
        package var start: String?
        package var due: String?
        /// 24-hour HH:MM.
        package var dueTime: String?
        /// Label names, matched without regard to case.
        package var labels: [String]
        /// Reminders as `Reminder.Trigger.resolving` reads them.
        package var reminders: [String]
        package var comments: [String]
        package var subtasks: [TaskDraft]
        /// For a task outside a new Project: an Avenue key, or "KEY/Project
        /// name" for an existing Project.
        package var place: String?
        /// For a Sub task of an existing task: its code, such as HEALTH-7K2Q.
        package var parent: String?

        package init(
            title: String, details: String = "", status: TaskStatus = .todo, priority: Priority = .none,
            start: String? = nil, due: String? = nil, dueTime: String? = nil, labels: [String] = [],
            reminders: [String] = [], comments: [String] = [], subtasks: [TaskDraft] = [],
            place: String? = nil, parent: String? = nil
        ) {
            self.title = title
            self.details = details
            self.status = status
            self.priority = priority
            self.start = start
            self.due = due
            self.dueTime = dueTime
            self.labels = labels
            self.reminders = reminders
            self.comments = comments
            self.subtasks = subtasks
            self.place = place
            self.parent = parent
        }
    }

    /// Whether the plan adds nothing.
    package var isEmpty: Bool {
        avenues.isEmpty && labels.isEmpty && projects.isEmpty && tasks.isEmpty
    }

    /// The plan without what `workspace` already has: an Avenue with the
    /// same key and name, and a label with the same name. Tasks that name
    /// such an Avenue's key go in the one that's there.
    package func leavingOut(_ workspace: Workspace) -> Plan {
        var plan = self
        plan.avenues.removeAll { draft in
            workspace.avenues.values.contains {
                $0.key.rawValue.caseInsensitiveCompare(draft.key) == .orderedSame
                    && $0.name.caseInsensitiveCompare(draft.name) == .orderedSame
            }
        }
        plan.labels.removeAll { workspace.label(named: $0.name) != nil }
        return plan
    }

    /// How many tasks and Sub tasks the plan adds.
    package var taskCount: Int {
        (projects.flatMap(\.tasks) + tasks).reduce(0) { $0 + 1 + $1.subtasks.count }
    }
}

/// Why a plan can't be added. `LifrError.plan` says where in the plan.
package indirect enum PlanProblem: Hashable, Sendable {
    case empty
    case unknownAvenue(key: String, suggestion: String?)
    case unknownProject(name: String, avenue: String, suggestion: String?)
    /// A new Project with the name of one already in that Avenue.
    case projectExists(name: String, avenue: String)
    /// A task outside a new Project with neither a place nor a parent.
    case noPlace
    /// A place or parent on a task that already has one, such as a task
    /// inside a new Project, or a Sub task.
    case placeGivenTwice
    /// Sub tasks under a Sub task.
    case subtaskTooDeep
    case invalidDate(String)
    case invalidReminder(String)
    /// A new Done or Won't Do task with open Sub tasks.
    case closedWithOpenSubtasks
    /// A Core rule refused one of the plan's records.
    case rule(LifrError)
}

extension Plan {
    package static let dateHelp =
        "Use YYYY-MM-DD, today, tomorrow, a weekday such as friday (the next one), or +3d, +2w, +1m."
    package static let reminderHelp =
        "Use \"1 day before due\" (minutes, hours, days or weeks), or a date and time such as "
        + "\"tomorrow 09:00\" or \"2026-11-18 17:30\"."
}

extension Workspace {
    /// Adds everything in `plan`, or nothing if any part of it is refused.
    /// The change covers every record added, so undoing it removes them all.
    package mutating func apply(_ plan: Plan, timeZone: TimeZone, in context: Context) throws(LifrError) -> Change {
        guard !plan.isEmpty else { throw .plan(at: "the plan", .empty) }
        var copy = self
        let change = try copy.add(plan, timeZone: timeZone, in: context)
        self = copy
        return change
    }

    /// Why `plan` can't be added at `now`, or nil when it can: the same
    /// checks `apply` makes, without changing anything.
    package func problem(with plan: Plan, timeZone: TimeZone, now: Date) -> LifrError? {
        var copy = self
        let context = Context(actor: .system(device: "", surface: .assistant), now: now)
        do {
            _ = try copy.apply(plan, timeZone: timeZone, in: context)
            return nil
        } catch {
            return error
        }
    }

    private mutating func add(_ plan: Plan, timeZone: TimeZone, in context: Context) throws(LifrError) -> Change {
        var change = Change()
        let today = CalendarDate(context.now, in: timeZone)

        for (index, draft) in plan.avenues.enumerated() {
            let at = "Avenue \(index + 1) “\(draft.name)”"
            let avenue = try Self.within(at) { () throws(LifrError) in
                try createAvenue(
                    key: draft.key, name: draft.name, color: draft.color, symbol: draft.symbol, in: context)
            }
            change.added.append(.avenue(avenue.id))
        }
        for draft in plan.labels where label(named: draft.name) == nil {
            change.added.append(.label(createLabel(name: draft.name, color: draft.color, in: context).id))
        }

        for (index, draft) in plan.projects.enumerated() {
            let at = "Project \(index + 1) “\(draft.name)”"
            let avenue = try avenue(key: draft.avenue, at: at)
            if projects.values.contains(where: {
                $0.avenueID == avenue.id && $0.name.caseInsensitiveCompare(draft.name) == .orderedSame
            }) {
                throw .plan(at: at, .projectExists(name: draft.name, avenue: avenue.key.rawValue))
            }
            let due = try draft.due.map { (text) throws(LifrError) in try Self.date(text, today: today, at: at) }
            let project = try Self.within(at) { () throws(LifrError) in
                try createProject(name: draft.name, inAvenue: avenue.id, dueDate: due, in: context)
            }
            projects[project.id]?.details = draft.details
            change.added.append(.project(project.id))
            for (taskIndex, task) in draft.tasks.enumerated() {
                let taskAt = "\(at) › task \(taskIndex + 1) “\(task.title)”"
                guard task.place == nil, task.parent == nil else { throw .plan(at: taskAt, .placeGivenTwice) }
                try add(
                    task, avenue: avenue.id, project: project.id, parent: nil, at: taskAt, today: today,
                    timeZone: timeZone, in: context, &change)
            }
        }

        for (index, task) in plan.tasks.enumerated() {
            let at = "task \(index + 1) “\(task.title)”"
            switch (task.place, task.parent) {
            case (nil, nil):
                throw .plan(at: at, .noPlace)
            case (.some, .some):
                throw .plan(at: at, .placeGivenTwice)
            case (let place?, nil):
                let (avenue, project) = try self.place(place, at: at)
                try add(
                    task, avenue: avenue, project: project, parent: nil, at: at, today: today,
                    timeZone: timeZone, in: context, &change)
            case (nil, let code?):
                let parent = try Self.within(at) { () throws(LifrError) in try self.task(code: code) }
                guard parent.deletedAt == nil else { throw .plan(at: at, .rule(.inTrash(parent.id))) }
                guard !parent.isSubtask else { throw .plan(at: at, .rule(.parentIsSubtask(parent.id))) }
                try add(
                    task, avenue: parent.avenueID, project: parent.projectID, parent: parent.id, at: at,
                    today: today, timeZone: timeZone, in: context, &change)
            }
        }
        return change
    }

    /// Adds one task and, when it's top-level, its Sub tasks.
    private mutating func add(
        _ draft: Plan.TaskDraft, avenue: UUID, project: UUID?, parent: UUID?, at: String, today: CalendarDate,
        timeZone: TimeZone, in context: Context, _ change: inout Change
    ) throws(LifrError) {
        if parent != nil, !draft.subtasks.isEmpty { throw .plan(at: at, .subtaskTooDeep) }
        if draft.status.isClosed, draft.subtasks.contains(where: { !$0.status.isClosed }) {
            throw .plan(at: at, .closedWithOpenSubtasks)
        }
        var new = Workspace.NewTask(title: draft.title, avenueID: avenue, projectID: project, parentID: parent)
        new.details = draft.details
        new.status = draft.status
        new.priority = draft.priority
        new.startDate = try draft.start.map { (text) throws(LifrError) in try Self.date(text, today: today, at: at) }
        new.dueDate = try draft.due.map { (text) throws(LifrError) in try Self.date(text, today: today, at: at) }
        new.dueTime = try draft.dueTime.map { (text) throws(LifrError) in
            try Self.within(at) { () throws(LifrError) in try TimeOfDay(text) }
        }
        new.labelIDs = Set(
            draft.labels.map { name in
                if let label = label(named: name) { return label.id }
                let label = createLabel(name: name, in: context)
                change.added.append(.label(label.id))
                return label.id
            })

        let created = try Self.within(at) { () throws(LifrError) in try createTask(new, in: context) }
        change.merge(created)
        guard let id = created.events.first(where: { $0.kind == .created })?.taskID else { return }
        change.added.append(.task(id))

        for text in draft.reminders {
            let trigger = try Reminder.Trigger.resolving(text, today: today, timeZone: timeZone, at: at)
            let reminder = try Self.within(at) { () throws(LifrError) in try addReminder(trigger, to: id, in: context) }
            change.added.append(.reminder(reminder.id))
        }
        for body in draft.comments {
            guard !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw .plan(at: at, .rule(.emptyComment))
            }
            let comment = Comment(
                id: context.newID(), taskID: id, body: body, author: context.actor, createdAt: context.now,
                editedAt: nil, deletedAt: nil)
            comments[comment.id] = comment
            record(.commentAdded, on: id, by: context.actor, in: context, &change)
            change.added.append(.comment(comment.id))
        }
        for (index, subtask) in draft.subtasks.enumerated() {
            let subtaskAt = "\(at) › Sub task \(index + 1) “\(subtask.title)”"
            guard subtask.place == nil, subtask.parent == nil else { throw .plan(at: subtaskAt, .placeGivenTwice) }
            try add(
                subtask, avenue: avenue, project: project, parent: id, at: subtaskAt, today: today,
                timeZone: timeZone, in: context, &change)
        }
    }

    /// The label called `name`, in any case.
    package func label(named name: String) -> Label? {
        labels.values
            .filter { $0.name.caseInsensitiveCompare(name) == .orderedSame }
            .min { $0.createdAt < $1.createdAt }
    }

    private func avenue(key: String, at: String) throws(LifrError) -> Avenue {
        if let avenue = avenues.values.first(where: { $0.key.rawValue.caseInsensitiveCompare(key) == .orderedSame }) {
            return avenue
        }
        let suggestion = Self.closest(to: key.uppercased(), in: avenues.values.map(\.key.rawValue))
        throw .plan(at: at, .unknownAvenue(key: key, suggestion: suggestion))
    }

    /// "KEY" or "KEY/Project name".
    private func place(_ text: String, at: String) throws(LifrError) -> (UUID, UUID?) {
        let parts = text.split(separator: "/", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
        let avenue = try avenue(key: parts.first ?? "", at: at)
        guard parts.count == 2 else { return (avenue.id, nil) }
        let name = parts[1]
        let inAvenue = projects.values.filter { $0.avenueID == avenue.id }
        guard let project = inAvenue.first(where: { $0.name.caseInsensitiveCompare(name) == .orderedSame }) else {
            let suggestion = Self.closest(to: name, in: inAvenue.map(\.name))
            throw .plan(
                at: at, .unknownProject(name: name, avenue: avenue.key.rawValue, suggestion: suggestion))
        }
        return (avenue.id, project.id)
    }

    private static func date(_ text: String, today: CalendarDate, at: String) throws(LifrError) -> CalendarDate {
        guard let date = CalendarDate.resolving(text, today: today) else { throw .plan(at: at, .invalidDate(text)) }
        return date
    }

    /// Runs `body`, saying where in the plan a refusal came from.
    private static func within<T>(_ at: String, _ body: () throws(LifrError) -> T) throws(LifrError) -> T {
        do {
            return try body()
        } catch {
            if case .plan = error { throw error }
            throw .plan(at: at, .rule(error))
        }
    }

    /// The candidate nearest `text` by edit distance, if it's close enough
    /// to be a likely typo.
    static func closest(to text: String, in candidates: [String]) -> String? {
        let wanted = Array(text.lowercased())
        let scored = candidates.map { candidate in (candidate, editDistance(wanted, Array(candidate.lowercased()))) }
        guard let best = scored.min(by: { ($0.1, $0.0) < ($1.1, $1.0) }) else { return nil }
        return best.1 <= max(2, wanted.count / 3) ? best.0 : nil
    }

    /// Levenshtein distance: the fewest insertions, deletions and
    /// substitutions that turn `a` into `b`.
    private static func editDistance(_ source: [Character], _ target: [Character]) -> Int {
        var row = Array(0...target.count)
        for (index, sourceChar) in source.enumerated() {
            var diagonal = row[0]
            row[0] = index + 1
            for (column, targetChar) in target.enumerated() {
                let above = row[column + 1]
                row[column + 1] = min(above + 1, row[column] + 1, diagonal + (sourceChar == targetChar ? 0 : 1))
                diagonal = above
            }
        }
        return row[target.count]
    }
}

extension CalendarDate {
    /// Reads a date a person might write: YYYY-MM-DD, today, tomorrow, a
    /// weekday (the next one after today, with or without "next"), or an
    /// offset such as +3d, +2w or +1m. `nil` when it's none of those.
    package static func resolving(_ text: String, today: CalendarDate) -> CalendarDate? {
        let text = text.trimmingCharacters(in: .whitespaces).lowercased()
        if let date = try? CalendarDate(text) { return date }
        switch text {
        case "today": return today
        case "tomorrow": return today.adding(.day, 1)
        default: break
        }
        let weekday = text.hasPrefix("next ") ? String(text.dropFirst(5)) : text
        if let target = weekdays.firstIndex(where: { $0 == weekday || $0.prefix(3) == weekday }) {
            let current = today.weekdayIndex
            let ahead = (target - current + 7) % 7
            return today.adding(.day, ahead == 0 ? 7 : ahead)
        }
        guard text.hasPrefix("+"), let unit = text.last, let count = Int(text.dropFirst().dropLast()), count >= 0
        else { return nil }
        switch unit {
        case "d": return today.adding(.day, count)
        case "w": return today.adding(.day, count * 7)
        case "m": return today.adding(.month, count)
        default: return nil
        }
    }

    private static let weekdays = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"]

    private static var utc: Calendar { utcCalendar }

    private var noon: Date { noonUTC }

    /// 0 for Sunday through 6 for Saturday.
    private var weekdayIndex: Int { Self.utc.component(.weekday, from: noon) - 1 }

    private func adding(_ component: Calendar.Component, _ value: Int) -> CalendarDate? {
        Self.utc.date(byAdding: component, value: value, to: noon).map { CalendarDate($0, in: Self.utc.timeZone) }
    }
}

extension Reminder.Trigger {
    /// Reads "N minutes|hours|days|weeks before due", or a date as
    /// `CalendarDate.resolving` reads it, optionally followed by HH:MM
    /// (09:00 when left out), in `timeZone`.
    static func resolving(
        _ text: String, today: CalendarDate, timeZone: TimeZone, at: String
    ) throws(LifrError) -> Self {
        let words = text.lowercased().split(separator: " ").map(String.init)
        if words.count == 4, words[2...] == ["before", "due"], let count = Int(words[0]), count > 0 {
            let unit = words[1].hasSuffix("s") ? String(words[1].dropLast()) : words[1]
            let seconds = ["minute": 60, "hour": 3600, "day": 86400, "week": 604_800][unit]
            if let seconds { return .beforeDue(seconds: count * seconds) }
        }
        var dateText = words
        var time = try? TimeOfDay("09:00")
        if let last = words.last, let parsed = try? TimeOfDay(last) {
            time = parsed
            dateText.removeLast()
        }
        guard let date = CalendarDate.resolving(dateText.joined(separator: " "), today: today), let time else {
            throw .plan(at: at, .invalidReminder(text))
        }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        let components = DateComponents(
            year: date.year, month: date.month, day: date.day, hour: time.hour, minute: time.minute)
        guard let fireAt = calendar.date(from: components) else { throw .plan(at: at, .invalidReminder(text)) }
        return .at(fireAt)
    }
}

// MARK: - Decoding

// Every field but the names has a default, so a plan written as JSON by an
// agent only needs what it sets, like the TOML form.

extension Plan {
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            avenues: try container.decodeIfPresent([AvenueDraft].self, forKey: .avenues) ?? [],
            labels: try container.decodeIfPresent([LabelDraft].self, forKey: .labels) ?? [],
            projects: try container.decodeIfPresent([ProjectDraft].self, forKey: .projects) ?? [],
            tasks: try container.decodeIfPresent([TaskDraft].self, forKey: .tasks) ?? [])
    }
}

extension Plan.AvenueDraft {
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            key: try container.decode(String.self, forKey: .key),
            name: try container.decode(String.self, forKey: .name),
            color: try container.decodeIfPresent(String.self, forKey: .color) ?? "blue",
            symbol: try container.decodeIfPresent(String.self, forKey: .symbol) ?? "circle")
    }
}

extension Plan.LabelDraft {
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            name: try container.decode(String.self, forKey: .name),
            color: try container.decodeIfPresent(String.self, forKey: .color) ?? "gray")
    }
}

extension Plan.ProjectDraft {
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            name: try container.decode(String.self, forKey: .name),
            avenue: try container.decode(String.self, forKey: .avenue),
            details: try container.decodeIfPresent(String.self, forKey: .details) ?? "",
            due: try container.decodeIfPresent(String.self, forKey: .due),
            tasks: try container.decodeIfPresent([Plan.TaskDraft].self, forKey: .tasks) ?? [])
    }
}

extension Plan.TaskDraft {
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            title: try container.decode(String.self, forKey: .title),
            details: try container.decodeIfPresent(String.self, forKey: .details) ?? "",
            status: try container.decodeIfPresent(TaskStatus.self, forKey: .status) ?? .todo,
            priority: try container.decodeIfPresent(Priority.self, forKey: .priority) ?? .none,
            start: try container.decodeIfPresent(String.self, forKey: .start),
            due: try container.decodeIfPresent(String.self, forKey: .due),
            dueTime: try container.decodeIfPresent(String.self, forKey: .dueTime),
            labels: try container.decodeIfPresent([String].self, forKey: .labels) ?? [],
            reminders: try container.decodeIfPresent([String].self, forKey: .reminders) ?? [],
            comments: try container.decodeIfPresent([String].self, forKey: .comments) ?? [],
            subtasks: try container.decodeIfPresent([Plan.TaskDraft].self, forKey: .subtasks) ?? [],
            place: try container.decodeIfPresent(String.self, forKey: .place),
            parent: try container.decodeIfPresent(String.self, forKey: .parent))
    }
}
