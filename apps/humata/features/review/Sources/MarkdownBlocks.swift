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
    /// A fenced block's language, as its opening fence names it, such as
    /// "swift" or "mermaid"; empty when it names none.
    package var language: String
    /// A heading's or paragraph's text with its inline Markdown rendered,
    /// worked out once when the block is made, not on every draw.
    package let inline: AttributedString

    package var id: Int { startLine }

    package init(kind: Kind, startLine: Int, endLine: Int, text: String, language: String = "") {
        self.kind = kind
        self.startLine = startLine
        self.endLine = endLine
        self.text = text
        self.language = language
        inline = kind == .code ? AttributedString(text) : Self.inlineMarkdown(text)
    }

    /// `text` with its inline Markdown, such as emphasis and code, rendered.
    package static func inlineMarkdown(_ text: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }

    /// `lines` split into blocks, as `blocks` does, kept for the texts
    /// shown lately: a view that draws again, on a click or a key, doesn't
    /// parse its file again.
    package static func cached(_ lines: [String]) -> [MarkdownBlock] {
        let key = lines.joined(separator: "\n")
        if let blocks = cache.blocks(for: key) { return blocks }
        let made = Self.blocks(lines)
        cache.keep(made, for: key)
        return made
    }

    /// The same, for `text` whole.
    package static func cached(_ text: String) -> [MarkdownBlock] {
        if let blocks = cache.blocks(for: text) { return blocks }
        let made = Self.blocks(text.components(separatedBy: "\n"))
        cache.keep(made, for: text)
        return made
    }

    private static let cache = BlockCache()

    /// `lines` split into blocks; blank lines separate them.
    package static func blocks(_ lines: [String]) -> [MarkdownBlock] {
        var blocks: [MarkdownBlock] = []
        var paragraph: (start: Int, lines: [String])?
        var fence: (start: Int, marker: String, language: String, lines: [String])?
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
                            text: open.lines.joined(separator: "\n"), language: open.language))
                    fence = nil
                } else {
                    fence?.lines.append(line)
                }
            } else if trimmed.hasPrefix("```") || trimmed.hasPrefix("~~~") {
                endParagraph(at: number)
                let language = trimmed.dropFirst(3).prefix { !$0.isWhitespace && $0 != "{" }
                fence = (number, String(trimmed.prefix(3)), language.lowercased(), [])
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
                    text: open.lines.joined(separator: "\n"), language: open.language))
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

/// The blocks of the few texts shown last, by their text.
private final class BlockCache: @unchecked Sendable {
    /// Enough for the files and specs on screen at once.
    private static let kept = 12
    private let lock = NSLock()
    private var entries: [(key: String, blocks: [MarkdownBlock])] = []

    func blocks(for key: String) -> [MarkdownBlock]? {
        lock.withLock {
            guard let index = entries.firstIndex(where: { $0.key == key }) else { return nil }
            let entry = entries.remove(at: index)
            entries.append(entry)
            return entry.blocks
        }
    }

    func keep(_ blocks: [MarkdownBlock], for key: String) {
        lock.withLock {
            entries.removeAll { $0.key == key }
            entries.append((key, blocks))
            if entries.count > Self.kept { entries.removeFirst(entries.count - Self.kept) }
        }
    }
}
