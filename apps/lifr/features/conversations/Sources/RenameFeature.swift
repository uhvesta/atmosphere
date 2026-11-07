import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrModelsClient

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
        /// The model that suggests a name this time, or nil for the one
        /// picked for names in Settings.
        package var model: ModelID?
        /// Apple's model and the downloaded ones, once they're looked up.
        package var models: [RunnableModel] = []
        /// The model picked for names in Settings, once it's looked up,
        /// and its name, even when it isn't downloaded.
        package var standard: Suggester?
        /// Why the last suggestion failed.
        package var problem: String?

        package init(_ conversation: Conversation) {
            id = conversation.id
            name = conversation.name ?? ""
            messages = conversation.messages
        }

        package var canSuggest: Bool { !suggesting && !messages.isEmpty }

        /// The model that suggests the name, such as "Gemma 4 E2B", once
        /// it's looked up.
        package var suggester: Suggester? {
            guard let model else { return standard }
            let name = models.first { $0.id == model }?.name ?? model.rawValue
            return Suggester(id: model, name: name)
        }
    }

    /// The model that suggests names.
    package struct Suggester: Equatable, Sendable {
        package var id: ModelID
        package var name: String

        package var isApple: Bool { id == .apple }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case modelsLoaded([RunnableModel], standard: Suggester)
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
    @Dependency(\.models) private var models
    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .task:
                return .run { [models] send in
                    let id = models.selection(.titles)
                    let runnable = await models.runnable()
                    let name =
                        runnable.first { $0.id == id }?.name ?? ModelCatalog.all.first { $0.id == id }?.name
                        ?? id.rawValue
                    await send(.modelsLoaded(runnable, standard: Suggester(id: id, name: name)))
                }

            case .modelsLoaded(let runnable, let standard):
                state.models = runnable
                state.standard = standard
                return .none

            case .suggestTapped:
                guard state.canSuggest else { return .none }
                state.suggesting = true
                state.problem = nil
                let messages = state.messages
                let model = state.model
                return .run { [assistant] send in
                    await send(.suggested(Result { try await assistant.suggestTitle(messages, model) }))
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
