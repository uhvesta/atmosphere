import ComposableArchitecture
import LifrProcrastinated
import LifrTaskList
import SwiftUI

package struct TodayView: View {
    @Bindable package var store: StoreOf<TodayFeature>
    private let add: (() -> Void)?

    /// `add` opens Quick Add for a task due today, from the empty list.
    package init(store: StoreOf<TodayFeature>, add: (() -> Void)? = nil) {
        self.store = store
        self.add = add
    }

    package var body: some View {
        TaskListView(store: store.scope(state: \.list, action: \.list)) {
            if store.procrastinatedCount > 0 {
                Section {
                    Button {
                        store.send(.procrastinatedTapped)
                    } label: {
                        LabeledContent {
                            stalled.monospacedDigit()
                        } label: {
                            Label("Procrastinated", systemImage: "tortoise")
                        }
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("Procrastinated")
                    .accessibilityAddTraits(.isButton)
                    .accessibilityValue(stalled)
                }
            }
        }
        .emptyAction(add.map { _ in "Add a Task for Today" }) { add?() }
        .navigationTitle("Today")
        .navigationDestination(item: $store.scope(state: \.procrastinated, action: \.procrastinated)) { store in
            ProcrastinatedView(store: store)
        }
    }

    /// "2 stalled", next to Procrastinated.
    private var stalled: Text {
        Text("\(store.procrastinatedCount) stalled")
    }
}

#Preview {
    NavigationStack {
        TodayView(store: Store(initialState: TodayFeature.State()) { TodayFeature() })
    }
}
