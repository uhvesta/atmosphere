import CaptureKit
import ChatKit
import ChatUI
import ComposableArchitecture
import ModelsClient
import SwiftUI

/// A chat, from its first message to the field: each message with what the
/// model thought, the branches to switch between, and under the last reply
/// Retry and Speak. `empty` shows before the first message, and `extra`
/// shows what the app adds under a reply.
public struct ChatThreadView<Empty: View, Extra: View>: View {
    @Bindable var store: StoreOf<ChatThreadFeature>
    let prompt: Text?
    let empty: Empty
    let extra: (ThreadMessage) -> Extra

    @State private var selecting: TextToSelect?

    public init(
        store: StoreOf<ChatThreadFeature>, prompt: Text? = nil, @ViewBuilder empty: () -> Empty,
        @ViewBuilder extra: @escaping (ThreadMessage) -> Extra
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
            .fileImporter(
                isPresented: $store.picksFiles, allowedContentTypes: PickedFile.types(photos: false),
                allowsMultipleSelection: true
            ) { store.send(.filesPicked($0)) }
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
                        MessageView(
                            role: message.role, text: message.text, thinking: message.thinking?.model,
                            showsThinking: store.shownThinking.contains(message.id),
                            toggleThinking: { store.send(.thinkingToggled(messageID: message.id)) },
                            select: { selecting = TextToSelect(text: $0) },
                            attachments: {
                                ForEach(message.files, id: \.self) { file in
                                    AttachmentChip(verbatim: file.name, systemImage: "doc.text")
                                }
                            }
                        )
                        .contextMenu { menu(for: message) }
                        if message.role == .assistant {
                            extra(message)
                        }
                        branches(at: message)
                        if message.id == store.thread.messages.last?.id, store.reply == nil {
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
            .frame(maxWidth: 720)
            .padding()
            .frame(maxWidth: .infinity)
        }
        .defaultScrollAnchor(.bottom)
    }

    /// Under the last message: Retry and Speak for a reply, and for the
    /// person's message, that its reply didn't come.
    @ViewBuilder private func after(_ message: ThreadMessage) -> some View {
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
    private func speak(_ message: ThreadMessage) -> some View {
        let speaking = store.speaking == message.id
        return Button(
            speaking ? "Stop Speaking" : "Speak", systemImage: speaking ? "stop.circle" : "speaker.wave.2"
        ) { store.send(.speakTapped(messageID: message.id)) }
    }

    /// ‹ 2/3 › under a message or reply written again, to show the others.
    @ViewBuilder private func branches(at message: ThreadMessage) -> some View {
        let branches = store.thread.branches(at: message.id)
        if branches.count > 1, let index = branches.firstIndex(where: { $0.id == message.id }) {
            BranchSwitcher(index: index, count: branches.count, canGo: store.canGo) {
                store.send(.branchTapped(messageID: message.id, by: $0))
            }
        }
    }

    /// Copying a message, editing it or writing its reply again.
    @ViewBuilder private func menu(for message: ThreadMessage) -> some View {
        TextMenu(text: message.text) { selecting = TextToSelect(text: $0) }
        Divider()
        if message.role == .person {
            Button("Edit", systemImage: "pencil") { store.send(.editTapped(messageID: message.id)) }
                .disabled(!store.canGo)
        } else {
            Button("Retry", systemImage: "arrow.clockwise") { store.send(.retryTapped(messageID: message.id)) }
                .disabled(!store.state.canRetry(message))
            speak(message)
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

    /// The field, with the chat's model, the mic, + for files, and above it
    /// what's being edited and attached.
    private var composer: some View {
        ChatComposer(
            text: $store.input, prompt: prompt, model: modelPill,
            mic: MicButton(isOn: store.dictating, mode: .dictate) { store.send(.micTapped) },
            send: SendButton(
                canSend: store.canSend, canStop: store.reply != nil, send: { store.send(.sendTapped) },
                stop: { store.send(.stopTapped) }),
            submit: { store.send(.sendTapped) }, add: { store.send(.addContextTapped) }, top: { composerTop })
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
        if !store.pending.isEmpty || store.dictating {
            ScrollView(.horizontal) {
                HStack(spacing: 8) {
                    if store.dictating {
                        Label("Listening…", systemImage: "waveform")
                            .foregroundStyle(.red)
                            .font(.callout)
                    }
                    ForEach(Array(store.pending.enumerated()), id: \.offset) { index, file in
                        AttachmentChip(verbatim: file.name, systemImage: "doc.text") {
                            store.send(.removeFileTapped(index))
                        }
                    }
                }
            }
            .scrollIndicators(.hidden)
        }
    }
}

extension ChatThreadView where Extra == EmptyView {
    public init(store: StoreOf<ChatThreadFeature>, prompt: Text? = nil, @ViewBuilder empty: () -> Empty) {
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
