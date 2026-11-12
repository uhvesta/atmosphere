import Foundation
import LifrRemindersClient
import UserNotifications

/// Why scheduling a notification failed.
package enum LiveRemindersError: Error, CustomStringConvertible {
    /// The reminders for `titles` failed; the others were scheduled.
    case scheduleFailed(titles: [String], underlying: any Error)

    package var description: String {
        switch self {
        case .scheduleFailed(let titles, let underlying):
            "Couldn't schedule \(titles.count == 1 ? "the reminder" : "\(titles.count) reminders") for "
                + "\(titles.map { "\"\($0)\"" }.joined(separator: ", ")) on this device: "
                + "\(underlying.localizedDescription). They're saved and ring on your other devices. Check that "
                + "notifications are on for Lifr in Settings, then open Lifr again."
        }
    }
}

extension RemindersClient {
    /// The user info key holding the task ID, so opening a notification can
    /// show its task.
    package static let taskIDKey = "taskID"

    /// Schedules through the system's notification center.
    package static func live(center: UNUserNotificationCenter = .current()) -> Self {
        center.setNotificationCategories(categories)
        return RemindersClient(
            permission: {
                switch await center.notificationSettings().authorizationStatus {
                case .notDetermined: .notDetermined
                case .denied: .denied
                default: .allowed
                }
            },
            requestPermission: {
                try await center.requestAuthorization(options: [.alert, .sound, .badge])
            },
            schedule: { reminders in
                center.removeAllPendingNotificationRequests()
                // One that fails doesn't stop the rest.
                var failed: [String] = []
                var firstError: (any Error)?
                for reminder in reminders {
                    let content = UNMutableNotificationContent()
                    content.title = reminder.title
                    content.body = reminder.body
                    content.sound = .default
                    content.threadIdentifier = reminder.taskID.uuidString
                    content.userInfo = [taskIDKey: reminder.taskID.uuidString]
                    content.categoryIdentifier =
                        reminder.hidesTitle ? ReminderAction.hiddenCategory : ReminderAction.category
                    // With its time zone, so the trigger fires at this
                    // instant even if the device's zone changes first.
                    let components = Calendar.current.dateComponents(
                        [.timeZone, .year, .month, .day, .hour, .minute, .second], from: reminder.fireAt)
                    let request = UNNotificationRequest(
                        identifier: reminder.id.uuidString, content: content,
                        trigger: UNCalendarNotificationTrigger(dateMatching: components, repeats: false))
                    do {
                        try await center.add(request)
                    } catch {
                        failed.append(reminder.title)
                        firstError = firstError ?? error
                    }
                }
                if let firstError { throw LiveRemindersError.scheduleFailed(titles: failed, underlying: firstError) }
            },
            setBadge: { count in try? await center.setBadgeCount(count) }
        )
    }

    /// Done, Snooze and Change Status… for a reminder, and only Snooze for
    /// one that hides its title.
    private static var categories: Set<UNNotificationCategory> {
        let done = UNNotificationAction(
            identifier: ReminderAction.done.rawValue, title: String(localized: "Done"),
            options: [.authenticationRequired], icon: UNNotificationActionIcon(systemImageName: "checkmark.circle"))
        let snooze = UNNotificationAction(
            identifier: ReminderAction.snooze.rawValue, title: String(localized: "Snooze 1 hour"),
            options: [], icon: UNNotificationActionIcon(systemImageName: "clock.arrow.circlepath"))
        let changeStatus = UNNotificationAction(
            identifier: ReminderAction.changeStatus.rawValue, title: String(localized: "Change Status…"),
            options: [.foreground], icon: UNNotificationActionIcon(systemImageName: "ellipsis.circle"))
        return [
            UNNotificationCategory(
                identifier: ReminderAction.category, actions: [done, snooze, changeStatus], intentIdentifiers: []),
            UNNotificationCategory(
                identifier: ReminderAction.hiddenCategory, actions: [snooze], intentIdentifiers: []),
        ]
    }
}

/// Its description is written for the person, so alerts show it.
extension LiveRemindersError: LocalizedError {
    package var errorDescription: String? { description }
}
