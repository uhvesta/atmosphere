import CaptureKit
import ChatKit
import ChatUI
import ComposableArchitecture
import ModelsClient
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// A chat, from its first message to the field: each message with what the
/// model thought, the branches to switch between, and under the last reply
/// Retry and Speak. The app's `parts` add to it, such as what shows before
/// the first message and under a reply.
public struct ChatThreadView<Replier: ChatReplier, Parts: ChatThreadParts>: View
where Parts.Message == Replier.Message {
    public typealias Message = Replier.Message

    @Bindable var store: StoreOf<ChatCore<Replier>>
    let prompt: Text?
    let parts: Parts

    @State private var selecting: TextToSelect?
    /// The photo or file opened from a message or the field.
    @State private var opened: OpenAttachment?
    /// Whether the system's photo picker is open, from + on the Mac.
    @State private var picksPhoto = false
    @State private var photo: PhotosPickerItem?
    /// Whether the end of the chat is in view; when it isn't, a button
    /// goes back to it.
    @State private var atEnd = true
    /// Whether files are being dragged over the chat.
    @State private var dropping = false

    public init(store: StoreOf<ChatCore<Replier>>, prompt: Text? = nil, parts: Parts) {
        self.store = store
        self.prompt = prompt
        self.parts = parts
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
                        parts.notices()
                        VStack(alignment: .leading, spacing: 8) {
                            parts.above()
                            composer
                        }
                        .padding(10)
                    }
                }
                .background(.bar)
            }
            .sheet(item: $selecting) { SelectTextSheet(text: $0.text) }
            .attachmentViewer($opened)
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
            .dropDestination(for: DroppedFile.self) { files, _ in
                guard !files.isEmpty else { return false }
                store.send(.filesPicked(.success(files.map(\.url))))
                return true
            } isTargeted: {
                dropping = $0
            }
            .overlay {
                if dropping {
                    RoundedRectangle(cornerRadius: 16)
                        .strokeBorder(.tint, lineWidth: 3)
                        .background(.tint.opacity(0.08), in: .rect(cornerRadius: 16))
                        .padding(6)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
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
                .onScrollGeometryChange(for: Bool.self) { geometry in
                    geometry.visibleRect.maxY >= geometry.contentSize.height - 80
                } action: { _, atEnd in
                    withAnimation(.easeOut(duration: 0.2)) { self.atEnd = atEnd }
                }
                .overlay(alignment: .bottom) {
                    if !atEnd {
                        Button {
                            withAnimation { proxy.scrollTo(Self.end, anchor: .bottom) }
                        } label: {
                            Image(systemName: "arrow.down")
                                .font(.body.weight(.semibold))
                                .frame(width: 40, height: 40)
                                .background(.regularMaterial, in: .circle)
                                .overlay { Circle().strokeBorder(.quaternary) }
                                .shadow(color: .black.opacity(0.12), radius: 6, y: 2)
                                .contentShape(.circle)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(Text("Go to Latest"))
                        .help("Go to the latest message")
                        .padding(.bottom, 10)
                        .transition(.opacity.combined(with: .scale(scale: 0.8)))
                    }
                }
        }
    }

    /// The end of the chat, to scroll back to.
    private static var end: String { "end" }

    private var scroll: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                parts.top()
                if store.thread.messages.isEmpty, !store.isReplying {
                    parts.empty()
                }
                let messages = store.thread.messages
                ForEach(Array(zip(messages.indices, messages)), id: \.1.id) { index, message in
                    if let mark = TimeMark.mark(at: message.at, after: index > 0 ? messages[index - 1].at : nil) {
                        TimeMarkView(message.at, mark: mark)
                    }
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
                        parts.after(message)
                        if message.id == store.thread.messages.last?.id, !store.isReplying, store.rival == nil {
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
                if store.isReplying {
                    // Its own view, so each word written redraws the reply
                    // alone, not every message above it.
                    Writing(store: store)
                }
                if let problem = store.problem {
                    Label(problem, systemImage: "exclamationmark.triangle")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                parts.bottom()
            }
            .overlay(alignment: .bottom) { Color.clear.frame(height: 1).id(Self.end) }
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
        let undo = parts.undo(of: message)
        MessageView(
            role: message.role, text: message.text, thinking: message.thinking?.model, skill: parts.tag(of: message),
            canUndo: undo != nil, showsThinking: store.shownThinking.contains(message.id),
            toggleThinking: { store.send(.thinkingToggled(messageID: message.id)) }, undo: { undo?() },
            select: { selecting = TextToSelect(text: $0) },
            attachments: {
                if !message.photos.isEmpty {
                    PhotoRow(
                        files: message.photos.map { store.photoFiles[$0.id] },
                        trailing: message.role == .person
                    ) { opened = .photo($0) }
                }
                ForEach(message.files, id: \.self) { file in
                    AttachmentChip(
                        verbatim: file.name, systemImage: "doc.text",
                        open: { opened = .file(name: file.name, text: file.text) })
                }
                parts.attached(message)
            }
        )
        .contextMenu { menu(for: message) }
        if message.role == .assistant {
            if message.stopped {
                Label("Stopped before it finished", systemImage: "stop.circle")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            parts.extra(message)
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
                CopyReplyButton(text: message.text)
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
                if store.stoppedBeforeReplying {
                    Label("Stopped before it replied", systemImage: "stop.circle")
                        .foregroundStyle(.secondary)
                } else {
                    Label("No reply came to this message.", systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.secondary)
                }
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
            if store.showsPrompt, store.prompts[message.id] != nil {
                Button("Show Prompt", systemImage: "text.magnifyingglass") {
                    store.send(.promptTapped(messageID: message.id))
                }
            }
        }
        parts.menu(message)
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
                    text: $store.input, prompt: fieldPrompt, model: modelPill, mic: mic, send: sendButton,
                    submit: { store.send(.sendTapped) }, top: { composerTop }, attach: { attachMenu }
                )
                .token(parts.token?.name) { parts.token?.remove() }
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
        #if os(macOS)
            Button("Paste Photo", systemImage: "doc.on.clipboard") { store.send(.photoPicked(Self.copiedPhoto)) }
                .disabled(store.attaching || Self.copiedPhoto == nil)
        #endif
    }

    #if os(macOS)
        /// A photo on the clipboard, as copied from Preview or a web page.
        private static var copiedPhoto: Data? {
            NSPasteboard.general.data(forType: .png) ?? NSPasteboard.general.data(forType: .tiff)
        }
    #endif

    /// The field with a + that sends `addContextTapped`.
    private var addingComposer: some View {
        ChatComposer(
            text: $store.input, prompt: fieldPrompt, model: modelPill, mic: mic, send: sendButton,
            submit: { store.send(.sendTapped) }, add: { store.send(.addContextTapped) }, top: { composerTop }
        )
        .token(parts.token?.name) { parts.token?.remove() }
    }

    /// What the field asks for: the app's once something's picked for the
    /// next message.
    private var fieldPrompt: Text? { parts.token?.prompt ?? prompt }

    private var mic: MicButton {
        parts.mic ?? MicButton(isOn: store.dictating, mode: .dictate) { store.send(.micTapped) }
    }

    private var sendButton: SendButton {
        SendButton(
            canSend: store.canSend, canStop: store.isReplying || store.rival?.done == false,
            send: { store.send(.sendTapped) },
            stop: { store.send(.stopTapped) })
    }

    /// Opens what the chat runs on: its model and how long it thinks.
    private var modelPill: ModelPill {
        let current = store.state.current
        return ModelPill(
            name: current?.name ?? String(localized: "Model"),
            detail: current?.thinks == true ? store.model.thinkingLevel.pillTitle : nil,
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
        if !store.pending.isEmpty || !store.pendingPhotos.isEmpty || store.attaching || store.dictating
            || parts.showsChips
        {
            ScrollView(.horizontal) {
                HStack(spacing: 8) {
                    parts.chips()
                    if store.dictating {
                        Label("Listening…", systemImage: "waveform")
                            .foregroundStyle(.red)
                            .font(.callout)
                    }
                    ForEach(store.pendingPhotos) { photo in
                        let file = store.photoFiles[photo.id]
                        ChatPhoto(
                            file: file, side: 56, open: file.map { file in { opened = .photo(file) } },
                            remove: { store.send(.removePhotoTapped(photo.id)) })
                    }
                    ForEach(Array(store.pending.enumerated()), id: \.offset) { index, file in
                        AttachmentChip(
                            verbatim: file.name, systemImage: "doc.text",
                            open: { opened = .file(name: file.name, text: file.text) },
                            remove: { store.send(.removeFileTapped(index)) })
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

/// A message's photos in a row, scrolling sideways when they don't fit,
/// each opening on its own when tapped. One photo shows larger.
/// The reply as it's written.
private struct Writing<Replier: ChatReplier>: View {
    let store: StoreOf<ChatCore<Replier>>

    var body: some View {
        if let reply = store.reply {
            ReplyInProgress(
                text: reply, thinking: store.replyThinking?.model,
                thinks: store.state.current?.thinks != false && store.model.thinkingLevel != .off,
                showsThinking: store.showsReplyThinking,
                toggleThinking: { store.send(.thinkingToggled(messageID: nil)) })
        }
    }
}

private struct PhotoRow: View {
    let files: [URL?]
    let trailing: Bool
    let open: (URL) -> Void

    var body: some View {
        let side: CGFloat = files.count == 1 ? 200 : 120
        ScrollView(.horizontal) {
            HStack(spacing: 6) {
                ForEach(files.indices, id: \.self) { index in
                    let file = files[index]
                    ChatPhoto(file: file, side: side, open: file.map { file in { open(file) } })
                }
            }
        }
        .scrollIndicators(.hidden)
        .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
        .defaultScrollAnchor(trailing ? .trailing : .leading)
        .frame(maxWidth: CGFloat(files.count) * (side + 6), alignment: trailing ? .trailing : .leading)
    }
}

/// Copy under the last reply, which says Copied for a moment after.
private struct CopyReplyButton: View {
    let text: String
    @State private var copied = false

    var body: some View {
        Button(copied ? "Copied" : "Copy", systemImage: copied ? "checkmark" : "doc.on.doc") {
            copyMessage(text)
            copied = true
        }
        .disabled(text.isEmpty)
        .help("Copy this reply")
        .sensoryFeedback(.success, trigger: copied) { _, copied in copied }
        .task(id: copied) {
            guard copied else { return }
            try? await Task.sleep(for: .seconds(2))
            copied = false
        }
    }
}

/// A photo sent in the chat or attached to the next message, opening when
/// tapped, with a button to take it off when `remove` is set. One that
/// can't be shown is a chip, which says so when its file isn't on this
/// device.
private struct ChatPhoto: View {
    let file: URL?
    var side: CGFloat = 160
    var open: (() -> Void)?
    var remove: (() -> Void)?

    var body: some View {
        if let image = file.flatMap(PhotoFile.image) {
            Button {
                open?()
            } label: {
                image
                    .resizable()
                    .scaledToFill()
                    .frame(width: side, height: side)
                    .clipShape(.rect(cornerRadius: side > 80 ? 14 : 10))
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .disabled(open == nil)
            .accessibilityLabel(Text("Photo"))
            .accessibilityHint(open == nil ? Text("") : Text("Opens it"))
            .contextMenu {
                if let file { PhotoMenu(file: file) }
            }
            .overlay(alignment: .topTrailing) {
                if let remove {
                    Button("Remove", systemImage: "xmark.circle.fill", action: remove)
                        .accessibilityLabel(Text("Remove Photo"))
                        .labelStyle(.iconOnly)
                        .buttonStyle(.borderless)
                        .symbolRenderingMode(.palette)
                        .foregroundStyle(.white, .black.opacity(0.6))
                        .padding(3)
                }
            }
        } else {
            AttachmentChip("Photo", systemImage: "photo", elsewhere: file == nil, remove: remove)
        }
    }
}

extension ChatThreadView {
    /// A chat that adds `empty` before the first message, and `extra` under
    /// each reply.
    public init<Empty: View, Extra: View>(
        store: StoreOf<ChatCore<Replier>>, prompt: Text? = nil, @ViewBuilder empty: () -> Empty,
        @ViewBuilder extra: @escaping (Message) -> Extra
    ) where Parts == BasicParts<Message, Empty, Extra> {
        self.init(store: store, prompt: prompt, parts: BasicParts(start: empty(), reply: extra))
    }

    public init<Empty: View>(
        store: StoreOf<ChatCore<Replier>>, prompt: Text? = nil, @ViewBuilder empty: () -> Empty
    ) where Parts == BasicParts<Message, Empty, EmptyView> {
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
        ContextButton(context)
    }
}

extension ThreadThinking {
    /// As ChatUI shows it.
    var model: ModelThinking { ModelThinking(text: text, duration: duration) }
}

/// A photo or document dropped on a chat, copied where the app can read
/// it, to attach as a picked one is.
private struct DroppedFile: Transferable {
    let url: URL

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(importedContentType: .item) { received in
            let folder = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            let copy = folder.appending(path: received.file.lastPathComponent)
            try FileManager.default.copyItem(at: received.file, to: copy)
            return DroppedFile(url: copy)
        }
    }
}
