import ChatUI
import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// A task or spec edit the agent proposed, in the chat where it proposed
/// it: its title and spec, then Apply, Edit and Decline until it's decided,
/// and for an applied task, a way to start work on it.
package struct ProposalCard: View {
    let proposal: Proposal
    /// What a spec edit takes out of its item's spec and puts in; nil until
    /// it has been worked out, and for a task.
    let change: ToolCall.Change?
    let deciding: Bool
    /// Whether work on an applied task can start from here: only from a
    /// planning session.
    let canStartWork: Bool
    let send: (ProposalsFeature.Action) -> Void
    @State private var showsAll = false
    @Environment(\.rowResizing) private var resizing
    /// Lines of the spec shown until the person asks for all of them.
    private static let shownLines = 8

    package init(
        proposal: Proposal, change: ToolCall.Change?, deciding: Bool, canStartWork: Bool,
        send: @escaping (ProposalsFeature.Action) -> Void
    ) {
        self.proposal = proposal
        self.change = change
        self.deciding = deciding
        self.canStartWork = canStartWork
        self.send = send
    }

    package var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Label(kind, systemImage: proposal.kind == .task ? "checklist" : "doc.text")
                    .scaledFont(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                Spacer()
                status
            }
            Text(proposal.title)
                .scaledFont(.headline)
                .textSelection(.enabled)
            if proposal.currentSpec != nil, !proposal.spec.isEmpty {
                // The change comes with the proposals' list, worked out off
                // the main thread; until it does, that it's on its way.
                if let change {
                    ChangeView(change: change)
                } else {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityLabel("Working out what the edit changes")
                }
            } else if !proposal.spec.isEmpty {
                Text(proposal.spec)
                    .scaledFont(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(showsAll ? nil : Self.shownLines)
                    .textSelection(.enabled)
                if TextPreview.lines(in: proposal.spec, atMost: Self.shownLines + 1) > Self.shownLines {
                    Button(showsAll ? "Show Less" : "Show All") {
                        resizing?()
                        showsAll.toggle()
                    }
                    .buttonStyle(.link)
                    .scaledFont(.caption)
                }
            }
            if !proposal.note.isEmpty {
                Text(proposal.note)
                    .scaledFont(.caption)
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
                .scaledFont(.caption)
                .foregroundStyle(.secondary)
        case .applied:
            Label("Applied", systemImage: "checkmark")
                .scaledFont(.caption)
                .foregroundStyle(.green)
        case .declined:
            Label("Declined", systemImage: "xmark")
                .scaledFont(.caption)
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
        if let editing = store.editing {
            VStack(alignment: .leading, spacing: 12) {
                Text("Edit the Proposal")
                    .scaledFont(.headline)
                TextField("Title", text: binding(\.title))
                Text("Spec")
                    .scaledFont(.caption)
                    .foregroundStyle(.secondary)
                TextEditor(text: binding(\.spec))
                    .scaledFont(.body.monospaced())
                    .frame(minHeight: 200)
                HStack {
                    Spacer()
                    Button("Cancel", role: .cancel) { store.send(.editCancelled) }
                        .keyboardShortcut(.cancelAction)
                    Button("Apply") { store.send(.editApplied) }
                        .buttonStyle(.borderedProminent)
                        .keyboardShortcut(.defaultAction)
                        .disabled(store.deciding.contains(editing.id))
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
