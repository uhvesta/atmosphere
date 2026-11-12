import Foundation
import LifrAPI
import LifrCore
import LifrStoreClient
import LifrTaskList
import LifrToday
import LifrUI

/// What Shortcuts and Siri can do in Lifr: add a task, say what's on
/// Today, finish a task, change its status or due date, comment on it,
/// find tasks and read a task's fields. With the lock on, nothing that names a task leaves the app; Today
/// gives counts only.
package struct Shortcuts: Sendable {
    package var store: StoreClient
    /// Whether the person turned on Lifr's lock.
    package var locksApp: @Sendable () -> Bool
    package var now: @Sendable () -> Date
    package var timeZone: TimeZone
    /// The reminder a new task due at a time, or on a day with no time,
    /// gets, as Quick Add gives it; nil for none.
    package var defaultReminder: @Sendable (TimeOfDay?) -> Reminder.Trigger?
    /// Whether "11/5" in a title is Nov 5, as in the US, or May 11.
    package var monthFirst: Bool

    package init(
        store: StoreClient, locksApp: @escaping @Sendable () -> Bool, now: @escaping @Sendable () -> Date,
        timeZone: TimeZone, defaultReminder: @escaping @Sendable (TimeOfDay?) -> Reminder.Trigger? = { _ in nil },
        locale: Locale = .current
    ) {
        self.store = store
        self.locksApp = locksApp
        self.now = now
        self.timeZone = timeZone
        self.defaultReminder = defaultReminder
        let order = DateFormatter.dateFormat(fromTemplate: "Md", options: 0, locale: locale) ?? "M/d"
        let month = order.firstIndex(of: "M") ?? order.startIndex
        monthFirst = month <= (order.firstIndex(of: "d") ?? order.endIndex)
    }

    /// When a new task is due.
    package enum Due: Sendable {
        case today, tomorrow, none
    }

    /// A task, Avenue, Project or label as Shortcuts lists it.
    package struct Item: Hashable, Identifiable, Sendable {
        package var id: UUID
        package var name: String
        /// The task's code, such as HEALTH-0004, or a Project's Avenue;
        /// empty for an Avenue or a label.
        package var detail: String
    }

    /// Today in numbers, and its tasks by name unless the lock is on.
    package struct Today: Hashable, Sendable {
        package var count: Int
        package var overdue: Int
        /// Empty while the lock is on.
        package var titles: [String]
        package var isLocked: Bool
    }

    /// Adds a task called `title`, in `avenue` or the Inbox.
    package func add(_ title: String, avenue: UUID?, due: Due) async throws -> Item {
        try await reported { try await adding(title, avenue: avenue, due: due) }
    }

    private func adding(_ title: String, avenue: UUID?, due: Due) async throws -> Item {
        let title = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty else { throw ShortcutError.noTitle }
        let today = CalendarDate(now(), in: timeZone)
        // Read like Quick Add: "Call dentist tomorrow 3pm !high #health".
        let entry = QuickEntry(title, today: today, in: try await store.load(), monthFirst: monthFirst)
        let avenue = entry.avenueID ?? avenue
        if avenue == nil { try await store.ensureInbox() }
        var new = Workspace.NewTask(
            title: entry.title.isEmpty ? title : entry.title, avenueID: avenue ?? Avenue.inboxID)
        new.projectID = entry.projectID
        new.priority = entry.priority ?? new.priority
        new.labelIDs = entry.labelIDs
        new.dueTime = entry.dueTime
        new.dueDate =
            entry.dueDate
            ?? {
                switch due {
                case .today: today
                case .tomorrow: today.adding(days: 1)
                case .none: nil
                }
            }()
        if new.dueDate != nil, let trigger = defaultReminder(new.dueTime) {
            new.reminders = [trigger]
        }
        let saved = try await store.createTask(new)
        guard
            let task = saved.change.previous.first(where: { $0.value == nil }).flatMap({ saved.workspace.tasks[$0.key] }
            )
        else {
            throw ShortcutError.notSaved
        }
        return Item(id: task.id, name: task.title, detail: saved.workspace.displayCode(of: task))
    }

    /// What Today shows now.
    package func today() async throws -> Today {
        let workspace = try await store.load()
        let rows = TaskQuery.today.sections(workspace, now(), timeZone).flatMap(\.rows)
        let overdue = rows.filter {
            if case .overdue = $0.due { true } else { false }
        }.count
        let locked = locksApp()
        return Today(count: rows.count, overdue: overdue, titles: locked ? [] : rows.map(\.title), isLocked: locked)
    }

    /// Open tasks whose title or code has `text`, or Today's when `text` is
    /// nil. None while the lock is on.
    package func openTasks(matching text: String?) async throws -> [Item] {
        guard !locksApp() else { return [] }
        let workspace = try await store.load()
        guard let text = text?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else {
            let today = TaskQuery.today.sections(workspace, now(), timeZone).flatMap(\.rows).map(\.id)
            return today.compactMap { workspace.tasks[$0] }.map { item($0, in: workspace) }
        }
        return workspace.tasks.values
            .filter { $0.isOpen && $0.deletedAt == nil }
            .filter {
                $0.title.localizedStandardContains(text)
                    || workspace.displayCode(of: $0).localizedStandardContains(text)
            }
            .sorted {
                $0.dueDate == $1.dueDate ? $0.title < $1.title : earlier($0.dueDate, than: $1.dueDate)
            }
            .map { item($0, in: workspace) }
    }

    /// The tasks with `ids` that still exist. None while the lock is on.
    package func tasks(_ ids: [UUID]) async throws -> [Item] {
        guard !locksApp() else { return [] }
        let workspace = try await store.load()
        return ids.compactMap { workspace.tasks[$0] }.filter { $0.deletedAt == nil }.map { item($0, in: workspace) }
    }

    /// A prompt to plan `goal` with another chat assistant: it names the
    /// Avenues, Projects and labels but no task. Refused while the lock is
    /// on, since it would leave the app.
    package func planPrompt(goal: String) async throws -> String {
        guard !locksApp() else { throw ShortcutError.locked }
        let workspace = try await store.load()
        let now = now()
        return PlanPrompt.make(
            goal: goal, workspace: workspace, today: CalendarDate(now, in: timeZone), timeZone: timeZone)
    }

    /// What Find Tasks looks for. Each that's set must match; with no
    /// status, open tasks match.
    package struct Filter: Hashable, Sendable {
        package var status: TaskStatus?
        package var avenue: UUID?
        package var project: UUID?
        package var label: UUID?
        /// Due on or before the day this falls on here.
        package var dueBy: Date?

        package init(
            status: TaskStatus? = nil, avenue: UUID? = nil, project: UUID? = nil, label: UUID? = nil,
            dueBy: Date? = nil
        ) {
            self.status = status
            self.avenue = avenue
            self.project = project
            self.label = label
            self.dueBy = dueBy
        }
    }

    /// The tasks `filter` matches, dated ones first by date, in no
    /// archived Avenue and not in the Trash. Refused while the lock is on.
    package func find(_ filter: Filter) async throws -> [Item] {
        guard !locksApp() else { throw ShortcutError.locked }
        let workspace = try await store.load()
        let dueBy = filter.dueBy.map { CalendarDate($0, in: timeZone) }
        return workspace.tasks.values
            .filter { task in
                task.deletedAt == nil && workspace.avenues[task.avenueID]?.archivedAt == nil
                    && (filter.status.map { task.status == $0 } ?? task.isOpen)
                    && (filter.avenue.map { task.avenueID == $0 } ?? true)
                    && (filter.project.map { task.projectID == $0 } ?? true)
                    && (filter.label.map { task.labelIDs.contains($0) } ?? true)
                    && (dueBy.map { day in task.dueDate.map { $0 <= day } ?? false } ?? true)
            }
            .sorted {
                $0.dueDate == $1.dueDate ? $0.title < $1.title : earlier($0.dueDate, than: $1.dueDate)
            }
            .map { item($0, in: workspace) }
    }

    /// A task's fields, as Get Task Details gives them.
    package struct Details: Hashable, Sendable {
        package var title: String
        package var code: String
        package var status: TaskStatus
        package var due: CalendarDate?
        package var avenue: String
        package var project: String?
        package var labels: [String]
        package var details: String

        /// One field to a line, then the task's description.
        package var text: String {
            var lines = ["\(title) (\(code))", String(localized: "Status: \(status.title)")]
            if let due { lines.append(String(localized: "Due: \(due.description)")) }
            lines.append(String(localized: "Avenue: \(avenue)"))
            if let project { lines.append(String(localized: "Project: \(project)")) }
            if !labels.isEmpty { lines.append(String(localized: "Labels: \(labels.joined(separator: ", "))")) }
            if !details.isEmpty { lines += ["", details] }
            return lines.joined(separator: "\n")
        }
    }

    /// The task's fields. Refused while the lock is on.
    package func details(of id: UUID) async throws -> Details {
        guard !locksApp() else { throw ShortcutError.locked }
        let workspace = try await store.load()
        guard let task = workspace.tasks[id], task.deletedAt == nil else { throw ShortcutError.noTask }
        return Details(
            title: task.title, code: workspace.displayCode(of: task), status: task.status, due: task.dueDate,
            avenue: workspace.avenues[task.avenueID]?.name ?? "",
            project: task.projectID.flatMap { workspace.projects[$0]?.name },
            labels: task.labelIDs.compactMap { workspace.labels[$0]?.name }.sorted(), details: task.details)
    }

    /// The active Projects, each with its Avenue's name, in the sidebar's
    /// order.
    package func projects() async throws -> [Item] {
        let workspace = try await store.load()
        return workspace.projects.values
            .filter { $0.status == .active }
            .sorted { $0.sortOrder < $1.sortOrder }
            .map { Item(id: $0.id, name: $0.name, detail: workspace.avenues[$0.avenueID]?.name ?? "") }
    }

    /// Every label, by name.
    package func labels() async throws -> [Item] {
        let workspace = try await store.load()
        return workspace.labels.values.sorted { $0.name < $1.name }.map { Item(id: $0.id, name: $0.name, detail: "") }
    }

    /// Every Avenue, in the sidebar's order.
    package func avenues() async throws -> [Item] {
        let workspace = try await store.load()
        return workspace.avenues.values.sorted { $0.sortOrder < $1.sortOrder }.map {
            Item(id: $0.id, name: $0.name, detail: "")
        }
    }

    /// Marks the task done.
    package func finish(_ id: UUID) async throws -> Item {
        try await setStatus(.done, of: id)
    }

    /// Gives the task `status`.
    package func setStatus(_ status: TaskStatus, of id: UUID) async throws -> Item {
        try await change(id) { try await store.setStatus(status, id) }
    }

    /// Makes the task due on the day `date` falls on here, or on no day.
    package func reschedule(_ id: UUID, to date: Date?) async throws -> Item {
        let due = date.map { CalendarDate($0, in: timeZone) }
        return try await change(id) { try await store.edit(id, Workspace.TaskEdit(dueDate: .some(due))) }
    }

    /// Adds `text` to the task's comments.
    package func comment(_ text: String, on id: UUID) async throws -> Item {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { throw ShortcutError.noComment }
        return try await change(id) { try await store.addComment(text, to: id) }
    }

    /// Saves a change to a task that exists and isn't in the Trash, unless
    /// the lock is on.
    private func change(_ id: UUID, _ save: () async throws -> Saved) async throws -> Item {
        guard !locksApp() else { throw ShortcutError.locked }
        guard let task = try await store.load().tasks[id], task.deletedAt == nil else { throw ShortcutError.noTask }
        let saved: Saved
        do {
            saved = try await reported(save)
        } catch LifrError.notFound {
            throw ShortcutError.noTask
        }
        return item(saved.workspace.tasks[id] ?? task, in: saved.workspace)
    }

    /// Rings the task's reminder again at `date`, as a new reminder; the due
    /// date stays. Allowed while locked, since it shows nothing.
    package func snooze(_ id: UUID, until date: Date) async throws {
        do {
            _ = try await reported { try await store.addReminder(.at(date), to: id) }
        } catch LifrError.notFound {
            throw ShortcutError.noTask
        }
    }

    /// What `work` gives, with any error Siri can't say turned into one it
    /// can.
    private func reported<Value>(_ work: () async throws -> Value) async throws -> Value {
        do {
            return try await work()
        } catch let error as ShortcutError {
            throw error
        } catch let error as LifrError {
            throw error
        } catch {
            throw ShortcutError.storeFailed(String(describing: error))
        }
    }

    private func item(_ task: LifrTask, in workspace: Workspace) -> Item {
        Item(id: task.id, name: task.title, detail: workspace.displayCode(of: task))
    }
}

