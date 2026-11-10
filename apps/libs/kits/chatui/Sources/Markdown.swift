import Foundation

/// A block of a reply written in Markdown: what the model's text breaks
/// into before each part is shown in its own way.
enum MarkdownBlock: Hashable, Sendable {
    /// Text with inline Markdown still in it, its lines joined by `\n`.
    case paragraph(String)
    case heading(level: Int, text: String)
    /// A fenced block; `language` is the word after the opening fence.
    case code(language: String?, code: String)
    case quote([MarkdownBlock])
    case list(MarkdownList)
    case rule
    case table(MarkdownTable)
    /// Maths on lines of its own, as the LaTeX the model wrote.
    case math(String)
}

struct MarkdownList: Hashable, Sendable {
    /// The first item's number, or nil for a bulleted list.
    var start: Int?
    /// What each item holds, which can be lists of their own.
    var items: [[MarkdownBlock]]
    /// For a list of tasks, whether each item is done, nil for an item
    /// that isn't a task; empty for a list with no tasks.
    var checks: [Bool?] = []
}

struct MarkdownTable: Hashable, Sendable {
    enum Alignment: Hashable, Sendable {
        case leading, center, trailing
    }

    var alignments: [Alignment]
    var header: [String]
    /// Each as many cells as the header.
    var rows: [[String]]
}

/// Reads the Markdown a model writes: paragraphs, headings, lists, quotes,
/// fenced code, rules and tables, with inline syntax left for
/// `inline(_:)`. Text that isn't valid Markdown still reads as paragraphs.
enum Markdown {
    static func blocks(in text: String) -> [MarkdownBlock] {
        parse(text[...]).map(\.block)
    }

    /// Each top-level block, with the UTF-8 offset in `text` of its first
    /// line.
    static func parse(_ text: Substring) -> [(block: MarkdownBlock, offset: Int)] {
        // "\r\n" is one Character.
        let lines = text.split(omittingEmptySubsequences: false) { $0 == "\n" || $0 == "\r\n" }
        var parser = BlockParser(lines: lines.map { $0.hasSuffix("\r") ? $0.dropLast() : $0 })
        return parser.blocks().map { block, line in
            (block, text.utf8.distance(from: text.startIndex, to: lines[line].startIndex))
        }
    }

    /// The schemes a link in a reply may open; others show as text.
    static let linkSchemes: Set<String> = ["http", "https", "mailto"]

    /// Inline Markdown as attributed text: bold, italic, strikethrough,
    /// code and links, with links only to the web or to mail.
    static func inline(_ text: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(
            allowsExtendedAttributes: false, interpretedSyntax: .inlineOnlyPreservingWhitespace,
            failurePolicy: .returnPartiallyParsedIfPossible)
        let text = Math.inline(text)
        var string = (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
        let unsafe = string.runs.compactMap { run -> Range<AttributedString.Index>? in
            guard let url = run.link else { return nil }
            return linkSchemes.contains(url.scheme?.lowercased() ?? "") ? nil : run.range
        }
        for range in unsafe { string[range].link = nil }
        breakLines(in: &string)
        linkBareAddresses(in: &string)
        return string
    }

    /// Breaks the line at each `<br>`, which models write in a table's
    /// cells where Markdown has no way to; in code it stays as written.
    private static func breakLines(in string: inout AttributedString) {
        let plain = String(string.characters)
        guard plain.localizedCaseInsensitiveContains("<br") else { return }
        let breaks = plain.matches(of: /(?i)<br\s*\/?>/).reversed()
        for found in breaks {
            let start = string.characters.index(
                string.startIndex, offsetBy: plain.distance(from: plain.startIndex, to: found.range.lowerBound))
            let end = string.characters.index(
                start, offsetBy: plain.distance(from: found.range.lowerBound, to: found.range.upperBound))
            let inCode = string[start..<end].runs.contains { $0.inlinePresentationIntent?.contains(.code) == true }
            if !inCode { string.replaceSubrange(start..<end, with: AttributedString("\n")) }
        }
    }

    /// Links web and mail addresses written without Markdown's brackets.
    /// Only those that read as an address, starting `http`, `https` or
    /// `www.`, or an email: a name like `main.rs` stays text. Code and
    /// what's linked already are left as they are.
    private static func linkBareAddresses(in string: inout AttributedString) {
        let plain = String(string.characters)
        guard plain.contains("http") || plain.contains("www.") || plain.contains("@"),
            let detector = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.link.rawValue)
        else { return }
        let matches = detector.matches(in: plain, range: NSRange(plain.startIndex..., in: plain))
        for match in matches {
            guard let url = match.url, linkSchemes.contains(url.scheme?.lowercased() ?? ""),
                let found = Range(match.range, in: plain)
            else { continue }
            let written = plain[found].lowercased()
            guard written.hasPrefix("http://") || written.hasPrefix("https://") || written.hasPrefix("www.")
                || url.scheme?.lowercased() == "mailto"
            else { continue }
            let start = string.characters.index(
                string.startIndex, offsetBy: plain.distance(from: plain.startIndex, to: found.lowerBound))
            let end = string.characters.index(start, offsetBy: plain.distance(from: found.lowerBound, to: found.upperBound))
            let range = start..<end
            let taken = string[range].runs.contains { run in
                run.link != nil || run.inlinePresentationIntent?.contains(.code) == true
            }
            if !taken { string[range].link = url }
        }
    }

