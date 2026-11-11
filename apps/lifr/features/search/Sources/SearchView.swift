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
        TaskListView(store: store.scope(state: \.list, action: \.list)) {
            if let error = store.error {
                Text(error)
                    .font(.footnote)
                    .foregroundStyle(.red)
            }
            if !store.chips.isEmpty {
                chips
            }
        }
        .emptyAction(store.filter.filterCount == 0 ? nil : "Clear Filters") { store.send(.clearFiltersTapped) }
        .searchable(text: $store.filter.text, prompt: "Titles, details and comments")
        .navigationTitle(store.savedFilter?.name ?? String(localized: "Search"))
        .toolbar {
            Menu {
                filters
            } label: {
                Label(
                    store.filter.filterCount == 0 ? "Filters" : "Filters, \(store.filter.filterCount) set",
                    systemImage: store.filter.filterCount == 0
                        ? "line.3.horizontal.decrease.circle" : "line.3.horizontal.decrease.circle.fill")
            }
            if let saved = store.savedFilter {
                Menu("More", systemImage: "ellipsis.circle") {
                    if store.differsFromSaved {
                        Button("Update “\(saved.name)”", systemImage: "arrow.down.doc") {
                            store.send(.updateSavedFilterTapped)
                        }
                        .disabled(store.filter.isEmpty)
                        Button("Save as New Filter…", systemImage: "plus") { store.send(.saveFilterTapped) }
                            .disabled(store.filter.isEmpty)
                    }
                    Button("Edit Filter…", systemImage: "pencil") { store.send(.editSavedFilterTapped) }
                }
            } else {
                Button("Save Filter…", systemImage: "square.and.arrow.down") { store.send(.saveFilterTapped) }
                    .disabled(store.filter.isEmpty)
            }
        }
        .sheet(item: $store.scope(state: \.editor, action: \.editor)) { editor in
            SavedFilterEditorView(store: editor)
        }
    }

    /// The set filters, each with ✕ to clear it; tapping one opens the
    /// filter menu.
    private var chips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(store.chips) { chip in
                    HStack(spacing: 4) {
                        Menu(chip.title) { filters }
                            .accessibilityHint("Opens the filters")
                        Button("Remove \(chip.title)", systemImage: "xmark") { store.send(.chipRemoved(chip.kind)) }
                            .labelStyle(.iconOnly)
                            .font(.caption.weight(.semibold))
                    }
                    .font(.subheadline)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(.tint.opacity(0.12), in: Capsule())
                }
            }
        }
        .buttonStyle(.borderless)
        .listRowInsets(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
        .listRowBackground(Color.clear)
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
                Toggle(priority.title ?? String(localized: "None"), isOn: $store.filter.priorities.contains(priority))
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
