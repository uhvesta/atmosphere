import Foundation

/// Every record, and the operations that change them. Each operation checks
/// the rules from the spec, writes history, and returns a `Change`. A refused
/// operation throws and changes nothing.
package struct Workspace: Hashable, Codable, Sendable {
    package internal(set) var avenues: [UUID: Avenue] = [:] {
        didSet { derived = Derived() }
    }
    package internal(set) var projects: [UUID: Project] = [:]
    package internal(set) var tasks: [UUID: LifrTask] = [:] {
        didSet { derived = Derived() }
    }
    package internal(set) var labels: [UUID: Label] = [:]
    package internal(set) var comments: [UUID: Comment] = [:]
    package internal(set) var reminders: [UUID: Reminder] = [:]
    package internal(set) var savedFilters: [UUID: SavedFilter] = [:]
    /// Append-only.
    package internal(set) var history: [HistoryEvent] = []
    /// Every operation's changes to the records, oldest first. Append-only.
    package internal(set) var journal: [JournalEntry] = []
    package var settings = Settings()
    /// Records storage has that this version can't read, such as one a
    /// newer version wrote or a task whose Avenue hasn't arrived. They're
    /// still there, so sync must never send them as removed.
    package var unreadable: Set<RecordID> = []
    /// Lists worked out from `tasks` and `avenues`, until they change.
    var derived = Derived()

    private enum CodingKeys: String, CodingKey {
        case avenues, projects, tasks, labels, comments, reminders, savedFilters, history, journal, settings
        case unreadable
    }

    /// How long trashed tasks are kept before `purgeTrash` deletes them.
    package static let trashRetention: TimeInterval = 30 * 24 * 60 * 60

    package init() {}

    /// Records loaded from storage, as they are. They may break a rule if
    /// sync brought them in from another device; call `reconcile` after.
    package init(
        avenues: [Avenue] = [], projects: [Project] = [], tasks: [LifrTask] = [], labels: [Label] = [],
        comments: [Comment] = [], reminders: [Reminder] = [], savedFilters: [SavedFilter] = [],
        history: [HistoryEvent] = [], journal: [JournalEntry] = [], settings: Settings = Settings()
    ) {
        self.avenues = Dictionary(avenues.map { ($0.id, $0) }) { _, last in last }
        self.projects = Dictionary(projects.map { ($0.id, $0) }) { _, last in last }
        self.tasks = Dictionary(tasks.map { ($0.id, $0) }) { _, last in last }
        self.labels = Dictionary(labels.map { ($0.id, $0) }) { _, last in last }
        self.comments = Dictionary(comments.map { ($0.id, $0) }) { _, last in last }
        self.reminders = Dictionary(reminders.map { ($0.id, $0) }) { _, last in last }
        self.savedFilters = Dictionary(savedFilters.map { ($0.id, $0) }) { _, last in last }
        self.history = history
        self.journal = journal
        self.settings = settings
    }

    // MARK: - Lookups

    package func task(_ id: UUID) throws(LifrError) -> LifrTask {
        guard let task = tasks[id] else { throw .notFound(.task, id.uuidString) }
        return task
    }

    /// Finds a task by code, with or without the Avenue key: HEALTH-7K2Q or 7K2Q.
    package func task(code: String) throws(LifrError) -> LifrTask {
        let code = try TaskCode(code)
        guard let task = tasks.values.first(where: { $0.code == code }) else {
            throw .notFound(.task, code.rawValue)
        }
        return task
    }

    /// A task's Sub tasks, not counting trashed ones, in sort order.
    package func subtasks(of id: UUID) -> [LifrTask] {
        liveChildren[id] ?? []
    }

    package func openSubtasks(of id: UUID) -> [LifrTask] {
        subtasks(of: id).filter(\.isOpen)
    }

    /// The task's display code, such as HEALTH-7K2Q.
    package func displayCode(of task: LifrTask) -> String {
        avenues[task.avenueID].map { task.code.display(in: $0.key) } ?? task.code.rawValue
    }

    func avenue(_ id: UUID) throws(LifrError) -> Avenue {
        guard let avenue = avenues[id] else { throw .notFound(.avenue, id.uuidString) }
        return avenue
    }

    func project(_ id: UUID) throws(LifrError) -> Project {
        guard let project = projects[id] else { throw .notFound(.project, id.uuidString) }
        return project
    }

    /// A task that isn't in the trash.
    func liveTask(_ id: UUID) throws(LifrError) -> LifrTask {
        let task = try task(id)
        guard task.deletedAt == nil else { throw .inTrash(id) }
        return task
    }

    // MARK: - Avenues, Projects and Labels

    @discardableResult
    package mutating func createAvenue(
        key: String, name: String, color: String = "blue", symbol: String = "circle",
        in context: Context
    ) throws(LifrError) -> Avenue {
        let key = try AvenueKey(key)
        guard !avenues.values.contains(where: { $0.key == key }) else { throw .avenueKeyTaken(key) }
        let last = avenues.values.map(\.sortOrder).max()
        let avenue = Avenue(
            id: context.newID(), key: key, name: name, color: color, symbol: symbol,
            sortOrder: SortKey.between(last, nil), agentAccess: .full,
            createdAt: context.now, archivedAt: nil
        )
        avenues[avenue.id] = avenue
        return avenue
    }

    /// Adds the Inbox if there isn't one, first in the Avenue order. Returns
    /// whether it added it.
    @discardableResult
    package mutating func ensureInbox(in context: Context) -> Bool {
        guard avenues[Avenue.inboxID] == nil else { return false }
        var key = AvenueKey(reconciled: "INBOX")
        var number = 2
        while avenues.values.contains(where: { $0.key == key }) {
            key = Self.suffixed(AvenueKey(reconciled: "INBOX"), number)
            number += 1
        }
        avenues[Avenue.inboxID] = Avenue(
            id: Avenue.inboxID, key: key, name: "Inbox", color: "gray", symbol: "tray",
            sortOrder: SortKey.between(nil, avenues.values.map(\.sortOrder).min()), agentAccess: .full,
            createdAt: context.now, archivedAt: nil
        )
        return true
    }

    /// Changes an Avenue's key, for example after reconciliation suffixed it.
    package mutating func setKey(_ key: String, ofAvenue id: UUID) throws(LifrError) {
        var avenue = try avenue(id)
        let key = try AvenueKey(key)
        guard !avenues.values.contains(where: { $0.key == key && $0.id != id }) else {
            throw .avenueKeyTaken(key)
        }
        avenue.key = key
        avenues[id] = avenue
    }

    /// What agents may do in an Avenue. People always see everything.
    package mutating func setAgentAccess(_ access: AgentAccess, ofAvenue id: UUID) throws(LifrError) {
        var avenue = try avenue(id)
        avenue.agentAccess = access
        avenues[id] = avenue
    }

    @discardableResult
    package mutating func createProject(
        name: String, inAvenue avenueID: UUID, dueDate: CalendarDate? = nil, in context: Context
    ) throws(LifrError) -> Project {
        _ = try avenue(avenueID)
        let last = projects.values.filter { $0.avenueID == avenueID }.map(\.sortOrder).max()
        let project = Project(
            id: context.newID(), avenueID: avenueID, name: name, details: "", status: .active,
            dueDate: dueDate, sortOrder: SortKey.between(last, nil), createdAt: context.now,
            completedAt: nil
        )
        projects[project.id] = project
        return project
    }

    /// A Project can be Completed only when all its tasks are closed.
    /// Archiving has no such check.
    package mutating func setStatus(
        _ status: ProjectStatus, ofProject id: UUID, in context: Context
    ) throws(LifrError) {
        var project = try project(id)
        if status == .completed {
            let open = tasks.values
                .filter { $0.projectID == id && $0.deletedAt == nil && $0.isOpen }
                .map(\.id)
                .sorted { $0.uuidString < $1.uuidString }
            guard open.isEmpty else { throw .projectHasOpenTasks(project: id, open: open) }
        }
        project.status = status
        project.completedAt = status == .completed ? context.now : nil
        projects[id] = project
    }

    /// What happens to the open tasks of a Project or Avenue being archived.
    package enum OpenTasks: Sendable {
        /// They stay in the Project's Avenue, outside any Project. Only for
        /// Projects.
        case moveToAvenue
        /// They and their open Sub tasks become Won't Do.
        case closeAsWontDo
    }

    /// Fields of a Project to change; `nil` leaves one as it is.
    package struct ProjectEdit: Sendable {
        package var name: String?
        package var details: String?
        package var dueDate: CalendarDate??

        package init(name: String? = nil, details: String? = nil, dueDate: CalendarDate?? = nil) {
            self.name = name
            self.details = details
            self.dueDate = dueDate
        }
    }

    package mutating func edit(project id: UUID, _ edit: ProjectEdit) throws(LifrError) {
        var project = try project(id)
        if let name = edit.name { project.name = try Self.validName(name) }
        if let details = edit.details { project.details = details }
        if let dueDate = edit.dueDate { project.dueDate = dueDate }
        projects[id] = project
    }

    /// Moves a Project to another Avenue, with every task in it.
    package mutating func move(project id: UUID, toAvenue avenueID: UUID, in context: Context) throws(LifrError)
        -> Change
    {
        var project = try project(id)
        _ = try avenue(avenueID)
        var change = Change()
        guard project.avenueID != avenueID else { return change }
        let last = projects.values.filter { $0.avenueID == avenueID }.map(\.sortOrder).max()
        project.avenueID = avenueID
        project.sortOrder = SortKey.between(last, nil)
        projects[id] = project
        let inProject = tasks.values.filter { $0.projectID == id && !$0.isSubtask }
            .sorted { $0.id.uuidString < $1.id.uuidString }
        for task in inProject {
            relocate(task.id, avenue: avenueID, project: id, by: context.actor, in: context, &change)
        }
        return change
    }

    /// Archives a Project, first doing `openTasks` with its open tasks.
    package mutating func archive(project id: UUID, openTasks: OpenTasks, in context: Context) throws(LifrError)
        -> Change
    {
        let project = try project(id)
        var change = Change()
        for task in openTopLevelTasks(where: { $0.projectID == id }) {
            switch openTasks {
            case .moveToAvenue:
                relocate(task.id, avenue: project.avenueID, project: nil, by: context.actor, in: context, &change)
            case .closeAsWontDo:
                change.merge(try setStatus(.wontDo, of: task.id, cascade: true, repeats: false, in: context))
            }
        }
        try setStatus(.archived, ofProject: id, in: context)
        return change
    }

    /// Fields of an Avenue to change; `nil` leaves one as it is.
    package struct AvenueEdit: Sendable {
        package var name: String?
        package var color: String?
        /// An SF Symbol name.
        package var symbol: String?

        package init(name: String? = nil, color: String? = nil, symbol: String? = nil) {
            self.name = name
            self.color = color
            self.symbol = symbol
        }
    }

    package mutating func edit(avenue id: UUID, _ edit: AvenueEdit) throws(LifrError) {
        var avenue = try avenue(id)
        if let name = edit.name { avenue.name = try Self.validName(name) }
        if let color = edit.color { avenue.color = color }
        if let symbol = edit.symbol { avenue.symbol = symbol }
        avenues[id] = avenue
    }

    /// Archives an Avenue, which takes its tasks out of every list. With open
    /// tasks it needs `openTasks: .closeAsWontDo`; moving them would need
    /// another Avenue, so move them by hand first instead.
    package mutating func archive(avenue id: UUID, openTasks: OpenTasks?, in context: Context) throws(LifrError)
        -> Change
    {
        var avenue = try avenue(id)
        guard !avenue.isInbox else { throw .inboxCannotBeArchived }
        let open = openTopLevelTasks(where: { $0.avenueID == id })
        var change = Change()
        if !open.isEmpty {
            guard openTasks == .closeAsWontDo else { throw .avenueHasOpenTasks(avenue: id, open: open.map(\.id)) }
            for task in open {
                change.merge(try setStatus(.wontDo, of: task.id, cascade: true, repeats: false, in: context))
            }
        }
        avenue.archivedAt = context.now
        avenues[id] = avenue
        return change
    }

    /// Brings an archived Avenue back, with its tasks as they were.
    package mutating func restore(avenue id: UUID) throws(LifrError) {
        var avenue = try avenue(id)
        avenue.archivedAt = nil
        avenues[id] = avenue
    }

    /// Open top-level tasks that aren't in the trash and match, in a fixed
    /// order.
    private func openTopLevelTasks(where matches: (LifrTask) -> Bool) -> [LifrTask] {
        tasks.values
            .filter { $0.deletedAt == nil && $0.isOpen && !$0.isSubtask && matches($0) }
            .sorted { $0.id.uuidString < $1.id.uuidString }
    }

    @discardableResult
    package mutating func createLabel(
        name: String, color: String = "gray", in context: Context
    ) -> Label {
        let label = Label(id: context.newID(), name: name, color: color, createdAt: context.now)
        labels[label.id] = label
        return label
    }

    /// Renames a label. A name another label has, ignoring case, is
    /// refused: merge the two instead.
    package mutating func rename(label id: UUID, to name: String) throws(LifrError) {
        guard var label = labels[id] else { throw .notFound(.label, id.uuidString) }
        let name = try Self.validName(name)
        let taken = labels.values.contains { $0.id != id && $0.name.caseInsensitiveCompare(name) == .orderedSame }
        guard !taken else { throw .labelNameTaken(name) }
        renameLabel(inSavedFilters: label.name, to: name)
        label.name = name
        labels[id] = label
    }

    /// Gives a label one of `Avenue.colors`.
    package mutating func recolor(label id: UUID, to color: String) throws(LifrError) {
        guard labels[id] != nil else { throw .notFound(.label, id.uuidString) }
        labels[id]?.color = color
    }

    /// Deletes a label, giving every task that had it `other` instead, or
    /// nothing when `other` is `nil`.
    package mutating func merge(label id: UUID, into other: UUID?, in context: Context) throws(LifrError) -> Change {
        guard labels[id] != nil else { throw .notFound(.label, id.uuidString) }
        if let other, other == id || labels[other] == nil { throw .notFound(.label, other.uuidString) }
        var change = Change()
        let tagged = tasks.values.filter { $0.labelIDs.contains(id) }.sorted { $0.id.uuidString < $1.id.uuidString }
        for task in tagged {
            var labelIDs = task.labelIDs
            labelIDs.remove(id)
            if let other { labelIDs.insert(other) }
            update(task.id, in: context, &change) { $0.labelIDs = labelIDs }
            record(
                .fieldChanged, on: task.id, field: .labels, oldValue: Self.describe(task.labelIDs),
                newValue: Self.describe(labelIDs), by: context.actor, in: context, &change)
        }
        if let name = labels[id]?.name { renameLabel(inSavedFilters: name, to: other.flatMap { labels[$0]?.name }) }
        labels[id] = nil
        return change
    }

    // MARK: - Tasks

    /// The fields of a new Task or Sub task.
    package struct NewTask: Sendable {
        package var title: String
        package var details = ""
        package var status = TaskStatus.todo
        package var priority = Priority.none
        package var avenueID: UUID?
        package var projectID: UUID?
        /// Makes it a Sub task. It then takes the parent's Avenue and Project.
        package var parentID: UUID?
        package var startDate: CalendarDate?
        package var dueDate: CalendarDate?
        package var dueTime: TimeOfDay?
        package var labelIDs: Set<UUID> = []
        /// Reminders to add with it. One before the due date is left out
        /// when there's no due date.
        package var reminders: [Reminder.Trigger] = []
        /// Makes it repeat. A Sub task can't.
        package var repeats: Recurrence.Rule?
        package var repeatInterval = 1

        package init(title: String, avenueID: UUID? = nil, projectID: UUID? = nil, parentID: UUID? = nil) {
            self.title = title
            self.avenueID = avenueID
            self.projectID = projectID
            self.parentID = parentID
        }
    }

    package mutating func createTask(_ new: NewTask, in context: Context) throws(LifrError) -> Change {
        let title = try Self.validTitle(new.title)
        if new.dueTime != nil, new.dueDate == nil { throw .dueTimeWithoutDueDate }
        if let start = new.startDate, let due = new.dueDate, start > due {
            throw .startsAfterDue(start: start, due: due)
        }
        try checkLabels(new.labelIDs)
        if new.repeats != nil, let parentID = new.parentID { throw .subtaskCannotRepeat(parentID) }

        let avenueID: UUID
        let projectID: UUID?
        var parent: LifrTask?
        if let parentID = new.parentID {
            let found = try liveTask(parentID)
            guard !found.isSubtask else { throw .parentIsSubtask(parentID) }
            parent = found
            avenueID = found.avenueID
            projectID = found.projectID
        } else {
            guard let id = new.avenueID else { throw .notFound(.avenue, "(none given)") }
            avenueID = try avenue(id).id
            projectID = new.projectID
            try checkProject(projectID, isIn: avenueID)
        }

        let siblings = tasks.values.filter { $0.parentID == new.parentID && $0.avenueID == avenueID }
        let id = context.newID()
        let recurrence = new.repeats.map { Recurrence(rule: $0, interval: new.repeatInterval, seriesID: id) }
        if let recurrence, !recurrence.isValid { throw .invalidRecurrence }
        let task = LifrTask(
            id: id, code: uniqueCode(context), title: title, details: new.details,
            status: new.status, priority: new.priority, startDate: new.startDate,
            dueDate: new.dueDate, dueTime: new.dueTime, avenueID: avenueID, projectID: projectID,
            parentID: new.parentID, labelIDs: new.labelIDs,
            sortOrder: SortKey.between(siblings.map(\.sortOrder).max(), nil),
            createdAt: context.now, updatedAt: context.now, lastActivityAt: context.now,
            closedAt: new.status.isClosed ? context.now : nil, deletedAt: nil, recurrence: recurrence
        )
        var change = Change()
        change.previous[task.id] = .some(nil)
        tasks[task.id] = task
        record(.created, on: task.id, newValue: task.code.rawValue, by: context.actor, in: context, &change)
        if let parent {
            touch(parent.id, in: context, &change)
            record(.subtaskAdded, on: parent.id, newValue: task.code.rawValue, by: context.actor, in: context, &change)
            if task.isOpen { reopenIfClosed(parent.id, in: context, &change) }
        }
        for trigger in new.reminders {
            if case .beforeDue = trigger, task.dueDate == nil { continue }
            let reminder = Reminder(id: context.newID(), taskID: task.id, trigger: trigger)
            reminders[reminder.id] = reminder
            change.added.append(.reminder(reminder.id))
        }
        return change
    }

    /// Changes to a task's own fields. `nil` leaves a field as it is; for the
    /// optional fields, `.some(nil)` clears it. Clearing the due date clears
    /// its time too, unless the edit sets one.
    package struct TaskEdit: Sendable {
        package var title: String?
        package var details: String?
        package var priority: Priority?
        package var startDate: CalendarDate??
        package var dueDate: CalendarDate??
        package var dueTime: TimeOfDay??
        package var labelIDs: Set<UUID>?

        package init(
            title: String? = nil, details: String? = nil, priority: Priority? = nil,
            startDate: CalendarDate?? = nil, dueDate: CalendarDate?? = nil,
            dueTime: TimeOfDay?? = nil, labelIDs: Set<UUID>? = nil
        ) {
            self.title = title
            self.details = details
            self.priority = priority
            self.startDate = startDate
            self.dueDate = dueDate
            self.dueTime = dueTime
            self.labelIDs = labelIDs
        }

        /// Makes `task` due on `day`, keeping its time. A task that runs
        /// over days moves as a whole, and one that would start after
        /// `day` starts on it. `nil` when it's already due then.
        package static func due(_ day: CalendarDate, for task: LifrTask) -> TaskEdit? {
            due(day, start: task.startDate, due: task.dueDate)
        }

        /// Makes a task that starts on `start` and is due on `due` due on
        /// `day` instead, as ``due(_:for:)`` does.
        package static func due(_ day: CalendarDate, start: CalendarDate?, due: CalendarDate?) -> TaskEdit? {
            guard due != day else { return nil }
            var edit = TaskEdit(dueDate: .some(day))
            if let start {
                if let due {
                    edit.startDate = .some(start.adding(days: due.days(until: day)))
                } else if start > day {
                    edit.startDate = .some(day)
                }
            }
            return edit
        }
    }

    package mutating func edit(_ id: UUID, _ edit: TaskEdit, in context: Context) throws(LifrError) -> Change {
        var task = try liveTask(id)
        let old = task
        if let title = edit.title { task.title = try Self.validTitle(title) }
        if let details = edit.details { task.details = details }
        if let priority = edit.priority { task.priority = priority }
        if let startDate = edit.startDate { task.startDate = startDate }
        if let dueDate = edit.dueDate { task.dueDate = dueDate }
        if let dueTime = edit.dueTime {
            task.dueTime = dueTime
        } else if case .some(.none) = edit.dueDate {
            // Taking the due date away takes its time with it.
            task.dueTime = nil
        }
        if let labelIDs = edit.labelIDs {
            try checkLabels(labelIDs)
            task.labelIDs = labelIDs
        }
        if task.dueTime != nil, task.dueDate == nil { throw .dueTimeWithoutDueDate }
        // Only when the dates change, so a task saved before the rule can
        // still be renamed.
        if task.startDate != old.startDate || task.dueDate != old.dueDate, let start = task.startDate,
            let due = task.dueDate, start > due
        {
            throw .startsAfterDue(start: start, due: due)
        }

        var change = Change()
        let fields: [(HistoryEvent.Field, String?, String?)] = [
            (.title, old.title, task.title),
            (.details, old.details, task.details),
            (.priority, old.priority.rawValue, task.priority.rawValue),
            (.startDate, old.startDate?.description, task.startDate?.description),
            (.dueDate, old.dueDate?.description, task.dueDate?.description),
            (.dueTime, old.dueTime?.description, task.dueTime?.description),
            (.labels, Self.describe(old.labelIDs), Self.describe(task.labelIDs)),
        ]
        guard fields.contains(where: { $0.1 != $0.2 }) else { return change }
        update(id, in: context, &change) { $0 = task }
        for (field, oldValue, newValue) in fields where oldValue != newValue {
            record(
                .fieldChanged, on: id, field: field, oldValue: oldValue, newValue: newValue,
                by: context.actor, in: context, &change)
        }
        return change
    }

    /// Any status can move to any other, except:
    ///
    /// - Done needs every Sub task closed (`openSubtasks`), unless
    ///   `cascade: true` closes the open ones as Won't Do first ("Won't Do +
    ///   finish").
    /// - Won't Do with open Sub tasks closes them as Won't Do too, but only
    ///   with `cascade: true` (`wontDoNeedsCascade`), so a client asks first.
    /// - Opening a Sub task under a closed parent moves the parent back to In
    ///   Progress.
    ///
    /// Setting In Progress again ("Restart") records activity, which clears
    /// automatic Procrastinated. Setting any other status again changes
    /// nothing.
    ///
    /// Closing a repeating task creates its next occurrence in the same
    /// change, so undo removes it too.
    package mutating func setStatus(
        _ status: TaskStatus, of id: UUID, cascade: Bool = false, in context: Context
    ) throws(LifrError) -> Change {
        try setStatus(status, of: id, cascade: cascade, repeats: true, in: context)
    }

    /// `repeats: false` closes a repeating task without a next one, as
    /// archiving its Project or Avenue does.
    mutating func setStatus(
        _ status: TaskStatus, of id: UUID, cascade: Bool, repeats: Bool, in context: Context
    ) throws(LifrError) -> Change {
        let task = try liveTask(id)
        var change = Change()
        guard task.status != status else {
            if status == .inProgress { touch(id, in: context, &change) }
            return change
        }
        let open = openSubtasks(of: id).map(\.id)
        if status == .done, !open.isEmpty, !cascade { throw .openSubtasks(task: id, open: open) }
        if status == .wontDo, !open.isEmpty, !cascade { throw .wontDoNeedsCascade(task: id, open: open) }

        if status.isClosed {
            for subtask in open {
                apply(.wontDo, to: subtask, by: context.actor, reason: .closedWithParent, in: context, &change)
            }
        }
        apply(status, to: id, by: context.actor, reason: nil, in: context, &change)
        if let parentID = task.parentID {
            touch(parentID, in: context, &change)
            if !status.isClosed { reopenIfClosed(parentID, in: context, &change) }
        }
        if repeats, status.isClosed, task.isOpen { createNextOccurrence(after: task, in: context, &change) }
        return change
    }

    /// Moves a top-level task, and its Sub tasks, to another Avenue or Project.
    package mutating func move(
        _ id: UUID, toAvenue avenueID: UUID, project projectID: UUID?, in context: Context
    ) throws(LifrError) -> Change {
        let task = try liveTask(id)
        guard !task.isSubtask else { throw .subtaskFollowsParent(id) }
        _ = try avenue(avenueID)
        try checkProject(projectID, isIn: avenueID)
        var change = Change()
        guard task.avenueID != avenueID || task.projectID != projectID else { return change }
        relocate(id, avenue: avenueID, project: projectID, by: context.actor, in: context, &change)
        return change
    }

    /// Puts a top-level task and every one of its Sub tasks in the Avenue
    /// and Project. Every move goes through here, so a Sub task is never
    /// left behind its parent: trashed ones move too, so a restored one is
    /// still with its parent.
    mutating func relocate(
        _ id: UUID, avenue avenueID: UUID, project projectID: UUID?, by actor: Actor, in context: Context,
        _ change: inout Change
    ) {
        place(id, avenue: avenueID, project: projectID, by: actor, reason: nil, in: context, &change)
        let subtasks = tasks.values.filter { $0.parentID == id }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
        for subtask in subtasks {
            place(
                subtask.id, avenue: avenueID, project: projectID, by: actor, reason: .movedWithParent, in: context,
                &change)
        }
    }

    /// Makes a task a Sub task of `parentID`, or a top-level task when
    /// `parentID` is nil. Only tasks without Sub tasks can move under another.
    package mutating func setParent(
        _ parentID: UUID?, of id: UUID, in context: Context
    ) throws(LifrError) -> Change {
        let task = try liveTask(id)
        var change = Change()
        guard task.parentID != parentID else { return change }
        var parent: LifrTask?
        if let parentID {
            guard parentID != id else { throw .cannotParentToItself(id) }
            let found = try liveTask(parentID)
            guard !found.isSubtask else { throw .parentIsSubtask(parentID) }
            guard subtasks(of: id).isEmpty else { throw .taskHasSubtasks(id) }
            parent = found
        }

        let oldParent = task.parentID
        update(id, in: context, &change) { $0.parentID = parentID }
        record(
            .fieldChanged, on: id, field: .parent,
            oldValue: oldParent.flatMap { tasks[$0]?.code.rawValue },
            newValue: parent?.code.rawValue, by: context.actor, in: context, &change)
        if let oldParent {
            touch(oldParent, in: context, &change)
            record(
                .subtaskRemoved, on: oldParent, newValue: task.code.rawValue, by: context.actor, in: context, &change)
        }
        if let parent {
            place(
                id, avenue: parent.avenueID, project: parent.projectID, by: context.actor,
                reason: .movedWithParent, in: context, &change)
            touch(parent.id, in: context, &change)
            record(.subtaskAdded, on: parent.id, newValue: task.code.rawValue, by: context.actor, in: context, &change)
            if task.isOpen { reopenIfClosed(parent.id, in: context, &change) }
        }
        return change
    }

    // MARK: - Manual order

    /// Puts a task between two others in manual order: after `lower` and
    /// before `upper`, either nil at an end. Only the moved task changes, and
    /// moving doesn't count as activity on it.
    package mutating func reorder(
        task id: UUID, after lower: UUID?, before upper: UUID?, in context: Context
    ) throws(LifrError) -> Change {
        var task = try liveTask(id)
        var change = Change()
        let key = Self.key(
            after: try lower.map(self.task)?.sortOrder, before: try upper.map(self.task)?.sortOrder)
        guard key != task.sortOrder else { return change }
        change.previous[id] = .some(task)
        task.sortOrder = key
        task.updatedAt = context.now
        tasks[id] = task
        return change
    }

    /// A copy of a task, open, right after it among its siblings: the same
    /// title, description, priority, dates, labels, repeat, and reminders
    /// still to come, with copies of its open Sub tasks. History, comments
    /// and closed Sub tasks stay with the original.
    package mutating func duplicate(task id: UUID, in context: Context) throws(LifrError) -> Change {
        let original = try liveTask(id)
        var change = try createTask(copy(of: original, parentID: original.parentID, now: context.now), in: context)
        guard let copyID = change.previous.first(where: { $0.value == nil })?.key, var made = tasks[copyID] else {
            return change
        }
        let next = tasks.values
            .filter {
                $0.deletedAt == nil && $0.id != copyID && $0.parentID == original.parentID
                    && $0.avenueID == original.avenueID && $0.sortOrder > original.sortOrder
            }
            .map(\.sortOrder).min()
        made.sortOrder = Self.key(after: original.sortOrder, before: next)
        tasks[copyID] = made
        for subtask in subtasks(of: id) where subtask.isOpen {
            change.merge(try createTask(copy(of: subtask, parentID: copyID, now: context.now), in: context))
        }
        return change
    }

    /// What `duplicate` creates for `task`.
    private func copy(of task: LifrTask, parentID: UUID?, now: Date) -> NewTask {
        var new = NewTask(title: task.title, avenueID: task.avenueID, projectID: task.projectID, parentID: parentID)
        new.details = task.details
        new.priority = task.priority
        new.startDate = task.startDate
        new.dueDate = task.dueDate
        new.dueTime = task.dueTime
        new.labelIDs = task.labelIDs
        new.repeats = task.recurrence?.rule
        new.repeatInterval = task.recurrence?.interval ?? 1
        new.reminders = reminders.values
            .filter { reminder in
                guard reminder.taskID == task.id, !reminder.isOneOff else { return false }
                if case .at(let date) = reminder.trigger { return date > now }
                return true
            }
            .sorted { $0.id.uuidString < $1.id.uuidString }
            .map(\.trigger)
        return new
    }

    /// Puts an Avenue between two others in the sidebar's order. The Inbox
    /// stays first: it doesn't move, and nothing goes above it.
    package mutating func reorder(avenue id: UUID, after lower: UUID?, before upper: UUID?) throws(LifrError) {
        var avenue = try avenue(id)
        var lowerKey = try lower.map(self.avenue)?.sortOrder
        var upperKey = try upper.map(self.avenue)?.sortOrder
        guard !avenue.isInbox else { return }
        if let inbox = avenues[Avenue.inboxID], lowerKey.map({ $0 < inbox.sortOrder }) ?? true {
            lowerKey = inbox.sortOrder
            if let key = upperKey, key <= inbox.sortOrder {
                upperKey = avenues.values
                    .filter { $0.id != id && $0.sortOrder > inbox.sortOrder }
                    .map(\.sortOrder).min()
            }
        }
        avenue.sortOrder = Self.key(after: lowerKey, before: upperKey)
        avenues[id] = avenue
    }

    /// Puts a Project between two others in its Avenue's order.
    package mutating func reorder(project id: UUID, after lower: UUID?, before upper: UUID?) throws(LifrError) {
        var project = try project(id)
        project.sortOrder = Self.key(
            after: try lower.map(self.project)?.sortOrder, before: try upper.map(self.project)?.sortOrder)
        projects[id] = project
    }

    /// A sort key between two neighbours'. Two with the same key, which two
    /// devices can make, leave no room between them, so it goes after both.
    static func key(after lower: String?, before upper: String?) -> String {
        guard let lower, let upper, lower >= upper else { return SortKey.between(lower, upper) }
        return SortKey.between(lower, nil)
    }

    /// Restores every task `change` touched to how it was, and removes tasks
    /// it created. History is kept, with an `undo` event per restored task.
    package mutating func undo(_ change: Change, in context: Context) -> Change {
        var undone = Change()
        undone.reverts = change.entry.map { [$0] } ?? []
        for (id, previous) in change.previous.sorted(by: { $0.key.uuidString < $1.key.uuidString }) {
            let current = tasks[id]
            undone.previous[id] = current
            guard let previous else {
                tasks[id] = nil
                continue
            }
            tasks[id] = previous
            if let current, current.status != previous.status {
                record(
                    .fieldChanged, on: id, field: .status, oldValue: current.status.rawValue,
                    newValue: previous.status.rawValue, reason: .undo, by: context.actor,
                    in: context, &undone)
            }
        }
        removeAdded(change.added)
        return undone
    }

    /// Removes records a change added, newest first. Avenues, Projects and
    /// labels stay while anything still uses them.
    private mutating func removeAdded(_ added: [Change.Added]) {
        for record in added.reversed() {
            switch record {
            case .task:
                break  // `previous` covers tasks.
            case .comment(let id):
                comments[id] = nil
            case .reminder(let id):
                reminders[id] = nil
            case .label(let id):
                if !tasks.values.contains(where: { $0.labelIDs.contains(id) }) { labels[id] = nil }
            case .project(let id):
                if !tasks.values.contains(where: { $0.projectID == id }) { projects[id] = nil }
            case .avenue(let id):
                if !tasks.values.contains(where: { $0.avenueID == id }),
                    !projects.values.contains(where: { $0.avenueID == id })
                {
                    avenues[id] = nil
                }
            }
        }
    }

    /// Whether `revert` can put back what `event` changed: a field of the
    /// task's own that nothing has changed again since.
    package func canRevert(_ event: HistoryEvent) -> Bool {
        guard event.kind == .fieldChanged, let field = event.field, let task = tasks[event.taskID],
            task.deletedAt == nil, let current = Self.value(of: field, in: task)
        else { return false }
        return current == event.newValue
    }

    /// Puts back the field `eventID` changed, as long as nothing changed it
    /// again since. Status goes through `setStatus`'s rules, so it can still
    /// be refused, for example Done with open Sub tasks.
    package mutating func revert(_ eventID: UUID, in context: Context) throws(LifrError) -> Change {
        guard let event = history.first(where: { $0.id == eventID }) else {
            throw .notFound(.task, eventID.uuidString)
        }
        guard event.kind == .fieldChanged, let field = event.field,
            Self.value(of: field, in: try liveTask(event.taskID)) != nil
        else { throw .cannotRevert(event: eventID) }
        guard canRevert(event) else { throw .changedSince(event: eventID) }
        let id = event.taskID
        let old = event.oldValue
        var change: Change
        switch field {
        case .title:
            change = try edit(id, TaskEdit(title: old ?? ""), in: context)
        case .details:
            change = try edit(id, TaskEdit(details: old ?? ""), in: context)
        case .priority:
            change = try edit(
                id, TaskEdit(priority: old.flatMap(Priority.init(rawValue:)) ?? Priority.none), in: context)
        case .startDate:
            change = try edit(id, TaskEdit(startDate: .some(try old.map(CalendarDate.init))), in: context)
        case .dueDate:
            change = try edit(id, TaskEdit(dueDate: .some(try old.map(CalendarDate.init))), in: context)
        case .dueTime:
            change = try edit(id, TaskEdit(dueTime: .some(try old.map(TimeOfDay.init))), in: context)
        case .labels:
            let ids = (old ?? "").split(separator: ",").compactMap { UUID(uuidString: String($0)) }
            change = try edit(id, TaskEdit(labelIDs: Set(ids)), in: context)
        case .status:
            change = try setStatus(old.flatMap(TaskStatus.init(rawValue:)) ?? .todo, of: id, in: context)
        case .avenue, .project, .parent, .code, .recurrence:
            throw .cannotRevert(event: eventID)
        }
        // The new events say they undo something, like ⌘Z's.
        let ids = Set(change.events.map(\.id))
        for index in history.indices where ids.contains(history[index].id) && history[index].reason == nil {
            history[index].reason = .undo
        }
        for index in change.events.indices where change.events[index].reason == nil {
            change.events[index].reason = .undo
        }
        return change
    }

    /// `field` of `task` as history records it, or `nil` for fields history
    /// can't put back, such as moves.
    private static func value(of field: HistoryEvent.Field, in task: LifrTask) -> String?? {
        switch field {
        case .title: .some(task.title)
        case .details: .some(task.details)
        case .priority: .some(task.priority.rawValue)
        case .startDate: .some(task.startDate?.description)
        case .dueDate: .some(task.dueDate?.description)
        case .dueTime: .some(task.dueTime?.description)
        case .labels: .some(describe(task.labelIDs))
        case .status: .some(task.status.rawValue)
        case .avenue, .project, .parent, .code, .recurrence: nil
        }
    }

    // MARK: - Trash

    /// Moves a task and its Sub tasks to the trash.
    package mutating func moveToTrash(_ id: UUID, in context: Context) throws(LifrError) -> Change {
        _ = try liveTask(id)
        var change = Change()
        for taskID in [id] + subtasks(of: id).map(\.id) {
            update(taskID, in: context, &change) { $0.deletedAt = context.now }
            record(.movedToTrash, on: taskID, by: context.actor, in: context, &change)
        }
        return change
    }

    /// Takes a task and the Sub tasks trashed with it back out of the trash.
    /// A Sub task whose parent is in the trash brings the parent back too,
    /// as a Sub task can't sit under a task in the trash.
    package mutating func restore(_ id: UUID, in context: Context) throws(LifrError) -> Change {
        var change = Change()
        try restore(id, in: context, &change)
        return change
    }

    private mutating func restore(_ id: UUID, in context: Context, _ change: inout Change) throws(LifrError) {
        let task = try task(id)
        if let parentID = task.parentID, tasks[parentID]?.deletedAt != nil {
            try restore(parentID, in: context, &change)
        }
        // The parent may have brought this one back with it.
        guard let deletedAt = tasks[id]?.deletedAt else { return }
        let trashedWithIt = tasks.values.filter { $0.parentID == id && $0.deletedAt == deletedAt }
        for taskID in [id] + trashedWithIt.map(\.id) {
            update(taskID, in: context, &change) { $0.deletedAt = nil }
            record(.restored, on: taskID, by: context.actor, in: context, &change)
        }
    }

    /// The tasks in the trash, newest first, with when each is deleted for
    /// good. Sub tasks trashed with their parent are left out: they go and
    /// come back with it.
    package func trash() -> [TrashedTask] {
        tasks.values
            .compactMap { task -> TrashedTask? in
                guard let deletedAt = task.deletedAt else { return nil }
                if let parentID = task.parentID, tasks[parentID]?.deletedAt != nil { return nil }
                return TrashedTask(task: task, purgedAt: deletedAt.addingTimeInterval(Self.trashRetention))
            }
            .sorted { ($1.purgedAt, $0.task.id.uuidString) < ($0.purgedAt, $1.task.id.uuidString) }
    }

    /// Deletes tasks trashed more than `trashRetention` ago, with their
    /// comments, reminders and history.
    package mutating func purgeTrash(now: Date) {
        purge(tasks.values.filter { $0.deletedAt.map { now.timeIntervalSince($0) > Self.trashRetention } ?? false })
    }

    /// Deletes every task in the trash now, with its comments, reminders and
    /// history. It can't be undone.
    package mutating func emptyTrash() {
        purge(tasks.values.filter { $0.deletedAt != nil })
    }

    /// Deletes every Avenue, Project, task, label, comment, reminder, saved
    /// filter and history event, and puts the settings back to their
    /// defaults, leaving an empty Inbox. Sync sends the deletions. Journal
    /// entries stay: they're written once and never removed. It can't be
    /// undone.
    package mutating func eraseAll(in context: Context) {
        avenues = [:]
        projects = [:]
        tasks = [:]
        labels = [:]
        comments = [:]
        reminders = [:]
        savedFilters = [:]
        history = []
        settings = Settings()
        _ = ensureInbox(in: context)
    }

    private mutating func purge(_ expired: [LifrTask]) {
        let ids = Set(expired.map(\.id))
        let purged = ids.union(tasks.values.filter { $0.parentID.map(ids.contains) ?? false }.map(\.id))
        for id in purged { tasks[id] = nil }
        comments = comments.filter { !purged.contains($0.value.taskID) }
        reminders = reminders.filter { !purged.contains($0.value.taskID) }
        history.removeAll { purged.contains($0.taskID) }
    }

    // MARK: - Comments

    package mutating func addComment(
        _ body: String, to taskID: UUID, in context: Context
    ) throws(LifrError) -> Comment {
        _ = try liveTask(taskID)
        guard !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw .emptyComment }
        let comment = Comment(
            id: context.newID(), taskID: taskID, body: body, author: context.actor,
            createdAt: context.now, editedAt: nil, deletedAt: nil)
        comments[comment.id] = comment
        var change = Change()
        touch(taskID, in: context, &change)
        record(.commentAdded, on: taskID, by: context.actor, in: context, &change)
        return comment
    }

    // MARK: - Reminders

    /// Adds a reminder to a task. One before the due date needs the task to
    /// have one. A one-off, such as a snooze, isn't copied to the next
    /// occurrence.
    @discardableResult
    package mutating func addReminder(
        _ trigger: Reminder.Trigger, to taskID: UUID, isOneOff: Bool = false, in context: Context
    ) throws(LifrError) -> Reminder {
        let task = try liveTask(taskID)
        if case .beforeDue = trigger, task.dueDate == nil { throw .reminderNeedsDueDate(taskID) }
        let reminder = Reminder(id: context.newID(), taskID: taskID, trigger: trigger, isOneOff: isOneOff)
        reminders[reminder.id] = reminder
        return reminder
    }

    package mutating func removeReminder(_ id: UUID) throws(LifrError) {
        guard reminders.removeValue(forKey: id) != nil else { throw .notFound(.reminder, id.uuidString) }
    }

    /// A task's reminders, soonest first; ones that can't fire last.
    package func reminders(of taskID: UUID, timeZone: TimeZone) -> [Reminder] {
        guard let task = tasks[taskID] else { return [] }
        return reminders.values.filter { $0.taskID == taskID }.sorted {
            ($0.fireDate(for: task, in: timeZone) ?? .distantFuture, $0.id.uuidString)
                < ($1.fireDate(for: task, in: timeZone) ?? .distantFuture, $1.id.uuidString)
        }
    }

    /// People edit their own comments; an agent edits only comments it wrote.
    package mutating func editComment(_ id: UUID, body: String, in context: Context) throws(LifrError) {
        var comment = try ownComment(id, context.actor)
        guard !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw .emptyComment }
        let old = comment.body
        comment.body = body
        comment.editedAt = context.now
        comments[id] = comment
        var change = Change()
        touch(comment.taskID, in: context, &change)
        record(
            .commentEdited, on: comment.taskID, oldValue: old, newValue: body, by: context.actor,
            in: context, &change)
    }

    package mutating func deleteComment(_ id: UUID, in context: Context) throws(LifrError) {
        var comment = try ownComment(id, context.actor)
        comment.deletedAt = context.now
        comments[id] = comment
        var change = Change()
        touch(comment.taskID, in: context, &change)
        record(.commentDeleted, on: comment.taskID, by: context.actor, in: context, &change)
    }

    func ownComment(_ id: UUID, _ actor: Actor) throws(LifrError) -> Comment {
        guard let comment = comments[id], comment.deletedAt == nil else {
            throw .notFound(.comment, id.uuidString)
        }
        let author = comment.author
        let isAuthor =
            switch actor.kind {
            case .user: author.kind == .user
            case .agent: author.kind == .agent && author.client == actor.client
            case .system: false
            }
        guard isAuthor else { throw .notCommentAuthor(comment: id) }
        return comment
    }

    // MARK: - Helpers

    static func validTitle(_ title: String) throws(LifrError) -> String {
        let title = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard (1...200).contains(title.count) else { throw .invalidTitle(length: title.count) }
        return title
    }

    static func validName(_ name: String) throws(LifrError) -> String {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard (1...100).contains(name.count) else { throw .invalidName(length: name.count) }
        return name
    }

    static func describe(_ labels: Set<UUID>) -> String {
        labels.map(\.uuidString).sorted().joined(separator: ",")
    }

    func checkProject(_ projectID: UUID?, isIn avenueID: UUID) throws(LifrError) {
        guard let projectID else { return }
        let project = try project(projectID)
        guard project.avenueID == avenueID else {
            throw .projectInDifferentAvenue(project: projectID, avenue: avenueID)
        }
    }

    func checkLabels(_ ids: Set<UUID>) throws(LifrError) {
        if let missing = ids.sorted(by: { $0.uuidString < $1.uuidString }).first(where: { labels[$0] == nil }) {
            throw .notFound(.label, missing.uuidString)
        }
    }

    func uniqueCode(_ context: Context) -> TaskCode {
        let used = Set(tasks.values.map(\.code))
        while true {
            let code = context.newCode()
            if !used.contains(code) { return code }
        }
    }

    /// Changes a task, remembering how it was for undo.
    mutating func update(_ id: UUID, in context: Context, _ change: inout Change, _ body: (inout LifrTask) -> Void) {
        guard var task = tasks[id] else { return }
        if change.previous[id] == nil { change.previous[id] = .some(task) }
        body(&task)
        task.updatedAt = context.now
        task.lastActivityAt = context.now
        tasks[id] = task
    }

    /// Records activity on a task without changing it, which clears automatic
    /// Procrastinated. Sub task changes count as activity on the parent.
    mutating func touch(_ id: UUID, in context: Context, _ change: inout Change) {
        update(id, in: context, &change) { _ in }
    }

    mutating func apply(
        _ status: TaskStatus, to id: UUID, by actor: Actor, reason: HistoryEvent.Reason?,
        in context: Context, _ change: inout Change
    ) {
        guard let old = tasks[id]?.status, old != status else { return }
        update(id, in: context, &change) { task in
            task.status = status
            task.closedAt = status.isClosed ? (task.closedAt ?? context.now) : nil
        }
        record(
            .fieldChanged, on: id, field: .status, oldValue: old.rawValue, newValue: status.rawValue,
            reason: reason, by: actor, in: context, &change)
    }

    mutating func reopenIfClosed(_ parentID: UUID, in context: Context, _ change: inout Change) {
        guard let parent = tasks[parentID], parent.status.isClosed else { return }
        apply(
            .inProgress, to: parentID, by: context.system, reason: .subtaskOpenedUnderClosedParent,
            in: context, &change)
        change.reopenedParent = parentID
    }

    mutating func place(
        _ id: UUID, avenue avenueID: UUID, project projectID: UUID?, by actor: Actor,
        reason: HistoryEvent.Reason?, in context: Context, _ change: inout Change
    ) {
        guard let old = tasks[id], old.avenueID != avenueID || old.projectID != projectID else { return }
        update(id, in: context, &change) { task in
            task.avenueID = avenueID
            task.projectID = projectID
        }
        if old.avenueID != avenueID {
            record(
                .fieldChanged, on: id, field: .avenue, oldValue: avenues[old.avenueID]?.key.rawValue,
                newValue: avenues[avenueID]?.key.rawValue, reason: reason, by: actor, in: context, &change)
        }
        if old.projectID != projectID {
            record(
                .fieldChanged, on: id, field: .project, oldValue: old.projectID.flatMap { projects[$0]?.name },
                newValue: projectID.flatMap { projects[$0]?.name }, reason: reason, by: actor,
                in: context, &change)
        }
    }

    mutating func record(
        _ kind: HistoryEvent.Kind, on taskID: UUID, field: HistoryEvent.Field? = nil,
        oldValue: String? = nil, newValue: String? = nil, reason: HistoryEvent.Reason? = nil,
        by actor: Actor, in context: Context, _ change: inout Change
    ) {
        let event = HistoryEvent(
            id: context.newID(), taskID: taskID, actor: actor, kind: kind, field: field,
            oldValue: oldValue, newValue: newValue, reason: reason, createdAt: context.now
        )
        history.append(event)
        change.events.append(event)
    }
}