    /// `text`'s words without its Markdown, on one line: what a list of
    /// chats shows of a reply under its title.
    static func plainText(_ text: String) -> String {
        plainText(blocks(in: text))
            .joined(separator: " ")
            .split(whereSeparator: \.isNewline)
            .joined(separator: " ")
    }

    private static func plainText(_ blocks: [MarkdownBlock]) -> [String] {
        blocks.flatMap { block -> [String] in
            switch block {
            case .paragraph(let text), .heading(_, let text): [plainInline(text)]
            case .code(_, let code): [code]
            case .quote(let inner): plainText(inner)
            case .list(let list): list.items.flatMap(plainText)
            case .rule: []
            case .math(let latex): [Math.text(latex)]
            case .table(let table):
                ([table.header] + table.rows).map { $0.map(plainInline).joined(separator: " ") }
            }
        }
    }

    private static func plainInline(_ text: String) -> String {
        String(inline(text).characters)
    }

    /// A line the person typed, such as a chat's title taken from their
    /// first message, without its bold, italics and other inline marks.
    /// Unlike `plainText`, a line like "1. Buy milk" stays as typed.
    static func plainLine(_ text: String) -> String {
        plainInline(text)
    }

    /// `block` as it's shown while the model is still writing it: marks
    /// it has opened but not yet closed, such as `**` or a link's `[`, are
    /// closed or left out, so the rest of the reply doesn't flicker.
    static func inProgress(_ block: MarkdownBlock) -> MarkdownBlock {
        switch block {
        case .paragraph(let text): return .paragraph(closingOpenMarks(text))
        case .heading(let level, let text): return .heading(level: level, text: closingOpenMarks(text))
        case .quote(let blocks): return .quote(inProgress(blocks))
        case .list(var list):
            if let last = list.items.indices.last { list.items[last] = inProgress(list.items[last]) }
            return .list(list)
        case .table(var table):
            if let last = table.rows.indices.last { table.rows[last] = table.rows[last].map(closingOpenMarks) }
            return .table(table)
        case .code, .rule, .math: return block
        }
    }

    private static func inProgress(_ blocks: [MarkdownBlock]) -> [MarkdownBlock] {
        guard let last = blocks.last else { return blocks }
        return blocks.dropLast() + [inProgress(last)]
    }

