import Foundation

/// A place in Lifr a link opens, for widgets, Shortcuts and other apps:
///
/// - `lifr://task/<id>` opens a task, and `lifr://task/<id>?status` opens it
///   with the statuses to pick.
/// - `lifr://today` shows Today.
/// - `lifr://upcoming` shows Upcoming.
/// - `lifr://avenue/<id>` shows an Avenue's page.
/// - `lifr://project/<id>` shows a Project's page.
/// - `lifr://filter/<id>` shows a saved filter's tasks.
/// - `lifr://calendar/2026-11-09` shows that day in the calendar.
/// - `lifr://add?title=Buy%20milk` opens Quick Add, with the title if given.
/// - `lifr://plan?text=…` opens a new conversation with the plan in another
///   assistant's reply on its draft card, to review before adding. Nothing
///   is added until the person taps Add.
package enum DeepLink: Hashable, Sendable {
    case task(UUID, picksStatus: Bool)
    case today
    case upcoming
    case avenue(UUID)
    case project(UUID)
    case filter(UUID)
    case calendar(CalendarDate)
    case add(title: String?)
    case plan(String)

    package static let scheme = "lifr"

    /// The Handoff activity for the open task, which carries its link in
    /// `userInfo` under `handoffKey`.
    package static let handoffType = "lifr.open-task"
    package static let handoffKey = "link"

    /// The most characters a plan link carries, well past any plan a reply
    /// holds.
    static let planLimit = 200_000

    /// A link to review `text`, or nil when it's empty or longer than
    /// `planLimit`.
    package init?(plan text: String) {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, text.count <= Self.planLimit else {
            return nil
        }
        self = .plan(text)
    }

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
        case ("project", 1):
            guard let id = UUID(uuidString: path[0]) else { return nil }
            self = .project(id)
        case ("filter", 1):
            guard let id = UUID(uuidString: path[0]) else { return nil }
            self = .filter(id)
        case ("calendar", 1):
            guard let date = try? CalendarDate(path[0]) else { return nil }
            self = .calendar(date)
        case ("add", 0):
            let title = items.first { $0.name == "title" }?.value?.trimmingCharacters(in: .whitespacesAndNewlines)
            self = .add(title: title?.isEmpty == false ? title : nil)
        case ("plan", 0):
            guard let text = items.first(where: { $0.name == "text" })?.value else { return nil }
            self.init(plan: text)
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
        case .project(let id):
            components.host = "project"
            components.path = "/\(id.uuidString)"
        case .filter(let id):
            components.host = "filter"
            components.path = "/\(id.uuidString)"
        case .calendar(let date):
            components.host = "calendar"
            components.path = "/\(date)"
        case .add(let title):
            components.host = "add"
            if let title { components.queryItems = [URLQueryItem(name: "title", value: title)] }
        case .plan(let text):
            components.host = "plan"
            components.queryItems = [URLQueryItem(name: "text", value: text)]
        }
        return components.url
    }
}
