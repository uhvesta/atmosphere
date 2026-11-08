import ChatFeatures
import ChatKit
import ComposableArchitecture
import Foundation
import InferenceKit
import LangReadrCore

/// A chat about a passage, on the shared chat: the model is told the book,
/// the passage and its paragraph, and each message is kept with the book.
@Reducer
package struct PassageChatFeature {
    @ObservableState
    package struct State: Equatable, Identifiable {
        package var chat: AnchoredChat
        package var book: ChatBook
        /// Small beside the text, or the whole screen.
        package var compact: Bool
        package var thread: ChatThreadFeature.State

        package var id: UUID { chat.id }

        /// The chat about `chat`'s passage of `book`, explained in `reader`.
        package init(chat: AnchoredChat, book: ChatBook, compact: Bool, reader: Language) {
            self.chat = chat
            self.book = book
            self.compact = compact
            thread = ChatThreadFeature.State(
                thread: chat.conversation, model: ModelChoice(chat.model), use: .chat,
                instructions: book.instructions(about: chat.passage, for: reader))
        }
    }

    package enum Action {
        case openFullTapped
        case doneTapped
        case thread(ChatThreadFeature.Action)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            /// A message or the chat's model changed, for the book to keep.
            case changed(AnchoredChat)
        }
    }

    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        Scope(state: \.thread, action: \.thread) { ChatThreadFeature() }
        Reduce { state, action in
            switch action {
            case .openFullTapped:
                state.compact = false
                return .none
            case .doneTapped:
                return .run { _ in await dismiss() }
            case .thread(.delegate(.changed(let thread, let model))):
                state.chat.conversation = thread
                state.chat.model = model.kept
                return .send(.delegate(.changed(state.chat)))
            case .thread, .delegate:
                return .none
            }
        }
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
