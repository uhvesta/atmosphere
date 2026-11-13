import ChatUI
import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// A task or spec edit the agent proposed, in the chat where it proposed
/// it: its title and spec, then Apply, Edit and Decline until it's decided,
/// and for an applied task, a way to start work on it.
package struct ProposalCard: View {
    let proposal: Proposal
    let deciding: Bool
    /// Whether work on an applied task can start from here: only from a
    /// planning session.
    let canStartWork: Bool
    let send: (ProposalsFeature.Action) -> Void
    @State private var showsAll = false
    /// Bumped when a change worked out off the main thread is kept, to draw
    /// it.
    @State private var diffs = 0
    /// Lines of the spec shown until the person asks for all of them.
    private static let shownLines = 8

    package init(
        proposal: Proposal, deciding: Bool, canStartWork: Bool,
        send: @escaping (ProposalsFeature.Action) -> Void
    ) {
        self.proposal = proposal
        self.deciding = deciding
        self.canStartWork = canStartWork
        self.send = send
    }

    package var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Label(kind, systemImage: proposal.kind == .task ? "checklist" : "doc.text")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                Spacer()
                status
            }
            Text(proposal.title)
                .font(.headline)
                .textSelection(.enabled)
            if let current = proposal.currentSpec, !proposal.spec.isEmpty {
                // Read so a change kept off the main thread draws.
                let _ = diffs
                if let change = Self.keptChange(proposal.id, current, proposal.spec) {
                    ChangeView(change: change)
                } else {
                    ProgressView()
                        .controlSize(.small)
                        .task(id: Diffed(old: current, new: proposal.spec)) {
                            await diff(current, proposal.spec)
                        }
                }
            } else if !proposal.spec.isEmpty {
                Text(proposal.spec)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(showsAll ? nil : Self.shownLines)
                    .textSelection(.enabled)
                if TextPreview.lines(in: proposal.spec, atMost: Self.shownLines + 1) > Self.shownLines {
                    Button(showsAll ? "Show Less" : "Show All") { showsAll.toggle() }
                        .buttonStyle(.link)
                        .font(.caption)
                }
            }
            if !proposal.note.isEmpty {
                Text(proposal.note)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
            actions
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.blue.opacity(proposal.state == .pending ? 0.08 : 0.03), in: .rect(cornerRadius: 8))
        .overlay {
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(.blue.opacity(proposal.state == .pending ? 0.3 : 0.1))
        }
    }

    /// The texts a change was worked out between, to work it out again
    /// when either changes.
    private struct Diffed: Equatable {
        let old: String
        let new: String
    }

    /// The change worked out for a proposal, with the texts it was worked
    /// out between.
    private final class KeptChange {
        let old: String
        let new: String
        let change: ToolCall.Change

        init(old: String, new: String, change: ToolCall.Change) {
            self.old = old
            self.new = new
            self.change = change
        }
    }

    /// The changes worked out lately, by proposal, as a card draws again
    /// with every change around it.
    @MainActor private static let changes = NSCache<NSString, KeptChange>()

    /// The change kept for proposal `id`, if it was worked out between
    /// these texts.
    @MainActor private static func keptChange(_ id: String, _ old: String, _ new: String) -> ToolCall.Change? {
        guard let kept = changes.object(forKey: id as NSString), kept.old == old, kept.new == new else { return nil }
        return kept.change
    }

    /// Works out the change off the main thread, as a long spec's diff
    /// would stall the chat, then keeps it and draws it.
    private func diff(_ old: String, _ new: String) async {
        let change = await Task.detached(priority: .userInitiated) { ToolCall.Change.between(old, new) }.value
        guard !Task.isCancelled else { return }
        Self.changes.setObject(KeptChange(old: old, new: new, change: change), forKey: proposal.id as NSString)
        diffs += 1
    }

    private var kind: String {
        switch proposal.kind {
        case .task: proposal.parentID == nil ? "Proposed task" : "Proposed subtask"
        case .specEdit: proposal.spec.isEmpty ? "Proposed new title" : "Proposed spec edit"
        }
    }

    @ViewBuilder
    private var status: some View {
        switch proposal.state {
        case .pending:
            Text("Not on the board yet")
                .font(.caption)
                .foregroundStyle(.secondary)
        case .applied:
            Label("Applied", systemImage: "checkmark")
                .font(.caption)
                .foregroundStyle(.green)
        case .declined:
            Label("Declined", systemImage: "xmark")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var actions: some View {
        switch proposal.state {
        case .pending:
            HStack(spacing: 8) {
                Spacer()
                Button("Decline") { send(.declineTapped(proposal.id)) }
                Button("Edit…") { send(.editTapped(proposal.id)) }
                Button("Apply") { send(.applyTapped(proposal.id)) }
                    .buttonStyle(.borderedProminent)
            }
            .disabled(deciding)
        case .applied(let item):
            if canStartWork, proposal.kind == .task {
                HStack {
                    Spacer()
                    if deciding {
                        ProgressView().controlSize(.small)
                    }
                    Button(deciding ? "Starting Work…" : "Start Work on It") {
                        send(.startWorkTapped(itemID: item))
                    }
                    .disabled(deciding)
                    .help("A new session on this task, with this chat and its own copy of the code")
                }
            }
        case .declined:
            EmptyView()
        }
    }
}

/// A proposal open to change before it's applied.
package struct ProposalEditor: View {
    @Bindable var store: StoreOf<ProposalsFeature>

    package init(store: StoreOf<ProposalsFeature>) {
        self.store = store
    }

    package var body: some View {
        if store.editing != nil {
            VStack(alignment: .leading, spacing: 12) {
                Text("Edit the Proposal")
                    .font(.headline)
                TextField("Title", text: binding(\.title))
                Text("Spec")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextEditor(text: binding(\.spec))
                    .font(.body.monospaced())
                    .frame(minHeight: 200)
                HStack {
                    Spacer()
                    Button("Cancel", role: .cancel) { store.send(.editCancelled) }
                        .keyboardShortcut(.cancelAction)
                    Button("Apply") { store.send(.editApplied) }
                        .buttonStyle(.borderedProminent)
                        .keyboardShortcut(.defaultAction)
                }
            }
            .padding()
            .frame(minWidth: 480)
        }
    }

    private func binding(_ field: WritableKeyPath<ProposalsFeature.Editing, String>) -> Binding<String> {
        Binding(
            get: { store.editing?[keyPath: field] ?? "" },
            set: { value in
                guard var editing = store.editing else { return }
                editing[keyPath: field] = value
                store.editing = editing
            })
    }
}
