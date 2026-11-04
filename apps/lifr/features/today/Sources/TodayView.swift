import ComposableArchitecture
import LifrCore
import LifrStoreClient
import LifrTaskDetail
import LifrUI
import SwiftUI

package struct TodayView: View {
    @Bindable package var store: StoreOf<TodayFeature>

    package init(store: StoreOf<TodayFeature>) {
        self.store = store
    }

    package var body: some View {
        List {
            section("Overdue", store.overdue)
            section("Today", store.dueToday)
        }
        .overlay {
            if store.workspace != nil, store.overdue.isEmpty, store.dueToday.isEmpty {
                ContentUnavailableView(
                    "Nothing due today", systemImage: "sun.max",
                    description: Text("Tasks due today, and overdue ones, show up here."))
            }
        }
        .navigationTitle("Today")
        .task { await store.send(.task).finish() }
        .navigationDestination(item: $store.scope(state: \.detail, action: \.detail)) { store in
            TaskDetailView(store: store)
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    @ViewBuilder
    private func section(_ title: String, _ rows: [TaskRowData]) -> some View {
        if !rows.isEmpty {
            Section(title) {
                ForEach(rows) { row in
                    TaskRow(row) { store.send(.toggled(row.id)) }
                        .contentShape(Rectangle())
                        .onTapGesture { store.send(.rowTapped(row.id)) }
                        .accessibilityAction { store.send(.rowTapped(row.id)) }
                }
            }
        }
    }
}

#Preview {
    NavigationStack {
        TodayView(store: Store(initialState: TodayFeature.State()) { TodayFeature() })
    }
}
