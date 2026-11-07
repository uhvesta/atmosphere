import ChatUI
import ComposableArchitecture
import LifrModelsClient
import SwiftUI

package struct ModelsView: View {
    @Bindable package var store: StoreOf<ModelsFeature>
    @Shared(.answersInBackground) private var answersInBackground

    package init(store: StoreOf<ModelsFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                ForEach(ModelUse.allCases, id: \.self) { use in
                    Picker(
                        use.title,
                        selection: Binding(
                            get: { store.state.selected(for: use) }, set: { store.send(.picked($0, for: use)) })
                    ) {
                        if use == .search { Text("Built-in").tag(ModelID.builtIn) }
                        ForEach(store.choices) { Text($0.name).tag($0.id) }
                    }
                }
            } header: {
                Text("Runs on")
            } footer: {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Downloaded models run on this device, like Apple's. Nothing you write leaves it.")
                    Text("Built-in search adds related words from the system's word lists, with no model.")
                }
            }
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
            Section {
                ForEach(store.downloadable) { model in
                    ModelRow(model: model, store: store)
                }
            } header: {
                Text("Models")
            }
            Section {
                TextField("Hugging Face link", text: $store.link)
                    .textContentType(.URL)
                    .autocorrectionDisabled()
                    #if os(iOS)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                    #endif
                TextField("SHA-256 (optional)", text: $store.sha256)
                    .autocorrectionDisabled()
                    #if os(iOS)
                        .textInputAutocapitalization(.never)
                    #endif
                if let error = store.addError {
                    Text(error).foregroundStyle(.red)
                }
                Button {
                    store.send(.addTapped)
                } label: {
                    if store.isAdding {
                        ProgressView()
                    } else {
                        Text("Download model")
                    }
                }
                .disabled(!store.canAdd)
            } header: {
                Text("From Hugging Face")
            } footer: {
                Text(
                    "Paste the link to a .litertlm model, such as one from litert-community. Lifr pins it to the "
                        + "current commit and checks every file. Give the SHA-256 its publisher lists to be sure "
                        + "it's theirs.")
            }
        }
        .navigationTitle("Models")
        .task { await store.send(.task).finish() }
        .confirmationDialog($store.scope(state: \.remove, action: \.remove))
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

/// A model to download, with what the store says of it.
private struct ModelRow: View {
    let model: ModelSpec
    let store: StoreOf<ModelsFeature>

    var body: some View {
        ChatUI.ModelRow(
            model, download: store.downloads[model.id], isInstalled: store.installed.contains(model.id),
            window: store.windows[model.id] ?? model.contextTokens,
            pickWindow: { store.send(.windowPicked($0, for: model.id)) },
            start: { store.send(.downloadTapped(model.id)) }, pause: { store.send(.pauseTapped(model.id)) },
            remove: { store.send(.removeTapped(model.id)) })
    }
}

#Preview {
    NavigationStack {
        ModelsView(store: Store(initialState: ModelsFeature.State()) { ModelsFeature() })
    }
}
