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
package struct ModelsView: View {
    @Bindable var store: StoreOf<ModelsFeature>

    package init(store: StoreOf<ModelsFeature>) { self.store = store }

    package var body: some View {
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
            options: { EmptyView() },
            voices: { VoicesView(store: store.scope(state: \.voices, action: \.voices)) }
        )
        .formStyle(.grouped)
        .task { await store.send(.task).finish() }
        .navigationDestination(item: $store.scope(state: \.tune, action: \.tune)) { store in
            TuneView(store: store)
        }
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
            measure: { store.send(.measureTapped($0)) }, pickBackend: { store.send(.backendPicked($0, for: $1)) },
            tune: { store.send(.tuneTapped($0)) })
    }
}

/// The voice that reads aloud, for every language and each on the shelf,
/// and the voice models to download.
struct VoicesView: View {
    @Bindable var store: StoreOf<VoicesFeature>

    var body: some View {
        VoicesSection(
            app: "LangReadr", models: store.models, installed: store.installed, downloads: store.downloads,
            pickers: [VoicePicker(language: nil, choice: store.standard)]
                + store.languages.map { VoicePicker(language: $0, choice: store.own[$0]) },
            actions: VoiceActions(
                start: { store.send(.downloadTapped($0)) }, pause: { store.send(.pauseTapped($0)) },
                remove: { store.send(.removeTapped($0)) }, pick: { store.send(.picked($0, language: $1)) })
        )
        .task { await store.send(.task).finish() }
        .confirmationDialog($store.scope(state: \.remove, action: \.remove))
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

/// What one chat about a passage runs on.
struct ChatModelView: View {
    @Bindable var store: StoreOf<ChatModelFeature>

    var body: some View {
        ModelSheet(
            model: $store.model, thinking: $store.choice.thinking, window: $store.choice.window, models: store.models,
            standard: store.standard, done: { store.send(.doneTapped) }
        )
        .task { await store.send(.task).finish() }
    }
}
