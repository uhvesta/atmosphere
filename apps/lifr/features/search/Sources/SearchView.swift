import ComposableArchitecture
import LifrCore
import LifrTaskList
import LifrUI
import SwiftUI

package struct SearchView: View {
    @Bindable package var store: StoreOf<SearchFeature>

    package init(store: StoreOf<SearchFeature>) {
        self.store = store
    }

    package var body: some View {
        TaskListView(store: store.scope(state: \.list, action: \.list))
            .searchable(text: $store.filter.text, prompt: "Titles, details and comments")
            .navigationTitle("Search")
            .toolbar {
                Menu {
                    filters
                } label: {
                    Label(
                        store.filter.filterCount == 0 ? "Filters" : "Filters, \(store.filter.filterCount) set",
                        systemImage: store.filter.filterCount == 0
                            ? "line.3.horizontal.decrease.circle" : "line.3.horizontal.decrease.circle.fill")
                }
            }
    }

    @ViewBuilder
    private var filters: some View {
        if !store.avenueChoices.isEmpty {
            Menu("Avenue") {
                ForEach(store.avenueChoices) { avenue in
                    Toggle(avenue.name, isOn: $store.filter.avenues.contains(avenue.id))
                }
            }
        }
        Menu("Status") {
            ForEach(TaskStatus.allCases, id: \.self) { status in
                Toggle(status.title, isOn: $store.filter.statuses.contains(status))
            }
        }
        Menu("Priority") {
            ForEach(Priority.allCases, id: \.self) { priority in
                Toggle(priority.title ?? "None", isOn: $store.filter.priorities.contains(priority))
            }
        }
        if !store.labelNames.isEmpty {
            Menu("Label") {
                ForEach(store.labelNames, id: \.self) { name in
                    Toggle("#\(name)", isOn: $store.filter.labels.contains(name))
                }
            }
        }
        Picker("Due", selection: $store.filter.due) {
            ForEach(SearchFilter.Due.allCases, id: \.self) { Text($0.title).tag($0) }
        }
        if store.filter.filterCount > 0 {
            Button("Clear filters", role: .destructive) { store.send(.clearFiltersTapped) }
        }
    }
}

extension Set {
    /// Whether the set holds `element`, as a binding a toggle can change.
    fileprivate subscript(contains element: Element) -> Bool {
        get { contains(element) }
        set {
            if newValue { insert(element) } else { remove(element) }
        }
    }
}

extension Binding {
    fileprivate func contains<Element: Hashable>(_ element: Element) -> Binding<Bool> where Value == Set<Element> {
        self[contains: element]
    }
}

#Preview {
    NavigationStack {
        SearchView(store: Store(initialState: SearchFeature.State()) { SearchFeature() })
    }
}
