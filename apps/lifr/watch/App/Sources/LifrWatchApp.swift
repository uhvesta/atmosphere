import ComposableArchitecture
import LifrGlance
import LifrWatchFeature
import LifrWatchLink
import OSLog
import SwiftUI
import UserNotifications
import WidgetKit

@main
struct LifrWatchApp: App {
    init() {
        #if os(watchOS)
            WatchSession.shared.activate()
        #endif
        UNUserNotificationCenter.current().setNotificationCategories(WatchReminders.categories)
        UNUserNotificationCenter.current().delegate = Storage.reminders
    }

    var body: some Scene {
        WindowGroup {
            WatchView(store: Storage.store)
        }
    }
}

@MainActor
private enum Storage {
    static let store = Store(initialState: WatchFeature.State()) {
        WatchFeature()
    } withDependencies: {
        $0.watch = .live
    }

    /// Kept for as long as Lifr runs: the notification center holds its
    /// delegate weakly.
    static let reminders = WatchReminders(changeStatus: { store.send(.reminderStatusRequested($0)) })
}

// The strings build compiles this on the Mac, which has no Watch
// connection.
#if os(watchOS)
    extension WatchClient {
        /// Today from the iPhone, kept in the App Group for the complications,
        /// which redraw when it changes; requests go to the iPhone.
        static let live = WatchClient(
            saved: {
                do {
                    return try GlanceFile.shared?.read()
                } catch {
                    log.error("\(String(describing: error), privacy: .public)")
                    return nil
                }
            },
            glances: {
                AsyncStream { continuation in
                    let task = Task {
                        for await glance in WatchSession.shared.glances() {
                            keep(glance)
                            continuation.yield(glance)
                        }
                        continuation.finish()
                    }
                    continuation.onTermination = { _ in task.cancel() }
                }
            },
            send: { request in try WatchSession.shared.send(request) })

        private static func keep(_ glance: TodayGlance) {
            guard let file = GlanceFile.shared else { return }
            do throws(GlanceError) {
                guard try file.read() != glance else { return }
                try file.write(glance)
            } catch {
                log.error("\(String(describing: error), privacy: .public)")
                return
            }
            WidgetCenter.shared.reloadAllTimelines()
        }

        private static let log = Logger(subsystem: "lifr", category: "watch")
    }
#else
    extension WatchClient {
        static let live = WatchClient()
    }
#endif

/// A reminder from the iPhone, shown on the Watch: its buttons do what
/// they do on the iPhone, through the iPhone.
private final class WatchReminders: NSObject, UNUserNotificationCenterDelegate, Sendable {
    /// The iPhone's buttons, by the same identifiers, so the Watch shows
    /// them on the iPhone's reminders.
    static var categories: Set<UNNotificationCategory> {
        let done = UNNotificationAction(identifier: WatchReminder.done, title: String(localized: "Done"), options: [])
        let snooze = UNNotificationAction(
            identifier: WatchReminder.snooze, title: String(localized: "Snooze 1 hour"), options: [])
        let changeStatus = UNNotificationAction(
            identifier: WatchReminder.changeStatus, title: String(localized: "Change Status…"),
            options: [.foreground])
        return [
            UNNotificationCategory(
                identifier: WatchReminder.category, actions: [done, snooze, changeStatus], intentIdentifiers: []),
            UNNotificationCategory(identifier: WatchReminder.hiddenCategory, actions: [snooze], intentIdentifiers: []),
        ]
    }

    private let changeStatus: @MainActor @Sendable (UUID) -> Void

    init(changeStatus: @escaping @MainActor @Sendable (UUID) -> Void) {
        self.changeStatus = changeStatus
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .list, .sound]
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let raw = response.notification.request.content.userInfo[WatchMessage.taskIDKey] as? String,
            let id = UUID(uuidString: raw)
        else { return }
        let request: WatchRequest
        switch response.actionIdentifier {
        case WatchReminder.done:
            request = .setStatus(id, .done)
        case WatchReminder.snooze:
            request = .snooze(id)
        case WatchReminder.changeStatus:
            await changeStatus(id)
            return
        default:
            return
        }
        do {
            try await WatchClient.live.send(request)
        } catch {
            Logger(subsystem: "lifr", category: "watch").error("\(String(describing: error), privacy: .public)")
        }
    }
}
