import SwiftUI

/// A reply written in Markdown, each block shown as it reads: headings,
/// lists, quotes, code with Copy, rules and tables. While the reply is
/// written, only its last block is read again as words come, and marks
/// it hasn't closed yet don't show.
struct MarkdownText: View {
    let text: String
    var isWriting = false
    @State private var stream = StreamBox()

    var body: some View {
        let blocks = stream.blocks(of: text, writing: isWriting)
        VStack(alignment: .leading, spacing: 10) {
            ForEach(blocks.indices, id: \.self) { index in
                MarkdownBlockView(block: blocks[index]).equatable()
            }
        }
    }

    /// What's been read of the reply, kept from one update to the next. It
    /// isn't observed: the text it's read from is.
    private final class StreamBox {
        private var stream = MarkdownStream()

        func blocks(of text: String, writing: Bool) -> [MarkdownBlock] {
            stream.update(text)
            guard writing, let last = stream.blocks.last else { return stream.blocks }
            return stream.blocks.dropLast() + [Markdown.inProgress(last)]
        }
    }
}

private struct MarkdownBlockView: View, Equatable {
    let block: MarkdownBlock
    /// How many lists this block is inside.
    var depth = 0

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
            CodeBlock(language: language, code: code)
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
private struct InlineText: View {
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

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                if let language {
                    Text(verbatim: language)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                }
                Spacer()
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
            }
            .padding(.horizontal, 10)
            .padding(.top, 6)
            ScrollView(.horizontal) {
                Text(CodeSyntax.colored(code, language: language))
                    .font(.system(.callout, design: .monospaced))
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 10)
                    .padding(.top, 4)
                    .padding(.bottom, 10)
                    .accessibilityLabel(language.map { Text("\($0) code") } ?? Text("Code"))
                    .accessibilityValue(code)
                    .accessibilityIdentifier(ModelText.identifier)
            }
            .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
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
