import AppKit
import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// A session's changes to review: the range to read, the files by
/// repository, and the selected file's diff, whole or rendered, with its
/// threads on their lines.
package struct ReviewView: View {
    @Bindable var store: StoreOf<ReviewFeature>

    package init(store: StoreOf<ReviewFeature>) {
        self.store = store
    }

    package var body: some View {
        VStack(spacing: 0) {
            toolbar
            Divider()
            if store.files.isEmpty {
                ContentUnavailableView {
                    Label(
                        store.loading ? "Reading the changes…" : "No changes",
                        systemImage: "doc.text.magnifyingglass")
                } description: {
                    Text(empty)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                HStack(spacing: 0) {
                    files
                        .frame(width: 250)
                    Divider()
                    content
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
        }
        .task { store.send(.refresh) }
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    private var empty: String {
        switch store.range {
        case .session: "The session hasn't changed any code yet."
        case .unstaged: "Nothing in the session's copy is waiting to be staged."
        case .staged: "Nothing is staged. Stage a file or a change from Unstaged."
        case .branch(let branch): "The session's copy has the same files as \(branch)."
        case .turn(let turn): "Turn \(turn) didn't change any code."
        }
    }

    // MARK: Toolbar

    private var toolbar: some View {
        HStack(spacing: 10) {
            Picker("Changes", selection: Binding(get: { store.rangeKind }, set: { store.send(.rangePicked($0)) })) {
                ForEach(ReviewFeature.RangeKind.allCases, id: \.self) { kind in
                    Text(Self.name(kind)).tag(kind)
                }
            }
            .pickerStyle(.menu)
            .fixedSize()
            .help("Which of the session's changes to review")
            if case .turn(let turn) = store.range {
                Picker("Turn", selection: Binding(get: { turn }, set: { store.send(.turnPicked($0)) })) {
                    ForEach(1...max(store.turns, turn, 1), id: \.self) { turn in
                        Text("Turn \(turn)").tag(turn)
                    }
                }
                .labelsHidden()
                .pickerStyle(.menu)
                .fixedSize()
            }
            if case .branch = store.range {
                TextField("Branch", text: $store.branch)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 140)
                    .onSubmit { store.send(.branchSubmitted) }
                    .help("The branch to compare with; press Return to read the changes")
            }
            HStack(spacing: 2) {
                Button {
                    store.send(.contextChanged(by: -3))
                } label: {
                    Image(systemName: "arrow.down.right.and.arrow.up.left")
                }
                .help("Show fewer unchanged lines around each change")
                .disabled(store.context == 0)
                Text("\(store.context) lines")
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .frame(minWidth: 52)
                Button {
                    store.send(.contextChanged(by: 3))
                } label: {
                    Image(systemName: "arrow.up.left.and.arrow.down.right")
                }
                .help("Show more unchanged lines around each change")
            }
            .disabled(store.wholeFile || store.rendered)
            Toggle(
                "Whole File", isOn: Binding(get: { store.wholeFile }, set: { _ in store.send(.wholeFileToggled) })
            )
            .toggleStyle(.button)
            .help("Show every line of the file, its changes marked")
            if store.isMarkdown {
                Toggle(
                    "Rendered", isOn: Binding(get: { store.rendered }, set: { _ in store.send(.renderedToggled) })
                )
                .toggleStyle(.button)
                .help("Show the Markdown as it reads, and comment on its paragraphs")
            }
            Toggle("Resolved", isOn: $store.showsResolved)
                .toggleStyle(.button)
                .help("Show resolved threads too")
            Spacer()
            Button(store.pending == 0 ? "Send Review" : "Send Review (\(store.pending))") {
                store.send(.sendTapped)
            }
            .buttonStyle(.borderedProminent)
            .disabled(store.pending == 0 || store.sending)
            .help(
                store.pending == 0
                    ? "Leave a comment on a line first; the review sends every comment at once"
                    : "Send every comment as one message; the agent answers each thread")
        }
        .controlSize(.small)
        .padding(.horizontal)
        .padding(.vertical, 8)
    }

    private static func name(_ kind: ReviewFeature.RangeKind) -> String {
        switch kind {
        case .session: "Everything"
        case .unstaged: "Unstaged"
        case .staged: "Staged"
        case .branch: "Against a Branch"
        case .turn: "One Turn"
        }
    }

    // MARK: Files

    private var files: some View {
        List(selection: Binding(get: { store.selected }, set: { $0.map { store.send(.fileSelected($0)) } })) {
            ForEach(store.repositories.filter { !$0.files.isEmpty }, id: \.name) { repository in
                Section(store.repositories.count > 1 ? repository.name : "Files") {
                    ForEach(repository.files, id: \.path) { file in
                        fileRow(file, FileRef(repository: repository.name, path: file.path))
                            .tag(FileRef(repository: repository.name, path: file.path))
                    }
                }
            }
        }
        .listStyle(.sidebar)
    }

    private func fileRow(_ file: FileDiff, _ ref: FileRef) -> some View {
        HStack(spacing: 6) {
            Text(Self.letter(file.status))
                .font(.caption.monospaced().bold())
                .foregroundStyle(Self.color(file.status))
                .frame(width: 12)
            Text(file.path)
                .lineLimit(1)
                .truncationMode(.middle)
                .foregroundStyle(file.viewed ? .secondary : .primary)
            Spacer(minLength: 4)
            let threads = store.state.shownThreads(on: ref).count
            if threads > 0 {
                Label("\(threads)", systemImage: "text.bubble")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .labelStyle(.titleAndIcon)
            }
            Button {
                store.send(.viewedToggled(ref))
            } label: {
                Image(systemName: file.viewed ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(file.viewed ? Color.accentColor : .secondary)
            }
            .buttonStyle(.plain)
            .disabled(file.blob.isEmpty)
            .help(file.viewed ? "Viewed; it stays so until the file changes" : "Mark viewed")
            .accessibilityLabel(file.viewed ? "Viewed" : "Mark viewed")
        }
    }

    private static func letter(_ status: FileStatus) -> String {
        switch status {
        case .added: "A"
        case .deleted: "D"
        case .modified: "M"
        case .renamed: "R"
        case .unchanged: "="
        }
    }

    private static func color(_ status: FileStatus) -> Color {
        switch status {
        case .added: .green
        case .deleted: .red
        case .modified, .renamed: .orange
        case .unchanged: .secondary
        }
    }

    // MARK: The file

    @ViewBuilder
    private var content: some View {
        if let ref = store.selected, let file = store.file {
            VStack(spacing: 0) {
                fileHeader(file, ref)
                Divider()
                if file.binary {
                    ContentUnavailableView(
                        "A binary file", systemImage: "doc",
                        description: Text("Its changes can't be shown as lines."))
                } else if store.rendered, store.isMarkdown {
                    RenderedMarkdown(store: store, file: file, ref: ref)
                } else {
                    DiffLines(store: store, file: file, ref: ref)
                }
            }
        } else {
            ContentUnavailableView("Pick a file", systemImage: "doc.text", description: Text("Its changes show here."))
        }
    }

    private func fileHeader(_ file: FileDiff, _ ref: FileRef) -> some View {
        HStack(spacing: 8) {
            Text(file.oldPath.map { "\($0) → \(file.path)" } ?? file.path)
                .font(.callout.monospaced())
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer()
            switch store.range {
            case .unstaged:
                Button("Stage File") { store.send(.editTapped(ChangeEdit(file: ref, action: .stage))) }
                    .help("Stage every change to this file")
                Button("Revert File…", role: .destructive) {
                    store.send(.editTapped(ChangeEdit(file: ref, action: .revert)))
                }
                .help("Throw away this file's unstaged changes")
            case .staged:
                Button("Unstage File") { store.send(.editTapped(ChangeEdit(file: ref, action: .unstage))) }
                    .help("Unstage this file, keeping its changes in the copy")
            case .session, .branch, .turn:
                EmptyView()
            }
        }
        .controlSize(.small)
        .padding(.horizontal)
        .padding(.vertical, 6)
    }
}

/// The file's hunks, line by line, with threads under the lines they end on.
private struct DiffLines: View {
    let store: StoreOf<ReviewFeature>
    let file: FileDiff
    let ref: FileRef

    var body: some View {
        let highlighter = Highlighter(path: file.path)
        let threads = store.state.shownThreads(on: ref)
        let shown = Set(file.hunks.flatMap(\.lines).compactMap(\.new))
        let inline = store.canComment ? threads.filter { shown.contains($0.endLine) } : []
        let elsewhere = threads.filter { thread in !inline.contains { $0.id == thread.id } }
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                if !store.canComment {
                    HStack {
                        Text("Comments go on Everything, Unstaged or Against a Branch.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Button("Show Everything") { store.send(.rangePicked(.session)) }
                            .buttonStyle(.link)
                            .controlSize(.small)
                            .help("Show every change this session made, where lines can be commented on")
                    }
                    .padding(.horizontal)
                    .padding(.vertical, 6)
                }
                if !elsewhere.isEmpty {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(store.canComment ? "On lines not shown here" : "Threads on this file")
                            .font(.caption.bold())
                            .foregroundStyle(.secondary)
                        ForEach(elsewhere) { thread in
                            ThreadView(store: store, thread: thread, showsCode: true)
                        }
                    }
                    .padding()
                }
                ForEach(Array(file.hunks.enumerated()), id: \.offset) { _, hunk in
                    // One view per hunk, so its first line's identity
                    // doesn't collide with its header's in the lazy stack.
                    VStack(alignment: .leading, spacing: 0) {
                        hunkHeader(hunk)
                        ForEach(Array(hunk.lines.enumerated()), id: \.offset) { _, line in
                            LineRow(
                                line: line, highlighter: highlighter, commentable: store.canComment,
                                picked: line.new.map { store.commenting?.covers($0, in: ref) ?? false } ?? false
                            )
                            .onTapGesture {
                                guard let new = line.new else { return }
                                store.send(.lineTapped(new, extending: NSEvent.modifierFlags.contains(.shift)))
                            }
                            if let new = line.new {
                                ForEach(inline.filter { $0.endLine == new }) { thread in
                                    ThreadView(store: store, thread: thread, showsCode: thread.outdated)
                                        .padding(.leading, 110)
                                        .padding([.trailing, .vertical], 8)
                                }
                                if let picked = store.commenting, picked.file == ref, picked.endLine == new {
                                    CommentEditor(store: store, commenting: picked)
                                        .padding(.leading, 110)
                                        .padding([.trailing, .vertical], 8)
                                }
                            }
                        }
                    }
                }
            }
            .padding(.bottom)
        }
    }

    private func hunkHeader(_ hunk: Hunk) -> some View {
        HStack {
            Text("@@ -\(hunk.oldStart),\(hunk.oldCount) +\(hunk.newStart),\(hunk.newCount) @@ \(hunk.header)")
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
            Spacer()
            let at = HunkAt(oldStart: hunk.oldStart, newStart: hunk.newStart, context: store.context)
            switch store.range {
            case .unstaged where !store.wholeFile:
                Button("Stage") { store.send(.editTapped(ChangeEdit(file: ref, action: .stage, hunk: at))) }
                    .help("Stage this change")
                Button("Revert…") { store.send(.editTapped(ChangeEdit(file: ref, action: .revert, hunk: at))) }
                    .help("Throw away this change")
            case .staged where !store.wholeFile:
                Button("Unstage") { store.send(.editTapped(ChangeEdit(file: ref, action: .unstage, hunk: at))) }
                    .help("Unstage this change, keeping it in the copy")
            default:
                EmptyView()
            }
        }
        .buttonStyle(.borderless)
        .controlSize(.small)
        .padding(.horizontal)
        .padding(.vertical, 4)
        .background(Color.accentColor.opacity(0.06))
    }
}

/// One line of a diff: its numbers before and after, and its text coloured.
private struct LineRow: View {
    let line: DiffLine
    let highlighter: Highlighter?
    let commentable: Bool
    let picked: Bool

    var body: some View {
        HStack(spacing: 0) {
            Text(line.old.map(String.init) ?? "")
                .frame(width: 44, alignment: .trailing)
                .foregroundStyle(.tertiary)
            Text(line.new.map(String.init) ?? "")
                .frame(width: 44, alignment: .trailing)
                .foregroundStyle(.tertiary)
            Text(marker)
                .frame(width: 22)
                .foregroundStyle(.secondary)
            Text(highlighted)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
        }
        .font(.system(size: 12, design: .monospaced))
        .padding(.vertical, 1)
        .background(background)
        .contentShape(Rectangle())
        .help(line.new == nil || !commentable ? "" : "Click to comment on this line; Shift-click to comment on several")
    }

    private var marker: String {
        switch line.kind {
        case .added: "+"
        case .removed: "−"
        case .context: ""
        }
    }

    private var background: Color {
        if picked { return Color.accentColor.opacity(0.22) }
        switch line.kind {
        case .added: return Color.green.opacity(0.16)
        case .removed: return Color.red.opacity(0.16)
        case .context: return .clear
        }
    }

    private var highlighted: AttributedString {
        highlighter?.attributed(line.text) ?? AttributedString(line.text)
    }
}

extension Highlighter {
    /// `line` coloured by its tokens.
    package func attributed(_ line: String) -> AttributedString {
        var text = AttributedString()
        for token in tokens(line) {
            var part = AttributedString(token.text)
            if let color = Self.color(token.kind) { part.foregroundColor = color }
            text += part
        }
        return text
    }

    private static func color(_ kind: Kind) -> Color? {
        switch kind {
        case .plain: nil
        case .keyword: Color(nsColor: .systemPurple)
        case .string: Color(nsColor: .systemRed)
        case .comment: Color(nsColor: .systemGreen)
        case .number: Color(nsColor: .systemBlue)
        case .type: Color(nsColor: .systemTeal)
        }
    }
}

/// A Markdown file as it reads, block by block; a click on a block comments
/// on it.
private struct RenderedMarkdown: View {
    let store: StoreOf<ReviewFeature>
    let file: FileDiff
    let ref: FileRef

    var body: some View {
        let lines = file.hunks.flatMap(\.lines).filter { $0.new != nil }
        let added = Set(lines.filter { $0.kind == .added }.compactMap(\.new))
        let blocks = MarkdownBlock.blocks(lines.map(\.text))
        let threads = store.state.shownThreads(on: ref)
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                ForEach(blocks) { block in
                    MarkdownBlockView(block: block)
                        .padding(.leading, 10)
                        .overlay(alignment: .leading) {
                            if (block.startLine...block.endLine).contains(where: added.contains) {
                                Rectangle().fill(.green).frame(width: 3)
                            }
                        }
                        .background(
                            store.commenting?.covers(block.endLine, in: ref) == true
                                ? Color.accentColor.opacity(0.15) : .clear, in: .rect(cornerRadius: 4)
                        )
                        .contentShape(Rectangle())
                        .onTapGesture {
                            store.send(
                                .blockTapped(
                                    startLine: block.startLine, endLine: block.endLine,
                                    quote: String(block.text.prefix(200))))
                        }
                        .help("Click to comment on this")
                    ForEach(threads.filter { (block.startLine...block.endLine).contains($0.endLine) }) { thread in
                        ThreadView(store: store, thread: thread, showsCode: false)
                    }
                    if let picked = store.commenting, picked.file == ref,
                        (block.startLine...block.endLine).contains(picked.endLine)
                    {
                        CommentEditor(store: store, commenting: picked)
                    }
                }
            }
            .frame(maxWidth: 720, alignment: .leading)
            .padding()
        }
    }
}

