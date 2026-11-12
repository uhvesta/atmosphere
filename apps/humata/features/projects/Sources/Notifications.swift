import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

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
            case .question: entry.request?.text ?? "Asked you something."
            case .move: "Finished; asks you to move it to \(entry.request?.kind.statusName ?? "another column")."
            }
        self.init(
            sessionID: entry.session.id, title: entry.session.title, body: place.isEmpty ? what : "\(place): \(what)")
    }
}

extension AgentRequest.Kind {
    /// The column a move asks for, as the board names it; nil for a question.
    var statusName: String? {
        guard case .move(_, let status) = self else { return nil }
        return status.title
    }
}

extension ItemStatus {
    /// The board column's name.
    package var title: String {
        switch self {
        case .todo: String(localized: "To Do")
        case .inProgress: String(localized: "In Progress")
        case .waitingOnYou: String(localized: "Waiting on You")
        case .inReview: String(localized: "In Review")
        case .done: String(localized: "Done")
        case .wontDo: String(localized: "Won't Do")
        }
    }
}
