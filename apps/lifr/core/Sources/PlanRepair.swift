import Foundation

extension Plan {
    /// The plan with the small mistakes an assistant makes put right, so
    /// the person needn't fix them by hand:
    /// - a place that names an Avenue by its name rather than its key, or
    ///   a Project name one or two letters off, or only a Project's name,
    ///   becomes the place it means; a Project of a new Avenue that the
    ///   plan doesn't add becomes the Avenue;
    /// - a new Avenue with the name of one that's there is left out, and
    ///   what named its key goes in the one that's there;
    /// - a task with both a place and the code of an existing parent goes
    ///   under the parent; a place on a task that can't have one is
    ///   dropped;
    /// - the same task twice in one place becomes one, keeping what either
    ///   says, and a Sub task with its task's title is left out;
    /// - a task placed in a Project its Avenue doesn't have goes in that
    ///   Project, which the plan adds;
    /// - a task with the name of a Project in its Avenue stands for that
    ///   Project: its Sub tasks go in it;
    /// - a task an Avenue already has open, by title, is left out, and its
    ///   new Sub tasks go under the one that's there;
    /// - in a workspace with no Avenues yet, an Avenue the plan puts things
    ///   in without adding is added, as there's none it could mean.
    /// Anything it can't be sure of is left for `Workspace.problem(with:)`
    /// to report, such as an Avenue key that's neither there nor added: a
    /// key a letter from one that's there, such as WEALTH for HEALTH, may
    /// be a new Avenue the plan left out, and a guess would put it all in
    /// the wrong one.
    package func repaired(for workspace: Workspace) -> Plan {
        var plan = self
        let places = PlaceNames(workspace)
        if places.avenues.isEmpty { plan.avenues += plan.missingAvenues }
        var renamed: [String: String] = [:]
        plan.avenues.removeAll { draft in
            guard let avenue = places.avenue(named: draft.name) else { return false }
            renamed[draft.key.uppercased()] = avenue.key.rawValue
            return true
        }
        let newKeys = plan.avenues.map(\.key)
        func avenueKey(_ text: String) -> String {
            let text = text.trimmingCharacters(in: .whitespaces)
            if let key = renamed[text.uppercased()] { return key }
            if let key = newKeys.first(where: { $0.caseInsensitiveCompare(text) == .orderedSame }) { return key }
            if let new = plan.avenues.first(where: { $0.name.caseInsensitiveCompare(text) == .orderedSame }) {
                return new.key
            }
            return places.avenueKey(text) ?? text
        }
        plan.projects = plan.projects.map { project in
            var project = project
            project.avenue = avenueKey(project.avenue)
            project.due = Self.date(project.due)
            project.tasks = project.tasks.map(Self.unplaced)
            return project
        }
        // A new Project named like one in its Avenue: its tasks go in that one.
        var intoExisting: [TaskDraft] = []
        plan.projects.removeAll { project in
            guard let existing = places.project(named: project.name, in: project.avenue) else { return false }
            intoExisting += project.tasks.map { task in
                var task = task
                task.place = project.avenue + "/" + existing
                return task
            }
            return true
        }
        plan.tasks += intoExisting
        // A new Avenue has only the Projects the plan adds to it.
        func inNewAvenue(_ place: String) -> String {
            let parts = place.split(separator: "/", maxSplits: 1).map(String.init)
            guard parts.count == 2, let key = parts.first, newKeys.contains(key) else { return place }
            let named = plan.projects.contains {
                $0.avenue == key && $0.name.caseInsensitiveCompare(parts[1]) == .orderedSame
            }
            return named ? place : key
        }
        plan.tasks = Self.nested(plan.tasks, in: workspace)
        plan.tasks = plan.tasks.map { task in
            var task = task
            task.subtasks = task.subtasks.map(Self.unplaced)
            if let parent = task.parent,
                let found = try? workspace.task(code: parent.trimmingCharacters(in: .whitespaces)),
                found.deletedAt == nil
            {
                task.place = nil
                task.parent = workspace.displayCode(of: found)
            } else if let place = task.place {
                if task.parent != nil { task.parent = nil }
                task.place = places.place(place, of: task.title, avenueKey: avenueKey)
                task.place = task.place.map(inNewAvenue)
            }
            return task
        }
        plan.tasks = plan.tasks.flatMap { task in Self.projectSteps(task, places: places) ?? [task] }
        plan.intoNewProjects(in: workspace)
        plan.leaveOutOpen(in: workspace)
        plan.tasks = Self.merged(plan.tasks).map(Self.dated)
        plan.projects = plan.projects.map { project in
            var project = project
            project.tasks = project.tasks.map(Self.dated)
            return project
        }
        return plan
    }

