import Foundation
import LifrGlance

/// A task's status as the Watch offers it, by the same names the records
/// keep, so the iPhone reads it straight into its own.
package enum WatchStatus: String, Codable, CaseIterable, Hashable, Sendable, Identifiable {
    case todo, inProgress, procrastinated, done, wontDo

    package var id: Self { self }

    package var title: String {
        switch self {
        case .todo: String(localized: "To Do")
        case .inProgress: String(localized: "In Progress")
        case .procrastinated: String(localized: "Procrastinated")
        case .done: String(localized: "Done")
        case .wontDo: String(localized: "Won't Do")
        }
    }

    /// An SF Symbol name.
    package var symbol: String {
        switch self {
        case .todo: "circle"
        case .inProgress: "circle.lefthalf.filled"
        case .procrastinated: "tortoise"
        case .done: "checkmark.circle.fill"
        case .wontDo: "xmark.circle"
        }
    }
}

/// What the Watch asks the iPhone to save. The Watch never opens the
/// records: the iPhone saves each request and sends Today back.
package enum WatchRequest: Codable, Hashable, Sendable {
    /// Gives a task a status.
    case setStatus(UUID, WatchStatus)
    /// Adds a task to the Inbox, read like Quick Add.
    case add(String)
    /// Puts a task's reminder off for an hour, as the iPhone's Snooze does.
    case snooze(UUID)
}

/// Why something sent between the iPhone and the Watch couldn't be read.
package enum WatchLinkError: Error, CustomStringConvertible {
    /// The iPhone's Today couldn't be read, such as one from a newer Lifr.
    case unreadableGlance(any Error)
    /// The Watch's request couldn't be read, such as one from a newer Lifr.
    case unreadableRequest(any Error)

    package var description: String {
        switch self {
        case .unreadableGlance(let error):
            String(
                localized: """
                    The Watch couldn't read Today from the iPhone (\(error.localizedDescription)). Update Lifr on \
                    both, then open it on the iPhone to send Today again.
                    """)
        case .unreadableRequest(let error):
            String(
                localized: """
                    The iPhone couldn't read a change made on the Watch (\(error.localizedDescription)), so it \
                    wasn't saved. Update Lifr on both, then make the change again.
                    """)
        }
    }
}

/// How Today and requests travel between the iPhone and the Watch: as
/// JSON under one key each, which the system's connection carries.
package enum WatchMessage {
    package static let glanceKey = "lifrToday"
    package static let requestKey = "lifrRequest"
    /// The notification user info key holding a reminder's task, the same
    /// one the iPhone's reminders write.
    package static let taskIDKey = "taskID"

    package static func encode(_ glance: TodayGlance) throws -> Data {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        return try encoder.encode(glance)
    }

    /// Today from what the iPhone sent, or nil when it sent none yet.
    package static func glance(in message: [String: Any]) throws(WatchLinkError) -> TodayGlance? {
        guard let data = message[glanceKey] as? Data else { return nil }
        do {
            let decoder = JSONDecoder()
            decoder.dateDecodingStrategy = .iso8601
            return try decoder.decode(TodayGlance.self, from: data)
        } catch {
            throw .unreadableGlance(error)
        }
    }

    package static func encode(_ request: WatchRequest) throws -> Data {
        try JSONEncoder().encode(request)
    }

    /// The request in what the Watch sent, or nil when it holds none.
    package static func request(in message: [String: Any]) throws(WatchLinkError) -> WatchRequest? {
        guard let data = message[requestKey] as? Data else { return nil }
        do {
            return try JSONDecoder().decode(WatchRequest.self, from: data)
        } catch {
            throw .unreadableRequest(error)
        }
    }
}

extension TodayGlance {
    /// What the Watch shows: today and tomorrow, so the Watch turns over
    /// at midnight on its own, with at most `limit` tasks each and without
    /// the Avenues, Projects, filters and calendar the widgets read, so it
    /// stays small enough to send.
    package func forWatch(limit: Int = 30) -> TodayGlance {
        func trimmed(_ day: TodayGlance) -> TodayGlance {
            var day = day
            day.rows = Array(day.rows.prefix(limit))
            day.later = []
            day.avenues = nil
            day.projects = nil
            day.filters = nil
            day.dueCounts = nil
            return day
        }
        var today = trimmed(self)
        today.later = days.dropFirst().prefix(1).map(trimmed)
        return today
    }
}

/// The iPhone's reminder buttons and categories, by the identifiers its
/// reminders use, so the Watch offers the same buttons on them.
package enum WatchReminder {
    package static let done = "lifrDone"
    package static let snooze = "lifrSnooze"
    package static let changeStatus = "lifrChangeStatus"
    package static let category = "lifrReminder"
    package static let hiddenCategory = "lifrReminderHidden"
}
