import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrCore

/// The conversations with the assistant, newest first, and the open one.
/// Each is kept as it changes; deleting one never deletes tasks.
@Reducer
package struct ConversationsFeature {
    @ObservableState
    package struct State: Equatable {
        package var conversations: IdentifiedArrayOf<Conversation> = []
        package var workspace = Workspace()
        @Presents package var chat: ChatFeature.State?
        @Presents package var confirm: ConfirmationDialogState<Confirm>?
        @Presents package var alert: AlertState<Never>?

        package init() {}

        package var sorted: [Conversation] {
            conversations.sorted { ($0.updatedAt, $0.id.uuidString) > ($1.updatedAt, $1.id.uuidString) }
        }

        package mutating func show(_ workspace: Workspace) {
            self.workspace = workspace
            chat?.workspace = workspace
        }
    }

    package enum Confirm: Equatable, Sendable {
        case deleteAll
    }

    package enum Action {
        case task
        case loaded(Result<[Conversation], any Error>)
        case newTapped
        case opened(UUID)
        case deleteTapped(UUID)
        case deleteAllTapped
        case failed(any Error)
        case chat(PresentationAction<ChatFeature.Action>)
        case confirm(PresentationAction<Confirm>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
        }
    }

    @Dependency(\.conversations) private var conversations
    @Dependency(\.uuid) private var uuid
    @Dependency(\.date.now) private var now

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                return .run { [conversations] send in
                    await send(.loaded(Result { try await conversations.load() }))
                }

            case .loaded(.success(let loaded)):
                state.conversations = IdentifiedArray(uniqueElements: loaded)
                if let open = state.chat?.conversation, !open.messages.isEmpty {
                    state.conversations[id: open.id] = open
                }
                return .none

            case .newTapped:
                state.chat = ChatFeature.State(
                    conversation: Conversation(id: uuid(), createdAt: now), workspace: state.workspace)
                return .none

            case .opened(let id):
                guard let conversation = state.conversations[id: id] else { return .none }
                state.chat = ChatFeature.State(conversation: conversation, workspace: state.workspace)
                return .none

            case .deleteTapped(let id):
                state.conversations.remove(id: id)
                if state.chat?.conversation.id == id { state.chat = nil }
                return .run { [conversations] _ in
                    try await conversations.delete([id])
                } catch: { error, send in
                    await send(.failed(error))
                }

            case .deleteAllTapped:
                state.confirm = ConfirmationDialogState {
                    TextState("Delete all conversations?")
                } actions: {
                    ButtonState(role: .destructive, action: .deleteAll) { TextState("Delete All") }
                } message: {
                    TextState(
                        "They're deleted on this device and from iCloud. Tasks the assistant added or changed stay.")
                }
                return .none

            case .confirm(.presented(.deleteAll)):
                state.conversations = []
                state.chat = nil
                return .run { [conversations] _ in
                    try await conversations.deleteAll()
                } catch: { error, send in
                    await send(.failed(error))
                }

            case .loaded(.failure(let error)), .failed(let error):
                state.alert = AlertState {
                    TextState("Conversations")
                } message: {
                    TextState(String(describing: error))
                }
                return .none

            case .chat(.presented(.delegate(.saved(let workspace)))):
                state.show(workspace)
                return .send(.delegate(.saved(workspace)))

            case .chat, .confirm, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$chat, action: \.chat) { ChatFeature() }
        .ifLet(\.$confirm, action: \.confirm)
        .ifLet(\.$alert, action: \.alert)
        // Keep each change to the open chat. A chat with no messages isn't
        // kept.
        Reduce { state, _ in
            guard let conversation = state.chat?.conversation, !conversation.messages.isEmpty,
                state.conversations[id: conversation.id] != conversation
            else { return .none }
            state.conversations[id: conversation.id] = conversation
            return .run { [conversations] _ in
                try await conversations.save(conversation)
            } catch: { error, send in
                await send(.failed(error))
            }
        }
    }
}