    /// The Sub tasks of `task`, placed in the Project its title names in
    /// its Avenue; nil when it names none. A model writes a task for a
    /// Project that's there with the Project's tasks as its steps.
    private static func projectSteps(_ task: TaskDraft, places: PlaceNames) -> [TaskDraft]? {
        guard !task.subtasks.isEmpty, let key = task.place, !key.contains("/"),
            let project = places.project(named: task.title, in: key)
        else { return nil }
        return task.subtasks.map { step in
            var step = step
            step.place = key + "/" + project
            return step
        }
    }

    /// Leaves out each task that its Avenue already has open by the same
    /// title, such as one this chat added that the model writes again.
    /// What's new under it goes under the one that's there.
    private mutating func leaveOutOpen(in workspace: Workspace) {
        func open(_ title: String, in key: String) -> LifrTask? {
            guard let avenue = workspace.avenues.values.first(where: { $0.key.rawValue == key }) else { return nil }
            let title = title.trimmingCharacters(in: .whitespaces)
            return workspace.tasks.values
                .filter {
                    $0.avenueID == avenue.id && $0.deletedAt == nil && $0.isOpen
                        && $0.title.caseInsensitiveCompare(title) == .orderedSame
                }
                .min { ($0.parentID == nil ? 0 : 1, $0.createdAt) < ($1.parentID == nil ? 0 : 1, $1.createdAt) }
        }
        func kept(_ task: TaskDraft, in key: String) -> [TaskDraft] {
            let steps = task.subtasks.filter { open($0.title, in: key) == nil }
            guard let existing = open(task.title, in: key) else {
                var task = task
                task.subtasks = steps
                return [task]
            }
            // A Sub task can't have its own.
            guard existing.parentID == nil else { return [] }
            let code = workspace.displayCode(of: existing)
            return steps.map { step in
                var step = step
                step.place = nil
                step.parent = code
                return step
            }
        }
        tasks = tasks.flatMap { task in
            guard task.parent == nil, let place = task.place,
                let key = place.split(separator: "/", maxSplits: 1).first.map(String.init)
            else { return [task] }
            return kept(task, in: key)
        }
        projects = projects.map { project in
            var project = project
            // Sub tasks of one that's there leave the new Project.
            let all = project.tasks.flatMap { kept($0, in: project.avenue) }
            project.tasks = all.filter { $0.parent == nil }
            tasks += all.filter { $0.parent != nil }
            return project
        }
    }

