import ComposableArchitecture
import Foundation
import LifrCore

/// A change the assistant makes, run the way the stores run every save.
package typealias AssistantOperation = @Sendable (inout Workspace, Context) throws(LifrError) -> Change

/// That the assistant may change records in a conversation: it proposes
/// each change, for the person to apply as one change of that conversation,
/// so it can be found and reverted with the rest.
package struct AssistantWrites: Sendable, Hashable {
    package var conversation: UUID

    package init(conversation: UUID) {
        self.conversation = conversation
    }
}

/// A change to one existing task, as the assistant's answer gives it.
/// Fields left nil stay as they are.
package struct TaskChange: Hashable, Codable, Sendable {
    /// Such as HOME-0003.
    package var code: String
    package var title: String?
    /// An Avenue key, or "KEY/Project name", to move the task to.
    package var place: String?
    /// todo, in_progress, done or wont_do.
    package var status: String?
    package var priority: String?
    /// A date as `CalendarDate.resolving` reads it, or "none" to clear it.
    package var due: String?

    package init(
        code: String, title: String? = nil, place: String? = nil, status: String? = nil, priority: String? = nil,
        due: String? = nil
    ) {
        self.code = code
        self.title = title
        self.place = place
        self.status = status
        self.priority = priority
        self.due = due
    }
}

/// Why a change the assistant wrote wasn't made. Nothing of that change
/// is saved.
package enum TaskChangeError: Error, Hashable, Sendable, CustomStringConvertible {
    case noTask(code: String)
    case unknownStatus(String)
    case unknownPriority(String)
    case invalidDate(String)
    case unknownAvenue(key: String, suggestion: String?)
    case unknownProject(name: String, avenue: String, suggestion: String?)
    /// Sub tasks stay with their parent.
    case subtaskMoves(code: String)
    case nothingToChange(code: String)
    /// The conversation can't change anything, such as with a skill that
    /// only reads.
    case readOnly
    /// A Core rule refused the change.
    case refused(code: String, LifrError)

    package var description: String {
        switch self {
        case .noTask(let code):
            "There's no task \(code), so it wasn't changed. Ask again with the task's title."
        case .unknownStatus(let status):
            "“\(status)” isn't a status Lifr can set. Ask for to do, in progress, done or won't do."
        case .unknownPriority(let priority):
            "“\(priority)” isn't a priority. Ask for none, low, medium, high or urgent."
        case .invalidDate(let text):
            "“\(text)” isn't a date Lifr reads, so the due date wasn't changed. " + Plan.dateHelp
        case .unknownAvenue(let key, let suggestion):
            "There's no Avenue \(key), so the task wasn't moved."
                + (suggestion.map { " Did you mean \($0)?" } ?? " Ask again with one from the sidebar.")
        case .unknownProject(let name, let avenue, let suggestion):
            "\(avenue) has no Project “\(name)”, so the task wasn't moved."
                + (suggestion.map { " Did you mean “\($0)”?" } ?? " Ask again with one of its Projects.")
        case .subtaskMoves(let code):
            "\(code) is a Sub task, so it moves with its parent. Move the parent task instead."
        case .nothingToChange(let code):
            "The change to \(code) didn't say what to change."
        case .readOnly:
            "Nothing can be changed in this conversation. Change the task yourself, or ask without the skill."
        case .refused(let code, let error):
            "\(code) wasn't changed: \(error)"
        }
    }
}

extension AssistantTools {
    /// The statuses the assistant can set. Procrastinated isn't one: Lifr
    /// works it out from activity.
    private static let settable: [String: TaskStatus] = [
        "todo": .todo, "to_do": .todo, "in_progress": .inProgress, "done": .done, "wont_do": .wontDo,
    ]

