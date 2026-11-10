import Foundation

extension Markdown {
    /// A reply as HTML, for apps that paste formatted text, such as Notes
    /// and Mail: its headings, lists, quotes, code and tables as they're
    /// shown, and maths as its symbols.
    static func html(_ text: String) -> String {
        html(blocks(in: text))
    }

    private static func html(_ blocks: [MarkdownBlock]) -> String {
        blocks.map(html).joined(separator: "\n")
    }

    private static func html(_ block: MarkdownBlock) -> String {
        switch block {
        case .paragraph(let text):
            return "<p>\(htmlInline(text).replacingOccurrences(of: "\n", with: "<br>"))</p>"
        case .heading(let level, let text):
            let level = min(max(level, 1), 6)
            return "<h\(level)>\(htmlInline(text))</h\(level)>"
        case .code(_, let code):
            return "<pre><code>\(escaped(code))</code></pre>"
        case .quote(let inner):
            return "<blockquote>\(html(inner))</blockquote>"
        case .list(let list):
            return html(list)
        case .rule:
            return "<hr>"
        case .math(let latex):
            return "<p>\(escaped(Math.text(latex)))</p>"
        case .table(let table):
            let header = table.header.map { "<th>\(htmlInline($0))</th>" }.joined()
            let rows = table.rows.map { row in
                "<tr>" + row.map { "<td>\(htmlInline($0))</td>" }.joined() + "</tr>"
            }
            return "<table><tr>\(header)</tr>\(rows.joined())</table>"
        }
    }

    private static func html(_ list: MarkdownList) -> String {
        let items = list.items.enumerated().map { index, item in
            let check = list.checks.indices.contains(index) ? list.checks[index] : nil
            let box = check.map { $0 ? "☑ " : "☐ " } ?? ""
            // A lone paragraph sits in the item without a <p>, as lists
            // are usually written.
            if case .paragraph(let text) = item.first, item.count == 1 {
                return "<li>\(box)\(htmlInline(text))</li>"
            }
            return "<li>\(box)\(html(item))</li>"
        }
        guard let start = list.start else { return "<ul>\(items.joined())</ul>" }
        let first = start == 1 ? "" : " start=\"\(start)\""
        return "<ol\(first)>\(items.joined())</ol>"
    }

    /// Inline Markdown as HTML: bold, italics, strikethrough, code and
    /// links, the rest escaped.
    private static func htmlInline(_ text: String) -> String {
        let string = inline(text)
        return string.runs.map { run in
            var out = escaped(String(string[run.range].characters))
            let intent = run.inlinePresentationIntent ?? []
            if intent.contains(.code) { out = "<code>\(out)</code>" }
            if intent.contains(.strikethrough) { out = "<s>\(out)</s>" }
            if intent.contains(.emphasized) { out = "<em>\(out)</em>" }
            if intent.contains(.stronglyEmphasized) { out = "<strong>\(out)</strong>" }
            if let link = run.link { out = "<a href=\"\(escaped(link.absoluteString))\">\(out)</a>" }
            return out
        }.joined()
    }

    private static func escaped(_ text: String) -> String {
        text.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;").replacingOccurrences(of: "\"", with: "&quot;")
    }
}
