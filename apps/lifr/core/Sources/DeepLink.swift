import Foundation

/// A place in Lifr a link opens, for widgets, Shortcuts and other apps:
///
/// - `lifr://task/<id>` opens a task, and `lifr://task/<id>?status` opens it
///   with the statuses to pick.
/// - `lifr://today` shows Today.
/// - `lifr://upcoming` shows Upcoming.
/// - `lifr://avenue/<id>` shows an Avenue's page.
/// - `lifr://calendar/2026-11-09` shows that day in the calendar.
/// - `lifr://add?title=Buy%20milk` opens Quick Add, with the title if given.
package enum DeepLink: Hashable, Sendable {
    case task(UUID, picksStatus: Bool)
    case today
    case upcoming
    case avenue(UUID)
    case calendar(CalendarDate)
    case add(title: String?)

    package static let scheme = "lifr"

    /// The link `url` names, or nil when it isn't a Lifr link or names
    /// nothing Lifr has.
    package init?(_ url: URL) {
        guard url.scheme?.lowercased() == Self.scheme,
            let components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        else { return nil }
        let path = components.path.split(separator: "/").map(String.init)
        let items = components.queryItems ?? []
        switch (components.host?.lowercased(), path.count) {
        case ("task", 1):
            guard let id = UUID(uuidString: path[0]) else { return nil }
            self = .task(id, picksStatus: items.contains { $0.name == "status" })
        case ("today", 0):
            self = .today
        case ("upcoming", 0):
            self = .upcoming
        case ("avenue", 1):
            guard let id = UUID(uuidString: path[0]) else { return nil }
            self = .avenue(id)
        case ("calendar", 1):
            guard let date = try? CalendarDate(path[0]) else { return nil }
            self = .calendar(date)
        case ("add", 0):
            let title = items.first { $0.name == "title" }?.value?.trimmingCharacters(in: .whitespacesAndNewlines)
            self = .add(title: title?.isEmpty == false ? title : nil)
        default:
            return nil
        }
    }

    /// The link, to hand to a widget or share.
    package var url: URL? {
        var components = URLComponents()
        components.scheme = Self.scheme
        switch self {
        case .task(let id, let picksStatus):
            components.host = "task"
            components.path = "/\(id.uuidString)"
            if picksStatus { components.queryItems = [URLQueryItem(name: "status", value: nil)] }
        case .today:
            components.host = "today"
        case .upcoming:
            components.host = "upcoming"
        case .avenue(let id):
            components.host = "avenue"
            components.path = "/\(id.uuidString)"
        case .calendar(let date):
            components.host = "calendar"
            components.path = "/\(date)"
        case .add(let title):
            components.host = "add"
            if let title { components.queryItems = [URLQueryItem(name: "title", value: title)] }
        }
        return components.url
    }
}
