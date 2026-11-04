import ComposableArchitecture
import Foundation
import LifrAssistantClient

/// Renames a conversation, with a title the assistant can suggest. A name
/// saved here is the person's, so the assistant never replaces it.
@Reducer
package struct RenameFeature {
    @ObservableState
    package struct State: Equatable {
        package var id: UUID
        package var name: String
        /// What the suggestion is written from.
        var messages: [ChatMessage]
        package var suggesting = false
        /// Why the last suggestion failed.
        package var problem: String?

        package init(_ conversation: Conversation) {
            id = conversation.id
            name = conversation.name ?? ""
            messages = conversation.messages
        }

        package var canSuggest: Bool { !suggesting && !messages.isEmpty }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case suggestTapped
        case suggested(Result<String, any Error>)
        case saveTapped
        case cancelTapped
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case renamed(UUID, String)
        }
    }

    @Dependency(\.assistant) private var assistant
    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .suggestTapped:
                guard state.canSuggest else { return .none }
                state.suggesting = true
                state.problem = nil
                let messages = state.messages
                return .run { [assistant] send in
                    await send(.suggested(Result { try await assistant.suggestTitle(messages) }))
                }

            case .suggested(.success(let name)):
                state.suggesting = false
                state.name = name
                return .none

            case .suggested(.failure(let error)):
                state.suggesting = false
                state.problem = String(describing: error)
                return .none

            case .saveTapped:
                return .send(.delegate(.renamed(state.id, state.name)))

            case .cancelTapped:
                return .run { [dismiss] _ in await dismiss() }

            case .binding, .delegate:
                return .none
            }
        }
    }
}
