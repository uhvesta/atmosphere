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

        package var id: UUID { chat.id }

        package init(chat: AnchoredChat, book: ChatBook, compact: Bool) {
            self.chat = chat
            self.book = book
            self.compact = compact
        }

        package var canSend: Bool { reply == nil && !input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case sendTapped
        case stopTapped
        case openFullTapped
        case doneTapped
        case modelTapped
        case model(PresentationAction<ChatModelFeature.Action>)
        case partial(String)
        case replied(Result<String, ModelRuntimeError>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            /// A message was added, for the book to keep.
            case changed(AnchoredChat)
        }
    }

    private enum CancelID { case reply }

    @Dependency(\.tutor) private var tutor
    @Dependency(\.date.now) private var now
    @Dependency(\.uuid) private var uuid
    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding:
                return .none
            case .sendTapped:
                guard state.canSend else { return .none }
                let text = state.input.trimmingCharacters(in: .whitespacesAndNewlines)
                state.input = ""
                state.problem = nil
                state.chat.conversation.append(ReaderMessage(id: uuid(), role: .person, text: text, at: now))
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
                // Kept as it's picked, so the next message runs on it.
                guard let choice = state.model?.choice.kept, choice != state.chat.model else { return .none }
                state.chat.model = choice
                return .send(.delegate(.changed(state.chat)))
            case .partial(let text):
                if state.reply != nil { state.reply = text }
                return .none
            case .replied(.success(let text)):
                return keep(text, in: &state)
            case .replied(.failure(let error)):
                state.reply = nil
                state.problem = error.description
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
