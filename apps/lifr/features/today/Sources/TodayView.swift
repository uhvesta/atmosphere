import ComposableArchitecture
import LifrProcrastinated
import LifrTaskList
import SwiftUI

package struct TodayView: View {
    @Bindable package var store: StoreOf<TodayFeature>

    package init(store: StoreOf<TodayFeature>) {
        self.store = store
    }

    package var body: some View {
        TaskListView(store: store.scope(state: \.list, action: \.list)) {
            if store.procrastinatedCount > 0 {
                Section {
                    Button {
                        store.send(.procrastinatedTapped)
                    } label: {
                        LabeledContent {
                            Text("\(store.procrastinatedCount)")
                                .monospacedDigit()
                        } label: {
                            Label("Procrastinated", systemImage: "tortoise")
                        }
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("Procrastinated")
                    .accessibilityAddTraits(.isButton)
                    .accessibilityValue("\(store.procrastinatedCount) stalled")
                }
            }
        }
        .navigationTitle("Today")
        .navigationDestination(item: $store.scope(state: \.procrastinated, action: \.procrastinated)) { store in
            ProcrastinatedView(store: store)
        }
    }
}

#Preview {
    NavigationStack {
        TodayView(store: Store(initialState: TodayFeature.State()) { TodayFeature() })
    }
}
