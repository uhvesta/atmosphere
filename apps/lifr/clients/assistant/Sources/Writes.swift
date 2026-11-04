import ComposableArchitecture
import Foundation
import LifrCore

/// A change the assistant makes, run the way the stores run every save.
package typealias AssistantOperation = @Sendable (inout Workspace, Context) throws(LifrError) -> Change

/// How the assistant saves what it changes, and the conversation the
/// changes belong to, so they can be found and reverted together.
package struct AssistantWrites: Sendable, Hashable {
    package var conversation: UUID
    /// Saves an operation and returns the records after it.
    package var save: @Sendable (_ operation: @escaping AssistantOperation) async throws -> Workspace

    package init(
        conversation: UUID,
        save: @escaping @Sendable (_ operation: @escaping AssistantOperation) async throws -> Workspace
    ) {
        self.conversation = conversation
        self.save = save
    }

    package static func == (lhs: Self, rhs: Self) -> Bool { lhs.conversation == rhs.conversation }
    package func hash(into hasher: inout Hasher) { hasher.combine(conversation) }
}

extension AssistantTools {
    /// The statuses the assistant can set. Procrastinated isn't one: Lifr
    /// works it out from activity.
    private static let settable: [String: TaskStatus] = [
        "todo": .todo, "in_progress": .inProgress, "done": .done, "wont_do": .wontDo,
    ]

    /// Sets the same fields on each task in `codes`. A title is only for one
    /// task. `due` is a date, or "none" to clear it. Won't Do also closes
    /// open Sub tasks. Nothing changes if any task or value is wrong.
    package func updateTasks(
        codes: [String], status: String?, priority: String?, due: String?, title: String?
    ) async -> String {
        let codes = codes.map { $0.trimmingCharacters(in: .whitespaces).uppercased() }.filter { !$0.isEmpty }
        guard !codes.isEmpty else { return "Name at least one task code. Use findTasks to look codes up." }
        var tasks: [LifrTask] = []
        for code in codes {
            guard let task = try? workspace.task(code: code), task.deletedAt == nil else {
                return "There's no task \(code). Use findTasks to look up its code. Nothing changed."
            }
            tasks.append(task)
        }
        let newStatus: TaskStatus?
        if let status = status.nonEmpty {
            guard let match = Self.settable[status] else {
                return "You can't set the status \(status). Use one of: "
                    + Self.settable.keys.sorted().joined(separator: ", ") + ". Nothing changed."
            }
            newStatus = match
        } else {
            newStatus = nil
        }
        let newPriority: Priority?
        if let priority = priority.nonEmpty {
            guard let match = Priority(rawValue: priority) else {
                return "There's no priority \(priority). Use none, low, medium, high or urgent. Nothing changed."
            }
            newPriority = match
        } else {
            newPriority = nil
        }
        let newDue: CalendarDate??
        if let due = due.nonEmpty {
            if due == "none" {
                newDue = .some(nil)
            } else if let date = CalendarDate.resolving(due, today: today) {
                newDue = date
            } else {
                return "\(due) isn't a date. Use YYYY-MM-DD, today, tomorrow, a weekday, +3d, +2w, +1m or none. "
                    + "Nothing changed."
            }
        } else {
            newDue = nil
        }
        let newTitle = title.nonEmpty
        if newTitle != nil, tasks.count > 1 { return "A new title is for one task at a time. Nothing changed." }
        guard newStatus != nil || newPriority != nil || newDue != nil || newTitle != nil else {
            return "Say what to change: status, priority, due or title."
        }
        let ids = tasks.map(\.id)
        let saved = await apply { workspace, context throws(LifrError) in
            var change = Change()
            for id in ids {
                if newTitle != nil || newPriority != nil || newDue != nil {
                    let edit = Workspace.TaskEdit(title: newTitle, priority: newPriority, dueDate: newDue)
                    change.merge(try workspace.edit(id, edit, in: context))
                }
                if let newStatus {
                    change.merge(try workspace.setStatus(newStatus, of: id, cascade: newStatus == .wontDo, in: context))
                }
            }
            return change
        }
        guard case .success = saved else { return saved.message }
        let updated = ids.compactMap { workspace.tasks[$0] }
        return "Saved. The person can undo it. Now:\n" + lines(updated)
    }

    /// Adds a comment to the task with `code`.
    package func addComment(code: String, text: String) async -> String {
        let code = code.trimmingCharacters(in: .whitespaces).uppercased()
        guard let task = try? workspace.task(code: code), task.deletedAt == nil else {
            return "There's no task \(code). Use findTasks to look up its code. Nothing changed."
        }
        let id = task.id
        let saved = await apply { workspace, context throws(LifrError) in
            _ = try workspace.addComment(text, to: id, in: context)
            return Change()
        }
        guard case .success = saved else { return saved.message }
        return "Saved the comment on \(code). The person can undo it."
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

    /// Puts back what changes did, as a new change: `ref` is one from
    /// `recentActivity`, "last" for the newest change, or "conversation"
    /// for everything done in this conversation.
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
        let skipped = LockIsolated(0)
        let saved = await apply { workspace, context throws(LifrError) in
            let change = try workspace.revert(ids, in: context)
            skipped.setValue(change.skipped.count)
            return change
        }
        guard case .success = saved else { return saved.message }
        var text = ids.count == 1 ? "Reverted the change." : "Reverted \(ids.count) changes."
        if skipped.value > 0 {
            text += " \(skipped.value) fields were changed again since, so they stay as they are now."
        }
        return text + " The revert can be reverted too."
    }

    /// Saves `operation` as the assistant, in this conversation.
    private func apply(_ operation: @escaping AssistantOperation) async -> SaveResult {
        guard let writes else {
            return .failure("You can't change anything here. Suggest the change and the person can make it.")
        }
        let conversation = writes.conversation
        do {
            workspace = try await writes.save { workspace, context throws(LifrError) in
                var context = context
                context.actor = .assistant(device: context.actor.device)
                var change = try operation(&workspace, context)
                change.actor = context.actor
                change.conversation = conversation
                return change
            }
            return .success
        } catch let error as LifrError {
            return .failure("\(error) Nothing changed.")
        } catch {
            return .failure("Saving failed: \(error.localizedDescription) Nothing changed.")
        }
    }

    private enum SaveResult {
        case success
        case failure(String)

        var message: String {
            if case .failure(let message) = self { message } else { "" }
        }
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
            let fields = step.fields.map(\.name).filter { !["id", "createdAt", "sortOrder"].contains($0) }
            return "\(verb) \(name)"
                + (verb == "changed" && !fields.isEmpty ? ": " + fields.joined(separator: ", ") : "")
        }.joined(separator: "; ") + (entry.steps.count > 3 ? "; and \(entry.steps.count - 3) more" : "")
    }
}

extension String? {
    /// The text without surrounding spaces, or nil when there's none.
    fileprivate var nonEmpty: String? {
        guard let text = self?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else { return nil }
        return text
    }
}
