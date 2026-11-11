import ComposableArchitecture
import LifrTaskList
import SwiftUI

package struct UpcomingView: View {
    package var store: StoreOf<UpcomingFeature>
    private let openCalendar: (() -> Void)?

    /// `openCalendar` shows the month instead, from the empty list.
    package init(store: StoreOf<UpcomingFeature>, openCalendar: (() -> Void)? = nil) {
        self.store = store
        self.openCalendar = openCalendar
    }

    package var body: some View {
        TaskListView(store: store.scope(state: \.list, action: \.list))
            .emptyAction(openCalendar.map { _ in "Open the Calendar" }) { openCalendar?() }
            .navigationTitle("Upcoming")
    }
}

#Preview {
    NavigationStack {
        UpcomingView(store: Store(initialState: UpcomingFeature.State()) { UpcomingFeature() })
    }
}
