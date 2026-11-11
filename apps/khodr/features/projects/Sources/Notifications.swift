import ComposableArchitecture
import KhodrDaemonClient

/// Notifications for sessions that come to wait on the person.
@DependencyClient
package struct NotificationClient: Sendable {
    /// Asks to show notifications; false if they're turned off.
    package var authorize: @Sendable () async -> Bool = { false }
    package var post: @Sendable (_ notice: Notice) async -> Void
    /// The session of each notification the person clicks, as they do.
    package var clicks: @Sendable () async -> AsyncStream<Session.ID> = { AsyncStream { $0.finish() } }
}

extension NotificationClient: TestDependencyKey {
    package static let testValue = NotificationClient()
    package static let previewValue = NotificationClient(
        authorize: { false }, post: { _ in }, clicks: { AsyncStream { $0.finish() } })
}

extension DependencyValues {
    package var notifications: NotificationClient {
        get { self[NotificationClient.self] }
        set { self[NotificationClient.self] = newValue }
    }
}

/// A session that waits on the person, as a notification says it.
package struct Notice: Hashable, Sendable {
    package var sessionID: Session.ID
    package var title: String
    package var body: String

    package init(sessionID: Session.ID, title: String, body: String) {
        self.sessionID = sessionID
        self.title = title
        self.body = body
    }

    init(_ entry: InboxEntry) {
        let place = [entry.projectName, entry.itemTitle].compactMap(\.self).joined(separator: " › ")
        let what =
            switch entry.reason {
            case .approval: "Waiting for you to answer a request."
            case .reply: "Finished its turn."
            case .failed: "Its turn failed."
            }
        self.init(
            sessionID: entry.session.id, title: entry.session.title, body: place.isEmpty ? what : "\(place): \(what)")
    }
}
