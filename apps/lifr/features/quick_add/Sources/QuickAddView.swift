import ComposableArchitecture
import LifrCore
import SwiftUI

package struct QuickAddView: View {
    @Bindable package var store: StoreOf<QuickAddFeature>
    @FocusState private var titleFocused: Bool

    package init(store: StoreOf<QuickAddFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                TextField(store.parent == nil ? "Title" : "Sub task title", text: $store.title)
                    .focused($titleFocused)
                    .onSubmit { store.send(.addTapped) }
                    .submitLabel(.done)
            } footer: {
                if let error = store.error {
                    Text(error).foregroundStyle(.red)
                }
            }
            if let parent = store.parent {
                Section {
                    ForEach(Array(store.addedSubtasks.enumerated()), id: \.offset) { _, title in
                        Label(title, systemImage: "checkmark.circle")
                    }
                } header: {
                    Text("Sub tasks of \(parent.name)")
                }
            } else {
                fields
            }
        }
        .navigationTitle(store.parent == nil ? "New Task" : "Add Sub tasks")
        #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
        #endif
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                if store.parent == nil {
                    Button("Cancel") { store.send(.cancelTapped) }
                }
            }
            ToolbarItem(placement: .confirmationAction) {
                if store.parent == nil {
                    Button("Add") { store.send(.addTapped) }.disabled(!store.canAdd)
                } else {
                    Button("Done") { store.send(.doneTapped) }
                }
            }
        }
        .onAppear { titleFocused = true }
    }

    @ViewBuilder
    private var fields: some View {
        Section {
            Picker("Avenue", selection: $store.avenueID) {
                ForEach(store.avenues) { Text($0.name).tag(Optional($0.id)) }
            }
            if !store.avenueProjects.isEmpty {
                Picker("Project", selection: $store.projectID) {
                    Text("None").tag(UUID?.none)
                    ForEach(store.avenueProjects) { Text($0.name).tag(Optional($0.id)) }
                }
            }
        } footer: {
            if store.avenues.isEmpty {
                Text("Add an Avenue first, on the Avenues tab.")
            }
        }
        Section {
            Picker("Due", selection: $store.due) {
                ForEach(QuickAddFeature.Due.allCases, id: \.self) { Text($0.title).tag($0) }
            }
            .pickerStyle(.segmented)
            if store.due == .pick {
                DatePicker("Due", selection: $store.pickedDate, displayedComponents: .date)
            }
            Picker("Priority", selection: $store.priority) {
                ForEach(Priority.allCases, id: \.self) { Text($0.title ?? "None").tag($0) }
            }
        }
        Section {
            Toggle("Add Sub tasks next", isOn: $store.subtaskMode)
        } footer: {
            Text("Adds this task, then keeps Quick Add open for its Sub tasks.")
        }
    }
}

#Preview {
    NavigationStack {
        QuickAddView(
            store: Store(
                initialState: QuickAddFeature.State(workspace: .sample(now: Date(), timeZone: .current), now: Date())
            ) {
                QuickAddFeature()
            })
    }
}
