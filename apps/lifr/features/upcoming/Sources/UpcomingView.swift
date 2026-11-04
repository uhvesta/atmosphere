import ComposableArchitecture
import LifrTaskList
import SwiftUI

package struct UpcomingView: View {
    package var store: StoreOf<UpcomingFeature>

    package init(store: StoreOf<UpcomingFeature>) {
        self.store = store
    }

    package var body: some View {
        TaskListView(store: store.scope(state: \.list, action: \.list))
            .navigationTitle("Upcoming")
    }
}

#Preview {
    NavigationStack {
        UpcomingView(store: Store(initialState: UpcomingFeature.State()) { UpcomingFeature() })
    }
}
