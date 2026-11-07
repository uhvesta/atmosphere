import ChatUI
import ComposableArchitecture
import LifrModelsClient
import SwiftUI

/// What one conversation runs on: each choice starts at the default from
/// Settings, and Settings stays as it is.
struct ChatModelView: View {
    @Bindable var store: StoreOf<ChatModelFeature>

    var body: some View {
        ModelChoiceForm(
            model: $store.model, thinking: $store.choice.thinking, window: $store.choice.window, models: store.models,
            standard: store.standard, reset: { store.send(.resetTapped) }, done: { store.send(.doneTapped) }
        )
        .task { await store.send(.task).finish() }
    }
}
