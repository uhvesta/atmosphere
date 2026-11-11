import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskList
import LifrToday
import LifrUI

/// What Shortcuts and Siri can do in Lifr: add a task, say what's on
/// Today, finish a task. With the lock on, nothing that names a task leaves
/// the app; Today gives counts only.
package struct Shortcuts: Sendable {
    package var store: StoreClient
    /// Whether the person turned on Lifr's lock.
    package var locksApp: @Sendable () -> Bool
    package var now: @Sendable () -> Date
    package var timeZone: TimeZone

    package init(
        store: StoreClient, locksApp: @escaping @Sendable () -> Bool, now: @escaping @Sendable () -> Date,
        timeZone: TimeZone
    ) {
        self.store = store
        self.locksApp = locksApp
        self.now = now
        self.timeZone = timeZone
    }

    /// When a new task is due.
    package enum Due: Sendable {
        case today, tomorrow, none
    }

    /// A task or an Avenue as Shortcuts lists it.
    package struct Item: Hashable, Identifiable, Sendable {
        package var id: UUID
        package var name: String
        /// The task's code, such as HEALTH-0004; empty for an Avenue.
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
        let title = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty else { throw ShortcutError.noTitle }
        if avenue == nil { try await store.ensureInbox() }
        let avenue = avenue ?? Avenue.inboxID
        var new = Workspace.NewTask(title: title, avenueID: avenue)
        let now = now()
        new.dueDate =
            switch due {
            case .today: CalendarDate(now, in: timeZone)
            case .tomorrow: CalendarDate(now, in: timeZone).adding(days: 1)
            case .none: nil
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

    /// Every Avenue, in the sidebar's order.
    package func avenues() async throws -> [Item] {
        let workspace = try await store.load()
        return workspace.avenues.values.sorted { $0.sortOrder < $1.sortOrder }.map {
            Item(id: $0.id, name: $0.name, detail: "")
        }
    }

    /// Marks the task done.
    package func finish(_ id: UUID) async throws -> Item {
        guard !locksApp() else { throw ShortcutError.locked }
        let saved = try await store.setStatus(.done, id)
        guard let task = saved.workspace.tasks[id] else { throw ShortcutError.noTask }
        return item(task, in: saved.workspace)
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
    /// The task was deleted, or never existed on this device.
    case noTask
    /// The new task's title was empty.
    case noTitle
    /// The store saved, but the new task wasn't in what it returned.
    case notSaved

    package var description: String {
        switch self {
        case .locked:
            String(localized: "Lifr is locked. Open Lifr and unlock it to finish tasks.")
        case .noTask:
            String(localized: "That task isn't in Lifr any more. Pick another.")
        case .noTitle:
            String(localized: "The task needs a title. Say or type what to add.")
        case .notSaved:
            String(
                localized: "Lifr couldn't confirm the task was added. Open Lifr to check Today before adding it again.")
        }
    }
}
