import ComposableArchitecture
import LifrCore
import LifrTaskList
import SwiftUI

/// Every Avenue with its open task count, and its active Projects under it.
package struct AvenuesView: View {
    @Bindable package var store: StoreOf<AvenuesFeature>

    package init(store: StoreOf<AvenuesFeature>) {
        self.store = store
    }

    package var body: some View {
        List {
            ForEach(store.avenues) { avenue in
                Section {
                    Button {
                        store.send(.opened(.avenue(avenue.id)))
                    } label: {
                        LabeledContent {
                            Text("\(avenue.openCount)")
                        } label: {
                            Label {
                                Text(avenue.name)
                                Text(avenue.key)
                            } icon: {
                                Image(systemName: avenue.symbol).foregroundStyle(Color(avenue: avenue.color))
                            }
                        }
                    }
                    .accessibilityValue("\(avenue.openCount) open")
                    ForEach(avenue.projects) { project in
                        Button {
                            store.send(.opened(.project(project.id)))
                        } label: {
                            LabeledContent(project.name) { Text("\(project.openCount)") }
                                .padding(.leading)
                        }
                        .accessibilityValue("\(project.openCount) open")
                    }
                }
            }
        }
        .tint(.primary)
        .overlay {
            if store.workspace != nil, store.avenues.isEmpty {
                ContentUnavailableView(
                    "No Avenues", systemImage: "square.stack.3d.up",
                    description: Text("An Avenue is an area of life, such as Health or Home. Add one with +."))
            }
        }
        .navigationTitle("Avenues")
        .toolbar {
            Button {
                store.send(.newAvenueTapped)
            } label: {
                Label("New Avenue", systemImage: "plus")
            }
        }
        .task { await store.send(.task).finish() }
        .navigationDestination(item: $store.scope(state: \.avenue, action: \.avenue)) { store in
            AvenueView(store: store)
        }
        .sheet(item: $store.scope(state: \.newAvenue, action: \.newAvenue)) { store in
            NavigationStack { NewAvenueView(store: store) }
        }
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

/// One Avenue or Project, grouped by status or Project.
package struct AvenueView: View {
    @Bindable package var store: StoreOf<AvenueFeature>

    package init(store: StoreOf<AvenueFeature>) {
        self.store = store
    }

    package var body: some View {
        TaskListView(store: store.scope(state: \.list, action: \.list)) {
            if store.canGroupByProject {
                Picker(
                    "Group by", selection: Binding(get: { store.grouping }, set: { store.send(.groupingPicked($0)) })
                ) {
                    ForEach(Grouping.allCases, id: \.self) { Text($0.title).tag($0) }
                }
                .pickerStyle(.segmented)
                .listRowBackground(Color.clear)
            }
        }
        .navigationTitle(store.title)
        .toolbar {
            if store.canGroupByProject {
                Button {
                    store.send(.newProjectTapped)
                } label: {
                    Label("New Project", systemImage: "folder.badge.plus")
                }
            }
        }
        .alert(
            "New Project",
            isPresented: Binding(
                get: { store.newProjectName != nil }, set: { if !$0 { store.send(.newProjectCancelled) } })
        ) {
            TextField("Name", text: Binding(get: { store.newProjectName ?? "" }, set: { store.newProjectName = $0 }))
            Button("Add") { store.send(.newProjectSubmitted) }
            Button("Cancel", role: .cancel) { store.send(.newProjectCancelled) }
        }
    }
}

/// The form for a new Avenue.
package struct NewAvenueView: View {
    @Bindable package var store: StoreOf<NewAvenueFeature>

    package init(store: StoreOf<NewAvenueFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                TextField("Name", text: $store.name)
                TextField("Key", text: $store.key)
                    .autocorrectionDisabled()
                    #if os(iOS)
                        .textInputAutocapitalization(.characters)
                    #endif
            } footer: {
                Text("The key starts each task's code, such as HEALTH-7K2Q. Use 2 to 10 letters.")
            }
            if let error = store.error {
                Section { Text(error).foregroundStyle(.red) }
            }
        }
        .navigationTitle("New Avenue")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { store.send(.cancelTapped) }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Add") { store.send(.addTapped) }.disabled(store.key.isEmpty)
            }
        }
    }
}

extension Color {
    /// An Avenue's color, stored by name.
    package init(avenue name: String) {
        self =
            switch name {
            case "red": .red
            case "orange": .orange
            case "yellow": .yellow
            case "green": .green
            case "mint": .mint
            case "teal": .teal
            case "cyan": .cyan
            case "indigo": .indigo
            case "purple": .purple
            case "pink": .pink
            case "brown": .brown
            case "gray": .gray
            default: .blue
            }
    }
}

#Preview {
    NavigationStack {
        AvenuesView(store: Store(initialState: AvenuesFeature.State()) { AvenuesFeature() })
    }
}