    /// Inline text cut off part way: a mark at the very end that opens
    /// nothing yet is left out, a link still being written shows its
    /// words, and each emphasis or code span still open is closed.
    static func closingOpenMarks(_ text: String) -> String {
        var chars = Array(text)
        // A run of marks after a space, with nothing after them yet.
        var end = chars.count
        while end > 0, "*_~`".contains(chars[end - 1]) { end -= 1 }
        if end < chars.count, end == 0 || chars[end - 1].isWhitespace { chars.removeSubrange(end...) }
        closeLink(&chars)

        var open: [String] = []
        var index = 0
        while index < chars.count {
            let char = chars[index]
            if char == "\\" {
                index += 2
                continue
            }
            let run = chars[index...].prefix { $0 == char }.count
            if char == "`" {
                let fence = String(repeating: "`", count: run)
                if let close = codeSpanEnd(in: chars, from: index + run, length: run) {
                    index = close
                    continue
                }
                // The rest is code: close it, then what was open before.
                while chars.last?.isWhitespace == true { chars.removeLast() }
                return String(chars) + fence + open.reversed().joined()
            }
            guard "*_~".contains(char) else {
                index += 1
                continue
            }
            let before = index > 0 ? chars[index - 1] : " "
            let after = index + run < chars.count ? chars[index + run] : " "
            let intraword = { (side: Character) in char == "_" && (side.isLetter || side.isNumber) }
            let opens = !after.isWhitespace && !intraword(before)
            let closes = !before.isWhitespace && !intraword(after)
            let marks: [String] =
                char == "~"
                ? (run >= 2 ? ["~~"] : [])
                : (run == 1
                    ? [String(char)] : run == 2 ? [String([char, char])] : [String([char, char]), String(char)])
            for mark in marks {
                if closes, let opened = open.lastIndex(of: mark) {
                    open.removeSubrange(opened...)
                } else if opens {
                    open.append(mark)
                }
            }
            index += run
        }
        guard !open.isEmpty else { return String(chars) }
        while chars.last?.isWhitespace == true { chars.removeLast() }
        return String(chars) + open.reversed().joined()
    }

    /// Where the code span opened by a run of `length` backticks ends: past
    /// the next run of exactly as many.
    private static func codeSpanEnd(in chars: [Character], from start: Int, length: Int) -> Int? {
        var index = start
        while index < chars.count {
            guard chars[index] == "`" else {
                index += 1
                continue
            }
            let run = chars[index...].prefix { $0 == "`" }.count
            if run == length { return index + run }
            index += run
        }
        return nil
    }

    /// Shows the words of a link that isn't finished: `[words` or
    /// `[words](https://exa` read as `words`.
    private static func closeLink(_ chars: inout [Character]) {
        guard let open = chars.lastIndex(of: "["), open == 0 || chars[open - 1] != "\\" else { return }
        let rest = chars[(open + 1)...]
        guard let close = rest.firstIndex(of: "]") else {
            chars.remove(at: open)
            return
        }
        let label = Array(chars[(open + 1)..<close])
        let after = chars[(close + 1)...]
        if after.isEmpty {
            chars.replaceSubrange(open..., with: label)
        } else if after.first == "(", !after.contains(")") {
            chars.replaceSubrange(open..., with: label)
        }
    }
}

/// A reply's blocks as it streams in. Every block but the last is done, so
/// as text is added only the last one is read again.
struct MarkdownStream {
    private var text = ""
    private var done: [MarkdownBlock] = []
    /// The UTF-8 offset where the last block, still being read, starts.
    private var doneEnd = 0
    private(set) var blocks: [MarkdownBlock] = []

    /// Reads `new`, again from the last block when it goes on from the
    /// text before, or from the start when it doesn't.
    mutating func update(_ new: String) {
        guard new != text else { return }
        if !new.utf8.starts(with: text.utf8.prefix(doneEnd)) {
            done = []
            doneEnd = 0
        }
        text = new
        let tail = new[new.utf8.index(new.startIndex, offsetBy: doneEnd)...]
        let parsed = Markdown.parse(tail)
        guard let last = parsed.last else {
            blocks = done
            return
        }
        done += parsed.dropLast().map(\.block)
        doneEnd += last.offset
        blocks = done + [last.block]
    }
}

/// Reads lines into blocks, CommonMark's rules where a model's text needs
/// them and leniently elsewhere: a nested list needs only be indented more
/// than its parent's marker, and a setext heading's underline is at least
/// three long, so a lone `-` being written doesn't turn a paragraph into a
/// heading.
private struct BlockParser {
    let lines: [Substring]
    var index = 0

    /// Each block with the index of its first line.
    mutating func blocks() -> [(MarkdownBlock, Int)] {
        var blocks: [(MarkdownBlock, Int)] = []
        while index < lines.count {
            let line = lines[index]
            let start = index
            if isBlank(line) {
                index += 1
                continue
            }
            if let fence = Fence(line) {
                blocks.append((code(in: fence), start))
            } else if let heading = heading(line) {
                index += 1
                blocks.append((heading, start))
            } else if isRule(line) {
                index += 1
                blocks.append((.rule, start))
            } else if quoted(line) != nil {
                blocks.append((quote(), start))
            } else if let marker = ListMarker(line) {
                blocks.append((.list(list(from: marker)), start))
            } else if let table = table() {
                blocks.append((.table(table), start))
            } else if let delimiters = Math.blockDelimiters(line) {
                blocks.append((math(delimiters), start))
            } else {
                blocks.append((paragraph(), start))
            }
        }
        return blocks
    }

