import ChatUI
import ComposableArchitecture
import LangReadrModels
import SwiftUI

package struct TuneView: View {
    let store: StoreOf<TuneFeature>

    package init(store: StoreOf<TuneFeature>) {
        self.store = store
    }

    package var body: some View {
        TuneScreen(
            TuneScreenState(
                model: store.model, candidates: store.candidates, results: store.results, failures: store.failures,
                running: store.running, isRunning: store.isRunning, setups: store.setups,
                otherFormats: store.otherFormats, downloading: store.downloading, downloadError: store.downloadError,
                editing: store.editing),
            actions: TuneScreenActions(
                tune: { store.send(.tuneTapped) }, stop: { store.send(.stopTapped) },
                edit: { store.send(.editTapped($0)) },
                change: { store.send(.optionsChanged($0, at: $1)) }, duplicate: { store.send(.duplicateTapped($0)) },
                remove: { store.send(.removeTapped($0)) }, pick: { store.send(.setupPicked($0, for: $1)) },
                download: { store.send(.downloadTapped($0)) })
        )
        .task { await store.send(.task).finish() }
    }
}

#Preview {
    NavigationStack {
        TuneView(
            store: Store(initialState: TuneFeature.State(model: ModelCatalog.gemma4E2B)) { TuneFeature() })
    }
}
