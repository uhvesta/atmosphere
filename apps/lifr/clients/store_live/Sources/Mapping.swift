import Foundation
import LifrCore

// Converting between the stored models and Core's records.
//
// Reading returns nil for a record Core can't use yet. CloudKit imports
// records in any order, so a task can arrive before its Avenue; it shows up
// once the Avenue does. A value written by a newer version of Lifr, such as a
// status this version doesn't know, is skipped the same way.

extension Actor {
    static func stored(kind: String, device: String, client: String?) -> Actor? {
        switch Kind(rawValue: kind) {
        case .user: .user(device: device)
        case .agent: .agent(client ?? "", device: device)
        case .system: .system(device: device)
        case nil: nil
        }
    }
}

extension StoredAvenue {
    var record: Avenue? {
        guard let key = try? AvenueKey(key), let access = AgentAccess(rawValue: agentAccess) else { return nil }
        return Avenue(
            id: id, key: key, name: name, color: color, symbol: symbol, sortOrder: sortOrder,
            agentAccess: access, createdAt: createdAt, archivedAt: archivedAt)
    }

    func write(_ avenue: Avenue) {
        key = avenue.key.rawValue
        name = avenue.name
        color = avenue.color
        symbol = avenue.symbol
        sortOrder = avenue.sortOrder
        agentAccess = avenue.agentAccess.rawValue
        createdAt = avenue.createdAt
        archivedAt = avenue.archivedAt
    }
}

extension StoredProject {
    var record: Project? {
        guard let avenue, let status = ProjectStatus(rawValue: status) else { return nil }
        return Project(
            id: id, avenueID: avenue.id, name: name, details: details, status: status,
            dueDate: dueDate.flatMap { try? CalendarDate($0) }, sortOrder: sortOrder, createdAt: createdAt,
            completedAt: completedAt)
    }

    func write(_ project: Project, in index: Index) {
        avenue = index.avenues[project.avenueID]
        name = project.name
        details = project.details
        status = project.status.rawValue
        dueDate = project.dueDate?.description
        sortOrder = project.sortOrder
        createdAt = project.createdAt
        completedAt = project.completedAt
    }
}

extension StoredTask {
    var record: LifrTask? {
        guard let avenue, let code = try? TaskCode(code), let status = TaskStatus(rawValue: status),
            let priority = Priority(rawValue: priority)
        else { return nil }
        return LifrTask(
            id: id, code: code, title: title, details: details, status: status, priority: priority,
            startDate: startDate.flatMap { try? CalendarDate($0) }, dueDate: dueDate.flatMap { try? CalendarDate($0) },
            dueTime: dueTime.flatMap { try? TimeOfDay($0) }, avenueID: avenue.id, projectID: project?.id,
            parentID: parent?.id, labelIDs: Set((labels ?? []).map(\.id)), sortOrder: sortOrder,
            createdAt: createdAt, updatedAt: updatedAt, lastActivityAt: lastActivityAt, closedAt: closedAt,
            deletedAt: deletedAt)
    }

    /// Everything but `parent`, which may not be written yet: see
    /// `writeParent`.
    func write(_ task: LifrTask, in index: Index) {
        code = task.code.rawValue
        title = task.title
        details = task.details
        status = task.status.rawValue
        priority = task.priority.rawValue
        startDate = task.startDate?.description
        dueDate = task.dueDate?.description
        dueTime = task.dueTime?.description
        avenue = index.avenues[task.avenueID]
        project = task.projectID.flatMap { index.projects[$0] }
        labels = task.labelIDs.compactMap { index.labels[$0] }
        sortOrder = task.sortOrder
        createdAt = task.createdAt
        updatedAt = task.updatedAt
        lastActivityAt = task.lastActivityAt
        closedAt = task.closedAt
        deletedAt = task.deletedAt
    }

    func writeParent(_ task: LifrTask, in index: Index) {
        parent = task.parentID.flatMap { index.tasks[$0] }
    }
}

extension StoredLabel {
    var record: Label { Label(id: id, name: name, color: color, createdAt: createdAt) }

    func write(_ label: Label) {
        name = label.name
        color = label.color
        createdAt = label.createdAt
    }
}

extension StoredComment {
    var record: Comment? {
        guard let task, let author = Actor.stored(kind: authorKind, device: authorDevice, client: authorClient) else {
            return nil
        }
        return Comment(
            id: id, taskID: task.id, body: body, author: author, createdAt: createdAt, editedAt: editedAt,
            deletedAt: deletedAt)
    }

    func write(_ comment: Comment, in index: Index) {
        task = index.tasks[comment.taskID]
        body = comment.body
        authorKind = comment.author.kind.rawValue
        authorDevice = comment.author.device
        authorClient = comment.author.client
        createdAt = comment.createdAt
        editedAt = comment.editedAt
        deletedAt = comment.deletedAt
    }
}

extension StoredReminder {
    var record: Reminder? {
        guard let task else { return nil }
        if let fireAt { return Reminder(id: id, taskID: task.id, trigger: .at(fireAt)) }
        if let secondsBeforeDue {
            return Reminder(id: id, taskID: task.id, trigger: .beforeDue(seconds: secondsBeforeDue))
        }
        return nil
    }

    func write(_ reminder: Reminder, in index: Index) {
        task = index.tasks[reminder.taskID]
        switch reminder.trigger {
        case .at(let date):
            fireAt = date
            secondsBeforeDue = nil
        case .beforeDue(let seconds):
            fireAt = nil
            secondsBeforeDue = seconds
        }
    }
}

extension StoredHistoryEvent {
    var record: HistoryEvent? {
        guard let task, let actor = Actor.stored(kind: actorKind, device: actorDevice, client: actorClient),
            let kind = HistoryEvent.Kind(rawValue: kind)
        else { return nil }
        let field = field.map(HistoryEvent.Field.init(rawValue:))
        let reason = reason.map(HistoryEvent.Reason.init(rawValue:))
        // A field or reason this version doesn't know.
        if field == .some(nil) || reason == .some(nil) { return nil }
        return HistoryEvent(
            id: id, taskID: task.id, actor: actor, kind: kind, field: field ?? nil, oldValue: oldValue,
            newValue: newValue, reason: reason ?? nil, createdAt: createdAt)
    }

    func write(_ event: HistoryEvent, in index: Index) {
        task = index.tasks[event.taskID]
        actorKind = event.actor.kind.rawValue
        actorDevice = event.actor.device
        actorClient = event.actor.client
        kind = event.kind.rawValue
        field = event.field?.rawValue
        oldValue = event.oldValue
        newValue = event.newValue
        reason = event.reason?.rawValue
        createdAt = event.createdAt
    }
}
