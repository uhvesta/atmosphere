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
        ModelsForm(
            app: "Lifr", models: store.downloadable, entry: entry, actions: actions, link: $store.link,
            sha256: $store.sha256, addError: store.addError, isAdding: store.isAdding, canAdd: store.canAdd,
            add: add
        ) {
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
        } voices: {
            VoicesView(store: store.scope(state: \.voices, action: \.voices))
        }
        .task { await store.send(.task).finish() }
        .navigationDestination(item: $store.scope(state: \.tune, action: \.tune)) { store in
            TuneView(store: store)
        }
        .confirmationDialog($store.scope(state: \.remove, action: \.remove))
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    private func add() {
        store.send(.addTapped)
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

#Preview {
    NavigationStack {
        ModelsView(store: Store(initialState: ModelsFeature.State()) { ModelsFeature() })
    }
}

/// The voice that reads answers aloud, and the voice models to download.
struct VoicesView: View {
    @Bindable var store: StoreOf<VoicesFeature>

    var body: some View {
        VoicesSection(
            app: "Lifr", models: store.models, installed: store.installed, downloads: store.downloads,
            pickers: [VoicePicker(language: nil, choice: store.standard)],
            actions: VoiceActions(
                start: { store.send(.downloadTapped($0)) }, pause: { store.send(.pauseTapped($0)) },
                remove: { store.send(.removeTapped($0)) }, pick: { store.send(.picked($0, language: $1)) })
        )
        .task { await store.send(.task).finish() }
        .confirmationDialog($store.scope(state: \.remove, action: \.remove))
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}
