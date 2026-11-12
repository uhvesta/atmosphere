import AppKit
import ComposableArchitecture
import HumataDaemonClient
import HumataReview
import SwiftUI

/// A folder's tree on the left, a level read each time one is opened, and
/// the picked file on the right.
package struct FileBrowserView: View {
    let store: StoreOf<FileBrowserFeature>
    @State private var rendered = true

    package init(store: StoreOf<FileBrowserFeature>) {
        self.store = store
    }

    package var body: some View {
        VStack(spacing: 0) {
            toolbar
            Divider()
            HStack(spacing: 0) {
                tree
                    .frame(width: 260)
                Divider()
                content
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .overlay(alignment: .bottom) {
            if let failure = store.failure {
                Label(failure, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .padding(8)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
                    .padding()
            }
        }
        .task { store.send(.task) }
    }

    private var isMarkdown: Bool {
        guard let path = store.selected else { return false }
        return ["md", "markdown"].contains((path as NSString).pathExtension.lowercased())
    }

    private var toolbar: some View {
        HStack(spacing: 10) {
            Text(store.absolutePath ?? store.root ?? "")
                .font(.callout.monospaced())
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.head)
                .textSelection(.enabled)
            Spacer()
            if isMarkdown {
                Toggle("Rendered", isOn: $rendered)
                    .toggleStyle(.button)
                    .help("Show the Markdown as it reads, or its source")
            }
            if let path = store.absolutePath {
                if store.place.host == nil {
                    Button("Open in Editor") {
                        NSWorkspace.shared.open(URL(fileURLWithPath: path))
                    }
                    .help("Open the file in the app this Mac opens it with")
                }
                Button("Copy Path") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(path, forType: .string)
                }
                .help("Copy where the file is on its host")
            }
            Button {
                store.send(.refreshTapped)
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .help("Read the open folders and the file again")
            .accessibilityLabel("Refresh")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    @ViewBuilder private var tree: some View {
        if store.root == nil {
            ProgressView()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if store.folders[""]?.isEmpty ?? true {
            ContentUnavailableView("Empty Folder", systemImage: "folder")
        } else {
            List {
                ForEach(store.rows) { row in
                    TreeRow(row: row, expanded: store.expanded.contains(row.path), selected: row.path == store.selected)
                        .contentShape(Rectangle())
                        .onTapGesture {
                            store.send(row.entry.isDirectory ? .folderToggled(row.path) : .fileSelected(row.path))
                        }
                    if row.entry.isDirectory, store.expanded.contains(row.path), store.cut.contains(row.path) {
                        cutNote(depth: row.depth + 1)
                    }
                }
                if store.cut.contains("") { cutNote(depth: 0) }
            }
            .listStyle(.sidebar)
        }
    }

    private func cutNote(depth: Int) -> some View {
        Text("More files than can be listed; open it in a terminal to see them all.")
            .font(.caption)
            .foregroundStyle(.secondary)
            .padding(.leading, CGFloat(depth) * 14 + 20)
    }

    @ViewBuilder private var content: some View {
        if let path = store.selected {
            if let contents = store.contents, contents.path == path {
                FileContentsView(contents: contents, rendered: rendered && isMarkdown)
            } else {
                ProgressView()
            }
        } else {
            ContentUnavailableView(
                "No File Selected", systemImage: "doc",
                description: Text("Pick a file to read it here."))
        }
    }
}

private struct TreeRow: View {
    let row: FileBrowserFeature.Row
    let expanded: Bool
    let selected: Bool

    var body: some View {
        HStack(spacing: 4) {
            Group {
                if row.entry.isDirectory {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                } else {
                    Color.clear
                }
            }
            .font(.caption2)
            .frame(width: 12)
            .foregroundStyle(.secondary)
            Label(row.entry.name, systemImage: row.entry.isDirectory ? "folder" : "doc")
                .lineLimit(1)
            Spacer()
        }
        .padding(.leading, CGFloat(row.depth) * 14)
        .padding(.vertical, 1)
        .background(selected ? Color.accentColor.opacity(0.18) : .clear, in: RoundedRectangle(cornerRadius: 4))
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
    }
}

/// A file as text with its lines numbered, rendered Markdown, an image, or
/// a note when it's none of those.
private struct FileContentsView: View {
    let contents: FileContents
    let rendered: Bool

    var body: some View {
        if let text = contents.text {
            let lines = text.components(separatedBy: "\n")
            VStack(spacing: 0) {
                if contents.truncated {
                    Label(
                        "Only the start of this \(sizeText) file is shown; open it in a terminal to read the rest.",
                        systemImage: "scissors"
                    )
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    Divider()
                }
                if rendered {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 12) {
                            ForEach(MarkdownBlock.blocks(lines)) { block in
                                MarkdownBlockView(block: block)
                            }
                        }
                        .frame(maxWidth: 720, alignment: .leading)
                        .padding()
                    }
                } else {
                    SourceText(path: contents.path, lines: lines)
                }
            }
        } else if let data = contents.data, let image = NSImage(data: data) {
            ScrollView([.horizontal, .vertical]) {
                Image(nsImage: image)
                    .padding()
            }
            .defaultScrollAnchor(.topLeading)
        } else {
            ContentUnavailableView(
                "Not Text", systemImage: "doc",
                description: Text("\(contents.path) (\(sizeText)) isn't text or an image small enough to show."))
        }
    }

    private var sizeText: String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: contents.size), countStyle: .file)
    }
}

private struct SourceText: View {
    let path: String
    let lines: [String]

    var body: some View {
        let highlighter = Highlighter(path: path)
        ScrollView([.horizontal, .vertical]) {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(Array(lines.enumerated()), id: \.offset) { index, line in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text("\(index + 1)")
                            .foregroundStyle(.secondary)
                            .frame(width: 48, alignment: .trailing)
                        Text(highlighter?.attributed(line) ?? AttributedString(line))
                            .fixedSize()
                    }
                    .font(.system(size: 12, design: .monospaced))
                    .padding(.vertical, 1)
                }
            }
            .padding(.vertical, 8)
            .textSelection(.enabled)
        }
        // A short file starts at the top left, not in the middle.
        .defaultScrollAnchor(.topLeading)
    }
}
