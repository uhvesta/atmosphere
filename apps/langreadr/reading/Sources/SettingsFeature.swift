import ComposableArchitecture

/// Settings, opened from the library: the models LangReadr runs on.
@Reducer
package struct SettingsFeature {
    @ObservableState
    package struct State: Equatable {
        package var models = ModelsFeature.State()

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