    private mutating func code(in fence: Fence) -> MarkdownBlock {
        index += 1
        var body: [Substring] = []
        var closed = false
        while index < lines.count {
            let line = lines[index]
            index += 1
            if fence.isClosed(by: line) {
                closed = true
                break
            }
            body.append(dropIndent(line, fence.indent))
        }
        // The closing fence being written.
        if !closed, let last = body.last, !last.isEmpty, fence.isClosing(partly: last) { body.removeLast() }
        return .code(language: fence.language, code: body.joined(separator: "\n"))
    }

    /// Maths from `$$` or `\[` to its closing, which can be on the same
    /// line, or to the end while it's written.
    private mutating func math(_ delimiters: (open: String, close: String)) -> MarkdownBlock {
        var rest = Substring(lines[index].trimmingCharacters(in: .whitespaces)).dropFirst(delimiters.open.count)
        var body: [Substring] = []
        index += 1
        while true {
            if let close = rest.range(of: delimiters.close) {
                body.append(rest[..<close.lowerBound])
                break
            }
            body.append(rest)
            guard index < lines.count else { break }
            rest = lines[index]
            index += 1
        }
        return .math(body.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines))
    }

    private mutating func quote() -> MarkdownBlock {
        var inner: [Substring] = []
        while index < lines.count {
            let line = lines[index]
            if let content = quoted(line) {
                inner.append(content)
            } else if !isBlank(line), let last = inner.last, !isBlank(last), !startsBlock(line) {
                // A paragraph's lazy continuation.
                inner.append(line)
            } else {
                break
            }
            index += 1
        }
        var parser = BlockParser(lines: inner)
        return .quote(parser.blocks().map(\.0))
    }

    private mutating func list(from first: ListMarker) -> MarkdownList {
        var items: [[MarkdownBlock]] = []
        var checks: [Bool?] = []
        var marker = first
        while true {
            let (check, content) = marker.number == nil ? Self.task(marker.content) : (nil, marker.content)
            checks.append(check)
            var item: [Substring] = [content]
            index += 1
            while index < lines.count {
                let line = lines[index]
                let indent = indentation(line)
                if isBlank(line) {
                    item.append("")
                } else if indent >= marker.contentIndent {
                    item.append(dropIndent(line, marker.contentIndent))
                } else if indent > marker.indent, ListMarker(line) != nil {
                    item.append(dropIndent(line, indent))
                } else if let last = item.last, !isBlank(last), !startsBlock(line) {
                    item.append(dropIndent(line, indent))
                } else {
                    break
                }
                index += 1
            }
            while let last = item.last, isBlank(last), item.count > 1 {
                item.removeLast()
                index -= 1
            }
            var parser = BlockParser(lines: item)
            items.append(parser.blocks().map(\.0))

            var next = index
            while next < lines.count, isBlank(lines[next]) { next += 1 }
            guard next < lines.count, let following = ListMarker(lines[next]), following.continues(first) else {
                break
            }
            index = next
            marker = following
        }
        return MarkdownList(
            start: first.number, items: items, checks: checks.contains { $0 != nil } ? checks : [])
    }

    /// A bulleted item's box, `[ ]` or `[x]`, whether it's ticked, and the
    /// text after it; nil and the text as it is for an item without one.
    static func task(_ content: Substring) -> (Bool?, Substring) {
        let boxes: [(String, Bool)] = [("[ ]", false), ("[x]", true), ("[X]", true)]
        for (box, checked) in boxes where content.hasPrefix(box) {
            let rest = content.dropFirst(box.count)
            if rest.isEmpty { return (checked, rest) }
            if rest.first == " " || rest.first == "\t" { return (checked, rest.dropFirst()) }
        }
        return (nil, content)
    }

    /// A table whose header is this line, with its delimiter row after it.
    private mutating func table() -> MarkdownTable? {
        guard let alignments = tableAlignments(at: index) else { return nil }
        let header = cells(lines[index])
        index += 2
        var rows: [[String]] = []
        while index < lines.count, !isBlank(lines[index]), lines[index].contains("|"), !startsBlock(lines[index]) {
            let row = cells(lines[index])
            rows.append(
                (0..<header.count).map { $0 < row.count ? row[$0] : "" })
            index += 1
        }
        return MarkdownTable(alignments: alignments, header: header, rows: rows)
    }

    private func tableAlignments(at line: Int) -> [MarkdownTable.Alignment]? {
        guard line + 1 < lines.count, lines[line].contains("|"), lines[line + 1].contains("-") else { return nil }
        let delimiters = cells(lines[line + 1])
        var alignments: [MarkdownTable.Alignment] = []
        for cell in delimiters {
            let dashes = cell.trimmingCharacters(in: CharacterSet(charactersIn: ":"))
            guard !dashes.isEmpty, dashes.allSatisfy({ $0 == "-" }) else { return nil }
            switch (cell.hasPrefix(":"), cell.hasSuffix(":")) {
            case (true, true): alignments.append(.center)
            case (false, true): alignments.append(.trailing)
            default: alignments.append(.leading)
            }
        }
        guard alignments.count == cells(lines[line]).count else { return nil }
        return alignments
    }

    private mutating func paragraph() -> MarkdownBlock {
        var parts = [lines[index].trimmingCharacters(in: .whitespaces)]
        index += 1
        while index < lines.count {
            let line = lines[index]
            if isBlank(line) || tableAlignments(at: index) != nil { break }
            if let level = setextLevel(line) {
                index += 1
                return .heading(level: level, text: parts.joined(separator: "\n"))
            }
            if startsBlock(line) { break }
            parts.append(line.trimmingCharacters(in: .whitespaces))
            index += 1
        }
        return .paragraph(parts.joined(separator: "\n"))
    }

    /// Whether `line` starts a block that ends a paragraph before it.
    private func startsBlock(_ line: Substring) -> Bool {
        if Fence(line) != nil || heading(line) != nil || isRule(line) || quoted(line) != nil { return true }
        if Math.blockDelimiters(line) != nil { return true }
        if let marker = ListMarker(line), !marker.content.isEmpty { return true }
        return false
    }

    private func heading(_ line: Substring) -> MarkdownBlock? {
        guard indentation(line) <= 3 else { return nil }
        let rest = line.drop { $0 == " " }
        let level = rest.prefix { $0 == "#" }.count
        guard (1...6).contains(level) else { return nil }
        let after = rest.dropFirst(level)
        guard after.isEmpty || after.first == " " || after.first == "\t" else { return nil }
        var text = after.trimmingCharacters(in: .whitespaces)
        // An optional closing sequence of #s.
        let closing = text.reversed().prefix { $0 == "#" }.count
        if closing > 0 {
            let kept = text.dropLast(closing)
            if kept.isEmpty || kept.last == " " { text = kept.trimmingCharacters(in: .whitespaces) }
        }
        return .heading(level: level, text: text)
    }

    private func setextLevel(_ line: Substring) -> Int? {
        guard indentation(line) <= 3 else { return nil }
        let marks = line.trimmingCharacters(in: .whitespaces)
        guard marks.count >= 3, let mark = marks.first, marks.allSatisfy({ $0 == mark }) else { return nil }
        return mark == "=" ? 1 : mark == "-" ? 2 : nil
    }

    private func isRule(_ line: Substring) -> Bool {
        guard indentation(line) <= 3 else { return false }
        let marks = line.filter { $0 != " " && $0 != "\t" }
        guard marks.count >= 3, let mark = marks.first, "-*_".contains(mark) else { return false }
        return marks.allSatisfy { $0 == mark }
    }

    private func quoted(_ line: Substring) -> Substring? {
        guard indentation(line) <= 3 else { return nil }
        let rest = line.drop { $0 == " " }
        guard rest.first == ">" else { return nil }
        let content = rest.dropFirst()
        return content.first == " " ? content.dropFirst() : content
    }

    /// A row's cells, without the pipes at its ends.
    private func cells(_ line: Substring) -> [String] {
        var row = line.trimmingCharacters(in: .whitespaces)[...]
        if row.hasPrefix("|") { row = row.dropFirst() }
        if row.hasSuffix("|"), !row.hasSuffix("\\|") { row = row.dropLast() }
        var cells: [String] = []
        var cell = ""
        var inCode = false
        var escaped = false
        for char in row {
            if escaped {
                cell.append(char == "|" ? "|" : "\\\(char)")
                escaped = false
            } else if char == "\\" {
                escaped = true
            } else if char == "`" {
                inCode.toggle()
                cell.append(char)
            } else if char == "|", !inCode {
                cells.append(cell.trimmingCharacters(in: .whitespaces))
                cell = ""
            } else {
                cell.append(char)
            }
        }
        cells.append(cell.trimmingCharacters(in: .whitespaces))
        return cells
    }
}

