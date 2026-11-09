import ChatUI
import ComposableArchitecture
import Foundation
import ModelsClient
import SwiftUI

/// Picks what one chat runs on: its model, how long that thinks, its
/// window, and in an app that picks skills, whether the chat picks one for
/// a message sent without one. Each starts at the app's default, which
/// this never changes, and picking the default goes back to following it.
/// The next message uses what's picked.
@Reducer
public struct ChatModelFeature {
    @ObservableState
    public struct State: Equatable {
        public var choice: ModelChoice
        /// Which of the app's uses gives the default.
        public var use: ModelUseKey
        /// Apple's model and the downloaded ones, once they're looked up.
        public var models: [RunnableModel] = []
        /// The model the use runs on in the Models screen.
        public var standard: ModelID?
        /// Whether the chat picks skills, or nil to follow the app.
        public var picksSkills: Bool?
        /// Whether the app picks skills, or nil for one with none to pick.
        public var standardPicksSkills: Bool?

        public init(
            choice: ModelChoice, use: ModelUseKey, picksSkills: Bool? = nil, standardPicksSkills: Bool? = nil
        ) {
            self.choice = choice
            self.use = use
            self.picksSkills = picksSkills
            self.standardPicksSkills = standardPicksSkills
        }

        /// Whether the chat picks skills. Matching the app goes back to
        /// following it.
        public var picksSkillsHere: Bool {
            get { picksSkills ?? standardPicksSkills ?? false }
            set { picksSkills = newValue == standardPicksSkills ? nil : newValue }
        }

        /// The model the chat runs on, once it's looked up.
        public var current: RunnableModel? {
            let id = choice.model ?? standard
            return models.first { $0.id == id }
        }

        /// The model picked for the chat, or nil for the default.
        /// A model that was deleted reads as the default.
        public var model: ModelID? {
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

    public enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case loaded([RunnableModel], standard: ModelID)
        case doneTapped
    }

    @Dependency(\.models) private var models
    @Dependency(\.dismiss) private var dismiss

    public init() {}

    public var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .task:
                return .run { [models, use = state.use] send in
                    await send(.loaded(await models.runnable(), standard: models.selection(use)))
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

/// The sheet of what a chat runs on.
public struct ChatModelView: View {
    @Bindable var store: StoreOf<ChatModelFeature>

    public init(store: StoreOf<ChatModelFeature>) {
        self.store = store
    }

    public var body: some View {
        ModelSheet(
            model: $store.model, thinking: $store.choice.thinking, window: $store.choice.window,
            picksSkills: store.standardPicksSkills == nil ? nil : $store.picksSkillsHere, models: store.models,
            standard: store.standard, done: { store.send(.doneTapped) }
        )
        .task { await store.send(.task).finish() }
    }
}
