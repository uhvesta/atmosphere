import ComposableArchitecture
import LifrAssistantClient
import LifrCore
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
            composer
                .padding()
                .background(.bar)
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
                    MessageView(
                        message: message, skill: store.state.skillTitle(of: message),
                        elsewhere: store.elsewhere,
                        canUndo: store.conversation.changes[message.id] != nil
                    ) {
                        store.send(.undoTapped(messageID: message.id))
                    }
                }
                if let replying = store.replying {
                    if replying.isEmpty {
                        HStack(spacing: 8) {
                            ProgressView().controlSize(.small)
                            Text("Thinking…")
                        }
                        .foregroundStyle(.secondary)
                        .accessibilityElement(children: .combine)
                    } else {
                        Text(replying).foregroundStyle(.secondary)
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
                Button("Rename", systemImage: "pencil") { store.send(.renameTapped) }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .help("Rename")
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

private struct DismissButton: View {
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

private struct MessageView: View {
    let message: ChatMessage
    let skill: String?
    let elsewhere: Set<UUID>
    let canUndo: Bool
    let undo: () -> Void

    var body: some View {
        VStack(alignment: message.role == .person ? .trailing : .leading, spacing: 4) {
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
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(.tint.opacity(0.2), in: .rect(cornerRadius: 16))
                        .frame(maxWidth: 520, alignment: .trailing)
                } else {
                    // The assistant's words read as text, not a bubble.
                    Text(message.text)
                        .textSelection(.enabled)
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