/// A fenced code block's opening: three or more backticks or tildes.
private struct Fence {
    let mark: Character
    let length: Int
    let indent: Int
    let language: String?

    init?(_ line: Substring) {
        indent = indentation(line)
        guard indent <= 3 else { return nil }
        let rest = line.drop { $0 == " " }
        guard let mark = rest.first, mark == "`" || mark == "~" else { return nil }
        length = rest.prefix { $0 == mark }.count
        guard length >= 3 else { return nil }
        let info = rest.dropFirst(length).trimmingCharacters(in: .whitespaces)
        if mark == "`", info.contains("`") { return nil }
        self.mark = mark
        language = info.split(separator: " ").first.map(String.init)
    }

    func isClosed(by line: Substring) -> Bool {
        guard indentation(line) <= 3 else { return false }
        let marks = line.trimmingCharacters(in: .whitespaces)
        return marks.count >= length && marks.allSatisfy { $0 == mark }
    }

    /// Whether `line` is the closing fence, still being written.
    func isClosing(partly line: Substring) -> Bool {
        let marks = line.trimmingCharacters(in: .whitespaces)
        return !marks.isEmpty && marks.count < length && marks.allSatisfy { $0 == mark }
    }
}

/// A list item's marker: `-`, `+` or `*`, or a number and `.` or `)`,
/// then a space.
private struct ListMarker {
    /// The bullet, or what follows the number.
    let mark: Character
    /// nil for a bullet.
    let number: Int?
    /// The column the marker starts at.
    let indent: Int
    /// The column the item's text starts at.
    let contentIndent: Int
    let content: Substring

