import ComposableArchitecture
import Foundation
import LifrCore

/// The conversations with the assistant, newest first, and the open one.
@Reducer
package struct ConversationsFeature {
    @ObservableState
    package struct State: Equatable {
        package var conversations: IdentifiedArrayOf<Conversation> = []
        package var workspace = Workspace()
        @Presents package var chat: ChatFeature.State?

        package init() {}

        package var sorted: [Conversation] {
            conversations.sorted { ($0.updatedAt, $0.id.uuidString) > ($1.updatedAt, $1.id.uuidString) }
        }

        package mutating func show(_ workspace: Workspace) {
            self.workspace = workspace
            chat?.workspace = workspace
        }
    }

    package enum Action {
        case newTapped
        case opened(UUID)
        case chat(PresentationAction<ChatFeature.Action>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
        }
    }

    @Dependency(\.uuid) private var uuid
    @Dependency(\.date.now) private var now

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .newTapped:
                state.chat = ChatFeature.State(
                    conversation: Conversation(id: uuid(), createdAt: now), workspace: state.workspace)
                return .none

            case .opened(let id):
                guard let conversation = state.conversations[id: id] else { return .none }
                state.chat = ChatFeature.State(conversation: conversation, workspace: state.workspace)
                return .none

            case .chat(.presented(.delegate(.saved(let workspace)))):
                state.show(workspace)
                return .send(.delegate(.saved(workspace)))

            case .chat, .delegate:
                return .none
            }
        }
        .ifLet(\.$chat, action: \.chat) { ChatFeature() }
        // Keep the list in step with the open chat. A chat with no messages
        // isn't kept.
        Reduce { state, _ in
            if let conversation = state.chat?.conversation, !conversation.messages.isEmpty {
                state.conversations[id: conversation.id] = conversation
            }
            return .none
        }
    }
}
