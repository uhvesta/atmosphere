import Foundation
import HumataDaemonClient
import HumataProjects
@preconcurrency import UserNotifications

extension NotificationClient {
    /// Posts to Notification Center, and reports the clicks on them.
    static func live() -> NotificationClient {
        let center = UNUserNotificationCenter.current()
        let delegate = NotificationDelegate()
        center.delegate = delegate
        return NotificationClient(
            authorize: {
                (try? await center.requestAuthorization(options: [.alert, .sound, .badge])) ?? false
            },
            post: { notice in
                let content = UNMutableNotificationContent()
                content.title = notice.title
                content.body = notice.body
                content.sound = notice.silent ? nil : .default
                content.threadIdentifier = notice.sessionID
                content.userInfo = [NotificationDelegate.sessionKey: notice.sessionID]
                // One per session: a newer wait replaces the one before.
                let request = UNNotificationRequest(identifier: notice.sessionID, content: content, trigger: nil)
                try? await center.add(request)
            },
            clicks: { delegate.clicks }
        )
    }
}

/// Shows notifications while Humata is in front too, and passes on which
/// session's the person clicked. Kept alive by the client that holds it,
/// since the center holds its delegate weakly.
private final class NotificationDelegate: NSObject, UNUserNotificationCenterDelegate, Sendable {
    static let sessionKey = "session"

    let clicks: AsyncStream<Session.ID>
    private let click: AsyncStream<Session.ID>.Continuation

    override init() {
        (clicks, click) = AsyncStream.makeStream(of: Session.ID.self, bufferingPolicy: .bufferingNewest(1))
        super.init()
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .list, .sound]
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        if let session = response.notification.request.content.userInfo[Self.sessionKey] as? String {
            click.yield(session)
        }
    }
}
