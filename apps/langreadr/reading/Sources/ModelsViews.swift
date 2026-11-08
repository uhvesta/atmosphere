import ChatUI
import ComposableArchitecture
import LangReadrModels
import ModelsFeatures
import SwiftUI

/// Settings: for now, the models LangReadr runs on.
struct SettingsView: View {
    @Bindable var store: StoreOf<SettingsFeature>

    var body: some View {
        NavigationStack {
            Form {
                NavigationLink("Models") {
                    ModelsView(store: store.scope(state: \.models, action: \.models))
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

/// The Models screen with LangReadr's uses, and a voice for each language
/// on the shelf.
package struct ModelsView: View {
    let store: StoreOf<ModelsFeature>

    package init(store: StoreOf<ModelsFeature>) { self.store = store }

    package var body: some View {
        ModelsSettings(
            store: store, app: "LangReadr", uses: ModelUse.allCases.map(ModelUseKey.init), title: \.title
        ) { _ in
            EmptyView()
        } footer: {
            Text(
                "Downloaded models run on this device, like Apple's. Nothing you read or write leaves it. "
                    + "A chat can pick its own model from its Model button.")
        } options: {
            EmptyView()
        }
        .formStyle(.grouped)
    }
}
