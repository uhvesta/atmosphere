import AssistantKit
import CaptureKit
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
    @State private var importsFiles = false

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
                // Compacting, finding and checkpoints are / commands.
                ToolbarItem(placement: .topBarTrailing) {
                    contextMenu
                }
            }
        #endif
        .background { findShortcut }
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
    /// say how much. /compact makes room.
    private var contextMenu: some View {
        Menu {
            Text(ContextRing.description(of: store.context))
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

    /// ⌘F opens the bar to find words in the messages shown, as /find
    /// does. The button isn't seen.
    private var findShortcut: some View {
        Button("Find in Conversation") { store.send(.findTapped) }
            .keyboardShortcut("f")
            .disabled(store.conversation.messages.isEmpty)
            .opacity(0)
            .frame(width: 0, height: 0)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
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
                            skill: message.skill,
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
            HStack(spacing: 12) {
                retry
                    .help("Write this reply again")
                if !message.text.isEmpty {
                    speak(message)
                        .help(store.speaking == message.id ? "Stop reading this reply aloud" : "Read this reply aloud")
                }
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .font(.caption)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
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

    /// Speak for a reply, or Stop Speaking while it's read aloud.
    private func speak(_ message: ChatMessage) -> some View {
        let speaking = store.speaking == message.id
        return Button(
            speaking ? "Stop Speaking" : "Speak", systemImage: speaking ? "stop.circle" : "speaker.wave.2"
        ) { store.send(.speakTapped(messageID: message.id)) }
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
            if !message.text.isEmpty { speak(message) }
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

    /// The message field in its card, starting with the skill for the next
    /// message, with what's attached to it above. Typing / lists the skills
    /// to pick and the commands to run.
    private var composer: some View {
        VStack(alignment: .leading, spacing: 8) {
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
            field
                .modifier(AddContext(store: store))
                .fileImporter(
                    isPresented: $importsFiles, allowedContentTypes: PickedFile.types(photos: readsPhotos),
                    allowsMultipleSelection: true
                ) { store.send(.filesPicked($0)) }
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

    private var readsPhotos: Bool { store.inputs.contains(.image) }

    /// The field. On the iPhone + opens a sheet of what to add; on the Mac
    /// it's a menu.
    @ViewBuilder private var field: some View {
        let prompt = store.picked.map { _ in Text("Add details, or just send") }
        #if os(iOS)
            ChatComposer(
                text: $store.input, prompt: prompt, model: modelPill, mic: mic, send: sendButton,
                submit: { store.send(.sendTapped) }, add: { store.send(.addContextTapped) }, top: { composerTop }
            )
            .token(store.picked?.id) { store.send(.clearSkillTapped) }
        #else
            ChatComposer(
                text: $store.input, prompt: prompt, model: modelPill, mic: mic, send: sendButton,
                submit: { store.send(.sendTapped) }, top: { composerTop }, attach: { attachMenu }
            )
            .token(store.picked?.id) { store.send(.clearSkillTapped) }
        #endif
    }

    /// Records for a model that listens; otherwise dictates into the field.
    private var mic: MicButton {
        MicButton(
            isOn: store.recording || store.dictating, mode: store.inputs.contains(.audio) ? .record : .dictate
        ) { store.send(.micTapped) }
    }

    /// What + offers on the Mac: a photo, for a model that reads them, and
    /// files.
    @ViewBuilder private var attachMenu: some View {
        if readsPhotos {
            Button("Attach a Photo…", systemImage: "photo") { picksPhoto = true }
                .disabled(store.attaching || store.recording)
        }
        Button("Add Files…", systemImage: "doc") { importsFiles = true }
            .disabled(store.attaching || store.recording)
    }

    /// Above the field: that a message is being edited, and what's
    /// attached.
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
        if !store.pending.isEmpty || store.attaching || store.recording || store.dictating {
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
                if store.dictating {
                    Label("Listening…", systemImage: "waveform")
                        .foregroundStyle(.red)
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

    /// Ways to start, in a chat with nothing in it yet.
    private static var suggestions: [Suggestions.Suggestion] {
        [
            .init(systemImage: "calendar", text: String(localized: "Plan my week")),
            .init(systemImage: "exclamationmark.circle", text: String(localized: "What's overdue?")),
            .init(systemImage: "checkmark.circle", text: String(localized: "What did I finish this week?")),
        ]
    }
}

/// + on the iPhone: the camera and recent photos, for a model that reads
/// them, and Add files, in a sheet.
private struct AddContext: ViewModifier {
    @Bindable var store: StoreOf<ChatFeature>

    func body(content: Content) -> some View {
        #if os(iOS)
            content.sheet(isPresented: $store.addsContext) {
                AddContextSheet(
                    photos: store.inputs.contains(.image)
                        ? AddContextSheet.Photos(
                            access: store.photosAccess, recents: store.recentPhotos,
                            attached: Set(store.attachedRecents.keys), camera: store.hasCamera)
                        : nil,
                    fileTypes: PickedFile.types(photos: store.inputs.contains(.image)),
                    close: { store.addsContext = false },
                    tapRecent: { store.send(.recentPhotoTapped($0)) },
                    photoPicked: { data in
                        if let data {
                            store.send(.photoPicked(data))
                        } else {
                            store.send(.attached(.failure(AttachmentError.unreadablePhoto)))
                        }
                    },
                    photoTaken: { store.send(.photoTaken($0)) },
                    filesPicked: { store.send(.filesPicked($0)) })
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
        case .text:
            AttachmentChip(verbatim: name ?? "File", systemImage: "doc.text", remove: remove)
        }
    }
}
