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
    @Environment(\.dynamicTypeSize) private var typeSize

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
                if store.conversation.forward != nil {
                    rewound
                    Divider()
                }
                composer
                    .padding()
            }
            .background(.bar)
        }
        .sheet(item: $selecting) { selecting in
            SelectTextSheet(text: selecting.text)
        }
        .sheet(isPresented: $store.showsCheckpoints) {
            NavigationStack {
                CheckpointLog(store: store)
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
                SkillFinder(store: store)
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
        .navigationTitle(store.conversation.title)
        .toolbarTitleMenu {
            Button("Rename", systemImage: "pencil") { store.send(.renameTapped) }
        }
        #if os(iOS)
            .toolbar {
                // One menu, as three buttons fold into the bar's overflow.
                ToolbarItem(placement: .topBarTrailing) {
                    Menu("Checkpoints", systemImage: "clock.arrow.circlepath") {
                        checkpointButtons
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
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if store.availability != .available {
                    Label(store.availability.description, systemImage: "exclamationmark.circle")
                        .foregroundStyle(.secondary)
                }
                if store.conversation.messages.isEmpty, store.availability == .available {
                    Suggestions { store.send(.suggestionTapped($0)) }
                }
                ForEach(store.conversation.messages) { message in
                    VStack(alignment: .trailing, spacing: 4) {
                        MessageView(
                            message: message, skill: store.state.skillTitle(of: message),
                            elsewhere: store.elsewhere,
                            canUndo: store.conversation.changes[message.id] != nil,
                            showsThinking: store.shownThinking.contains(message.id),
                            toggleThinking: { store.send(.thinkingToggled(messageID: message.id)) },
                            undo: { store.send(.undoTapped(messageID: message.id)) },
                            select: { selecting = TextToSelect(text: $0) }
                        )
                        .contextMenu { messageMenu(for: message) }
                        if message.role == .person { branches(at: message) }
                    }
                }
                if let replying = store.replying {
                    if let thinking = store.replyingThinking {
                        ThinkingDisclosure(
                            thinking: thinking, isExpanded: store.showsReplyThinking,
                            toggle: { store.send(.thinkingToggled(messageID: nil)) })
                    }
                    if replying.isEmpty, store.replyingThinking == nil {
                        HStack(spacing: 8) {
                            ProgressView().controlSize(.small)
                            Text("Thinking…")
                        }
                        .foregroundStyle(.secondary)
                        .accessibilityElement(children: .combine)
                    } else {
                        Text(replying).foregroundStyle(.secondary)
                            .accessibilityIdentifier(ModelText.identifier)
                    }
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

    /// Says the conversation is shown as it was earlier, above the field.
    private var rewound: some View {
        HStack {
            Label("Going on from an earlier point. What came after is kept.", systemImage: "clock.arrow.circlepath")
                .font(.footnote)
                .foregroundStyle(.secondary)
            Spacer()
            Button("Back to Latest") { store.send(.latestTapped) }
                .font(.footnote.weight(.semibold))
                .disabled(!store.canGo)
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
    }

    /// ‹ 2/3 › under a message written again after going back, to show
    /// the others.
    @ViewBuilder private func branches(at message: ChatMessage) -> some View {
        let branches = store.conversation.branches(at: message.id)
        if branches.count > 1, let index = branches.firstIndex(where: { $0.id == message.id }) {
            HStack(spacing: 4) {
                Button("Previous Version", systemImage: "chevron.left") {
                    store.send(.branchTapped(messageID: message.id, by: -1))
                }
                .disabled(index == 0 || !store.canGo)
                Text("\(index + 1)/\(branches.count)")
                    .monospacedDigit()
                    .accessibilityLabel(Text("Version \(index + 1) of \(branches.count)"))
                Button("Next Version", systemImage: "chevron.right") {
                    store.send(.branchTapped(messageID: message.id, by: 1))
                }
                .disabled(index == branches.count - 1 || !store.canGo)
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    /// Copying a message, going back to it and forking from it.
    @ViewBuilder private func messageMenu(for message: ChatMessage) -> some View {
        if !message.text.isEmpty {
            TextMenu(text: message.text) { selecting = TextToSelect(text: $0) }
            Divider()
        }
        let end = store.conversation.end(of: message.id)
        if end != store.conversation.messages.last?.id {
            Button("Go Back to Here", systemImage: "arrow.uturn.backward") { store.send(.goTapped(end)) }
                .disabled(!store.canGo)
        }
        Button("Fork From Here", systemImage: "arrow.triangle.branch") { store.send(.forkTapped(messageID: end)) }
            .disabled(!store.canGo)
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
                Group {
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

    /// The message field, with the skill for the next message: a pill once
    /// one is picked, a button to find one before. Above it, what's
    /// attached to the next message.
    private var composer: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !store.pending.isEmpty || store.attaching || store.recording {
                attachments
            }
            field
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
                    AttachmentChip(attachment: attachment, elsewhere: false) {
                        store.send(.removeAttachmentTapped(attachment.id))
                    }
                }
                if store.attaching {
                    ProgressView().controlSize(.small)
                }
            }
        }
        .scrollIndicators(.hidden)
    }

    /// Buttons to attach a photo or a recording, for a model that reads them.
    @ViewBuilder private var attachButtons: some View {
        if store.inputs.contains(.image) {
            PhotosPicker(selection: $photo, matching: .images, preferredItemEncoding: .compatible) {
                Label("Attach a Photo", systemImage: "photo")
                    .labelStyle(.iconOnly)
                    .font(.title2)
            }
            .disabled(store.attaching || store.recording)
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
        if store.inputs.contains(.audio) {
            Button {
                store.send(.recordTapped)
            } label: {
                Label(
                    store.recording ? "Stop Recording" : "Record",
                    systemImage: store.recording ? "stop.circle.fill" : "mic"
                )
                .labelStyle(.iconOnly)
                .font(.title2)
            }
            .tint(store.recording ? .red : nil)
            .disabled(store.attaching)
        }
    }

    /// At the largest text sizes the message field takes its own row, so
    /// the buttons can't squeeze it out.
    @ViewBuilder private var field: some View {
        if typeSize.isAccessibilitySize, store.picked == nil {
            VStack(alignment: .leading, spacing: 8) {
                textField
                HStack {
                    skillButton
                    attachButtons
                    Spacer()
                    sendButton
                }
            }
            .modifier(SkillFinderPopover(store: store))
        } else {
            HStack {
                skillButton
                attachButtons
                textField
                sendButton
            }
            .modifier(SkillFinderPopover(store: store))
        }
    }

    @ViewBuilder private var skillButton: some View {
        if let skill = store.picked {
            SkillPill(title: skill.title) {
                store.send(.findSkillTapped)
            } remove: {
                store.send(.clearSkillTapped)
            }
        } else {
            Button {
                store.send(.findSkillTapped)
            } label: {
                Label("Use a Skill", systemImage: "sparkles")
                    .labelStyle(.iconOnly)
                    .font(.title2)
            }
        }
    }

    private var textField: some View {
        TextField(
            "Message", text: $store.input, prompt: store.picked.map { _ in Text("Add details, or just send") },
            axis: .vertical
        )
        .textFieldStyle(.roundedBorder)
        .onSubmit { store.send(.sendTapped) }
    }

    private var sendButton: some View {
        Button {
            store.send(.sendTapped)
        } label: {
            Label("Send", systemImage: "arrow.up.circle.fill")
                .labelStyle(.iconOnly)
                .font(.title2)
        }
        .disabled(!store.canSend)
    }
}

/// On the Mac the skill finder hangs off the composer, where the button is,
/// instead of covering the chat.
private struct SkillFinderPopover: ViewModifier {
    @Bindable var store: StoreOf<ChatFeature>

    func body(content: Content) -> some View {
        #if os(macOS)
            content.popover(isPresented: $store.findsSkill, arrowEdge: .top) {
                SkillFinder(store: store)
            }
        #else
            content
        #endif
    }
}

struct DismissButton: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Button("Done") { dismiss() }
    }
}

/// The skill picked for the next message: tap it to pick another, or
/// remove it.
private struct SkillPill: View {
    let title: String
    let change: () -> Void
    let remove: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            Button(action: change) {
                Label(title, systemImage: "sparkles")
                    .font(.callout.weight(.medium))
            }
            .buttonStyle(.plain)
            .accessibilityHint(Text("Picks another skill"))
            Button(action: remove) {
                Label("Remove \(title)", systemImage: "xmark.circle.fill")
                    .labelStyle(.iconOnly)
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.tint.opacity(0.2), in: .capsule)
    }
}

/// Finds a skill for the next message by its name or what it does.
private struct SkillFinder: View {
    @Bindable var store: StoreOf<ChatFeature>

    var body: some View {
        #if os(macOS)
            VStack(spacing: 0) {
                TextField("Find a skill", text: $store.skillQuery)
                    .textFieldStyle(.roundedBorder)
                    .padding(10)
                list
                Divider()
                HStack {
                    Button("Manage Skills…") { store.send(.manageSkillsTapped) }
                    Spacer()
                    Button("Cancel") { store.findsSkill = false }
                        .keyboardShortcut(.cancelAction)
                }
                .padding(10)
            }
            .frame(width: 340, height: 420)
        #else
            NavigationStack {
                // The search field and Manage Skills sit above the list:
                // at the bottom they'd float over its last rows.
                list
                    .searchable(
                        text: $store.skillQuery, placement: .navigationBarDrawer(displayMode: .always),
                        prompt: Text("Find a skill")
                    )
                    .navigationTitle("Skills")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { store.findsSkill = false }
                        }
                        ToolbarItem(placement: .primaryAction) {
                            Button("Manage Skills…") { store.send(.manageSkillsTapped) }
                        }
                    }
            }
            .presentationDetents([.medium, .large])
        #endif
    }

    private var list: some View {
        List(store.skillChoices) { skill in
            Button {
                store.send(.pickSkillTapped(skill.id))
            } label: {
                VStack(alignment: .leading, spacing: 2) {
                    Text(skill.title)
                    Text(skill.summary)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
        }
        .overlay {
            if store.skillChoices.isEmpty {
                if store.skillQuery.isEmpty {
                    ContentUnavailableView(
                        "No skills on", systemImage: "sparkles",
                        description: Text("Turn skills on in Manage Skills."))
                } else {
                    ContentUnavailableView.search(text: store.skillQuery)
                }
            }
        }
    }
}

/// Ways to start, in a chat with nothing in it yet.
private struct Suggestions: View {
    let send: (String) -> Void

    private let suggestions: [(symbol: String, text: String)] = [
        ("calendar", String(localized: "Plan my week")),
        ("exclamationmark.circle", String(localized: "What's overdue?")),
        ("checkmark.circle", String(localized: "What did I finish this week?")),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Ask about your tasks, change them, or plan what to add.")
                .foregroundStyle(.secondary)
            ForEach(suggestions, id: \.text) { suggestion in
                Button {
                    send(suggestion.text)
                } label: {
                    Label(suggestion.text, systemImage: suggestion.symbol)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 10)
                        .background(.fill.quaternary, in: .rect(cornerRadius: 12))
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
            }
        }
    }
}

/// A photo or recording: on the next message with a button to remove it,
/// or on a sent one, saying when its file is on another device.
private struct AttachmentChip: View {
    let attachment: ChatAttachment
    let elsewhere: Bool
    var remove: (() -> Void)?

    var body: some View {
        HStack(spacing: 6) {
            Label(title, systemImage: attachment.kind == .image ? "photo" : "waveform")
            if elsewhere {
                Text("Not on this device").foregroundStyle(.secondary)
            }
            if let remove {
                Button("Remove", systemImage: "xmark.circle.fill", action: remove)
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .tint(.secondary)
            }
        }
        .font(.callout)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Color.secondary.opacity(0.15), in: .capsule)
        .accessibilityElement(children: .combine)
    }

    private var title: LocalizedStringKey {
        switch attachment.kind {
        case .image: "Photo"
        case .audio: "Recording, \(attachment.seconds ?? 0) s"
        }
    }
}

/// Every point the conversation can go to, on every branch, oldest first:
/// tap one to go there, or swipe to fork from it.
private struct CheckpointLog: View {
    let store: StoreOf<ChatFeature>

    var body: some View {
        List(store.conversation.checkpoints) { checkpoint in
            Button {
                store.send(.goTapped(checkpoint.end))
            } label: {
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(title(of: checkpoint))
                            .lineLimit(2)
                            .foregroundStyle(checkpoint.isShown ? .primary : .secondary)
                        HStack(spacing: 6) {
                            Text(checkpoint.message.at, format: .dateTime.hour().minute())
                            if checkpoint.changes > 0 {
                                Text("^[\(checkpoint.changes) change](inflect: true)")
                            }
                            if !checkpoint.isShown {
                                Text("Not shown")
                            }
                        }
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if checkpoint.isCurrent {
                        Image(systemName: "checkmark")
                            .foregroundStyle(.tint)
                            .accessibilityLabel(Text("Where the conversation is"))
                    }
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .disabled(!store.canGo)
            .accessibilityAddTraits(checkpoint.isCurrent ? .isSelected : [])
            .swipeActions {
                Button("Fork", systemImage: "arrow.triangle.branch") {
                    store.send(.forkTapped(messageID: checkpoint.end))
                }
                .tint(.indigo)
            }
            .contextMenu {
                Button("Fork From Here", systemImage: "arrow.triangle.branch") {
                    store.send(.forkTapped(messageID: checkpoint.end))
                }
            }
        }
    }

    private func title(of checkpoint: Conversation.Checkpoint) -> String {
        checkpoint.message.text.isEmpty ? String(localized: "Attachment") : checkpoint.message.text
    }
}

private struct MessageView: View {
    let message: ChatMessage
    let skill: String?
    let elsewhere: Set<UUID>
    let canUndo: Bool
    let showsThinking: Bool
    let toggleThinking: () -> Void
    let undo: () -> Void
    /// Opens text to select part of.
    let select: (String) -> Void

    var body: some View {
        VStack(alignment: message.role == .person ? .trailing : .leading, spacing: 4) {
            if let thinking = message.thinking, message.role == .assistant {
                ThinkingDisclosure(
                    thinking: thinking, isExpanded: showsThinking, toggle: toggleThinking, select: select)
            }
            if let skill {
                Label(skill, systemImage: "sparkles")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.tint)
            }
            ForEach(message.attachments) { attachment in
                AttachmentChip(attachment: attachment, elsewhere: elsewhere.contains(attachment.id))
            }
            if !message.text.isEmpty {
                if message.role == .person {
                    Text(message.text)
                        .selectableOnMac()
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(.tint.opacity(0.2), in: .rect(cornerRadius: 16))
                        .frame(maxWidth: 520, alignment: .trailing)
                } else {
                    // The assistant's words read as text, not a bubble.
                    Text(message.text)
                        .selectableOnMac()
                        .accessibilityIdentifier(ModelText.identifier)
                }
            }
            if canUndo {
                Button("Undo", action: undo)
                    .font(.caption)
            }
        }
        .frame(maxWidth: .infinity, alignment: message.role == .person ? .trailing : .leading)
    }
}

/// What the model thought before it replied, behind a chevron: closed, a
/// line saying how long it thought and on what; open, the thoughts too.
private struct ThinkingDisclosure: View {
    let thinking: ModelThinking
    let isExpanded: Bool
    let toggle: () -> Void
    /// Opens the thoughts to select part of; nil while they're written.
    var select: ((String) -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button(action: toggle) {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.semibold))
                        .rotationEffect(.degrees(isExpanded ? 90 : 0))
                    Text(summary)
                    if thinking.duration == nil { ProgressView().controlSize(.mini) }
                }
                .font(.footnote)
                .foregroundStyle(.secondary)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(summary)
            .accessibilityHint(isExpanded ? Text("Hides the thinking") : Text("Shows the thinking"))
            .accessibilityAddTraits(isExpanded ? .isSelected : [])
            if isExpanded {
                Text(thoughts)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .selectableOnMac()
                    .contextMenu {
                        if let select { TextMenu(text: thoughts, select: select) }
                    }
                    .padding(.leading, 10)
                    .overlay(alignment: .leading) {
                        Rectangle().fill(.quaternary).frame(width: 2)
                    }
            }
        }
        .animation(.snappy(duration: 0.2), value: isExpanded)
    }

    private var thoughts: String { thinking.text.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var summary: String {
        guard let duration = thinking.duration else { return String(localized: "Thinking…") }
        let time = duration.formatted(
            .units(allowed: [.seconds], width: .abbreviated, fractionalPart: .show(length: 1)))
        switch thinking.processor {
        case .gpu?: return String(localized: "Thoughts · \(time) on the GPU")
        case .cpu?: return String(localized: "Thoughts · \(time) on the CPU")
        case nil: return String(localized: "Thoughts · \(time)")
        }
    }
}

extension Binding {
    /// The value, or `fallback` once it's gone, with writes dropped then,
    /// while SwiftUI catches up. SwiftUI's own `Binding(_:)` over an
    /// optional force-unwraps it, which traps if the view reads it once more
    /// after it's set to nil, as Discard does.
    fileprivate func or<Wrapped>(_ fallback: Wrapped) -> Binding<Wrapped> where Value == Wrapped? {
        Binding<Wrapped>(
            get: { wrappedValue ?? fallback },
            set: { if wrappedValue != nil { wrappedValue = $0 } })
    }
}

/// Marks what the model wrote, such as its replies and its drafts, so a
/// test on a real model can leave out words that differ from one GPU to the
/// next. Tests' accessibility trees don't show it.
enum ModelText {
    static let identifier = "lifr.model-text"
}

/// Copy and Select Text, for a message or its thoughts.
private struct TextMenu: View {
    let text: String
    let select: (String) -> Void

    var body: some View {
        Button("Copy", systemImage: "doc.on.doc") { copyToClipboard(text) }
        Button("Select Text", systemImage: "selection.pin.in.out") { select(text) }
    }
}

extension View {
    /// Lets the Mac select any part of the text by dragging. On iOS a long
    /// press opens the message's menu instead, whose Select Text does.
    @ViewBuilder fileprivate func selectableOnMac() -> some View {
        #if os(macOS)
            textSelection(.enabled)
        #else
            self
        #endif
    }
}
