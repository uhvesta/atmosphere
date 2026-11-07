/// The text of a web page as headings and paragraphs. Forgiving, like a
/// browser: tags left open or closed twice still read.
struct HTMLReader {
    private(set) var title: String?
    private(set) var blocks: [Block] = []

    /// Elements whose content is never read.
    private static let skipped: Set<String> = [
        "script", "style", "noscript", "template", "svg", "nav", "header", "footer", "aside", "form", "button",
        "select", "iframe", "head",
    ]
    /// Elements that end the paragraph before them.
    private static let breaking: Set<String> = [
        "p", "div", "li", "br", "blockquote", "tr", "td", "th", "section", "article", "main", "dd", "dt", "pre",
        "figcaption", "ul", "ol", "table", "hr", "h4", "h5", "h6",
    ]
    private static let headings: Set<String> = ["h1", "h2", "h3"]

    init(_ html: String) {
        let scalars = Array(html.unicodeScalars)
        let lower = html.lowercased()
        let marked = lower.contains("<article") || lower.contains("<main")
        var skipping: [String] = []
        var content = 0
        var heading = false
        var inTitle = false
        var text = ""
        var titleText = ""

        func flush() {
            let collapsed = Self.collapse(text)
            text = ""
            guard !collapsed.isEmpty else { return }
            blocks.append(heading ? .heading(collapsed) : .paragraph(collapsed))
        }

        var index = 0
        while index < scalars.count {
            let scalar = scalars[index]
            guard scalar == "<" else {
                var end = index
                while end < scalars.count, scalars[end] != "<" { end += 1 }
                let run = Self.decode(String(String.UnicodeScalarView(scalars[index..<end])))
                if inTitle {
                    titleText += run
                } else if skipping.isEmpty, !marked || content > 0 {
                    text += run
                }
                index = end
                continue
            }
            if Self.starts(scalars, at: index, with: "<!--") {
                index = Self.find(scalars, "-->", from: index + 4).map { $0 + 3 } ?? scalars.count
                continue
            }
            let close = Self.find(scalars, ">", from: index + 1) ?? scalars.count
            let tag = Tag(String(String.UnicodeScalarView(scalars[(index + 1)..<close])))
            index = close + 1
            guard let tag else { continue }

            if Self.skipped.contains(tag.name), !tag.selfClosing {
                if tag.closing {
                    if let last = skipping.lastIndex(of: tag.name) { skipping.removeSubrange(last...) }
                } else {
                    skipping.append(tag.name)
                }
                continue
            }
            if tag.name == "title", skipping.isEmpty || skipping == ["head"] {
                inTitle = !tag.closing
                if tag.closing { title = Self.collapse(titleText).nilIfEmpty }
                continue
            }
            guard skipping.isEmpty else { continue }
            if tag.name == "article" || tag.name == "main" {
                flush()
                content = max(0, content + (tag.closing ? -1 : 1))
                continue
            }
            if Self.headings.contains(tag.name) {
                flush()
                heading = !tag.closing
            } else if Self.breaking.contains(tag.name) {
                flush()
            }
        }
        flush()
    }

    private static func collapse(_ text: String) -> String {
        text.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    }

    private static func starts(_ scalars: [Unicode.Scalar], at index: Int, with prefix: String) -> Bool {
        let prefix = Array(prefix.unicodeScalars)
        guard index + prefix.count <= scalars.count else { return false }
        return Array(scalars[index..<(index + prefix.count)]) == prefix
    }

    private static func find(_ scalars: [Unicode.Scalar], _ needle: String, from start: Int) -> Int? {
        var index = start
        while index < scalars.count {
            if starts(scalars, at: index, with: needle) { return index }
            index += 1
        }
        return nil
    }

    private static let entities: [String: String] = [
        "amp": "&", "lt": "<", "gt": ">", "quot": "\"", "apos": "'", "nbsp": " ", "mdash": "—", "ndash": "–",
        "hellip": "…", "lsquo": "‘", "rsquo": "’", "ldquo": "“", "rdquo": "”", "laquo": "«", "raquo": "»",
        "middot": "·", "copy": "©",
    ]

    /// Decodes character references: named, decimal and hex. One it doesn't
    /// know stays as written.
    static func decode(_ text: String) -> String {
        guard text.contains("&") else { return text }
        var out = ""
        var rest = Substring(text)
        while let amp = rest.firstIndex(of: "&") {
            out += rest[..<amp]
            let after = rest[rest.index(after: amp)...]
            guard let semi = after.prefix(10).firstIndex(of: ";") else {
                out += "&"
                rest = after
                continue
            }
            let name = after[..<semi]
            let decoded: String? =
                if name.hasPrefix("#x") || name.hasPrefix("#X") {
                    UInt32(name.dropFirst(2), radix: 16).flatMap(Unicode.Scalar.init).map { String(Character($0)) }
                } else if name.hasPrefix("#") {
                    UInt32(name.dropFirst()).flatMap(Unicode.Scalar.init).map { String(Character($0)) }
                } else {
                    entities[String(name)]
                }
            if let decoded {
                out += decoded
                rest = after[after.index(after: semi)...]
            } else {
                out += "&"
                rest = after
            }
        }
        return out + rest
    }
}

private struct Tag {
    var name: String
    var closing: Bool
    var selfClosing: Bool

    init?(_ inside: String) {
        var body = Substring(inside)
        closing = body.hasPrefix("/")
        if closing { body = body.dropFirst() }
        selfClosing = body.hasSuffix("/")
        let name = body.prefix(while: { $0.isLetter || $0.isNumber }).lowercased()
        guard !name.isEmpty else { return nil }
        self.name = name
    }
}

extension String {
    fileprivate var nilIfEmpty: String? { isEmpty ? nil : self }
}
