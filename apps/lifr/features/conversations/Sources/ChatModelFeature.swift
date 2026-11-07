import ComposableArchitecture
import Foundation
import LifrModelsClient

/// Picks what one conversation runs on: its model, how long that thinks
/// and its window. Each starts at the default from Settings, which this
/// never changes. The next message uses what's picked.
@Reducer
package struct ChatModelFeature {
    @ObservableState
    package struct State: Equatable {
        package var choice: ModelChoice
        /// Apple's model and the downloaded ones, once they're looked up.
        package var models: [RunnableModel] = []
        /// The assistant's model in Settings.
        package var standard: ModelID?

        package init(choice: ModelChoice) {
            self.choice = choice
        }

        /// The model the conversation runs on, once it's looked up.
        package var current: RunnableModel? {
            let id = choice.model ?? standard
            return models.first { $0.id == id }
        }

        package var standardName: String? { models.first { $0.id == standard }?.name }

        /// The model picked for the conversation, or nil for the default.
        /// A model that was deleted reads as the default.
        package var model: ModelID? {
            get { choice.model }
            set {
                choice.model = newValue
                // A window the new model doesn't offer goes back to its own.
                if let window = choice.window, !(current?.windows.contains(window) ?? false) {
                    choice.window = nil
                }
            }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case loaded([RunnableModel], standard: ModelID)
        case resetTapped
        case doneTapped
    }

    @Dependency(\.models) private var models
    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .task:
                return .run { [models] send in
                    await send(.loaded(await models.runnable(), standard: models.selection(.assistant)))
                }

            case .loaded(let runnable, let standard):
                state.models = runnable
                state.standard = standard
                if let picked = state.choice.model, !runnable.contains(where: { $0.id == picked }) {
                    state.model = nil
                }
                return .none

            case .resetTapped:
                state.choice = .defaults
                return .none

            case .doneTapped:
                return .run { [dismiss] _ in await dismiss() }

            case .binding:
                return .none
            }
        }
    }
}
