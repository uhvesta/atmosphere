import ComposableArchitecture
import LifrModelsClient
import ModelsFeatures
import SwiftUI

/// The Models screen with Lifr's uses: searching can run on the built-in
/// word lists, and a downloaded model can answer in the background.
package struct ModelsView: View {
    let store: StoreOf<ModelsFeature>
    @Shared(.answersInBackground) private var answersInBackground

    package init(store: StoreOf<ModelsFeature>) {
        self.store = store
    }

    package var body: some View {
        ModelsSettings(
            store: store, app: "Lifr", uses: ModelUse.allCases.map(ModelUseKey.init), title: \.title
        ) { use in
            if use == .search { Text("Built-in").tag(ModelID.builtIn) }
        } footer: {
            VStack(alignment: .leading, spacing: 6) {
                Text("Downloaded models run on this device, like Apple's. Nothing you write leaves it.")
                Text("Built-in search adds related words from the system's word lists, with no model.")
            }
        } options: {
            #if os(iOS)
                // The Mac doesn't suspend apps, so a model there always
                // goes on answering.
                Section {
                    Toggle("Answer in the background", isOn: Binding($answersInBackground))
                } footer: {
                    Text(
                        "When you leave Lifr mid-answer, a downloaded model keeps going on the iPhone's CPU, "
                            + "which is slower and uses more battery, and Lifr shows its progress on the Lock "
                            + "Screen. Off, the model stops and carries on where it was when you come back.")
                }
            #endif
        }
    }
}

#Preview {
    NavigationStack {
        ModelsView(store: Store(initialState: ModelsFeature.State()) { ModelsFeature() })
    }
}
