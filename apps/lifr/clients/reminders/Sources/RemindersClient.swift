import ComposableArchitecture
import Foundation
import LifrCore

/// A notification to show at a time: one of a task's reminders, worked out
/// for this device's time zone.
package struct ScheduledReminder: Identifiable, Hashable, Sendable {
    /// The reminder's ID, so rescheduling replaces it.
    package var id: UUID
    package var taskID: UUID
    package var title: String
    package var body: String
    package var fireAt: Date

    package init(id: UUID, taskID: UUID, title: String, body: String, fireAt: Date) {
        self.id = id
        self.taskID = taskID
        self.title = title
        self.body = body
        self.fireAt = fireAt
    }
}

extension Workspace {
    /// The most notifications a device keeps scheduled. iOS drops any past 64.
    package static let reminderLimit = 64

    /// What this device should have scheduled: the reminders of open tasks
    /// that aren't in the trash or an archived Avenue and that fire after
    /// `now`, soonest first, at most `reminderLimit`. With `hidingTitles`,
    /// as when the app lock is on, each says only that a task is due, so
    /// the lock screen doesn't show what.
    package func scheduledReminders(
        now: Date, timeZone: TimeZone, hidingTitles: Bool = false
    ) -> [ScheduledReminder] {
        let scheduled = reminders.values.compactMap { reminder -> ScheduledReminder? in
            guard let task = tasks[reminder.taskID], task.deletedAt == nil, task.isOpen,
                avenues[task.avenueID]?.archivedAt == nil,
                let fireAt = reminder.fireDate(for: task, in: timeZone), fireAt > now
            else { return nil }
            guard !hidingTitles else {
                return ScheduledReminder(
                    id: reminder.id, taskID: task.id, title: String(localized: "Reminder"),
                    body: String(localized: "Open Lifr to see the task."), fireAt: fireAt)
            }
            return ScheduledReminder(
                id: reminder.id, taskID: task.id, title: task.title,
                body: dueText(task) ?? displayCode(of: task), fireAt: fireAt)
        }
        return Array(
            scheduled.sorted { ($0.fireAt, $0.id.uuidString) < ($1.fireAt, $1.id.uuidString) }
                .prefix(Self.reminderLimit))
    }

    private func dueText(_ task: LifrTask) -> String? {
        guard let due = task.dueDate else { return nil }
        let time = task.dueTime.map { " at \($0)" } ?? ""
        return "\(displayCode(of: task)) · due \(due)\(time)"
    }
}

/// Whether the person lets Lifr show notifications.
package enum ReminderPermission: Hashable, Sendable {
    /// Not asked yet. Asking shows the system prompt.
    case notDetermined
    case allowed
    /// Turned off. Only Settings can turn it back on.
    case denied
}

/// Schedules this device's notifications for reminders. Each device works
/// out its own from the synced records, so a reminder rings once per
/// device, at the right local time.
@DependencyClient
package struct RemindersClient: Sendable {
    package var permission: @Sendable () async -> ReminderPermission = { .notDetermined }
    /// Shows the system prompt if it hasn't been shown. Returns whether
    /// notifications are allowed.
    package var requestPermission: @Sendable () async throws -> Bool
    /// Replaces every scheduled notification with `reminders`.
    package var schedule: @Sendable (_ reminders: [ScheduledReminder]) async throws -> Void
}

extension RemindersClient: TestDependencyKey {
    package static let testValue = RemindersClient()
    package static let previewValue = RemindersClient.inMemory().client
}

extension RemindersClient {
    /// Keeps what's scheduled in memory, for tests and previews.
    package static func inMemory(permission: ReminderPermission = .allowed) -> InMemoryReminders {
        InMemoryReminders(permission: permission)
    }
}

/// Reminders kept in memory. `requestPermission` allows them unless
/// `permission` is `.denied`.
package final class InMemoryReminders: Sendable {
    private let state: LockIsolated<(permission: ReminderPermission, scheduled: [ScheduledReminder])>

    init(permission: ReminderPermission) {
        state = LockIsolated((permission, []))
    }

    /// What's scheduled now.
    package var scheduled: [ScheduledReminder] { state.value.scheduled }
    package var permission: ReminderPermission { state.value.permission }

    package var client: RemindersClient {
        RemindersClient(
            permission: { [state] in state.value.permission },
            requestPermission: { [state] in
                state.withValue { value in
                    if value.permission == .notDetermined { value.permission = .allowed }
                    return value.permission == .allowed
                }
            },
            schedule: { [state] reminders in state.withValue { $0.scheduled = reminders } }
        )
    }
}

extension DependencyValues {
    package var reminders: RemindersClient {
        get { self[RemindersClient.self] }
        set { self[RemindersClient.self] = newValue }
    }
}
