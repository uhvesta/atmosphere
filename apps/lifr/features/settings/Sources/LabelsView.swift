import ComposableArchitecture
import LifrCore
import LifrUI
import SwiftUI

/// Settings › Labels.
package struct LabelsView: View {
    @Bindable package var store: StoreOf<LabelsFeature>

    package init(store: StoreOf<LabelsFeature>) {
        self.store = store
    }

    package var body: some View {
        List {
            ForEach(store.rows) { row in
                Button {
                    store.send(.rowTapped(row.id))
                } label: {
                    LabeledContent("#\(row.name)") { Text("\(row.taskCount)") }
                }
                .accessibilityValue("\(row.taskCount) tasks")
            }
        }
        .tint(.primary)
        .overlay {
            if store.rows.isEmpty {
                ContentUnavailableView(
                    "No Labels", systemImage: "number",
                    description: Text("Add a label from a task’s Labels section."))
            }
        }
        .navigationTitle("Labels")
        .sheet(item: $store.scope(state: \.edit, action: \.edit)) { store in
            EditLabelView(store: store)
        }
    }
}

struct EditLabelView: View {
    @Bindable var store: StoreOf<EditLabelFeature>

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
                if !store.others.isEmpty {
                    Section {
                        Menu("Merge Into…") {
                            ForEach(store.others) { other in
                                Button("#\(other.name)") { store.send(.mergeTapped(other.id)) }
                            }
                        }
                    } footer: {
                        Text("Its tasks get the other label instead, and this one is deleted.")
                    }
                }
                Section {
                    Button("Delete Label", role: .destructive) { store.send(.deleteTapped) }
                }
            }
            .navigationTitle("#\(store.original)")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { store.send(.cancelTapped) }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { store.send(.saveTapped) }
                        .disabled(store.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
            .alert($store.scope(state: \.confirm, action: \.confirm))
        }
        .presentationDetents([.medium, .large])
    }
}
