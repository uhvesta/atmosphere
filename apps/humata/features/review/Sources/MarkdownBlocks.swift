import Foundation

/// A Markdown file's blocks as rendered: headings, paragraphs, lists, quotes
/// and fenced code, each with the lines it spans so a comment on it is a
/// comment on those lines.
package struct MarkdownBlock: Hashable, Sendable, Identifiable {
    package enum Kind: Hashable, Sendable {
        case heading(level: Int)
        case paragraph
        case code
    }

    package var kind: Kind
    /// Its first and last lines in the file, from 1.
    package var startLine: Int
    package var endLine: Int
    /// The block's source, without a heading's marks or code's fences.
    package var text: String

    package var id: Int { startLine }

    package init(kind: Kind, startLine: Int, endLine: Int, text: String) {
        self.kind = kind
        self.startLine = startLine
        self.endLine = endLine
        self.text = text
    }

    /// `lines` split into blocks; blank lines separate them.
    package static func blocks(_ lines: [String]) -> [MarkdownBlock] {
        var blocks: [MarkdownBlock] = []
        var paragraph: (start: Int, lines: [String])?
        var fence: (start: Int, marker: String, lines: [String])?
        func endParagraph(at line: Int) {
            guard let open = paragraph else { return }
            blocks.append(
                MarkdownBlock(
                    kind: .paragraph, startLine: open.start, endLine: line - 1,
                    text: open.lines.joined(separator: "\n")))
            paragraph = nil
        }
        for (index, line) in lines.enumerated() {
            let number = index + 1
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if let open = fence {
                if trimmed.hasPrefix(open.marker) {
                    blocks.append(
                        MarkdownBlock(
                            kind: .code, startLine: open.start, endLine: number,
                            text: open.lines.joined(separator: "\n")))
                    fence = nil
                } else {
                    fence?.lines.append(line)
                }
            } else if trimmed.hasPrefix("```") || trimmed.hasPrefix("~~~") {
                endParagraph(at: number)
                fence = (number, String(trimmed.prefix(3)), [])
            } else if trimmed.isEmpty {
                endParagraph(at: number)
            } else if let level = heading(trimmed) {
                endParagraph(at: number)
                let text = trimmed.dropFirst(level).trimmingCharacters(in: .whitespaces)
                blocks.append(
                    MarkdownBlock(kind: .heading(level: level), startLine: number, endLine: number, text: text))
            } else if paragraph != nil {
                paragraph?.lines.append(line)
            } else {
                paragraph = (number, [line])
            }
        }
        if let open = fence {
            // An unclosed fence runs to the end, as renderers show it.
            blocks.append(
                MarkdownBlock(
                    kind: .code, startLine: open.start, endLine: lines.count,
                    text: open.lines.joined(separator: "\n")))
        }
        endParagraph(at: lines.count + 1)
        return blocks
    }

    private static func heading(_ line: String) -> Int? {
        let level = line.prefix { $0 == "#" }.count
        guard (1...6).contains(level) else { return nil }
        let rest = line.dropFirst(level)
        return rest.isEmpty || rest.first == " " ? level : nil
    }
}
