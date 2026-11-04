import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrCore

/// The conversations with the assistant, newest first, and the open one.
/// Each is kept as it changes; deleting one never deletes tasks. A chat
/// stays here while it's open or its reply is still being written, so
/// leaving it or opening another doesn't stop the reply.
@Reducer
package struct ConversationsFeature {
    @ObservableState
    package struct State: Equatable {
        package var conversations: IdentifiedArrayOf<Conversation> = []
        package var workspace = Workspace()
        /// The open chat, and chats whose replies are still being written.
        package var chats: IdentifiedArrayOf<ChatFeature.State> = []
        /// The ID of the chat on screen.
        package var openID: UUID?
        @Presents package var confirm: ConfirmationDialogState<Confirm>?
        @Presents package var alert: AlertState<Never>?

        package init() {}

        package var sorted: [Conversation] {
            conversations.sorted { ($0.updatedAt, $0.id.uuidString) > ($1.updatedAt, $1.id.uuidString) }
        }

        /// The chat on screen.
        package var chat: ChatFeature.State? { openID.flatMap { chats[id: $0] } }

        package mutating func show(_ workspace: Workspace) {
            self.workspace = workspace
            for id in chats.ids { chats[id: id]?.workspace = workspace }
        }

        /// Opens `conversation`, keeping its chat if it's still working.
        mutating func open(_ conversation: Conversation) {
            if chats[id: conversation.id] == nil {
                chats.append(ChatFeature.State(conversation: conversation, workspace: workspace))
            }
            openID = conversation.id
            prune()
        }

        /// Lets go of chats that are neither on screen nor working.
        mutating func prune() {
            let openID = openID
            chats.removeAll { $0.id != openID && !$0.isWorking }
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
        /// The chat on screen was left.
        case closed
        /// Lifr is back on screen: replies the system stopped go again.
        case resumed
        case deleteTapped(UUID)
        case deleteAllTapped
        case failed(any Error)
        case chats(IdentifiedActionOf<ChatFeature>)
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
                for chat in state.chats where !chat.conversation.messages.isEmpty {
                    state.conversations[id: chat.id] = chat.conversation
                }
                return .none

            case .newTapped:
                state.open(Conversation(id: uuid(), createdAt: now))
                return .none

            case .opened(let id):
                guard let conversation = state.conversations[id: id] else { return .none }
                state.open(conversation)
                return .none

            case .closed:
                state.openID = nil
                state.prune()
                return .none

            case .resumed:
                return .merge(
                    state.chats.filter { $0.waiting != nil }.map { .send(.chats(.element(id: $0.id, action: .resume))) }
                )

            case .deleteTapped(let id):
                state.conversations.remove(id: id)
                state.chats.remove(id: id)
                if state.openID == id { state.openID = nil }
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
                state.chats = []
                state.openID = nil
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

            case .chats(.element(_, .delegate(.saved(let workspace)))):
                state.show(workspace)
                return .send(.delegate(.saved(workspace)))

            case .chats, .confirm, .alert, .delegate:
                return .none
            }
        }
        .forEach(\.chats, action: \.chats) { ChatFeature() }
        .ifLet(\.$confirm, action: \.confirm)
        .ifLet(\.$alert, action: \.alert)
        // Keep each change to a chat, on screen or not. A chat with no
        // messages isn't kept. A chat that finished off screen is let go.
        Reduce { state, action in
            let changed = state.chats.map(\.conversation).filter {
                !$0.messages.isEmpty && state.conversations[id: $0.id] != $0
            }
            for conversation in changed { state.conversations[id: conversation.id] = conversation }
            if case .chats = action { state.prune() }
            guard !changed.isEmpty else { return .none }
            return .run { [conversations] _ in
                for conversation in changed { try await conversations.save(conversation) }
            } catch: { error, send in
                await send(.failed(error))
            }
        }
    }
}
