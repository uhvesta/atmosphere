import Foundation

/// What another app shared, made into a task's title and description.
struct Shared: Equatable {
    var title: String
    var details: String

    init(title: String, details: String) {
        self.title = title
        self.details = details
    }

    /// From the page's or item's `title`, the selected or shared `text` and
    /// the shared `url`: the title is the first of those that has words,
    /// and the description keeps the link and any text that isn't the title.
    init(title: String?, text: String?, url: URL?) {
        let title = title?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let text = text?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let firstLine =
            text.split(whereSeparator: \.isNewline).first.map { String($0).trimmingCharacters(in: .whitespaces) } ?? ""
        let link = url?.absoluteString ?? ""
        let name = !title.isEmpty ? title : !firstLine.isEmpty ? firstLine : url.map(Self.name) ?? ""
        self.title = String(name.prefix(200))
        var details: [String] = []
        if !link.isEmpty { details.append(link) }
        if !text.isEmpty, text != name, text != link { details.append(text) }
        self.details = details.joined(separator: "\n\n")
    }

    /// A link's site and path, such as "example.com/article", for when
    /// nothing better names it.
    private static func name(_ url: URL) -> String {
        let host = url.host(percentEncoded: false).map { $0.hasPrefix("www.") ? String($0.dropFirst(4)) : $0 }
        let path = url.path(percentEncoded: false)
        return (host ?? url.absoluteString) + (path == "/" ? "" : path)
    }
}
