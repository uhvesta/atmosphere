import SwiftUI

/// A reply written in Markdown, each block shown as it reads: headings,
/// lists, quotes, code with Copy, rules and tables. While the reply is
/// written, only its last block is read again as words come, and marks
/// it hasn't closed yet don't show.
public struct MarkdownText: View {
    let text: String
    var isWriting = false
    @State private var stream = StreamBox()

    /// A finished piece of Markdown, such as a spec.
    public init(_ text: String) {
        self.text = text
    }

    /// A long piece of Markdown cut where one block ends and the next
    /// begins, each part at least `size` bytes but the last, so a long
    /// reply can be rows of a lazy list and only those on screen are laid
    /// out. Each part reads as the same blocks as it did in the whole. Text
    /// under `size` is one part; finding the cuts reads each line once, and
    /// finds its end by byte, as a reply can be megabytes. A line ends at a
    /// line feed, but not at one after a carriage return, which is one
    /// character with it.
    ///
    /// It touches no view, so a chat's rows can be worked out off the main
    /// thread.
    public nonisolated static func parts(of text: String, size: Int = 2_000) -> [Substring] {
        guard text.utf8.count > size else { return [text[...]] }
        var parts: [Substring] = []
        var start = text.startIndex
        var fence: Substring?
        var blank = false
        var index = text.startIndex
        let utf8 = text.utf8
        let lineFeed = UInt8(ascii: "\n")
        let carriageReturn = UInt8(ascii: "\r")
        while index < text.endIndex {
            var end = index
            var last: UInt8 = 0
            while end < utf8.endIndex {
                let byte = utf8[end]
                if byte == lineFeed, last != carriageReturn { break }
                last = byte
                utf8.formIndex(after: &end)
            }
            let line = text[index..<end]
            let trimmed = line.drop { $0 == " " }
            if let open = fence {
                if trimmed.hasPrefix(open) { fence = nil }
            } else if trimmed.hasPrefix("```") || trimmed.hasPrefix("~~~") {
                if blank, Self.startsBlock(line), text.utf8.distance(from: start, to: index) >= size {
                    parts.append(text[start..<index])
                    start = index
                }
                fence = trimmed.prefix(3)
            } else if line.allSatisfy(\.isWhitespace) {
                blank = true
                index = end < utf8.endIndex ? utf8.index(after: end) : end
                continue
            } else if blank, Self.startsBlock(line), text.utf8.distance(from: start, to: index) >= size {
                parts.append(text[start..<index])
                start = index
            }
            blank = false
            index = end < utf8.endIndex ? utf8.index(after: end) : end
        }
        parts.append(text[start...])
        return parts
    }

    /// Whether `line`, after a blank one, starts a block of its own rather
    /// than going on with the one before: not indented, as a list item's
    /// next paragraph is, and not an item of a list that goes on.
    private nonisolated static func startsBlock(_ line: Substring) -> Bool {
        guard let first = line.first, first != " ", first != "\t" else { return false }
        if "-*+".contains(first), line.dropFirst().first == " " { return false }
        let digits = line.prefix { $0.isASCII && $0.isNumber }
        if !digits.isEmpty, let mark = line.dropFirst(digits.count).first, mark == "." || mark == ")" { return false }
        return true
    }

    init(text: String, isWriting: Bool = false) {
        self.text = text
        self.isWriting = isWriting
    }

    public var body: some View {
        let blocks = stream.blocks(of: text, writing: isWriting)
        VStack(alignment: .leading, spacing: 10) {
            ForEach(blocks.indices, id: \.self) { index in
                MarkdownBlockView(block: blocks[index]).equatable()
                    .environment(\.markdownUnfinished, isWriting && index == blocks.count - 1)
            }
        }
    }

    /// What's been read of the reply, kept from one update to the next. It
    /// isn't observed: the text it's read from is.
    @MainActor private final class StreamBox {
        private var stream = MarkdownStream()

        func blocks(of text: String, writing: Bool) -> [MarkdownBlock] {
            // A finished text read lately isn't read again, as when its row
            // scrolls away and back and this box starts over.
            if !writing, let kept = Self.finished.object(forKey: text as NSString) { return kept.value }
            stream.update(text)
            guard writing, let last = stream.blocks.last else {
                if !writing {
                    Self.finished.setObject(Kept(stream.blocks), forKey: text as NSString, cost: text.utf8.count * 4)
                }
                return stream.blocks
            }
            return stream.blocks.dropLast() + [Markdown.inProgress(last)]
        }

