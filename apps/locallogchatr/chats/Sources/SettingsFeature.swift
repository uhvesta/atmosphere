import ComposableArchitecture
import ModelsFeatures

/// Settings, opened from the chats: the models they run on, and voices.
@Reducer
package struct SettingsFeature {
    @ObservableState
    package struct State: Equatable {
        package var models = ModelsFeature.State(languages: [])

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
