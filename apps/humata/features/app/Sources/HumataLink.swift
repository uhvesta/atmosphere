import Foundation
import HumataDaemonClient

/// A place in the app a `humata://` URL names, so another program, such as
/// `humata mcp`'s show tool, can put it in front of the person.
package enum HumataLink: Equatable, Sendable {
    /// `humata://session/<id>`
    case session(Session.ID)
    /// `humata://board/<project>`
    case board(Project.ID)
    /// `humata://task/<project>/<item>`
    case task(projectID: Project.ID, itemID: String)

    /// The link a URL names, or nil when it names none.
    package init?(_ url: URL) {
        guard url.scheme?.lowercased() == "humata", let kind = url.host() else { return nil }
        let path = url.pathComponents.filter { $0 != "/" }
        switch (kind, path.count) {
        case ("session", 1): self = .session(path[0])
        case ("board", 1): self = .board(path[0])
        case ("task", 2): self = .task(projectID: path[0], itemID: path[1])
        default: return nil
        }
    }
}