/// Dated before undated, then by date.
private func earlier(_ lhs: CalendarDate?, than rhs: CalendarDate?) -> Bool {
    switch (lhs, rhs) {
    case (.some(let lhs), .some(let rhs)): lhs < rhs
    case (.some, .none): true
    case (.none, _): false
    }
}

/// Why a shortcut couldn't do what it was asked.
package enum ShortcutError: Error, Hashable, Sendable, CustomStringConvertible {
    /// The lock is on, and the shortcut would name or change a task.
    case locked
    /// The comment was empty.
    case noComment
    /// The task was deleted, or never existed on this device.
    case noTask
    /// The new task's title was empty.
    case noTitle
    /// The store saved, but the new task wasn't in what it returned.
    case notSaved
    /// The records couldn't be read or saved, for this reason.
    case storeFailed(String)

    package var description: String {
        switch self {
        case .locked:
            String(localized: "Lifr is locked. Open Lifr and unlock it, then run the shortcut again.")
        case .noComment:
            String(localized: "The comment is empty. Say or type what to add.")
        case .noTask:
            String(localized: "That task isn't in Lifr any more. Pick another.")
        case .noTitle:
            String(localized: "The task needs a title. Say or type what to add.")
        case .notSaved:
            String(
                localized: "Lifr couldn't confirm the task was added. Open Lifr to check Today before adding it again.")
        case .storeFailed(let reason):
            String(localized: "Lifr couldn't save the change: \(reason) Open Lifr to check, then try again.")
        }
    }
}

/// Its description is written for the person, so alerts show it.
extension ShortcutError: LocalizedError {
    package var errorDescription: String? { description }
}

/// Shortcuts and Siri say the description.
extension ShortcutError: CustomLocalizedStringResourceConvertible {
    package var localizedStringResource: LocalizedStringResource { LocalizedStringResource(stringLiteral: description) }
}