        /// About as many replies as a few screens show, so a long chat
        /// scrolled through doesn't keep all of it.
        @MainActor private static let finished: NSCache<NSString, Kept<[MarkdownBlock]>> = {
            let finished = NSCache<NSString, Kept<[MarkdownBlock]>>()
            finished.countLimit = 200
            // About 4 bytes kept for each byte of Markdown read.
            finished.totalCostLimit = 20 << 20
            return finished
        }()
    }
}

/// Draws fenced code of some languages as something else, such as a
/// diagram; nil for code it doesn't draw, which shows as code.
public struct CodeBlockRenderer: Sendable {
    let render: @MainActor @Sendable (_ language: String, _ code: String) -> AnyView?

    public init(_ render: @escaping @MainActor @Sendable (_ language: String, _ code: String) -> AnyView?) {
        self.render = render
    }
}

/// Puts code from a reply into the composer as a quote.
public struct CodeQuoter: Sendable {
    let quote: @MainActor @Sendable (_ language: String?, _ code: String) -> Void

    public init(_ quote: @escaping @MainActor @Sendable (_ language: String?, _ code: String) -> Void) {
        self.quote = quote
    }
}

extension EnvironmentValues {
    /// How fenced code is drawn where it isn't shown as code.
    @Entry public var codeBlockRenderer: CodeBlockRenderer?
    /// Where a code block's Quote puts it; with none, there's no Quote.
    @Entry public var codeQuoter: CodeQuoter?
    /// A block still being written, so drawn as it reads rather than by a
    /// renderer, which would redraw with every word.
    @Entry var markdownUnfinished = false
    /// Tells the list a row is in that the row is about to change its own
    /// height, such as opening, so it's measured again rather than kept at
    /// the height it had.
    @Entry public var rowResizing: RowResizing?
}

/// What a row calls before it changes its own height. Two are the same
/// for the same row of the same list, so a list setting one each time it
/// draws doesn't draw its rows again.
public struct RowResizing: Equatable, Sendable {
    let row: String
    let list: ObjectIdentifier
    let forget: @MainActor @Sendable (String) -> Void

    public init(row: String, list: ObjectIdentifier, forget: @escaping @MainActor @Sendable (String) -> Void) {
        self.row = row
        self.list = list
        self.forget = forget
    }

    @MainActor public func callAsFunction() { forget(row) }

    public static func == (lhs: Self, rhs: Self) -> Bool { lhs.row == rhs.row && lhs.list == rhs.list }
}

private struct MarkdownBlockView: View, Equatable {
    let block: MarkdownBlock
    /// How many lists this block is inside.
    var depth = 0
    @Environment(\.codeBlockRenderer) private var renderer
    @Environment(\.markdownUnfinished) private var unfinished

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.block == rhs.block && lhs.depth == rhs.depth
    }

    var body: some View {
        switch block {
        case .paragraph(let text):
            InlineText(text)
        case .heading(let level, let text):
            InlineText(text)
                .font(Self.font(level: level))
                .padding(.top, level <= 2 ? 4 : 2)
                .accessibilityAddTraits(.isHeader)
        case .code(let language, let code):
            if !unfinished, let language, let drawn = renderer?.render(language, code) {
                drawn
            } else {
                CodeBlock(language: language, code: code)
            }
        case .quote(let blocks):
            VStack(alignment: .leading, spacing: 8) {
                ForEach(blocks.indices, id: \.self) { MarkdownBlockView(block: blocks[$0], depth: depth) }
            }
            .foregroundStyle(.secondary)
            .padding(.leading, 12)
            .overlay(alignment: .leading) {
                RoundedRectangle(cornerRadius: 1.5).fill(.quaternary).frame(width: 3)
            }
        case .list(let list):
            ListBlock(list: list, depth: depth)
        case .rule:
            Divider().padding(.vertical, 4)
        case .table(let table):
            TableBlock(table: table)
        case .math(let latex):
            Text(Math.styled(latex))
                .font(.system(.title3, design: .serif))
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 2)
                .accessibilityIdentifier(ModelText.identifier)
        }
    }

    /// Sized for a message rather than a page.
    private static func font(level: Int) -> Font {
        switch level {
        case 1: .title2.weight(.semibold)
        case 2: .title3.weight(.semibold)
        case 3: .headline
        default: .subheadline.weight(.semibold)
        }
    }
}

