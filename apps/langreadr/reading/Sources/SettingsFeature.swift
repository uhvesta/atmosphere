import ComposableArchitecture
import ModelsFeatures

/// Settings, opened from the library: the models LangReadr runs on.
@Reducer
package struct SettingsFeature {
    @ObservableState
    package struct State: Equatable {
        package var models: ModelsFeature.State

        /// `languages` are the shelf's, as BCP 47 tags, each of which can
        /// have a voice of its own.
        package init(languages: [String] = []) {
            models = ModelsFeature.State(languages: languages)
        }
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
