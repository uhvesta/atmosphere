import ChatUI
import ComposableArchitecture
import LangReadrModels
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

/// What each use runs on, and the models to download.
struct ModelsView: View {
    @Bindable var store: StoreOf<ModelsFeature>

    var body: some View {
        ModelsForm(
            app: "LangReadr", models: store.downloadable, entry: entry, actions: actions, link: $store.link,
            sha256: $store.sha256, addError: store.addError, isAdding: store.isAdding, canAdd: store.canAdd,
            add: { store.send(.addTapped) },
            uses: {
                Section {
                    ForEach(ModelUse.allCases, id: \.self) { use in
                        Picker(
                            use.title,
                            selection: Binding(
                                get: { store.state.selected(for: use) }, set: { store.send(.picked($0, for: use)) })
                        ) {
                            ForEach(store.choices) { Text($0.name).tag($0.id) }
                        }
                    }
                } header: {
                    Text("Runs on")
                } footer: {
                    Text(
                        "Downloaded models run on this device, like Apple's. Nothing you read or write leaves it. "
                            + "A chat can pick its own model from its Model button.")
                }
            },
            options: { EmptyView() }
        )
        .formStyle(.grouped)
        .task { await store.send(.task).finish() }
        .confirmationDialog($store.scope(state: \.remove, action: \.remove))
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    private func entry(_ model: ModelSpec) -> ModelEntryState {
        ModelEntryState(
            download: store.downloads[model.id], isInstalled: store.installed.contains(model.id),
            window: store.windows[model.id] ?? model.contextTokens, tuning: store.tunings[model.id],
            isMeasuring: store.measuring.contains(model.id), measureError: store.measureErrors[model.id])
    }

    private var actions: ModelEntryActions {
        ModelEntryActions(
            start: { store.send(.downloadTapped($0)) }, pause: { store.send(.pauseTapped($0)) },
            remove: { store.send(.removeTapped($0)) }, pickWindow: { store.send(.windowPicked($0, for: $1)) },
            measure: { store.send(.measureTapped($0)) }, pickBackend: { store.send(.backendPicked($0, for: $1)) })
    }
}

/// What one chat about a passage runs on.
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
