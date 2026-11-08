import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrModelsClient

/// Picks what one conversation runs on: its model, how long that thinks
/// and its window, and whether Lifr picks a skill for a message sent
/// without one. Each starts at the default from Settings, which this
/// never changes, and picking the default goes back to following it. The
/// next message uses what's picked.
@Reducer
package struct ChatModelFeature {
    @ObservableState
    package struct State: Equatable {
        package var choice: ModelChoice
        /// Whether the conversation picks skills, or nil to follow Settings.
        package var picksSkills: Bool?
        /// Apple's model and the downloaded ones, once they're looked up.
        package var models: [RunnableModel] = []
        /// The assistant's model in Settings.
        package var standard: ModelID?
        /// Whether Lifr picks skills, in Settings.
        @Shared(.picksSkills) package var standardPicksSkills

        package init(choice: ModelChoice, picksSkills: Bool? = nil) {
            self.choice = choice
            self.picksSkills = picksSkills
        }

        /// The model the conversation runs on, once it's looked up.
        package var current: RunnableModel? {
            let id = choice.model ?? standard
            return models.first { $0.id == id }
        }

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

        /// Whether the conversation picks skills. Matching Settings goes
        /// back to following it.
        package var picksSkillsHere: Bool {
            get { picksSkills ?? standardPicksSkills }
            set { picksSkills = newValue == standardPicksSkills ? nil : newValue }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case loaded([RunnableModel], standard: ModelID)
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

            case .doneTapped:
                return .run { [dismiss] _ in await dismiss() }

            case .binding:
                return .none
            }
        }
    }
}
