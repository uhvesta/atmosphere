import ComposableArchitecture
import LifrCore
import SwiftUI

/// A symbol a saved filter can have, and what VoiceOver calls it.
struct FilterSymbol: Identifiable, Sendable {
    let name: String
    let title: LocalizedStringResource
    var id: String { name }

    static let all = [
        FilterSymbol(name: SavedFilter.defaultSymbol, title: "Filter"),
        FilterSymbol(name: "star", title: "Star"),
        FilterSymbol(name: "flag", title: "Flag"),
        FilterSymbol(name: "bolt", title: "Bolt"),
        FilterSymbol(name: "tag", title: "Tag"),
        FilterSymbol(name: "briefcase", title: "Briefcase"),
        FilterSymbol(name: "house", title: "House"),
        FilterSymbol(name: "cart", title: "Cart"),
        FilterSymbol(name: "heart", title: "Heart"),
        FilterSymbol(name: "book", title: "Book"),
        FilterSymbol(name: "phone", title: "Phone"),
        FilterSymbol(name: "calendar", title: "Calendar"),
    ]
}

struct SavedFilterEditorView: View {
    @Bindable var store: StoreOf<SavedFilterEditor>

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $store.name)
                        .onSubmit { store.send(.saveTapped) }
                        .submitLabel(.done)
                } footer: {
                    if let error = store.error {
                        Text(error).foregroundStyle(.red)
                    }
                }
                Section("Symbol") {
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 6), spacing: 12) {
                        ForEach(FilterSymbol.all) { symbol in
                            let isPicked = store.symbol == symbol.name
                            Button {
                                store.symbol = symbol.name
                            } label: {
                                Image(systemName: symbol.name)
                                    .frame(width: 36, height: 36)
                                    .background(
                                        isPicked ? AnyShapeStyle(.tint.opacity(0.2)) : AnyShapeStyle(.clear),
                                        in: Circle())
                            }
                            .buttonStyle(.borderless)
                            .accessibilityLabel(Text(symbol.title))
                            .accessibilityAddTraits(isPicked ? .isSelected : [])
                        }
                    }
                    .padding(.vertical, 4)
                }
                if !store.isNew {
                    Section {
                        Button("Delete Filter", role: .destructive) { store.send(.deleteTapped) }
                    }
                }
            }
            .navigationTitle(store.isNew ? "Save Filter" : "Edit Filter")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { store.send(.cancelTapped) }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { store.send(.saveTapped) }
                        .disabled(!store.canSave)
                }
            }
            .alert($store.scope(state: \.confirm, action: \.confirm))
        }
        .presentationDetents([.medium, .large])
    }
}
