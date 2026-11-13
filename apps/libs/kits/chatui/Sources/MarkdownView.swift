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
                if !writing { Self.finished.setObject(Kept(stream.blocks), forKey: text as NSString) }
                return stream.blocks
            }
            return stream.blocks.dropLast() + [Markdown.inProgress(last)]
        }

        @MainActor private static let finished = NSCache<NSString, Kept<[MarkdownBlock]>>()
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
        Text(Self.styled(text))
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
        styledTexts.setObject(Kept(string), forKey: key)
        return string
    }

    @MainActor private static let styledTexts = NSCache<NSString, Kept<AttributedString>>()
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
private struct CodeBlock: View {
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
                    .buttonStyle(.borderless)
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
                .buttonStyle(.borderless)
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
            } else {
                ScrollView(.horizontal) { text }
                    .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
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
}

/// A table, scrolling sideways when it's wider than the message, its
/// columns aligned as the delimiter row says.
private struct TableBlock: View {
    let table: MarkdownTable

    var body: some View {
        ScrollView(.horizontal) {
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
        .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
    }

    private func alignment(_ column: Int) -> HorizontalAlignment {
        switch column < table.alignments.count ? table.alignments[column] : .leading {
        case .leading: .leading
        case .center: .center
        case .trailing: .trailing
        }
    }
}
