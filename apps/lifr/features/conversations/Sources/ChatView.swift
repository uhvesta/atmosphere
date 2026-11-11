import AssistantKit
import CaptureKit
import ChatFeatures
import ChatUI
import ComposableArchitecture
import LifrAssistantClient
import LifrCore
import LifrModelsClient
import LifrSkills
import LifrUI
import QuickLook
import SwiftUI

package struct ChatView: View {
    @Bindable package var store: StoreOf<ChatFeature>

    package init(store: StoreOf<ChatFeature>) {
        self.store = store
    }

    @Environment(\.locale) private var locale

    package var body: some View {
        VStack(spacing: 0) {
            #if os(macOS)
                header
                Divider()
            #endif
            ChatThreadView(
                store: store.scope(state: \.core, action: \.core), parts: LifrParts(store: store, meanings: meanings))
        }
        .quickLookPreview($store.playing)
        .sheet(isPresented: $store.showsCheckpoints) {
            NavigationStack {
                CheckpointList(
                    store.conversation.checkpoints, canGo: store.canGo,
                    canFork: { store.state.canFork(at: $0) }, go: { store.send(.goTapped($0)) },
                    fork: { store.send(.forkTapped(messageID: $0)) }
                )
                .navigationTitle("Checkpoints")
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        DismissButton()
                    }
                }
            }
            .sheetLayout(height: 560)
        }
        .sheet(item: $store.scope(state: \.manageSkills, action: \.manageSkills)) { store in
            NavigationStack {
                SkillsView(store: store)
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) {
                            DismissButton()
                        }
                    }
            }
            .sheetLayout(height: 520)
        }
        .sheet(item: $store.scope(state: \.model, action: \.model)) { store in
            ChatModelView(store: store)
        }
        .sheet(isPresented: $store.editsDraft) {
            if let draft = store.conversation.draft {
                NavigationStack {
                    DraftEditor(
                        plan: $store.conversation.draft.or(draft), meanings: meanings, problem: store.problem
                    )
                    .navigationTitle("Edit Draft")
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { store.send(.cancelEditDraftTapped) }
                        }
                        ToolbarItem(placement: .confirmationAction) {
                            // The environment's dismiss doesn't close this
                            // sheet on the Mac.
                            Button("Done") { store.editsDraft = false }
                        }
                    }
                }
                .sheetLayout(height: 560)
            }
        }
        .sheet(item: $store.editingProposal) { edit in
            NavigationStack {
                ChangesEditor(
                    edit: $store.editingProposal.or(edit), workspace: store.workspace,
                    isBusy: store.applying == edit.id,
                    done: { store.send(.editProposalDone) })
            }
            .sheetLayout(height: 560)
        }
        .chatTitle(store.conversation.title, subtitle: subtitle)
        .toolbarTitleMenu {
            Button("Rename", systemImage: "pencil") { store.send(.renameTapped) }
        }
        #if os(iOS)
            .toolbar {
                // Compacting, finding and checkpoints are / commands.
                ToolbarItem(placement: .topBarTrailing) {
                    contextMenu
                }
            }
        #endif
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    private var meanings: DraftMeanings {
        DraftMeanings(workspace: store.workspace, today: store.today, locale: locale)
    }

    /// Where the conversation is kept, under its title.
    private var subtitle: String {
        store.conversation.isSynced
            ? String(localized: "Synced with iCloud") : String(localized: "On this device")
    }

    /// How full the model's window was at the last reply, which opens to
    /// say how much. /compact makes room.
    private var contextMenu: some View {
        ContextButton(store.core.context)
    }

    #if os(macOS)
        /// The Mac shows no title for the third column, so the chat names
        /// itself here. Renaming, finding and checkpoints are / commands.
        private var header: some View {
            HStack {
                Text(store.conversation.title)
                    .font(.headline)
                    .lineLimit(1)
                    .accessibilityAddTraits(.isHeader)
                Spacer()
                contextMenu
                    .fixedSize()
            }
            .padding(.horizontal)
            .padding(.vertical, 10)
        }
    #endif
}

/// What Lifr adds to the shared chat: why the assistant can't reply, the
/// suggestions to start with, what replies propose, the summary, the
/// draft, Compact, the / list, the skill picked, and recordings.
private struct LifrParts: ChatThreadParts {
    let store: StoreOf<ChatFeature>
    let meanings: DraftMeanings

