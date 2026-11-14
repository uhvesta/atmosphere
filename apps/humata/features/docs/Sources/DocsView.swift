import AppKit
import ComposableArchitecture
import HumataDaemonClient
import HumataReview
import SwiftUI

/// A chat's folder: its files, the selected one rendered or as source, and
/// the comments waiting to be sent. Focused, the file is wide, with a file
/// picker for the sidebar, and its versions can be listed beside it.
package struct DocsView: View {
    @Bindable var store: StoreOf<DocsFeature>

    package init(store: StoreOf<DocsFeature>) {
        self.store = store
    }

    package var body: some View {
        VStack(spacing: 0) {
            toolbar
            Divider()
            if store.files.isEmpty, let failure = store.listFailure {
                ContentUnavailableView {
                    Label("Couldn’t List the Files", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(failure)
                } actions: {
                    Button("Try Again") { store.send(.refresh) }
                        .accessibilityIdentifier("docs.list.retry")
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if store.files.isEmpty {
                ContentUnavailableView {
                    Label(store.loading ? "Reading the folder…" : "No files yet", systemImage: "doc.text")
                } description: {
                    Text("Files the agent writes in this chat's folder show here, to read and comment on.")
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                HStack(spacing: 0) {
                    if !store.focused {
                        sidebar
                            .frame(width: 250)
                        Divider()
                    }
                    shown
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                    if store.versions != nil {
                        Divider()
                        VersionsList(store: store)
                            .frame(width: 230)
                    }
                }
            }
        }
        .task { store.send(.refresh) }
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    /// The toolbar with its buttons' titles, or only their icons where the
    /// pane is too narrow for the titles.
    private var toolbar: some View {
        ViewThatFits(in: .horizontal) {
            toolbar(labels: .titleOnly)
            toolbar(labels: .iconOnly)
        }
        .controlSize(.small)
        .padding(.horizontal)
        .padding(.vertical, 8)
    }

    private func toolbar<Style: LabelStyle>(labels: Style) -> some View {
        HStack(spacing: 10) {
            if store.isMarkdown || store.focused {
                Toggle(isOn: Binding(get: { store.focused }, set: { _ in store.send(.focusToggled) })) {
                    Label("Focus", systemImage: "arrow.up.left.and.arrow.down.right")
                }
                .toggleStyle(.button)
                .help(
                    store.focused
                        ? "Show the folder's files beside the document again"
                        : "Read the document wide, beside the chat and its composer")
            }
            if store.focused, !store.files.isEmpty {
                Picker(
                    "File", selection: Binding(get: { store.selected ?? "" }, set: { store.send(.fileSelected($0)) })
                ) {
                    ForEach(store.files) { file in
                        Text(file.path).tag(file.path)
                    }
                }
                .labelsHidden()
                .frame(maxWidth: 240)
                .help("The file shown")
            }
            if store.isMarkdown {
                Toggle(isOn: Binding(get: { store.rendered }, set: { _ in store.send(.renderedToggled) })) {
                    Label("Rendered", systemImage: "doc.richtext")
                }
                .toggleStyle(.button)
                .help("Show the Markdown as it reads, or its source")
            }
            Button {
                store.send(.refresh)
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .accessibilityLabel("Refresh")
            .help("Read the folder again")
            if store.selected != nil {
                Toggle(isOn: Binding(get: { store.versions != nil }, set: { _ in store.send(.versionsToggled) })) {
                    Label("Versions", systemImage: "clock.arrow.circlepath")
                }
                .toggleStyle(.button)
                .help("List the versions of this file the chat's turns left")
            }
            if let versions = store.versions, versions.selected != nil {
                Toggle(isOn: Binding(get: { versions.comparing }, set: { _ in store.send(.compareToggled) })) {
                    Label("Compare with Now", systemImage: "arrow.left.arrow.right")
                }
                .toggleStyle(.button)
                .help("Show this version and the file now as one document, with what changed since marked")
                Button {
                    store.send(.restoreTapped)
                } label: {
                    Label("Restore…", systemImage: "arrow.uturn.backward")
                }
                .disabled(versions.restoring)
                .help("Write this version over the file, as a change of its own")
                if versions.restoring { ProgressView().controlSize(.small) }
            }
            Spacer()
            if store.sending {
                ProgressView()
                    .controlSize(.small)
                    .accessibilityLabel("Sending the comments")
            } else if store.loading, !store.files.isEmpty {
                ProgressView()
                    .controlSize(.small)
                    .accessibilityLabel("Reading the folder")
            }
            Button {
                store.send(.sendTapped)
            } label: {
                Label(
                    store.drafts.isEmpty ? "Send Comments" : "Send Comments (\(store.drafts.count))",
                    systemImage: "paperplane")
            }
            .buttonStyle(.borderedProminent)
            .disabled(store.drafts.isEmpty || store.sending)
            .help(
                store.drafts.isEmpty
                    ? "Click a paragraph or a line to comment on it first; the comments are sent at once"
                    : "Send every comment to the chat as one message")
        }
        .labelStyle(labels)
    }

    private var sidebar: some View {
        List(selection: Binding(get: { store.selected }, set: { $0.map { store.send(.fileSelected($0)) } })) {
            Section("Files") {
                ForEach(store.files) { file in
                    Label(file.path, systemImage: file.isMarkdown ? "doc.richtext" : "doc.text")
                        .lineLimit(1)
                        .truncationMode(.head)
                        .tag(file.path)
                }
                if store.truncated {
                    Text("Only the first \(store.files.count) files are listed.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            if !store.drafts.isEmpty {
                Section("Not Sent") {
                    ForEach(store.drafts) { draft in
                        DraftRow(draft: draft) { store.send(.draftRemoved(draft.id)) }
                    }
                }
            }
        }
        .listStyle(.sidebar)
    }

    /// The file now, or the version picked.
    @ViewBuilder private var shown: some View {
        if let versions = store.versions, let turn = versions.selected {
            VStack(spacing: 0) {
                Label(
                    "\(versions.path) as \(DocsFeature.name(turn)) left it"
                        + (versions.comparing ? ", compared with now" : ""),
                    systemImage: "clock.arrow.circlepath"
                )
                .font(.callout)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal)
                .padding(.vertical, 6)
                .background(.fill.quinary)
                Divider()
                if versions.comparing {
                    if let comparison = versions.comparison {
                        ComparedDocument(blocks: comparison)
                    } else {
                        ProgressView("Comparing…")
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                } else if let lines = versions.lines {
                    VersionFile(lines: lines, rendered: store.isMarkdown && store.rendered)
                } else {
                    ProgressView("Reading the version…")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
        } else {
            content
        }
    }

    @ViewBuilder private var content: some View {
        if let path = store.selected, let lines = store.lines {
            if store.isMarkdown && store.rendered {
                RenderedFile(store: store, path: path, lines: lines)
            } else {
                SourceFile(store: store, path: path, lines: lines)
            }
        } else if let contents = store.contents, contents.path == store.selected, contents.text == nil {
            ContentUnavailableView(
                "Not text", systemImage: "doc",
                description: Text("\(contents.path) isn't text, so it can't be shown here."))
        } else if let failure = store.readFailure {
            ContentUnavailableView {
                Label("Couldn’t Read the File", systemImage: "exclamationmark.triangle")
            } description: {
                Text(failure)
            } actions: {
                Button("Try Again") { store.send(.refresh) }
            }
        } else {
            ProgressView()
        }
    }
}

private struct RenderedFile: View {
    let store: StoreOf<DocsFeature>
    let path: String
    let lines: [String]

    var body: some View {
        let blocks = MarkdownBlock.cached(lines)
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 12) {
                ForEach(blocks) { block in
                    MarkdownBlockView(block: block)
                        .padding(.leading, 10)
                        .overlay(alignment: .leading) {
                            if drafts(block.startLine...block.endLine).isEmpty == false {
                                Rectangle().fill(Color.accentColor).frame(width: 3)
                            }
                        }
                        .padding(.leading, 6)
                        .changedMark((block.startLine...block.endLine).contains { store.changed.contains($0) })
                        .background(
                            store.picked?.covers(block.endLine, in: path) == true
                                ? Color.accentColor.opacity(0.15) : .clear, in: .rect(cornerRadius: 4)
                        )
                        .contentShape(Rectangle())
                        .onTapGesture {
                            store.send(
                                .blockTapped(startLine: block.startLine, endLine: block.endLine, quote: block.text))
                        }
                        .help("Click to comment on this")
                    ForEach(drafts(block.startLine...block.endLine)) { draft in
                        DraftNote(draft: draft) { store.send(.draftRemoved(draft.id)) }
                    }
                    if let picked = store.picked, picked.covers(block.endLine, in: path) {
                        CommentEditor(store: store)
                    }
                }
            }
            .frame(maxWidth: store.focused ? 820 : 720, alignment: .leading)
            .padding()
            .frame(maxWidth: .infinity, alignment: store.focused ? .center : .leading)
        }
    }

    private func drafts(_ lines: ClosedRange<Int>) -> [DocsFeature.Draft] {
        store.drafts.filter { $0.picked.path == path && lines.contains($0.picked.endLine) }
    }
}

private struct SourceFile: View {
    let store: StoreOf<DocsFeature>
    let path: String
    let lines: [String]

    var body: some View {
        let highlighter = Highlighter(path: path)
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(Array(lines.enumerated()), id: \.offset) { index, line in
                    let number = index + 1
                    VStack(alignment: .leading, spacing: 0) {
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Text("\(number)")
                                .foregroundStyle(.secondary)
                                .frame(width: 40, alignment: .trailing)
                            Text(highlighter?.attributed(line) ?? AttributedString(line))
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .font(.system(size: 12, design: .monospaced))
                        .padding(.vertical, 1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(
                            store.picked?.covers(number, in: path) == true
                                ? Color.accentColor.opacity(0.18)
                                : store.changed.contains(number) ? Color.orange.opacity(0.12) : .clear
                        )
                        .contentShape(Rectangle())
                        .onTapGesture {
                            store.send(
                                .lineTapped(number, extending: NSEvent.modifierFlags.contains(.shift)))
                        }
                        .help("Click to comment on this line; Shift-click to take in the lines up to it")
                        ForEach(drafts(on: number)) { draft in
                            DraftNote(draft: draft) { store.send(.draftRemoved(draft.id)) }
                                .padding(.leading, 48)
                                .padding(.vertical, 4)
                        }
                        if let picked = store.picked, picked.path == path, picked.endLine == number {
                            CommentEditor(store: store)
                                .padding(.leading, 48)
                                .padding(.vertical, 4)
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, 8)
        }
    }

    private func drafts(on line: Int) -> [DocsFeature.Draft] {
        store.drafts.filter { $0.picked.path == path && $0.picked.endLine == line }
    }
}

/// A version of a file, to read: rendered when it's Markdown, else its source.
private struct VersionFile: View {
    let lines: [String]
    let rendered: Bool

    var body: some View {
        ScrollView {
            if rendered {
                LazyVStack(alignment: .leading, spacing: 12) {
                    ForEach(MarkdownBlock.cached(lines)) { block in
                        MarkdownBlockView(block: block)
                            .padding(.leading, 16)
                    }
                }
                .frame(maxWidth: 820, alignment: .leading)
                .padding()
                .frame(maxWidth: .infinity)
            } else {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(lines.enumerated()), id: \.offset) { index, line in
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Text("\(index + 1)")
                                .foregroundStyle(.secondary)
                                .frame(width: 40, alignment: .trailing)
                            Text(line)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .font(.system(size: 12, design: .monospaced))
                        .padding(.vertical, 1)
                    }
                }
                .padding(.vertical, 8)
            }
        }
        .textSelection(.enabled)
    }
}

/// The selected file's versions, newest first, with the file now on top.
private struct VersionsList: View {
    let store: StoreOf<DocsFeature>

    /// Stands for the file now, which isn't a version.
    private static let now = -1

    var body: some View {
        let versions = store.versions
        List(
            selection: Binding(
                get: { versions?.selected ?? Self.now },
                set: { store.send(.versionPicked($0 == Self.now ? nil : $0)) })
        ) {
            Section("Versions") {
                Text("Now").tag(Self.now)
                if let list = versions?.list {
                    ForEach(list.reversed()) { version in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(version.turn == 0 ? "Before the first turn" : "After turn \(version.turn)")
                            Text(version.time, format: .dateTime.month().day().hour().minute())
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        .tag(version.turn)
                    }
                    if list.isEmpty {
                        Text(
                            "No versions yet. Turns are snapshotted in a project's repositories, once each turn "
                                + "ends; a file outside them, or in a chat with no repository, has none."
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }
                } else {
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.small)
                        Text("Reading the versions…")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
        .listStyle(.sidebar)
    }
}

/// The comment being written, with the text it's on, which can be cut down
/// to the words meant.
private struct CommentEditor: View {
    @Bindable var store: StoreOf<DocsFeature>

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let picked = store.picked {
                Text(
                    picked.startLine == picked.endLine
                        ? "Comment on line \(picked.startLine)"
                        : "Comment on lines \(picked.startLine)–\(picked.endLine)"
                )
                .font(.caption.bold())
            }
            TextField(
                "The words you mean",
                text: Binding(get: { store.picked?.quote ?? "" }, set: { store.picked?.quote = $0 }),
                axis: .vertical
            )
            .lineLimit(1...4)
            .textFieldStyle(.roundedBorder)
            .font(.callout.italic())
            .accessibilityLabel("Quote")
            .help("What's quoted with the comment; cut it down to the words the comment is about")
            TextField("What should change here?", text: $store.comment, axis: .vertical)
                .lineLimit(3...8)
                .textFieldStyle(.roundedBorder)
                .accessibilityLabel("Comment")
            HStack {
                Text("It waits with your other comments until you send them.")
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

/// A comment not sent yet, where it's on.
private struct DraftNote: View {
    let draft: DocsFeature.Draft
    let remove: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "text.bubble")
                .foregroundStyle(Color.accentColor)
            VStack(alignment: .leading, spacing: 4) {
                if !draft.picked.quote.isEmpty {
                    Text(draft.picked.quote)
                        .font(.caption.italic())
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                Text(draft.text)
                    .font(.callout)
                Text("Not sent")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button(action: remove) {
                Image(systemName: "trash")
            }
            .buttonStyle(.borderless)
            .accessibilityLabel("Remove Comment")
            .help("Remove this comment")
        }
        .padding(8)
        .frame(maxWidth: 640, alignment: .leading)
        .background(Color.accentColor.opacity(0.08), in: .rect(cornerRadius: 8))
    }
}

/// A comment not sent yet, in the sidebar.
private struct DraftRow: View {
    let draft: DocsFeature.Draft
    let remove: () -> Void

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(draft.text)
                    .lineLimit(1)
                Text("\(draft.picked.path):\(draft.picked.startLine)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            Button(action: remove) {
                Image(systemName: "xmark.circle.fill")
            }
            .buttonStyle(.borderless)
            .foregroundStyle(.secondary)
            .accessibilityLabel("Remove Comment")
            .help("Remove this comment")
        }
    }
}