/// Inline Markdown in a run of text, with code on a tint of its own.
struct InlineText: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(Self.cachedStyled(text))
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityIdentifier(ModelText.identifier)
    }

    static func styled(_ text: String) -> AttributedString {
        var string = Markdown.inline(text)
        let code = string.runs.compactMap { run in
            run.inlinePresentationIntent?.contains(.code) == true ? run.range : nil
        }
        for range in code { string[range].backgroundColor = Color.secondary.opacity(0.15) }
        return string
    }

    /// `styled(text)`, kept for the texts drawn lately, as a message is
    /// drawn again each time its row comes back into view.
    @MainActor static func cachedStyled(_ text: String) -> AttributedString {
        let key = text as NSString
        if let kept = styledTexts.object(forKey: key) { return kept.value }
        let string = styled(text)
        styledTexts.setObject(Kept(string), forKey: key, cost: text.utf8.count * 8)
        return string
    }

    /// About as many paragraphs as a few screens show.
    @MainActor private static let styledTexts: NSCache<NSString, Kept<AttributedString>> = {
        let styled = NSCache<NSString, Kept<AttributedString>>()
        styled.countLimit = 2_000
        // An attributed string keeps about 8 bytes for each byte of text.
        styled.totalCostLimit = 30 << 20
        return styled
    }()
}

/// A value an `NSCache` can hold.
final class Kept<Value> {
    let value: Value

    init(_ value: Value) {
        self.value = value
    }
}

private struct ListBlock: View {
    let list: MarkdownList
    let depth: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(list.items.indices, id: \.self) { index in
                let item = list.items[index]
                VStack(alignment: .leading, spacing: 6) {
                    // The marker reads with the item's first line; what's
                    // under it, such as a list of its own, reads on its own.
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        marker(at: index)
                        if let first = item.first { MarkdownBlockView(block: first, depth: depth + 1) }
                    }
                    .accessibilityElement(children: item.first.map(Self.reads) ?? true ? .combine : .contain)
                    ForEach(item.indices.dropFirst(), id: \.self) { block in
                        HStack(alignment: .top, spacing: 6) {
                            marker(at: index).hidden()
                            MarkdownBlockView(block: item[block], depth: depth + 1)
                        }
                    }
                }
            }
        }
    }

    /// As wide as the widest marker, so the items line up; a task's box
    /// for a task.
    @ViewBuilder private func marker(at index: Int) -> some View {
        if index < list.checks.count, let checked = list.checks[index] {
            Image(systemName: checked ? "checkmark.square.fill" : "square")
                .foregroundStyle(checked ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                .accessibilityLabel(checked ? Text("Done") : Text("Not done"))
        } else {
            textMarker(at: index)
        }
    }

    private func textMarker(at index: Int) -> some View {
        ZStack(alignment: .trailing) {
            Text(verbatim: marker(list.items.count - 1)).hidden()
            Text(verbatim: marker(index))
        }
        .monospacedDigit()
        .foregroundStyle(.secondary)
        .accessibilityHidden(list.start == nil)
    }

    /// Whether a block reads as one line of text with the marker.
    private static func reads(_ block: MarkdownBlock) -> Bool {
        switch block {
        case .paragraph, .heading: true
        default: false
        }
    }

    private func marker(_ index: Int) -> String {
        guard let start = list.start else { return ["•", "◦", "▪"][depth % 3] }
        return "\(start + index)."
    }
}