    /// Moves each task placed in a Project its Avenue doesn't have into
    /// that Project, which the plan adds. A model names the Project it
    /// means to make in the task's place and leaves it out of the plan. A
    /// task placed in a Project the plan adds stays where it is.
    private mutating func intoNewProjects(in workspace: Workspace) {
        let planned = projects.count
        tasks.removeAll { task in
            guard let place = task.place else { return false }
            let parts = place.split(separator: "/", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            guard parts.count == 2, !parts[1].isEmpty,
                let avenue = workspace.avenues.values.first(where: { $0.key.rawValue == parts[0] }),
                !workspace.projects.values.contains(where: {
                    $0.avenueID == avenue.id && $0.name.caseInsensitiveCompare(parts[1]) == .orderedSame
                })
            else { return false }
            let index = projects.firstIndex {
                $0.avenue == parts[0] && $0.name.caseInsensitiveCompare(parts[1]) == .orderedSame
            }
            if let index, index < planned { return false }
            let unplaced = Self.unplaced(task)
            if let index {
                projects[index].tasks.append(unplaced)
            } else {
                projects.append(ProjectDraft(name: parts[1], avenue: parts[0], tasks: [unplaced]))
            }
            return true
        }
    }

    /// `tasks` with each whose parent, or Project, is the title of another
    /// of them made that one's Sub task, with its own Sub tasks after it:
    /// a model writes the steps of a new task that way.
    private static func nested(_ tasks: [TaskDraft], in workspace: Workspace) -> [TaskDraft] {
        func named(_ task: TaskDraft) -> [String] {
            let project = task.place.flatMap { place -> String? in
                let parts = place.split(separator: "/", maxSplits: 1)
                return parts.count == 2 ? parts[1].trimmingCharacters(in: .whitespaces) : nil
            }
            let parent = task.parent.flatMap { (try? workspace.task(code: $0)) == nil ? $0 : nil }
            return [parent, project].compactMap(\.self)
        }
        var moves: [Int: Int] = [:]
        for (index, task) in tasks.enumerated() {
            let names = named(task)
            guard !names.isEmpty else { continue }
            let target = tasks.indices.first { other in
                other != index && names.contains { $0.caseInsensitiveCompare(tasks[other].title) == .orderedSame }
            }
            if let target { moves[index] = target }
        }
        // A task another goes under stays where it is.
        for (index, target) in moves where moves[target] != nil {
            moves[index] = nil
        }
        var result = tasks
        for (index, task) in tasks.enumerated() {
            guard let target = moves[index] else { continue }
            var step = task
            step.subtasks = []
            result[target].subtasks += [step] + task.subtasks
        }
        return result.enumerated().filter { moves[$0.offset] == nil }.map(\.element)
    }

    /// `task` and its Sub tasks with dates that say there's none left out,
    /// and its Sub tasks of the same title made one.
    private static func dated(_ task: TaskDraft) -> TaskDraft {
        var task = task
        task.start = date(task.start)
        task.due = date(task.due)
        task.dueTime = date(task.dueTime)
        var titles: Set<String> = [task.title.lowercased()]
        task.subtasks = task.subtasks.filter { titles.insert($0.title.lowercased()).inserted }.map(dated)
        return task
    }

    /// New Avenues for the places that name one the plan doesn't add, by
    /// key or name: the key from its letters, and the name as written, or
    /// in title case when only a key was written.
    private var missingAvenues: [AvenueDraft] {
        let named =
            projects.map(\.avenue)
            + tasks.filter { $0.parent == nil }.compactMap { task in
                task.place?.split(separator: "/", maxSplits: 1).first.map(String.init)
            }
        var known = Set(avenues.flatMap { [$0.key.lowercased(), $0.name.lowercased()] })
        var added: [AvenueDraft] = []
        for text in named.map({ $0.trimmingCharacters(in: .whitespaces) }) where !known.contains(text.lowercased()) {
            let letters = String(text.filter { $0.isLetter && $0.isASCII }.prefix(10))
            guard let key = try? AvenueKey(letters), !known.contains(key.rawValue.lowercased()) else { continue }
            let name = text == key.rawValue ? text.capitalized : text
            added.append(AvenueDraft(key: key.rawValue, name: name))
            known.formUnion([text.lowercased(), key.rawValue.lowercased(), name.lowercased()])
        }
        return added
    }

    /// Nil for a date that says there's none, such as "none" or "".
    private static func date(_ text: String?) -> String? {
        guard let text = text?.trimmingCharacters(in: .whitespaces), !text.isEmpty else { return nil }
        return ["none", "null", "n/a", "no"].contains(text.lowercased()) ? nil : text
    }

    /// A task inside a new Project, or a Sub task: its place comes from
    /// where it is.
    private static func unplaced(_ task: TaskDraft) -> TaskDraft {
        var task = task
        task.place = nil
        task.parent = nil
        return task
    }

    /// `tasks` with those of the same title and place made one, in the
    /// place of the first: fields the first leaves empty come from the
    /// others, and their Sub tasks are added.
    private static func merged(_ tasks: [TaskDraft]) -> [TaskDraft] {
        /// Names that differ only in case, spacing or a closing full stop.
        func sameName(_ one: String?, _ other: String?) -> Bool {
            func key(_ name: String?) -> String? {
                name.map {
                    $0.split(whereSeparator: \.isWhitespace).joined(separator: " ")
                        .trimmingCharacters(in: CharacterSet(charactersIn: ".")).lowercased()
                }
            }
            return key(one) == key(other)
        }
        var kept: [TaskDraft] = []
        for task in tasks {
            guard
                let index = kept.firstIndex(where: {
                    sameName($0.title, task.title) && sameName($0.place, task.place) && $0.parent == task.parent
                })
            else {
                kept.append(task)
                continue
            }
            var first = kept[index]
            if first.details.isEmpty { first.details = task.details }
            if first.priority == .none { first.priority = task.priority }
            first.start = first.start ?? task.start
            first.due = first.due ?? task.due
            first.dueTime = first.dueTime ?? task.dueTime
            first.labels += task.labels.filter { label in
                !first.labels.contains { $0.caseInsensitiveCompare(label) == .orderedSame }
            }
            first.reminders += task.reminders.filter { !first.reminders.contains($0) }
            first.comments += task.comments.filter { !first.comments.contains($0) }
            first.subtasks += task.subtasks.filter { subtask in
                !first.subtasks.contains { $0.title.caseInsensitiveCompare(subtask.title) == .orderedSame }
            }
            kept[index] = first
        }
        return kept
    }
}

/// The Avenues and Projects of a workspace, as a plan may name them.
private struct PlaceNames {
    let avenues: [Avenue]
    let projects: [Project]