/// A Markdown block as it reads.
package struct MarkdownBlockView: View {
    let block: MarkdownBlock

    package init(block: MarkdownBlock) {
        self.block = block
    }

    package var body: some View {
        switch block.kind {
        case .heading(let level):
            Text(Self.inline(block.text))
                .font(level == 1 ? .title : level == 2 ? .title2 : .title3)
                .bold()
        case .paragraph:
            Text(Self.inline(block.text))
                .frame(maxWidth: .infinity, alignment: .leading)
        case .code:
            Text(block.text)
                .font(.system(size: 12, design: .monospaced))
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(8)
                .background(.fill.quaternary, in: .rect(cornerRadius: 6))
        }
    }

    private static func inline(_ text: String) -> AttributedString {
        inlineMarkdown(text)
    }
}

/// `text` with its inline Markdown, such as emphasis and code, rendered.
private func inlineMarkdown(_ text: String) -> AttributedString {
    let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
    return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
}

/// A thread: where it is, its comments, the agent's answers, a reply and
/// whether it's resolved.
private struct ThreadView: View {
    let store: StoreOf<ReviewFeature>
    let thread: ReviewThread
    /// Shows the lines it was left on, for a thread away from them.
    let showsCode: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Text(
                    thread.startLine == thread.endLine
                        ? "\(thread.file.path):\(thread.startLine)"
                        : "\(thread.file.path):\(thread.startLine)–\(thread.endLine)"
                )
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
                if thread.outdated {
                    Text("Outdated")
                        .font(.caption2.bold())
                        .padding(.horizontal, 5)
                        .background(.orange.opacity(0.2), in: .capsule)
                        .help("The lines it was left on have changed")
                }
                if thread.resolved {
                    Text("Resolved")
                        .font(.caption2.bold())
                        .padding(.horizontal, 5)
                        .background(.green.opacity(0.2), in: .capsule)
                }
                Spacer()
                if thread.comments.contains(where: { $0.turn != nil }) {
                    Button("Show Fix") { store.send(.showFixTapped(thread.id)) }
                        .help("Show what the agent changed in the turn it answered this")
                }
                Button(thread.resolved ? "Reopen" : "Resolve") { store.send(.resolveToggled(thread.id)) }
            }
            .buttonStyle(.borderless)
            .controlSize(.small)
            if showsCode, !thread.code.isEmpty {
                Text(thread.code.joined(separator: "\n"))
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(6)
                    .background(.fill.quaternary, in: .rect(cornerRadius: 4))
            }
            if !thread.quote.isEmpty {
                Text("“\(thread.quote)”")
                    .font(.callout.italic())
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
            }
            ForEach(thread.comments) { comment in
                CommentRow(store: store, comment: comment)
            }
            if !thread.resolved {
                HStack {
                    TextField(
                        "Reply",
                        text: Binding(
                            get: { store.replies[thread.id] ?? "" },
                            set: { store.send(.replyChanged(thread.id, $0)) }),
                        axis: .vertical
                    )
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { store.send(.replySent(thread.id)) }
                    Button("Reply") { store.send(.replySent(thread.id)) }
                        .disabled((store.replies[thread.id] ?? "").trimmingCharacters(in: .whitespaces).isEmpty)
                }
                .controlSize(.small)
            }
        }
        .padding(10)
        .frame(maxWidth: 640, alignment: .leading)
        .background(.background, in: .rect(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.separator))
        .font(.system(size: 13))
    }
}