/// Fenced code: monospaced on a tint, scrolling sideways rather than
/// wrapping, with its language and Copy.
struct CodeBlock: View {
    let language: String?
    let code: String
    @State private var copied = false
    @Environment(\.codeQuoter) private var quoter
    /// Code longer than this many lines scrolls in a box of its own.
    private static let boxedLines = 40

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                if let language {
                    Text(verbatim: CodeSyntax.name(of: language))
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                }
                Spacer()
                if let quoter {
                    Button {
                        quoter.quote(language, code)
                    } label: {
                        Label("Quote", systemImage: "text.quote")
                    }
                    .font(.caption)
                    .buttonStyle(CodeBlockButtonStyle())
                    .help("Put this code in the message box as a quote")
                    .accessibilityLabel(Text("Quote Code in Message"))
                }
                Button {
                    copyToClipboard(code)
                    copied = true
                } label: {
                    if copied {
                        Label("Copied", systemImage: "checkmark")
                    } else {
                        Label("Copy", systemImage: "doc.on.doc")
                    }
                }
                .font(.caption)
                .buttonStyle(CodeBlockButtonStyle())
                .accessibilityLabel(copied ? Text("Copied") : Text("Copy Code"))
                .sensoryFeedback(.success, trigger: copied) { _, copied in copied }
            }
            .padding(.horizontal, 10)
            .padding(.top, 6)
            let preview = TextPreview(code, lines: Self.boxedLines, tallBytes: .max)
            let text = Text(CodeSyntax.cachedColored(preview.shown, language: language))
                .font(.system(.callout, design: .monospaced))
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 10)
                .padding(.top, 4)
                .padding(.bottom, 10)
                .accessibilityLabel(language.map { Text("\(CodeSyntax.name(of: $0)) code") } ?? Text("Code"))
                .accessibilityValue(preview.shown)
                .accessibilityIdentifier(ModelText.identifier)
            if preview.isTall {
                // Long code scrolls in a box of its own, so the reply
                // around it stays quick to lay out.
                ScrollView([.horizontal, .vertical]) { text }
                    .frame(height: 480)
            } else if Self.isWide(preview.shown) {
                ScrollView(.horizontal) { text }
                    .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
            } else {
                // No scroll view for code that fits: one each made a long
                // reply slow to scroll into view.
                text
            }
            if preview.isCut {
                Text("Only the first 20,000 characters show here; Copy takes all of it.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 10)
                    .padding(.bottom, 8)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.fill.quaternary, in: .rect(cornerRadius: 10))
        .task(id: copied) {
            guard copied else { return }
            try? await Task.sleep(for: .seconds(2))
            copied = false
        }
    }

    /// Whether a line of `code` may be too long to fit a message's width.
    static func isWide(_ code: String) -> Bool {
        code.utf8.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: false).contains {
            $0.count > 80
        }
    }
}

/// Copy and Quote above code, drawn by SwiftUI: a borderless button on
/// the Mac is an AppKit button, and measuring one for each block of code
/// held up a reply's first frame.
private struct CodeBlockButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        Hover { hovering in
            configuration.label
                .foregroundStyle(hovering || configuration.isPressed ? .primary : .secondary)
                .opacity(configuration.isPressed ? 0.6 : 1)
                .padding(.horizontal, 4)
                .padding(.vertical, 2)
                .contentShape(.rect)
        }
    }

    private struct Hover<Label: View>: View {
        @ViewBuilder let label: (Bool) -> Label
        @State private var hovering = false

        var body: some View {
            label(hovering).onHover { hovering = $0 }
        }
    }
}

/// A table, scrolling sideways when it's wider than the message, its
/// columns aligned as the delimiter row says.
struct TableBlock: View {
    let table: MarkdownTable

    var body: some View {
        if Self.isWide(table) {
            ScrollView(.horizontal) { grid }
                .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
        } else {
            // No scroll view for a table that fits, as for code.
            grid
        }
    }

    /// Whether `table` may be wider than a message: many columns, or long
    /// cells.
    static func isWide(_ table: MarkdownTable) -> Bool {
        let columns = ([table.header] + table.rows).map(\.count).max() ?? 0
        guard columns <= 5 else { return true }
        let widest = (0..<columns).map { column in
            ([table.header] + table.rows).map { column < $0.count ? $0[column].utf8.count : 0 }.max() ?? 0
        }
        return widest.reduce(0, +) > 70
    }

    private var grid: some View {
        Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
            GridRow {
                ForEach(table.header.indices, id: \.self) { column in
                    InlineText(table.header[column])
                        .fontWeight(.semibold)
                        .gridColumnAlignment(alignment(column))
                }
            }
            ForEach(table.rows.indices, id: \.self) { row in
                Divider().gridCellUnsizedAxes(.horizontal)
                GridRow {
                    ForEach(table.rows[row].indices, id: \.self) { column in
                        InlineText(table.rows[row][column])
                    }
                }
            }
        }
        .padding(.vertical, 2)
    }

    private func alignment(_ column: Int) -> HorizontalAlignment {
        switch column < table.alignments.count ? table.alignments[column] : .leading {
        case .leading: .leading
        case .center: .center
        case .trailing: .trailing
        }
    }
}
