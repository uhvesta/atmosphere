import ChatUI
import ComposableArchitecture
import ModelsClient
import SwiftUI

/// The Models screen: what each of the app's uses runs on, the models to
/// download, and the voices. The app names its uses and adds its own
/// choices and options.
public struct ModelsSettings<Extra: View, Footer: View, Options: View>: View {
    @Bindable var store: StoreOf<ModelsFeature>
    let app: String
    let uses: [ModelUseKey]
    let title: (ModelUseKey) -> String
    let extra: (ModelUseKey) -> Extra
    let footer: () -> Footer
    let options: () -> Options

    /// `uses` are listed in order, each titled with `title`. `extra` gives
    /// a use's choices besides the models, tagged with their `ModelID`.
    public init(
        store: StoreOf<ModelsFeature>, app: String, uses: [ModelUseKey], title: @escaping (ModelUseKey) -> String,
        @ViewBuilder extra: @escaping (ModelUseKey) -> Extra, @ViewBuilder footer: @escaping () -> Footer,
        @ViewBuilder options: @escaping () -> Options
    ) {
        self.store = store
        self.app = app
        self.uses = uses
        self.title = title
        self.extra = extra
        self.footer = footer
        self.options = options
    }

    public var body: some View {
        ModelsForm(
            app: app, models: store.downloadable, entry: entry, actions: actions, link: $store.link,
            sha256: $store.sha256, addError: store.addError, isAdding: store.isAdding, canAdd: store.canAdd,
            add: { store.send(.addTapped) },
            uses: {
                Section {
                    ForEach(uses, id: \.self) { use in
                        Picker(
                            title(use),
                            selection: Binding(
                                get: { store.state.selected(for: use) }, set: { store.send(.picked($0, for: use)) })
                        ) {
                            extra(use)
                            ForEach(store.choices) { Text($0.name).tag($0.id) }
                        }
                    }
                } header: {
                    Text("Runs on")
                } footer: {
                    footer()
                }
            },
            options: options,
            voices: { VoicesSettings(store: store.scope(state: \.voices, action: \.voices), app: app) }
        )
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

/// The voice that reads aloud, for every language and each of the app's
/// own, and the voice models to download.
struct VoicesSettings: View {
    @Bindable var store: StoreOf<VoicesFeature>
    let app: String

    var body: some View {
        VoicesSection(
            app: app, models: store.models, installed: store.installed, downloads: store.downloads,
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
