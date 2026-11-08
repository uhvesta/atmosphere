import ChatUI
import ComposableArchitecture
import LocalLogChatrModels
import ModelsFeatures
import SwiftUI

/// Settings: the models the chats run on.
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