    init?(_ line: Substring) {
        indent = indentation(line)
        guard indent <= 3 else { return nil }
        let rest = line.drop { $0 == " " }
        let digits = rest.prefix(while: \.isASCIIDigit)
        let markerWidth: Int
        if let bullet = rest.first, "-+*".contains(bullet) {
            mark = bullet
            number = nil
            markerWidth = 1
        } else if (1...9).contains(digits.count), let delimiter = rest.dropFirst(digits.count).first,
            delimiter == "." || delimiter == ")", let value = Int(digits)
        {
            mark = delimiter
            number = value
            markerWidth = digits.count + 1
        } else {
            return nil
        }
        let after = rest.dropFirst(markerWidth)
        let spaces = after.prefix { $0 == " " }.count
        if after.isEmpty {
            contentIndent = indent + markerWidth + 1
            content = after
        } else if after.first == "\t" {
            contentIndent = indent + markerWidth + 1
            content = after.dropFirst()
        } else if spaces == 0 {
            return nil
        } else if spaces > 4 || spaces == after.count {
            contentIndent = indent + markerWidth + 1
            content = spaces == after.count ? after.dropFirst(after.count) : after.dropFirst()
        } else {
            contentIndent = indent + markerWidth + spaces
            content = after.dropFirst(spaces)
        }
    }

    /// Whether this item goes on the list `first` started.
    func continues(_ first: ListMarker) -> Bool {
        mark == first.mark && (number == nil) == (first.number == nil)
    }
}

private func isBlank(_ line: Substring) -> Bool {
    line.allSatisfy { $0 == " " || $0 == "\t" }
}

/// The column a line's text starts at, a tab counting as four.
private func indentation(_ line: Substring) -> Int {
    var column = 0
    for char in line {
        if char == " " {
            column += 1
        } else if char == "\t" {
            column += 4 - column % 4
        } else {
            break
        }
    }
    return column
}

/// `line` with up to `columns` of its indentation taken off.
private func dropIndent(_ line: Substring, _ columns: Int) -> Substring {
    var column = 0
    var rest = line
    while column < columns, let char = rest.first, char == " " || char == "\t" {
        column += char == " " ? 1 : 4 - column % 4
        rest = rest.dropFirst()
    }
    return rest
}

extension Character {
    fileprivate var isASCIIDigit: Bool { isASCII && isNumber }
}
