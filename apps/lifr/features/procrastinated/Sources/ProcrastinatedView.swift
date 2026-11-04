import ComposableArchitecture
import LifrTaskList
import SwiftUI

package struct ProcrastinatedView: View {
    package var store: StoreOf<ProcrastinatedFeature>

    package init(store: StoreOf<ProcrastinatedFeature>) {
        self.store = store
    }

    package var body: some View {
        TaskListView(store: store.scope(state: \.list, action: \.list))
            .navigationTitle("Procrastinated")
    }
}

#Preview {
    NavigationStack {
        ProcrastinatedView(store: Store(initialState: ProcrastinatedFeature.State()) { ProcrastinatedFeature() })
    }
}