    /// Makes `change` to its task, all of it or none, and returns the task
    /// after it. Fields that say what the task already has change nothing.
    /// Won't Do also closes open Sub tasks; a move takes the Sub tasks along.
    package func change(_ change: TaskChange) async throws(TaskChangeError) -> LifrTask {
        let code = change.code.trimmingCharacters(in: .whitespaces).uppercased()
        guard let task = try? workspace.task(code: code), task.deletedAt == nil else { throw .noTask(code: code) }
        let shown = workspace.displayCode(of: task)
        let newStatus = try change.status.nonEmpty.map { (status) throws(TaskChangeError) in
            guard let match = Self.settable[status.lowercased().replacingOccurrences(of: " ", with: "_")] else {
                throw .unknownStatus(status)
            }
            return match
        }.flatMap { $0 == task.status ? nil : $0 }
        let newPriority = try change.priority.nonEmpty.map { (priority) throws(TaskChangeError) in
            guard let match = Priority(rawValue: priority.lowercased()) else { throw .unknownPriority(priority) }
            return match
        }.flatMap { $0 == task.priority ? nil : $0 }
        let newDue = try change.due.nonEmpty.flatMap { (due) throws(TaskChangeError) -> Workspace.TaskEdit? in
            if due.lowercased() == "none" { return task.dueDate == nil ? nil : Workspace.TaskEdit(dueDate: .some(nil)) }
            guard let date = CalendarDate.resolving(due, today: today) else { throw .invalidDate(due) }
            return .due(date, for: task)
        }
        let newTitle = change.title.nonEmpty.flatMap { $0 == task.title ? nil : $0 }
        let newPlace = try change.place.nonEmpty
            .map { (place) throws(TaskChangeError) in try self.place(place, of: task) }
            .flatMap { $0.avenue == task.avenueID && $0.project == task.projectID ? nil : $0 }
        if newPlace != nil, task.isSubtask { throw .subtaskMoves(code: shown) }
        guard newStatus != nil || newPriority != nil || newDue != nil || newTitle != nil || newPlace != nil else {
            let given = [change.title, change.place, change.status, change.priority, change.due]
            if given.contains(where: { $0.nonEmpty != nil }) { return task }
            throw .nothingToChange(code: shown)
        }
        let id = task.id
        var asked = change
        asked.code = code
        try apply(.task(asked), code: shown) { workspace, context throws(LifrError) in
            var saved = Change()
            if newTitle != nil || newPriority != nil || newDue != nil {
                var edit = newDue ?? Workspace.TaskEdit()
                edit.title = newTitle
                edit.priority = newPriority
                saved.merge(try workspace.edit(id, edit, in: context))
            }
            if let newPlace {
                saved.merge(try workspace.move(id, toAvenue: newPlace.avenue, project: newPlace.project, in: context))
            }
            if let newStatus {
                saved.merge(try workspace.setStatus(newStatus, of: id, cascade: newStatus == .wontDo, in: context))
            }
            return saved
        }
        return workspace.tasks[id] ?? task
    }

