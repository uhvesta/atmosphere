import ChatUI
import ComposableArchitecture
import HumataDaemonClient
import HumataReview
import SwiftUI

/// The specs a session reads, each rendered, with Edit to change it, and
/// Focus to read one wide beside the chat, with its versions.
struct SpecsView: View {
    @Bindable var store: StoreOf<SpecsFeature>

    var body: some View {
        Group {
            if store.loading {
                ProgressView("Reading the specs…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if store.pages.isEmpty {
                ContentUnavailableView(
                    "No Specs", systemImage: "doc.text",
                    description: Text("This session reads no project instructions or task spec."))
            } else if let id = store.focused, let page = store.pages[id: id] {
                focused(page)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 24) {
                        ForEach(store.pages) { page in
                            section(page)
                        }
                    }
                    .padding()
                    .frame(maxWidth: 760, alignment: .leading)
                    .frame(maxWidth: .infinity)
                }
            }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    @ViewBuilder
    private func section(_ page: SpecsFeature.Page) -> some View {
        let editing = store.draft?.pageID == page.id
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(Self.kind(page.kind))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text(page.title).font(.title3.bold())
                }
                Spacer()
                if !editing {
                    Button("Focus") { store.send(.focusTapped(page.id)) }
                        .help("Read this spec wide, beside the chat and its composer, with its versions")
                    if page.kind != .project {
                        Button("Open on the Board") { store.send(.openTapped(page.id)) }
                            .buttonStyle(.link)
                    }
                    Button("Edit") { store.send(.editTapped(page.id)) }
                        .disabled(store.draft != nil)
                }
            }
            if editing, let draft = store.draft {
                editor(draft, of: page.kind)
            } else if page.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text(page.kind == .project ? "No instructions yet." : "No spec yet.")
                    .foregroundStyle(.secondary)
            } else {
                MarkdownText(page.text).textSelection(.enabled)
            }
        }
    }

    @ViewBuilder
    private func editor(_ draft: SpecsFeature.Draft, of kind: SpecsFeature.Page.Kind) -> some View {
        Picker("Spec", selection: Binding(get: { draft.previewing }, set: { store.send(.previewToggled($0)) })) {
            Text("Write").tag(false)
            Text("Preview").tag(true)
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .fixedSize()
        if draft.previewing {
            Group {
                if draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text("Nothing to preview yet.").foregroundStyle(.secondary)
                } else {
                    MarkdownText(draft.text).textSelection(.enabled)
                }
            }
            .frame(maxWidth: .infinity, minHeight: 200, alignment: .topLeading)
        } else {
            TextEditor(text: Binding(get: { draft.text }, set: { store.send(.draftChanged($0)) }))
                .font(.body.monospaced())
                .frame(minHeight: 200)
                .scrollContentBackground(.hidden)
                .padding(6)
                .background(.fill.quaternary, in: .rect(cornerRadius: 8))
                .accessibilityLabel("Spec")
        }
        HStack {
            Text(
                kind == .project
                    ? "Saved to the project; sessions started from now on read it."
                    : "Saved to the board; the session is told, as for any spec edit."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            Spacer()
            if draft.saving { ProgressView().controlSize(.small) }
            Button("Cancel") { store.send(.cancelTapped) }
                .disabled(draft.saving)
            Button("Save") { store.send(.saveTapped) }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut("s", modifiers: .command)
                .disabled(draft.saving)
        }
    }

    /// One spec wide: the spec now with what an edit changed marked, or a
    /// version of it, on its own or compared with now.
    @ViewBuilder
    private func focused(_ page: SpecsFeature.Page) -> some View {
        let editing = store.draft?.pageID == page.id
        VStack(spacing: 0) {
            focusToolbar(page, editing: editing)
            Divider()
            HStack(spacing: 0) {
                Group {
                    if editing, let draft = store.draft {
                        ScrollView {
                            VStack(alignment: .leading, spacing: 8) {
                                editor(draft, of: page.kind)
                            }
                            .padding()
                        }
                    } else if let versions = store.versions, let version = versions.shown {
                        shownVersion(version, of: versions)
                    } else if page.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        ContentUnavailableView(
                            page.kind == .project ? "No instructions yet" : "No spec yet", systemImage: "doc.text",
                            description: Text("Click Edit to write it."))
                    } else {
                        ScrollView {
                            LazyVStack(alignment: .leading, spacing: 12) {
                                ForEach(MarkdownBlock.cached(page.text)) { block in
                                    MarkdownBlockView(block: block)
                                        .padding(.leading, 16)
                                        .changedMark(
                                            (block.startLine...block.endLine).contains { store.changed.contains($0) })
                                }
                            }
                            .frame(maxWidth: 820, alignment: .leading)
                            .padding()
                            .frame(maxWidth: .infinity)
                        }
                        .textSelection(.enabled)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                if let versions = store.versions {
                    Divider()
                    versionsList(versions, of: page)
                        .frame(width: 230)
                }
            }
        }
    }

    private func focusToolbar(_ page: SpecsFeature.Page, editing: Bool) -> some View {
        HStack(spacing: 10) {
            Toggle("Focus", isOn: Binding(get: { true }, set: { _ in store.send(.focusTapped(nil)) }))
                .toggleStyle(.button)
                .help("Show every spec this session reads again")
            if store.pages.count > 1 {
                Picker(
                    "Spec", selection: Binding(get: { page.id }, set: { store.send(.focusTapped($0)) })
                ) {
                    ForEach(store.pages) { page in
                        Text("\(Self.kind(page.kind)): \(page.title)").tag(page.id)
                    }
                }
                .labelsHidden()
                .frame(maxWidth: 260)
                .disabled(store.draft != nil)
                .help("The spec shown")
            }
            Toggle(
                "Versions", isOn: Binding(get: { store.versions != nil }, set: { _ in store.send(.versionsToggled) })
            )
            .toggleStyle(.button)
            .disabled(editing)
            .help("List the versions of this spec the board kept")
            if let versions = store.versions, versions.shown != nil {
                Toggle(
                    "Compare with Now",
                    isOn: Binding(get: { versions.comparing }, set: { _ in store.send(.compareToggled) })
                )
                .toggleStyle(.button)
                .help("Show this version and the spec now as one document, with what changed since marked")
                Button("Restore…") { store.send(.restoreTapped) }
                    .disabled(versions.restoring)
                    .help("Save this version over the spec, as a change of its own")
                if versions.restoring { ProgressView().controlSize(.small) }
            }
            Spacer()
            if !editing {
                if page.kind != .project {
                    Button("Open on the Board") { store.send(.openTapped(page.id)) }
                        .buttonStyle(.link)
                }
                Button("Edit") { store.send(.editTapped(page.id)) }
                    .disabled(store.draft != nil || store.versions?.shown != nil)
            }
        }
        .controlSize(.small)
        .padding(.horizontal)
        .padding(.vertical, 8)
    }

    @ViewBuilder
    private func shownVersion(_ version: SpecsFeature.Version, of versions: SpecsFeature.Versions) -> some View {
        VStack(spacing: 0) {
            Label {
                Text(
                    "As saved \(version.time.formatted(.dateTime.month().day().hour().minute()))"
                        + (versions.comparing ? ", compared with now" : ""))
            } icon: {
                Image(systemName: "clock.arrow.circlepath")
            }
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
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 12) {
                        ForEach(MarkdownBlock.cached(version.text)) { block in
                            MarkdownBlockView(block: block)
                                .padding(.leading, 16)
                        }
                    }
                    .frame(maxWidth: 820, alignment: .leading)
                    .padding()
                    .frame(maxWidth: .infinity)
                }
                .textSelection(.enabled)
            }
        }
    }

    /// Stands for the spec now, which isn't a version.
    private static let now: UInt64 = .max

    private func versionsList(_ versions: SpecsFeature.Versions, of page: SpecsFeature.Page) -> some View {
        List(
            selection: Binding(
                get: { versions.selected ?? Self.now },
                set: { store.send(.versionPicked($0 == Self.now ? nil : $0)) })
        ) {
            Section("Versions") {
                Text("Now").tag(Self.now)
                if let list = versions.list {
                    ForEach(list) { version in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(version.time, format: .dateTime.month().day().hour().minute())
                            if let author = Self.author(version.author) {
                                Text(author)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .tag(version.id)
                    }
                    if list.isEmpty {
                        Text(
                            page.kind == .project
                                ? "Project instructions aren't kept on the board, so they have no versions."
                                : "No versions yet: each save of this spec to the board is one."
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

    private static func author(_ author: BoardAuthor) -> String? {
        switch author {
        case .app: "By you"
        case .commandLine: "By you, with the humata command"
        case .mcp: "By an outside agent"
        case .humata: "By Humata"
        case .session: "By a session, approved by you"
        case .unknown: nil
        }
    }

    private static func kind(_ kind: SpecsFeature.Page.Kind) -> String {
        switch kind {
        case .project: "Project Instructions"
        case .task: "Task"
        case .subtask: "Subtask"
        }
    }
}
