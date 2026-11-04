import Foundation

/// Every record, and the operations that change them. Each operation checks
/// the rules from the spec, writes history, and returns a `Change`. A refused
/// operation throws and changes nothing.
package struct Workspace: Hashable, Codable, Sendable {
    package internal(set) var avenues: [UUID: Avenue] = [:]
    package internal(set) var projects: [UUID: Project] = [:]
    package internal(set) var tasks: [UUID: LifrTask] = [:]
    package internal(set) var labels: [UUID: Label] = [:]
    package internal(set) var comments: [UUID: Comment] = [:]
    package internal(set) var reminders: [UUID: Reminder] = [:]
    /// Append-only.
    package internal(set) var history: [HistoryEvent] = []
    package var settings = Settings()

    /// How long trashed tasks are kept before `purgeTrash` deletes them.
    package static let trashRetention: TimeInterval = 30 * 24 * 60 * 60

    package init() {}

    /// Records loaded from storage, as they are. They may break a rule if
    /// sync brought them in from another device; call `reconcile` after.
    package init(
        avenues: [Avenue] = [], projects: [Project] = [], tasks: [LifrTask] = [], labels: [Label] = [],
        comments: [Comment] = [], reminders: [Reminder] = [], history: [HistoryEvent] = [],
        settings: Settings = Settings()
    ) {
        self.avenues = Dictionary(avenues.map { ($0.id, $0) }) { _, last in last }
        self.projects = Dictionary(projects.map { ($0.id, $0) }) { _, last in last }
        self.tasks = Dictionary(tasks.map { ($0.id, $0) }) { _, last in last }
        self.labels = Dictionary(labels.map { ($0.id, $0) }) { _, last in last }
        self.comments = Dictionary(comments.map { ($0.id, $0) }) { _, last in last }
        self.reminders = Dictionary(reminders.map { ($0.id, $0) }) { _, last in last }
        self.history = history
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
        tasks.values
            .filter { $0.parentID == id && $0.deletedAt == nil }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
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

    @discardableResult
    package mutating func createLabel(
        name: String, color: String = "gray", in context: Context
    ) -> Label {
        let label = Label(id: context.newID(), name: name, color: color, createdAt: context.now)
        labels[label.id] = label
        return label
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
        try checkLabels(new.labelIDs)

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
        let task = LifrTask(
            id: context.newID(), code: uniqueCode(context), title: title, details: new.details,
            status: new.status, priority: new.priority, startDate: new.startDate,
            dueDate: new.dueDate, dueTime: new.dueTime, avenueID: avenueID, projectID: projectID,
            parentID: new.parentID, labelIDs: new.labelIDs,
            sortOrder: SortKey.between(siblings.map(\.sortOrder).max(), nil),
            createdAt: context.now, updatedAt: context.now, lastActivityAt: context.now,
            closedAt: new.status.isClosed ? context.now : nil, deletedAt: nil
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
        return change
    }

    /// Changes to a task's own fields. `nil` leaves a field as it is; for the
    /// optional fields, `.some(nil)` clears it.
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
    }

    package mutating func edit(_ id: UUID, _ edit: TaskEdit, in context: Context) throws(LifrError) -> Change {
        var task = try liveTask(id)
        let old = task
        if let title = edit.title { task.title = try Self.validTitle(title) }
        if let details = edit.details { task.details = details }
        if let priority = edit.priority { task.priority = priority }
        if let startDate = edit.startDate { task.startDate = startDate }
        if let dueDate = edit.dueDate { task.dueDate = dueDate }
        if let dueTime = edit.dueTime { task.dueTime = dueTime }
        if let labelIDs = edit.labelIDs {
            try checkLabels(labelIDs)
            task.labelIDs = labelIDs
        }
        if task.dueTime != nil, task.dueDate == nil { throw .dueTimeWithoutDueDate }

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
    package mutating func setStatus(
        _ status: TaskStatus, of id: UUID, cascade: Bool = false, in context: Context
    ) throws(LifrError) -> Change {
        let task = try liveTask(id)
        var change = Change()
        guard task.status != status else { return change }
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
        for (taskID, reason) in [(id, nil)] + subtasks(of: id).map({ ($0.id, HistoryEvent.Reason.movedWithParent) }) {
            place(taskID, avenue: avenueID, project: projectID, by: context.actor, reason: reason, in: context, &change)
        }
        return change
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

    /// Restores every task `change` touched to how it was, and removes tasks
    /// it created. History is kept, with an `undo` event per restored task.
    package mutating func undo(_ change: Change, in context: Context) -> Change {
        var undone = Change()
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
        return undone
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
    package mutating func restore(_ id: UUID, in context: Context) throws(LifrError) -> Change {
        let task = try task(id)
        var change = Change()
        guard let deletedAt = task.deletedAt else { return change }
        let trashedWithIt = tasks.values.filter { $0.parentID == id && $0.deletedAt == deletedAt }
        for taskID in [id] + trashedWithIt.map(\.id) {
            update(taskID, in: context, &change) { $0.deletedAt = nil }
            record(.restored, on: taskID, by: context.actor, in: context, &change)
        }
        return change
    }

    /// Deletes tasks trashed more than `trashRetention` ago, with their
    /// comments, reminders and history.
    package mutating func purgeTrash(now: Date) {
        let expired = Set(
            tasks.values.compactMap { task in
                task.deletedAt.flatMap { now.timeIntervalSince($0) > Self.trashRetention ? task.id : nil }
            })
        let purged = expired.union(tasks.values.filter { $0.parentID.map(expired.contains) ?? false }.map(\.id))
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