    @ViewBuilder func top() -> some View {
        if store.availability != .available {
            Label(store.availability.description, systemImage: "exclamationmark.circle")
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder func empty() -> some View {
        if store.availability == .available {
            Suggestions(
                "Ask about your tasks, change them, or plan what to add.", suggestions: Self.suggestions
            ) { store.send(.suggestionTapped($0)) }
        }
    }

    @ViewBuilder func extra(_ message: ChatMessage) -> some View {
        if let proposal = message.proposal {
            proposal.card(
                in: store.workspace,
                canUndo: store.conversation.changes[message.id] != nil,
                isBusy: store.applying == message.id,
                apply: { store.send(.applyTapped(messageID: message.id)) },
                edit: { store.send(.editProposalTapped(messageID: message.id)) },
                decline: { store.send(.declineTapped(messageID: message.id)) },
                undo: { store.send(.undoTapped(messageID: message.id)) })
        }
    }

    @ViewBuilder func after(_ message: ChatMessage) -> some View {
        if let summary = store.conversation.summary, summary.through == message.id {
            SummaryMarker(text: summary.summary.text, canForget: !store.compacting) {
                store.send(.forgetSummaryTapped(message.id))
            }
        }
    }

    @ViewBuilder func bottom() -> some View {
        if let draft = store.conversation.draft {
            DraftCard(
                plan: draft, meanings: meanings, problem: store.problem, canChange: store.canChangeDraft,
                canAdd: store.canAdd
            ) {
                store.send(.addTapped)
            } discard: {
                store.send(.discardDraftTapped)
            } edit: {
                store.send(.editDraftTapped)
            }
        } else if store.discardedDraft != nil {
            ChatNotice("Draft discarded.", systemImage: "trash", action: "Undo") {
                store.send(.undoDiscardTapped)
            }
        }
    }

    /// Says the last reply was written without older messages, with
    /// Compact; or that they're being summarized.
    @ViewBuilder func notices() -> some View {
        if store.compacting {
            ChatNotice("Summarizing older messages…", systemImage: "text.badge.minus", isBusy: true)
            Divider()
        } else if store.state.offersCompact {
            ChatNotice(
                "Older messages no longer fit what the model reads. Compact them into a summary it keeps.",
                systemImage: "text.badge.minus", action: "Compact"
            ) { store.send(.compactTapped) }
            Divider()
        }
    }

    /// The skills to pick and the commands to run, for what's typed after
    /// a /.
    @ViewBuilder func above() -> some View {
        let choices = store.state.slashChoices
        if !choices.isEmpty {
            SlashSuggestions(choices.map { SlashSuggestion($0.id, summary: $0.summary) }) { name in
                switch choices.first(where: { $0.id == name }) {
                case .skill(let skill): store.send(.pickSkillTapped(skill.id))
                case .command(let command): store.send(.pickCommandTapped(command))
                case nil: break
                }
            }
        }
    }

    var showsChips: Bool { store.recording || !store.recordings.isEmpty }

    @ViewBuilder func chips() -> some View {
        if store.recording {
            Label("Recording…", systemImage: "record.circle")
                .foregroundStyle(.red)
                .font(.callout)
            Button("Cancel") { store.send(.cancelRecordingTapped) }
                .font(.callout)
        }
        ForEach(store.recordings) { recording in
            recording.chip(elsewhere: false) { store.send(.removeRecordingTapped(recording.id)) }
        }
    }

    @ViewBuilder func attached(_ message: ChatMessage) -> some View {
        ForEach(message.attachments.filter { $0.kind == .audio }) { recording in
            let elsewhere = store.elsewhere.contains(recording.id)
            recording.chip(elsewhere: elsewhere, open: elsewhere ? nil : { store.send(.recordingTapped(recording.id)) })
        }
    }

    /// Going back to a message and forking from it.
    @ViewBuilder func menu(_ message: ChatMessage) -> some View {
        let end = store.conversation.end(of: message.id)
        if end != store.conversation.messages.last?.id {
            Button("Go Back to Here", systemImage: "arrow.uturn.backward") { store.send(.goTapped(end)) }
                .disabled(!store.canGo)
        }
        Button("Fork From Here", systemImage: "arrow.triangle.branch") { store.send(.forkTapped(messageID: end)) }
            .disabled(!store.state.canFork(at: end))
    }

    func tag(of message: ChatMessage) -> String? { message.skill }

    /// A proposal's card offers its own Undo.
    func undo(of message: ChatMessage) -> (() -> Void)? {
        guard message.proposal == nil, store.conversation.changes[message.id] != nil else { return nil }
        return { store.send(.undoTapped(messageID: message.id)) }
    }

    var token: ChatToken? {
        store.state.picked.map { skill in
            ChatToken(name: skill.id, prompt: Text("Add details, or just send")) { store.send(.clearSkillTapped) }
        }
    }

    /// Records for a model that listens; the shared chat dictates for one
    /// that doesn't.
    var mic: MicButton? {
        guard store.inputs.contains(.audio) else { return nil }
        return MicButton(isOn: store.recording, mode: .record) { store.send(.core(.micTapped)) }
    }

    /// Ways to start, in a chat with nothing in it yet.
    private static var suggestions: [Suggestions.Suggestion] {
        [
            .init(systemImage: "calendar", text: String(localized: "Plan my week")),
            .init(systemImage: "exclamationmark.circle", text: String(localized: "What's overdue?")),
            .init(systemImage: "checkmark.circle", text: String(localized: "What did I finish this week?")),
        ]
    }
}

extension ChatAttachment {
    /// The attachment's chip: on the next message with a button to remove
    /// it, or on a sent one, saying when its file is on another device.
    fileprivate func chip(
        elsewhere: Bool, open: (() -> Void)? = nil, remove: (() -> Void)? = nil
    ) -> AttachmentChip {
        switch kind {
        case .image: AttachmentChip("Photo", systemImage: "photo", elsewhere: elsewhere, remove: remove)
        case .audio:
            AttachmentChip(
                "Recording, \(seconds ?? 0) s", systemImage: "waveform", elsewhere: elsewhere, open: open,
                remove: remove)
        case .text:
            AttachmentChip(verbatim: name ?? "File", systemImage: "doc.text", remove: remove)
        }
    }
}
