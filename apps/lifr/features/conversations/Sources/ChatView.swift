import AssistantKit
import ChatUI
import ComposableArchitecture
import LifrAssistantClient
import LifrCore
import LifrModelsClient
import LifrSkills
import LifrUI
import PhotosUI
import SwiftUI

package struct ChatView: View {
    @Bindable package var store: StoreOf<ChatFeature>

    package init(store: StoreOf<ChatFeature>) {
        self.store = store
    }

    @Environment(\.locale) private var locale
    @State private var photo: PhotosPickerItem?
    @State private var selecting: TextToSelect?
    @State private var picksPhoto = false

    package var body: some View {
        VStack(spacing: 0) {
            #if os(macOS)
                header
                Divider()
            #endif
            messages
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            VStack(spacing: 0) {
                if store.finds {
                    findBar
                        .padding()
                } else {
                    if store.conversation.forward != nil {
                        rewound
                        Divider()
                    }
                    if store.compacting || store.state.offersCompact {
                        compactBanner
                        Divider()
                    }
                    composer
                        .padding()
                }
            }
            .background(.bar)
        }
        .sheet(item: $selecting) { selecting in
            SelectTextSheet(text: selecting.text)
        }
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
        #if os(iOS)
            .sheet(isPresented: $store.findsSkill) {
                skillFinder
            }
        #endif
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
                        ToolbarItem(placement: .confirmationAction) {
                            DismissButton()
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
                // The ring and one menu share a capsule, as more buttons
                // fold into the bar's overflow.
                ToolbarItemGroup(placement: .topBarTrailing) {
                    contextMenu
                    Menu("More", systemImage: "ellipsis") {
                        findButton
                        compactButton
                        Section("Checkpoints") { checkpointButtons }
                    }
                    .disabled(store.conversation.messages.isEmpty)
                }
            }
        #endif
        .task { await store.send(.task).finish() }
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    private var meanings: DraftMeanings {
        DraftMeanings(workspace: store.workspace, today: store.today, locale: locale)
    }

    private var messages: some View {
        ScrollViewReader { proxy in
            scroll
                .onChange(of: store.state.currentMatch) { _, id in
                    guard let id else { return }
                    withAnimation { proxy.scrollTo(id, anchor: .center) }
                }
        }
    }

    /// Where the conversation is kept, under its title.
    private var subtitle: String {
        store.conversation.isSynced
            ? String(localized: "Synced with iCloud") : String(localized: "On this device")
    }

    /// How full the model's window was at the last reply, which opens to
    /// say how much and to compact.
    private var contextMenu: some View {
        Menu {
            Text(ContextRing.description(of: store.context))
            compactButton
        } label: {
            ContextRing(store.context)
        }
        .menuIndicator(.hidden)
        .help(ContextRing.description(of: store.context))
    }

    /// Opens what the conversation runs on: its model and how long it
    /// thinks, also before its first message.
    private var modelPill: ModelPill {
        let current = store.state.current
        return ModelPill(
            name: current?.name ?? String(localized: "Model"),
            detail: current?.thinks == true ? store.conversation.context.model.thinkingLevel.title : nil,
            pick: { store.send(.modelTapped) })
    }

    /// Opens the bar to find words in the messages shown.
    private var findButton: some View {
        Button("Find in Conversation", systemImage: "magnifyingglass") { store.send(.findTapped) }
            .keyboardShortcut("f")
            .disabled(store.conversation.messages.isEmpty)
            .help("Find in Conversation")
    }

    /// Finds words in the messages shown, one match at a time.
    private var findBar: some View {
        FindBar(
            query: $store.findQuery, matches: store.state.findMatches.count, position: store.findPosition,
            move: { store.send(.findMoved(by: $0)) }, close: { store.send(.findClosed) })
    }

    private var scroll: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if store.availability != .available {
                    Label(store.availability.description, systemImage: "exclamationmark.circle")
                        .foregroundStyle(.secondary)
                }
                if store.conversation.messages.isEmpty, store.availability == .available {
                    Suggestions(
                        "Ask about your tasks, change them, or plan what to add.", suggestions: Self.suggestions
                    ) { store.send(.suggestionTapped($0)) }
                }
                ForEach(store.conversation.messages) { message in
                    VStack(alignment: .trailing, spacing: 4) {
                        MessageView(
                            role: message.role, text: message.text, thinking: message.thinking,
                            skill: store.state.skillTitle(of: message),
                            // A proposal's card offers its own Undo.
                            canUndo: message.proposal == nil && store.conversation.changes[message.id] != nil,
                            showsThinking: store.shownThinking.contains(message.id),
                            toggleThinking: { store.send(.thinkingToggled(messageID: message.id)) },
                            undo: { store.send(.undoTapped(messageID: message.id)) },
                            select: { selecting = TextToSelect(text: $0) },
                            attachments: {
                                ForEach(message.attachments) { attachment in
                                    attachment.chip(elsewhere: store.elsewhere.contains(attachment.id))
                                }
                            }
                        )
                        .contextMenu { messageMenu(for: message) }
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
                        branches(at: message)
                        if let summary = store.conversation.summary, summary.through == message.id {
                            SummaryMarker(text: summary.summary.text, canForget: !store.compacting) {
                                store.send(.forgetSummaryTapped(message.id))
                            }
                        }
                        if message.id == store.conversation.messages.last?.id, store.replying == nil {
                            after(message)
                        }
                    }
                    .background {
                        if store.finds, store.state.findMatches.contains(message.id) {
                            RoundedRectangle(cornerRadius: 12)
                                .fill(.yellow.opacity(message.id == store.state.currentMatch ? 0.35 : 0.12))
                                .padding(-6)
                        }
                    }
                    .id(message.id)
                }
                if let replying = store.replying {
                    ReplyInProgress(
                        text: replying, thinking: store.replyingThinking, showsThinking: store.showsReplyThinking,
                        toggleThinking: { store.send(.thinkingToggled(messageID: nil)) })
                }
                if let draft = store.conversation.draft {
                    DraftCard(
                        plan: draft, meanings: meanings, problem: store.problem, canAdd: store.canAdd
                    ) {
                        store.send(.addTapped)
                    } discard: {
                        store.send(.discardDraftTapped)
                    } edit: {
                        store.send(.editDraftTapped)
                    }
                }
            }
            .frame(maxWidth: 720)
            .padding()
            .frame(maxWidth: .infinity)
        }
        .defaultScrollAnchor(.bottom)
    }

    /// Back, forward and the log of every point to go to.
    @ViewBuilder private var checkpointButtons: some View {
        Button("Back", systemImage: "arrow.uturn.backward") { store.send(.backTapped) }
            .disabled(!store.canGo || store.conversation.back == nil)
            .help("Go back to the message before")
        Button("Forward", systemImage: "arrow.uturn.forward") { store.send(.forwardTapped) }
            .disabled(!store.canGo || store.conversation.forward == nil)
            .help("Go forward to the message after")
        Button("All Checkpoints", systemImage: "clock.arrow.circlepath") { store.showsCheckpoints = true }
            .disabled(store.conversation.messages.isEmpty)
            .help("Every point to go back or forward to")
    }

    /// Summarizes older messages, so the model keeps them in mind.
    private var compactButton: some View {
        Button("Compact Conversation", systemImage: "rectangle.compress.vertical") { store.send(.compactTapped) }
            .disabled(!store.state.canCompact)
            .help("Summarize older messages, so the assistant keeps them in mind")
    }

    /// Says the last reply was written without older messages, above the
    /// field, with Compact; or that they're being summarized.
    @ViewBuilder private var compactBanner: some View {
        if store.compacting {
            ChatNotice("Summarizing older messages…", systemImage: "text.badge.minus", isBusy: true)
        } else {
            ChatNotice(
                "Older messages no longer fit what the model reads. Compact them into a summary it keeps.",
                systemImage: "text.badge.minus", action: "Compact"
            ) { store.send(.compactTapped) }
        }
    }

    /// Says the conversation is shown as it was earlier, above the field.
    private var rewound: some View {
        ChatNotice(
            "Going on from an earlier point. What came after is kept.", systemImage: "clock.arrow.circlepath",
            action: "Back to Latest", isEnabled: store.canGo
        ) { store.send(.latestTapped) }
    }

    /// Under the last message: Retry for a reply, and for the person's
    /// message, why its reply didn't come.
    @ViewBuilder private func after(_ message: ChatMessage) -> some View {
        let retry = Button("Retry", systemImage: "arrow.clockwise") {
            store.send(.retryTapped(messageID: message.id))
        }
        .disabled(!store.state.canRetry(message))
        if message.role == .assistant {
            retry
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .help("Write this reply again")
        } else if store.waiting == nil {
            VStack(alignment: .leading, spacing: 6) {
                Label(
                    store.failure?.reason ?? String(localized: "No reply came to this message."),
                    systemImage: "exclamationmark.triangle"
                )
                .foregroundStyle(.secondary)
                if store.state.canRetry(message) { retry.buttonStyle(.bordered) }
            }
            .font(.callout)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// ‹ 2/3 › under a message or reply written again, to show the others.
    @ViewBuilder private func branches(at message: ChatMessage) -> some View {
        let branches = store.conversation.branches(at: message.id)
        if branches.count > 1, let index = branches.firstIndex(where: { $0.id == message.id }) {
            BranchSwitcher(index: index, count: branches.count, canGo: store.canGo) {
                store.send(.branchTapped(messageID: message.id, by: $0))
            }
        }
    }

    /// Copying a message, editing it or writing its reply again, going back
    /// to it and forking from it.
    @ViewBuilder private func messageMenu(for message: ChatMessage) -> some View {
        if !message.text.isEmpty {
            TextMenu(text: message.text) { selecting = TextToSelect(text: $0) }
            Divider()
        }
        if message.role == .person {
            Button("Edit", systemImage: "pencil") { store.send(.editTapped(messageID: message.id)) }
                .disabled(!store.canGo)
        } else {
            Button("Retry", systemImage: "arrow.clockwise") { store.send(.retryTapped(messageID: message.id)) }
                .disabled(!store.state.canRetry(message))
        }
        let end = store.conversation.end(of: message.id)
        if end != store.conversation.messages.last?.id {
            Button("Go Back to Here", systemImage: "arrow.uturn.backward") { store.send(.goTapped(end)) }
                .disabled(!store.canGo)
        }
        Button("Fork From Here", systemImage: "arrow.triangle.branch") { store.send(.forkTapped(messageID: end)) }
            .disabled(!store.state.canFork(at: end))
    }

    #if os(macOS)
        /// The Mac shows no title for the third column, so the chat names
        /// itself here, with Rename beside it.
        private var header: some View {
            HStack {
                Text(store.conversation.title)
                    .font(.headline)
                    .lineLimit(1)
                    .accessibilityAddTraits(.isHeader)
                Spacer()
                contextMenu
                    .fixedSize()
                Group {
                    findButton
                    compactButton
                    checkpointButtons
                    Button("Rename", systemImage: "pencil") { store.send(.renameTapped) }
                        .help("Rename")
                }
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
            }
            .padding(.horizontal)
            .padding(.vertical, 10)
        }
    #endif

    /// The message field in its card, with the skill for the next message
    /// and what's attached to it above. Typing / lists the skills to pick.
    private var composer: some View {
        VStack(alignment: .leading, spacing: 8) {
            let choices = store.state.slashChoices
            if !choices.isEmpty {
                SkillSuggestions(choices) { store.send(.pickSkillTapped($0)) }
            }
            ChatComposer(
                text: $store.input, prompt: store.picked.map { _ in Text("Add details, or just send") },
                model: modelPill,
                mic: store.inputs.contains(.audio)
                    ? MicButton(isRecording: store.recording) { store.send(.recordTapped) } : nil,
                send: sendButton, submit: { store.send(.sendTapped) },
                top: { composerTop }, attach: { attachMenu }
            )
            .modifier(SkillFinderPopover(store: store))
            .photosPicker(
                isPresented: $picksPhoto, selection: $photo, matching: .images, preferredItemEncoding: .compatible
            )
            .onChange(of: photo) { _, item in
                guard let item else { return }
                photo = nil
                Task {
                    guard let data = try? await item.loadTransferable(type: Data.self) else {
                        store.send(.attached(.failure(AttachmentError.unreadablePhoto)))
                        return
                    }
                    store.send(.photoPicked(data))
                }
            }
        }
    }

    /// What + offers: a photo, for a model that reads them, and a skill.
    @ViewBuilder private var attachMenu: some View {
        if store.inputs.contains(.image) {
            Button("Attach a Photo", systemImage: "photo") { picksPhoto = true }
                .disabled(store.attaching || store.recording)
        }
        Button("Use a Skill", systemImage: "sparkles") { store.send(.findSkillTapped) }
    }

    /// Above the field: that a message is being edited, the skill picked
    /// and what's attached.
    @ViewBuilder private var composerTop: some View {
        if store.editing != nil {
            HStack {
                Label("Editing your message. Sending it writes the reply again.", systemImage: "pencil")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Cancel") { store.send(.cancelEditTapped) }
                    .font(.footnote.weight(.semibold))
            }
            .padding(.horizontal, 4)
        }
        if let picked = store.picked {
            SkillPill(
                title: picked.title, change: { store.send(.findSkillTapped) },
                remove: { store.send(.clearSkillTapped) })
        }
        if !store.pending.isEmpty || store.attaching || store.recording {
            attachments
        }
    }

    private var attachments: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 8) {
                if store.recording {
                    Label("Recording…", systemImage: "record.circle")
                        .foregroundStyle(.red)
                        .font(.callout)
                    Button("Cancel") { store.send(.cancelRecordingTapped) }
                        .font(.callout)
                }
                ForEach(store.pending) { attachment in
                    attachment.chip(elsewhere: false) { store.send(.removeAttachmentTapped(attachment.id)) }
                }
                if store.attaching {
                    ProgressView().controlSize(.small)
                }
            }
        }
        .scrollIndicators(.hidden)
    }

    /// Send, or Stop while a reply is being written.
    private var sendButton: SendButton {
        SendButton(
            canSend: store.canSend, canStop: store.canStop, send: { store.send(.sendTapped) },
            stop: { store.send(.stopTapped) })
    }

    /// Finds a skill for the next message by its name or what it does.
    private var skillFinder: some View {
        SkillFinder(
            store.skillChoices, query: $store.skillQuery, pick: { store.send(.pickSkillTapped($0)) },
            manage: { store.send(.manageSkillsTapped) }, cancel: { store.findsSkill = false })
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

/// On the Mac the skill finder hangs off the composer, where the button is,
/// instead of covering the chat.
private struct SkillFinderPopover: ViewModifier {
    @Bindable var store: StoreOf<ChatFeature>

    func body(content: Content) -> some View {
        #if os(macOS)
            content.popover(isPresented: $store.findsSkill, arrowEdge: .top) {
                SkillFinder(
                    store.skillChoices, query: $store.skillQuery, pick: { store.send(.pickSkillTapped($0)) },
                    manage: { store.send(.manageSkillsTapped) }, cancel: { store.findsSkill = false })
            }
        #else
            content
        #endif
    }
}

extension ChatAttachment {
    /// The attachment's chip: on the next message with a button to remove
    /// it, or on a sent one, saying when its file is on another device.
    fileprivate func chip(elsewhere: Bool, remove: (() -> Void)? = nil) -> AttachmentChip {
        switch kind {
        case .image: AttachmentChip("Photo", systemImage: "photo", elsewhere: elsewhere, remove: remove)
        case .audio:
            AttachmentChip(
                "Recording, \(seconds ?? 0) s", systemImage: "waveform", elsewhere: elsewhere, remove: remove)
        }
    }
}
