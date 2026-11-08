import CaptureKit
import ChatKit
import ComposableArchitecture
import Foundation
import InferenceKit
import LangReadrCore

/// A chat about a passage: the person asks, the model answers as it
/// writes, and each message is kept with the book.
@Reducer
package struct PassageChatFeature {
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var chat: AnchoredChat
        package var book: ChatBook
        /// Small beside the text, or the whole screen.
        package var compact: Bool
        package var input = ""
        /// The reply as it's written; nil when none is.
        package var reply: String?
        package var problem: String?
        /// The sheet picking what the chat runs on.
        @Presents package var model: ChatModelFeature.State?
        /// Apple's model and the downloaded ones, for the composer's pill.
        package var models: [RunnableModel] = []
        /// The chats' model in Settings.
        package var standard: ModelID?
        /// How much of the model's window the last reply took; nil before
        /// one, and once the model or its window changes.
        package var context: ContextUse?
        /// The files that go with the next message.
        package var pending: [ReaderFile] = []
        /// Whether the file picker is open. Chats read only text, so +
        /// opens it straight away.
        package var picksFiles = false
        /// Whether the microphone is writing what's said into the field.
        package var dictating = false
        /// What the field held when dictation started, which what's said
        /// follows; nil once the dictation has ended.
        package var dictatedAfter: String?

        package var id: UUID { chat.id }

        package init(chat: AnchoredChat, book: ChatBook, compact: Bool) {
            self.chat = chat
            self.book = book
            self.compact = compact
        }

        package var canSend: Bool {
            reply == nil && dictatedAfter == nil
                && (!input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !pending.isEmpty)
        }

        /// The model the chat runs on, once it's looked up.
        package var current: RunnableModel? {
            let id = ModelChoice(chat.model).model ?? standard
            return models.first { $0.id == id }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case modelsLoaded([RunnableModel], standard: ModelID)
        case sendTapped
        case stopTapped
        case openFullTapped
        case doneTapped
        case modelTapped
        case model(PresentationAction<ChatModelFeature.Action>)
        case partial(String)
        case replied(Result<TutorReply, ModelRuntimeError>)
        case addContextTapped
        case filesPicked(Result<[URL], any Error>)
        case filesRead([ReaderFile], failures: [String])
        case removeFileTapped(Int)
        case micTapped
        case dictated(String)
        case dictationEnded
        case dictationFailed(String)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            /// A message was added, for the book to keep.
            case changed(AnchoredChat)
        }
    }

    private enum CancelID { case reply, dictation }

    @Dependency(\.tutor) private var tutor
    @Dependency(\.models) private var library
    @Dependency(\.date.now) private var now
    @Dependency(\.uuid) private var uuid
    @Dependency(\.dismiss) private var dismiss
    @Dependency(\.dictation) private var dictation
    @Dependency(\.fileReader) private var fileReader

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding:
                return .none
            case .task:
                return .run { [library] send in
                    await send(.modelsLoaded(await library.runnable(), standard: library.selection(.chat)))
                }
            case .modelsLoaded(let models, let standard):
                state.models = models
                state.standard = standard
                return .none
            case .sendTapped:
                guard state.canSend else { return .none }
                let text = state.input.trimmingCharacters(in: .whitespacesAndNewlines)
                state.input = ""
                state.problem = nil
                state.chat.conversation.append(
                    ReaderMessage(id: uuid(), role: .person, text: text, at: now, files: state.pending))
                state.pending = []
                state.reply = ""
                return .merge(.send(.delegate(.changed(state.chat))), write(state))
            case .stopTapped:
                guard let reply = state.reply else { return .none }
                return .merge(.cancel(id: CancelID.reply), keep(reply, in: &state))
            case .openFullTapped:
                state.compact = false
                return .none
            case .doneTapped:
                return .run { _ in await dismiss() }
            case .modelTapped:
                state.model = ChatModelFeature.State(choice: ModelChoice(state.chat.model))
                return .none
            case .model:
                if let sheet = state.model, !sheet.models.isEmpty {
                    state.models = sheet.models
                    state.standard = sheet.standard
                }
                // Kept as it's picked, so the next message runs on it.
                guard let choice = state.model?.choice.kept, choice != state.chat.model else { return .none }
                if choice.model != state.chat.model.model || choice.window != state.chat.model.window {
                    state.context = nil
                }
                state.chat.model = choice
                return .send(.delegate(.changed(state.chat)))
            case .partial(let text):
                if state.reply != nil { state.reply = text }
                return .none
            case .replied(.success(let reply)):
                state.context = reply.context
                return keep(reply.text, in: &state)
            case .replied(.failure(let error)):
                state.reply = nil
                state.problem = error.description
                return .none
            case .addContextTapped:
                state.picksFiles = true
                return .none
            case .filesPicked(.failure(let error)):
                state.picksFiles = false
                state.problem = String(
                    localized: "Couldn't open that file: \(error.localizedDescription) Try picking it again.")
                return .none
            case .filesPicked(.success(let urls)):
                state.picksFiles = false
                guard !urls.isEmpty else { return .none }
                return .run { [fileReader] send in
                    var read: [ReaderFile] = []
                    var failures: [String] = []
                    for url in urls {
                        do {
                            switch try await fileReader.read(url) {
                            case .text(let file):
                                read.append(ReaderFile(name: file.name, text: file.text))
                            case .photo:
                                failures.append(
                                    String(
                                        localized: """
                                            \(url.lastPathComponent) is a photo, and chats read only text. \
                                            Add a text, Markdown or PDF file instead.
                                            """))
                            }
                        } catch {
                            failures.append(String(describing: error))
                        }
                    }
                    await send(.filesRead(read, failures: failures))
                }
            case .filesRead(let files, let failures):
                state.pending += files
                state.problem = failures.isEmpty ? nil : failures.joined(separator: "\n")
                return .none
            case .removeFileTapped(let index):
                guard state.pending.indices.contains(index) else { return .none }
                state.pending.remove(at: index)
                return .none
            case .micTapped:
                if state.dictating {
                    // The last words still come, until the dictation ends.
                    state.dictating = false
                    return .run { [dictation] _ in await dictation.stop() }
                }
                guard state.dictatedAfter == nil else { return .none }
                state.dictating = true
                state.problem = nil
                state.dictatedAfter = state.input.trimmingCharacters(in: .whitespacesAndNewlines)
                return .run { [dictation] send in
                    do {
                        for try await heard in try await dictation.start() {
                            await send(.dictated(heard))
                        }
                        await send(.dictationEnded)
                    } catch is CancellationError {
                        await send(.dictationEnded)
                    } catch {
                        await send(.dictationFailed(String(describing: error)))
                    }
                }
                .cancellable(id: CancelID.dictation, cancelInFlight: true)
            case .dictated(let heard):
                guard let before = state.dictatedAfter else { return .none }
                state.input = before.isEmpty || heard.isEmpty ? before + heard : before + " " + heard
                return .none
            case .dictationEnded:
                state.dictating = false
                state.dictatedAfter = nil
                return .none
            case .dictationFailed(let reason):
                state.dictating = false
                state.dictatedAfter = nil
                state.problem = reason
                return .none
            case .delegate:
                return .none
            }
        }
        .ifLet(\.$model, action: \.model) { ChatModelFeature() }
    }

    /// Writes the reply to the last message, sending it as it's written.
    private func write(_ state: State) -> Effect<Action> {
        .run { [tutor, chat = state.chat, book = state.book] send in
            let (partials, continuation) = AsyncStream.makeStream(of: String.self, bufferingPolicy: .bufferingNewest(1))
            async let written = outcome { () throws(ModelRuntimeError) in
                defer { continuation.finish() }
                return try await tutor.reply(in: chat, about: book) { continuation.yield($0) }
            }
            for await text in partials { await send(.partial(text)) }
            await send(.replied(await written))
        }
        .cancellable(id: CancelID.reply, cancelInFlight: true)
    }

    /// Ends the reply, keeping what's written of it.
    private func keep(_ text: String, in state: inout State) -> Effect<Action> {
        state.reply = nil
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return .none }
        state.chat.conversation.append(ReaderMessage(id: uuid(), role: .assistant, text: text, at: now))
        return .send(.delegate(.changed(state.chat)))
    }
}

/// The chats about a book's passages, the most recent first.
@Reducer
package struct ChatsFeature {
    @ObservableState
    package struct State: Equatable {
        package var chats: [AnchoredChat]

        package init(chats: [AnchoredChat]) {
            self.chats = chats.sorted { $0.conversation.updatedAt > $1.conversation.updatedAt }
        }
    }

    package enum Action: Equatable {
        case chatTapped(UUID)
        case doneTapped
    }

    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { _, action in
            switch action {
            case .chatTapped:
                return .none
            case .doneTapped:
                return .run { _ in await dismiss() }
            }
        }
    }
}