    init(_ workspace: Workspace) {
        avenues = workspace.avenues.values.filter { $0.archivedAt == nil }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
        projects = workspace.projects.values.filter { $0.status == .active }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
    }

    /// The name of the active Project in the Avenue with `key` that
    /// `name` names, in any case.
    func project(named name: String, in key: String) -> String? {
        guard let avenue = avenues.first(where: { $0.key.rawValue == key }) else { return nil }
        return projects.first {
            $0.avenueID == avenue.id
                && $0.name.caseInsensitiveCompare(name.trimmingCharacters(in: .whitespaces)) == .orderedSame
        }?.name
    }

    func avenue(named name: String) -> Avenue? {
        avenues.first { $0.name.caseInsensitiveCompare(name.trimmingCharacters(in: .whitespaces)) == .orderedSame }
    }

    /// The key of the Avenue `text` names: its key in any case, or its
    /// name. Nil when it names none.
    func avenueKey(_ text: String) -> String? {
        if let avenue = avenues.first(where: { $0.key.rawValue.caseInsensitiveCompare(text) == .orderedSame }) {
            return avenue.key.rawValue
        }
        return avenue(named: text)?.key.rawValue
    }

    /// "KEY" or "KEY/Project name" for `text`, or `text` when it can't
    /// tell. `avenueKey` reads the Avenue part, new Avenues included. A
    /// Project part that isn't one, but is in the task's `title` or has
    /// it, is the Avenue: a model writes the task's title there.
    func place(_ text: String, of title: String, avenueKey: (String) -> String) -> String {
        let parts = text.split(separator: "/", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
        guard let first = parts.first, !first.isEmpty else { return text }
        guard parts.count == 2 else {
            let key = avenueKey(first)
            if key != first || avenues.contains(where: { $0.key.rawValue == key }) { return key }
            // Only a Project's name, in one Avenue.
            let named = projects.filter { $0.name.caseInsensitiveCompare(first) == .orderedSame }
            if named.count == 1, let project = named.first,
                let avenue = avenues.first(where: { $0.id == project.avenueID })
            {
                return avenue.key.rawValue + "/" + project.name
            }
            return text
        }
        let key = avenueKey(first)
        guard let avenue = avenues.first(where: { $0.key.rawValue == key }) else { return key + "/" + parts[1] }
        let inAvenue = projects.filter { $0.avenueID == avenue.id }.map(\.name)
        let project =
            inAvenue.first { $0.caseInsensitiveCompare(parts[1]) == .orderedSame }
            ?? Workspace.closest(to: parts[1], in: inAvenue)
        if let project { return key + "/" + project }
        let lowered = (parts[1].lowercased(), title.lowercased())
        if lowered.1.contains(lowered.0) || lowered.0.contains(lowered.1) { return key }
        return key + "/" + parts[1]
    }
}
