import ChatUI
import ComposableArchitecture
import LocalLogChatrModels
import ModelsFeatures
import SwiftUI

/// Settings: the models the chats run on, and experiments.
struct SettingsView: View {
    @Bindable var store: StoreOf<SettingsFeature>

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    NavigationLink("Models") {
                        ModelsView(store: store.scope(state: \.models, action: \.models))
                    }
                } footer: {
                    Text("Chats run on a model on this device. Nothing you write leaves it.")
                }
                Section {
                    Toggle(isOn: Binding(store.$showsSpeed)) {
                        Text("Speed")
                        Text(
                            "Under each reply: its model, tokens a second, time to first token, tokens in and out."
                        )
                    }
                    Toggle(isOn: Binding(store.$showsPrompt)) {
                        Text("Prompt")
                        Text("A button under each reply shows exactly what the model was sent.")
                    }
                    Toggle(isOn: Binding(store.$compares)) {
                        Text("Compare")
                        Text("Ask a second model too, see both answers and keep one.")
                    }
                } header: {
                    Text("Experiments")
                } footer: {
                    Text(
                        "Apple's model doesn't count tokens, so its counts are guessed from the text and marked ≈."
                    )
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Settings")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { store.send(.doneTapped) }
                }
            }
        }
        .sheetLayout(height: 560)
    }
}

/// The Models screen with LocalLogChatr's one use.
struct ModelsView: View {
    let store: StoreOf<ModelsFeature>

    var body: some View {
        ModelsSettings(
            store: store, app: "LangReadr", uses: ModelUse.allCases.map(ModelUseKey.init), title: \.title
        ) { _ in
            EmptyView()
        } footer: {
            Text(
                "Downloaded models run on this device, like Apple's. "
                    + "A chat can pick its own model from the model button by its field.")
        } options: {
            EmptyView()
        }
        .formStyle(.grouped)
    }
}
