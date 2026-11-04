import Foundation
import LifrRemindersClient
import UserNotifications

/// Why scheduling a notification failed.
package enum LiveRemindersError: Error, CustomStringConvertible {
    case scheduleFailed(title: String, underlying: any Error)

    package var description: String {
        switch self {
        case .scheduleFailed(let title, let underlying):
            "Couldn't schedule the reminder for \"\(title)\": \(underlying.localizedDescription). "
                + "Check that notifications are on for Lifr in System Settings, then open Lifr again."
        }
    }
}

extension RemindersClient {
    /// The user info key holding the task ID, so opening a notification can
    /// show its task.
    package static let taskIDKey = "taskID"

    /// Schedules through the system's notification center.
    package static func live(center: UNUserNotificationCenter = .current()) -> Self {
        RemindersClient(
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
                for reminder in reminders {
                    let content = UNMutableNotificationContent()
                    content.title = reminder.title
                    content.body = reminder.body
                    content.sound = .default
                    content.threadIdentifier = reminder.taskID.uuidString
                    content.userInfo = [taskIDKey: reminder.taskID.uuidString]
                    let components = Calendar.current.dateComponents(
                        [.year, .month, .day, .hour, .minute, .second], from: reminder.fireAt)
                    let request = UNNotificationRequest(
                        identifier: reminder.id.uuidString, content: content,
                        trigger: UNCalendarNotificationTrigger(dateMatching: components, repeats: false))
                    do {
                        try await center.add(request)
                    } catch {
                        throw LiveRemindersError.scheduleFailed(title: reminder.title, underlying: error)
                    }
                }
            }
        )
    }
}