    /// The Avenue and Project "KEY" or "KEY/Project name" names, in any
    /// case, or by the Avenue's name. "KEY/" and the `task`'s own title,
    /// which a model writes for a move, is the Avenue.
    private func place(_ text: String, of task: LifrTask) throws(TaskChangeError) -> (avenue: UUID, project: UUID?) {
        let parts = text.split(separator: "/", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
        let wanted = parts.first ?? ""
        let avenues = workspace.avenues.values.filter { $0.archivedAt == nil }
        guard
            let avenue = avenues.first(where: { $0.key.rawValue.caseInsensitiveCompare(wanted) == .orderedSame })
                ?? avenues.first(where: { $0.name.caseInsensitiveCompare(wanted) == .orderedSame })
        else {
            throw .unknownAvenue(
                key: wanted, suggestion: Workspace.closest(to: wanted.uppercased(), in: avenues.map(\.key.rawValue)))
        }
        guard parts.count == 2 else { return (avenue.id, nil) }
        let projects = workspace.projects.values.filter { $0.avenueID == avenue.id && $0.status == .active }
        guard let project = projects.first(where: { $0.name.caseInsensitiveCompare(parts[1]) == .orderedSame }) else {
            if parts[1].caseInsensitiveCompare(task.title) == .orderedSame { return (avenue.id, nil) }
            throw .unknownProject(
                name: parts[1], avenue: avenue.key.rawValue,
                suggestion: Workspace.closest(to: parts[1], in: projects.map(\.name)))
        }
        return (avenue.id, project.id)
    }

    /// Proposes `operation` for the change to `code`, as `TaskChangeError`.
    private func apply(
        _ action: AssistantAction, code: String, _ operation: @escaping AssistantOperation
    ) throws(TaskChangeError) {
        switch apply(action, operation) {
        case .success: return
        case .readOnly: throw .readOnly
        case .refused(let error): throw .refused(code: code, error)
        }
    }

    /// Proposes a comment on the task with `code`.
    package func addComment(code: String, text: String) async -> String {
        let code = code.trimmingCharacters(in: .whitespaces).uppercased()
        do {
            try comment(code: code, text: text)
            return "The comment on \(code) waits for the person to apply it."
        } catch {
            return error.description
        }
    }

    private func comment(code: String, text: String) throws(ActionError) {
        guard let task = try? workspace.task(code: code), task.deletedAt == nil else { throw .noTask(code: code) }
        let id = task.id
        try propose(.comment(code: code, text: text)) { workspace, context throws(LifrError) in
            _ = try workspace.addComment(text, to: id, in: context)
            return Change()
        }
    }

    /// The newest changes, each with a ref for `revert`.
    package func recentActivity() -> String {
        let entries = workspace.journal.suffix(10).reversed()
        guard !entries.isEmpty else { return "Nothing has changed yet." }
        let time = Date.FormatStyle(date: .abbreviated, time: .shortened, timeZone: timeZone)
        return entries.map { entry in
            "- ref \(Self.ref(entry.id)) · \(entry.at.formatted(time)) · \(who(entry)) · \(what(entry))"
        }.joined(separator: "\n")
    }

    /// Proposes putting back what changes did, as a new change: `ref` is
    /// one from `recentActivity`, "last" for the newest change, or
    /// "conversation" for everything done in this conversation.
    package func revert(ref: String) async -> String {
        let ref = ref.trimmingCharacters(in: .whitespaces).lowercased()
        let entries: [JournalEntry]
        switch ref {
        case "last":
            entries = workspace.journal.last.map { [$0] } ?? []
        case "conversation":
            entries = writes.map { workspace.entries(inConversation: $0.conversation) } ?? []
        default:
            entries = workspace.journal.filter { Self.ref($0.id) == ref }
        }
        guard !entries.isEmpty else {
            return ref == "conversation"
                ? "Nothing was changed in this conversation."
                : "There's no change \(ref). Use recentActivity for refs."
        }
        let ids = entries.map(\.id)
        let skipped: Int
        do {
            skipped = try revert(ids)
        } catch {
            return error.description
        }
        var text =
            ids.count == 1
            ? "The revert waits for the person to apply it." : "Reverting \(ids.count) changes waits for the person."
        if skipped > 0 {
            text += " \(skipped) fields were changed again since, so they'd stay as they are now."
        }
        return text
    }

    /// Proposes reverting the entries, and says how many fields would stay
    /// as they are, being changed again since.
    private func revert(_ ids: [UUID]) throws(ActionError) -> Int {
        let known = Set(workspace.journal.map(\.id))
        if let missing = ids.first(where: { !known.contains($0) }) { throw .noChange(ref: Self.ref(missing)) }
        let skipped = LockIsolated(0)
        try propose(.revert(entries: ids)) { workspace, context throws(LifrError) in
            let change = try workspace.revert(ids, in: context)
            skipped.setValue(change.skipped.count)
            return change
        }
        return skipped.value
    }

    /// Makes `action` again, as `Proposal.make` does for the person.
    func run(_ action: AssistantAction) async throws(ActionError) {
        switch action {
        case .task(let change):
            do {
                _ = try await self.change(change)
            } catch {
                throw .task(error)
            }
        case .project(let change):
            do {
                _ = try await self.change(change)
            } catch {
                throw .project(error)
            }
        case .comment(let code, let text):
            try comment(code: code, text: text)
        case .revert(let entries):
            _ = try revert(entries)
        }
    }

    /// Runs `operation` on these tools' records, so later lookups see it,
    /// and keeps it with `action` for the person to apply. Saves nothing.
    func apply(_ action: AssistantAction, _ operation: @escaping AssistantOperation) -> Proposed {
        guard writes != nil else { return .readOnly }
        var records = workspace
        do {
            _ = try operation(&records, Context(actor: .assistant(device: ""), now: now))
        } catch {
            return .refused(error)
        }
        if let avenue = records.readOnlyForAgents(since: workspace) {
            return .refused(.readOnlyForAgents(avenue: avenue.name))
        }
        workspace = records
        proposed.withValue { $0.append((action, operation)) }
        return .success
    }

    /// `apply`, as `ActionError`.
    private func propose(_ action: AssistantAction, _ operation: @escaping AssistantOperation) throws(ActionError) {
        switch apply(action, operation) {
        case .success: return
        case .readOnly: throw .readOnly
        case .refused(let error): throw .refused(error)
        }
    }

    /// What these tools were asked to change, for the person to apply, or
    /// `nil` when nothing.
    package var proposal: Proposal? {
        let actions = proposed.value.map(\.action)
        guard !actions.isEmpty else { return nil }
        return Proposal(actions: actions, steps: workspace.steps(since: initial))
    }

    /// What saves every change these tools were asked for, as one change by
    /// the assistant in the conversation, or `nil` when nothing.
    var operation: AssistantOperation? {
        let operations = proposed.value.map(\.operation)
        guard !operations.isEmpty, let conversation = writes?.conversation else { return nil }
        return { workspace, context throws(LifrError) in
            var context = context
            context.actor = .assistant(device: context.actor.device)
            var change = Change()
            for operation in operations {
                change.merge(try operation(&workspace, context))
            }
            change.actor = context.actor
            change.conversation = conversation
            return change
        }
    }

    /// Forgets what was proposed, for answering again from the start.
    package func discardProposal() {
        proposed.setValue([])
        workspace = initial
    }

    /// Whether a change could be proposed.
    enum Proposed {
        case success
        case readOnly
        case refused(LifrError)
    }

    /// The first six characters of an entry's ID: enough to tell the
    /// newest changes apart, short enough for the model to copy.
    static func ref(_ id: UUID) -> String {
        String(id.uuidString.prefix(6)).lowercased()
    }

    private func who(_ entry: JournalEntry) -> String {
        let actor = entry.actor
        let place = actor.surface.rawValue
        switch actor.kind {
        case .user: return "the person on \(place)"
        case .agent where actor.surface == .assistant: return "you"
        case .agent: return "\(actor.client ?? "an agent") over \(place)"
        case .system: return "Lifr"
        }
    }

    private func what(_ entry: JournalEntry) -> String {
        if !entry.reverts.isEmpty { return "reverted \(entry.reverts.count) change(s)" }
        return entry.steps.prefix(3).map { step in
            let kind = step.record.kind.rawValue
            let verb = step.before == nil ? "added" : step.after == nil ? "removed" : "changed"
            var name = kind
            if case .task(let task)? = step.after ?? step.before {
                name += " \(workspace.displayCode(of: task)) \"\(task.title)\""
            }
            let fields = step.fields.map(\.field).filter { !$0.isInternal }.map { $0.title.lowercased() }
            return "\(verb) \(name)"
                + (verb == "changed" && !fields.isEmpty ? ": " + fields.joined(separator: ", ") : "")
        }.joined(separator: "; ") + (entry.steps.count > 3 ? "; and \(entry.steps.count - 3) more" : "")
    }
}

extension String? {
    /// The text without surrounding spaces, or nil when there's none.
    var nonEmpty: String? {
        guard let text = self?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else { return nil }
        return text
    }
}

/// Its description is written for the person, so alerts show it.
extension TaskChangeError: LocalizedError {
    package var errorDescription: String? { description }
}