private struct CommentRow: View {
    let store: StoreOf<ReviewFeature>
    let comment: ReviewComment

    private var author: (name: String, symbol: String) {
        switch comment.author {
        case .you: ("You", "person.fill")
        case .agent: ("Agent", "sparkles")
        case .github(let login): ("@\(login) on GitHub", "person.2")
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Label(author.name, systemImage: author.symbol)
                    .font(.caption.bold())
                if let turn = comment.turn {
                    Text("turn \(turn)")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                if comment.author != .agent, !comment.sent {
                    Text("Not sent")
                        .font(.caption2)
                        .foregroundStyle(.orange)
                }
                if comment.author == .you, !comment.sent {
                    Button("Delete") { store.send(.deleteTapped(comment.id)) }
                        .buttonStyle(.borderless)
                        .controlSize(.small)
                }
            }
            Text(inlineMarkdown(comment.text))
                .textSelection(.enabled)
        }
    }
}

/// The comment being written on the picked lines.
private struct CommentEditor: View {
    @Bindable var store: StoreOf<ReviewFeature>
    let commenting: ReviewFeature.Commenting

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(
                commenting.startLine == commenting.endLine
                    ? "Comment on line \(commenting.startLine)"
                    : "Comment on lines \(commenting.startLine)–\(commenting.endLine)"
            )
            .font(.caption.bold())
            TextField("What should change here?", text: $store.comment, axis: .vertical)
                .lineLimit(3...8)
                .textFieldStyle(.roundedBorder)
                .accessibilityLabel("Comment")
            HStack {
                Text("It's sent with the review, with your other comments.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Cancel") { store.send(.commentCancelled) }
                Button("Add Comment") { store.send(.commentSaved) }
                    .buttonStyle(.borderedProminent)
                    .disabled(store.comment.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .controlSize(.small)
        }
        .padding(10)
        .frame(maxWidth: 640, alignment: .leading)
        .background(.background, in: .rect(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color.accentColor))
    }
}

extension ReviewFeature.Commenting {
    fileprivate func covers(_ line: Int, in ref: FileRef) -> Bool {
        file == ref && (startLine...endLine).contains(line)
    }
}
