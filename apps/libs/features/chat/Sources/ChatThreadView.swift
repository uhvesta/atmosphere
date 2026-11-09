import CaptureKit
import ChatKit
import ChatUI
import ComposableArchitecture
import ModelsClient
import PhotosUI
import SwiftUI

/// A chat, from its first message to the field: each message with what the
/// model thought, the branches to switch between, and under the last reply
/// Retry and Speak. `empty` shows before the first message, and `extra`
/// shows what the app adds under a reply.
public struct ChatThreadView<Replier: ChatReplier, Empty: View, Extra: View>: View {
    public typealias Message = Replier.Message

    @Bindable var store: StoreOf<ChatCore<Replier>>
    let prompt: Text?
    let empty: Empty
    let extra: (Message) -> Extra

    @State private var selecting: TextToSelect?
    /// Whether the system's photo picker is open, from + on the Mac.
    @State private var picksPhoto = false
    @State private var photo: PhotosPickerItem?

    public init(
        store: StoreOf<ChatCore<Replier>>, prompt: Text? = nil, @ViewBuilder empty: () -> Empty,
        @ViewBuilder extra: @escaping (Message) -> Extra
    ) {
        self.store = store
        self.prompt = prompt
        self.empty = empty()
        self.extra = extra
    }

    public var body: some View {
        messages
            .safeAreaInset(edge: .bottom, spacing: 0) {
                VStack(spacing: 0) {
                    if store.finds {
                        findBar.padding()
                    } else {
                        if store.thread.forward != nil {
                            rewound
                            Divider()
                        }
                        composer.padding(10)
                    }
                }
                .background(.bar)
            }
            .sheet(item: $selecting) { SelectTextSheet(text: $0.text) }
            .sheet(item: $store.scope(state: \.modelSheet, action: \.modelSheet)) { ChatModelView(store: $0) }
            .sheet(item: $store.scope(state: \.rivalSheet, action: \.rivalSheet)) { ChatModelView(store: $0) }
            .sheet(
                isPresented: Binding(
                    get: { store.state.shownPromptValue != nil },
                    set: { if !$0 { store.send(.promptTapped(messageID: nil)) } })
            ) {
                if let prompt = store.state.shownPromptValue {
                    PromptSheet(prompt: prompt) { store.send(.promptTapped(messageID: nil)) }
                }
            }
            .fileImporter(
                isPresented: $store.picksFiles, allowedContentTypes: PickedFile.types(photos: store.readsPhotos),
                allowsMultipleSelection: true
            ) { store.send(.filesPicked($0)) }
            .photosPicker(
                isPresented: $picksPhoto, selection: $photo, matching: .images, preferredItemEncoding: .compatible
            )
            .onChange(of: photo) { _, item in
                guard let item else { return }
                photo = nil
                Task { store.send(.photoPicked(try? await item.loadTransferable(type: Data.self))) }
            }
            .modifier(AddContext(store: store))
            .background { findShortcut }
            .task { await store.send(.task).finish() }
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

    private var scroll: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if store.thread.messages.isEmpty, store.reply == nil {
                    empty
                }
                ForEach(store.thread.messages) { message in
                    VStack(alignment: .trailing, spacing: 4) {
                        if let rival = store.rival, rival.answering == message.id {
                            Comparison(
                                first: message, rival: rival, showsSpeed: store.showsSpeed,
                                select: { selecting = TextToSelect(text: $0) },
                                keep: { store.send(.keepTapped(rival: $0)) })
                        } else {
                            reply(message)
                        }
                        branches(at: message)
                        if message.id == store.thread.messages.last?.id, store.reply == nil, store.rival == nil {
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
                if let reply = store.reply {
                    ReplyInProgress(
                        text: reply, thinking: store.replyThinking?.model, showsThinking: store.showsReplyThinking,
                        toggleThinking: { store.send(.thinkingToggled(messageID: nil)) })
                }
                if let problem = store.problem {
                    Label(problem, systemImage: "exclamationmark.triangle")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .frame(maxWidth: store.rival == nil ? 720 : 1_080)
            .padding()
            .frame(maxWidth: .infinity)
        }
        .defaultScrollAnchor(.bottom)
    }

    /// A message, with what the app adds under a reply and, when asked
    /// for in Settings, how fast it was written and what the model was
    /// sent.
    @ViewBuilder private func reply(_ message: Message) -> some View {
        MessageView(
            role: message.role, text: message.text, thinking: message.thinking?.model,
            showsThinking: store.shownThinking.contains(message.id),
            toggleThinking: { store.send(.thinkingToggled(messageID: message.id)) },
            select: { selecting = TextToSelect(text: $0) },
            attachments: {
                ForEach(message.photos) { photo in
                    ChatPhoto(file: store.photoFiles[photo.id])
                }
                ForEach(message.files, id: \.self) { file in
                    AttachmentChip(verbatim: file.name, systemImage: "doc.text")
                }
            }
        )
        .contextMenu { menu(for: message) }
        if message.role == .assistant {
            extra(message)
            let prompt = store.showsPrompt && store.prompts[message.id] != nil
            if store.showsSpeed || prompt {
                ReplyDetails(
                    model: store.showsSpeed ? message.model : nil,
                    speed: store.showsSpeed ? message.speed : nil,
                    prompt: prompt ? { store.send(.promptTapped(messageID: message.id)) } : nil)
            }
        }
    }

    /// Under the last message: Retry and Speak for a reply, and for the
    /// person's message, that its reply didn't come.
    @ViewBuilder private func after(_ message: Message) -> some View {
        let retry = Button("Retry", systemImage: "arrow.clockwise") {
            store.send(.retryTapped(messageID: message.id))
        }
        .disabled(!store.state.canRetry(message))
        if message.role == .assistant {
            HStack(spacing: 12) {
                retry
                    .help("Write this reply again")
                speak(message)
                    .help(store.speaking == message.id ? "Stop reading this reply aloud" : "Read this reply aloud")
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .font(.caption)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
        } else if store.problem == nil {
            VStack(alignment: .leading, spacing: 6) {
                Label("No reply came to this message.", systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.secondary)
                retry.buttonStyle(.bordered)
            }
            .font(.callout)
            .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            retry
                .buttonStyle(.bordered)
                .font(.callout)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// Speak for a reply, or Stop Speaking while it's read aloud.
    private func speak(_ message: Message) -> some View {
        let speaking = store.speaking == message.id
        return Button(
            speaking ? "Stop Speaking" : "Speak", systemImage: speaking ? "stop.circle" : "speaker.wave.2"
        ) { store.send(.speakTapped(messageID: message.id)) }
    }

    /// ‹ 2/3 › under a message or reply written again, to show the others.
    @ViewBuilder private func branches(at message: Message) -> some View {
        let branches = store.thread.branches(at: message.id)
        if branches.count > 1, let index = branches.firstIndex(where: { $0.id == message.id }) {
            BranchSwitcher(index: index, count: branches.count, canGo: store.canGo) {
                store.send(.branchTapped(messageID: message.id, by: $0))
            }
        }
    }

    /// Copying a message, editing it or writing its reply again.
    @ViewBuilder private func menu(for message: Message) -> some View {
        TextMenu(text: message.text) { selecting = TextToSelect(text: $0) }
        Divider()
        if message.role == .person {
            Button("Edit", systemImage: "pencil") { store.send(.editTapped(messageID: message.id)) }
                .disabled(!store.canGo)
        } else {
            Button("Retry", systemImage: "arrow.clockwise") { store.send(.retryTapped(messageID: message.id)) }
                .disabled(!store.state.canRetry(message))
            speak(message)
            if store.showsPrompt, store.prompts[message.id] != nil {
                Button("Show Prompt", systemImage: "text.magnifyingglass") {
                    store.send(.promptTapped(messageID: message.id))
                }
            }
        }
    }

    /// Says the chat is shown as it was earlier, above the field.
    private var rewound: some View {
        ChatNotice(
            "Going on from an earlier point. What came after is kept.", systemImage: "clock.arrow.circlepath",
            action: "Back to Latest", isEnabled: store.canGo
        ) { store.send(.latestTapped) }
    }

    /// ⌘F opens the bar to find words in the messages. The button isn't
    /// seen.
    private var findShortcut: some View {
        Button("Find in Chat") { store.send(.findTapped) }
            .keyboardShortcut("f")
            .disabled(store.thread.messages.isEmpty)
            .opacity(0)
            .frame(width: 0, height: 0)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
    }

    private var findBar: some View {
        FindBar(
            query: $store.findQuery, matches: store.state.findMatches.count, position: store.findPosition,
            move: { store.send(.findMoved(by: $0)) }, close: { store.send(.findClosed) })
    }

    /// The field, with the chat's model, the mic and +, and above it what's
    /// being edited and attached. For a model that reads photos, + opens a
    /// sheet of what to add on the iPhone and a menu on the Mac; for one
    /// that doesn't, it opens the file picker.
    @ViewBuilder private var composer: some View {
        #if os(macOS)
            if store.readsPhotos {
                ChatComposer(
                    text: $store.input, prompt: prompt, model: modelPill, mic: mic, send: sendButton,
                    submit: { store.send(.sendTapped) }, top: { composerTop }, attach: { attachMenu })
            } else {
                addingComposer
            }
        #else
            addingComposer
        #endif
    }

    /// What + offers on the Mac for a model that reads photos.
    @ViewBuilder private var attachMenu: some View {
        Button("Attach a Photo…", systemImage: "photo") { picksPhoto = true }
            .disabled(store.attaching)
        Button("Add Files…", systemImage: "doc") { store.picksFiles = true }
            .disabled(store.attaching)
    }

    /// The field with a + that sends `addContextTapped`.
    private var addingComposer: some View {
        ChatComposer(
            text: $store.input, prompt: prompt, model: modelPill, mic: mic, send: sendButton,
            submit: { store.send(.sendTapped) }, add: { store.send(.addContextTapped) }, top: { composerTop })
    }

    private var mic: MicButton {
        MicButton(isOn: store.dictating, mode: .dictate) { store.send(.micTapped) }
    }

    private var sendButton: SendButton {
        SendButton(
            canSend: store.canSend, canStop: store.reply != nil || store.rival?.done == false,
            send: { store.send(.sendTapped) },
            stop: { store.send(.stopTapped) })
    }

    /// Opens what the chat runs on: its model and how long it thinks.
    private var modelPill: ModelPill {
        let current = store.state.current
        return ModelPill(
            name: current?.name ?? String(localized: "Model"),
            detail: current?.thinks == true ? store.model.thinkingLevel.title : nil,
            pick: { store.send(.modelTapped) })
    }

    @ViewBuilder private var composerTop: some View {
        if store.compares {
            RivalBar(
                model: store.rivalModel == nil ? nil : store.state.rivalCurrent?.name ?? String(localized: "Model"),
                pick: { store.send(.rivalTapped) }, remove: { store.send(.rivalRemoved) })
        }
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
        if !store.pending.isEmpty || !store.pendingPhotos.isEmpty || store.attaching || store.dictating {
            ScrollView(.horizontal) {
                HStack(spacing: 8) {
                    if store.dictating {
                        Label("Listening…", systemImage: "waveform")
                            .foregroundStyle(.red)
                            .font(.callout)
                    }
                    ForEach(store.pendingPhotos) { photo in
                        ChatPhoto(file: store.photoFiles[photo.id], side: 56) {
                            store.send(.removePhotoTapped(photo.id))
                        }
                    }
                    ForEach(Array(store.pending.enumerated()), id: \.offset) { index, file in
                        AttachmentChip(verbatim: file.name, systemImage: "doc.text") {
                            store.send(.removeFileTapped(index))
                        }
                    }
                    if store.attaching {
                        ProgressView().controlSize(.small)
                    }
                }
            }
            .scrollIndicators(.hidden)
        }
    }
}

/// + on the iPhone: the camera and recent photos, for a model that reads
/// them, and files, in a sheet.
private struct AddContext<Replier: ChatReplier>: ViewModifier {
    @Bindable var store: StoreOf<ChatCore<Replier>>

    func body(content: Content) -> some View {
        #if os(iOS)
            content.sheet(isPresented: $store.addsContext) {
                AddContextSheet(
                    photos: AddContextSheet.Photos(
                        access: store.photosAccess, recents: store.recentPhotos,
                        attached: Set(store.attachedRecents.keys), camera: store.hasCamera),
                    fileTypes: PickedFile.types(photos: true),
                    close: { store.addsContext = false },
                    tapRecent: { store.send(.recentPhotoTapped($0)) },
                    photoPicked: { store.send(.photoPicked($0)) },
                    photoTaken: { store.send(.photoTaken($0)) },
                    filesPicked: { store.send(.filesPicked($0)) })
            }
        #else
            content
        #endif
    }
}

/// A photo sent in the chat or attached to the next message, with a button
/// to take it off when `remove` is set. One whose file isn't on this device
/// shows as a chip saying so.
private struct ChatPhoto: View {
    let file: URL?
    var side: CGFloat = 160
    var remove: (() -> Void)?

    var body: some View {
        if let image = file.flatMap(Self.image) {
            image
                .resizable()
                .scaledToFill()
                .frame(width: side, height: side)
                .clipShape(.rect(cornerRadius: side > 80 ? 14 : 10))
                .overlay(alignment: .topTrailing) {
                    if let remove {
                        Button("Remove", systemImage: "xmark.circle.fill", action: remove)
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                            .symbolRenderingMode(.palette)
                            .foregroundStyle(.white, .black.opacity(0.6))
                            .padding(3)
                    }
                }
                .accessibilityElement(children: .contain)
                .accessibilityLabel(Text("Photo"))
        } else {
            AttachmentChip("Photo", systemImage: "photo", elsewhere: true, remove: remove)
        }
    }

    private static func image(_ file: URL) -> Image? {
        #if canImport(UIKit)
            UIImage(contentsOfFile: file.path).map { Image(uiImage: $0) }
        #else
            NSImage(contentsOf: file).map { Image(nsImage: $0) }
        #endif
    }
}

extension ChatThreadView where Extra == EmptyView {
    public init(store: StoreOf<ChatCore<Replier>>, prompt: Text? = nil, @ViewBuilder empty: () -> Empty) {
        self.init(store: store, prompt: prompt, empty: empty) { _ in EmptyView() }
    }
}

/// How full the chat's model was at the last reply, which opens to say how
/// much. For the app's toolbar.
public struct ChatContextButton: View {
    let context: ContextUse?

    public init(_ context: ContextUse?) {
        self.context = context
    }

    public var body: some View {
        Menu {
            Text(ContextRing.description(of: context))
        } label: {
            ContextRing(context)
        }
        .menuIndicator(.hidden)
        .help(ContextRing.description(of: context))
    }
}

extension ThreadThinking {
    /// As ChatUI shows it.
    var model: ModelThinking { ModelThinking(text: text, duration: duration) }
}
