import ChatFeatures
import ComposableArchitecture
import ModelsFeatures

/// Settings, opened from the chats: the models they run on, voices, and
/// experiments to try in the chat.
@Reducer
package struct SettingsFeature {
    @ObservableState
    package struct State: Equatable {
        package var models = ModelsFeature.State(languages: [])
        /// The chat's experiments, kept on this device and off until turned
        /// on.
        @Shared(.chatShowsSpeed) package var showsSpeed
        @Shared(.chatShowsPrompt) package var showsPrompt
        @Shared(.chatCompares) package var compares

        package init() {}
    }

    package enum Action {
        case models(ModelsFeature.Action)
        case doneTapped
    }

    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        Scope(state: \.models, action: \.models) { ModelsFeature() }
        Reduce { _, action in
            switch action {
            case .doneTapped:
                return .run { [dismiss] _ in await dismiss() }
            case .models:
                return .none
            }
        }
    }
}
