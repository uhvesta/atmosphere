import ChatUI
import ComposableArchitecture
import SwiftUI

/// The specs a session reads, each rendered, with Edit to change it.
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

    private static func kind(_ kind: SpecsFeature.Page.Kind) -> String {
        switch kind {
        case .project: "Project Instructions"
        case .task: "Task"
        case .subtask: "Subtask"
        }
    }
}
