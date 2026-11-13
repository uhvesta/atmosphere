import ChatUI
import ComposableArchitecture
import LifrCore
import LifrProjectDetail
import LifrTaskList
import LifrUI
import SwiftUI

/// Move Up and Move Down for a row in a list of `ids`, each left out at
/// its end of the list.
package struct MoveButtons: View {
    var ids: [UUID]
    var id: UUID
    var move: (_ up: Bool) -> Void

    package init(ids: [UUID], id: UUID, move: @escaping (_ up: Bool) -> Void) {
        self.ids = ids
        self.id = id
        self.move = move
    }

    package var body: some View {
        if ids.first != id {
            Button("Move Up", systemImage: "arrow.up") { move(true) }
        }
        if ids.last != id {
            Button("Move Down", systemImage: "arrow.down") { move(false) }
        }
    }
}

/// Every Avenue with its open task count, and its active Projects under it.
/// Drags an Avenue or Project row, and takes another of the same kind
/// dropped onto it, outlining the row while one is over it.
package struct OrderDrop: ViewModifier {
    private let item: OrderDrag
    private let dropped: (UUID) -> Void
    @State private var isTargeted = false

    /// `dropped` gets the ID of the Avenue or Project dropped onto `item`.
    package init(_ item: OrderDrag, dropped: @escaping (UUID) -> Void) {
        self.item = item
        self.dropped = dropped
    }

    package func body(content: Content) -> some View {
        content
            .draggable(item.text)
            .dropDestination(for: String.self) { items, _ in
                let moved = items.compactMap(OrderDrag.init).compactMap { drag -> UUID? in
                    switch (drag, item) {
                    case (.avenue(let id), .avenue), (.project(let id), .project): id
                    default: nil
                    }
                }
                guard let id = moved.first else { return false }
                dropped(id)
                return true
            } isTargeted: {
                isTargeted = $0
            }
            .overlay {
                if isTargeted {
                    RoundedRectangle(cornerRadius: 6).strokeBorder(.tint, lineWidth: 2).padding(-4)
                        .accessibilityHidden(true)
                }
            }
    }
}

package struct AvenuesView<Footer: View>: View {
    @Bindable package var store: StoreOf<AvenuesFeature>
    /// Rows after the Avenues, such as the trash.
    private let footer: Footer

    package init(store: StoreOf<AvenuesFeature>, @ViewBuilder footer: () -> Footer) {
        self.store = store
        self.footer = footer()
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
                    .modifier(OrderDrop(.avenue(avenue.id)) { store.send(.avenueDropped($0, onto: avenue.id)) })
                    .contextMenu {
                        MoveButtons(
                            ids: store.avenues.map(\.id), id: avenue.id,
                            move: { store.send(.moveAvenue(avenue.id, up: $0)) })
                    }
                    ForEach(avenue.projects) { project in
                        Button {
                            store.send(.opened(.project(project.id)))
                        } label: {
                            LabeledContent(project.name) { Text("\(project.openCount)") }
                                .padding(.leading)
                        }
                        .accessibilityValue("\(project.openCount) open")
                        .modifier(OrderDrop(.project(project.id)) { store.send(.projectDropped($0, onto: project.id)) })
                        .contextMenu {
                            MoveButtons(
                                ids: avenue.projects.map(\.id), id: project.id,
                                move: { store.send(.moveProject(project.id, up: $0)) })
                        }
                    }
                }
            }
            if !store.archived.isEmpty {
                Section("Archived") {
                    ForEach(store.archived) { row in
                        Button {
                            store.send(.opened(row.scope))
                        } label: {
                            LabeledContent(row.name) { Text(row.detail) }
                        }
                    }
                }
            }
            footer
        }
        .tint(.primary)
        .overlay {
            if let loadError = store.loadError {
                ContentUnavailableView {
                    Label("Couldn't load your Avenues", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(loadError)
                } actions: {
                    Button("Try Again") { store.send(.task) }
                }
            } else if store.workspace != nil, store.avenues.isEmpty {
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
                .sheetLayout(height: 260)
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
            if let project = store.scope(state: \.project, action: \.project) {
                ProjectDetailView(store: project)
            }
            Picker(
                "Group by", selection: Binding(get: { store.grouping }, set: { store.send(.groupingPicked($0)) })
            ) {
                ForEach(store.groupings, id: \.self) { Text($0.title).tag($0) }
            }
            .pickerStyle(.menu)
            // Only status groups show closed tasks.
            if store.grouping == .status {
                Toggle(
                    "Show Closed",
                    isOn: Binding(get: { store.showsCompleted }, set: { store.send(.showsCompletedToggled($0)) }))
            }
        }
        .emptyAction("Add a Task") { store.send(.addTaskTapped) }
        .emptyAction(store.canGroupByProject ? "Add a Project" : nil) { store.send(.newProjectTapped) }
        .emptyBelowHeader(store.project != nil)
        .navigationTitle(store.title)
        .toolbar {
            #if os(iOS)
                // The Mac's toolbar has the window's + menu.
                ToolbarItem(placement: .primaryAction) {
                    Button("Add Task", systemImage: "plus") { store.send(.addTaskTapped) }
                }
            #endif
            if store.canGroupByProject {
                ToolbarItemGroup {
                    Button {
                        store.send(.newProjectTapped)
                    } label: {
                        Label("New Project", systemImage: "folder.badge.plus")
                    }
                    Button {
                        store.send(.editAvenueTapped)
                    } label: {
                        Label("Edit Avenue", systemImage: "slider.horizontal.3")
                    }
                }
            }
        }
        .sheet(item: $store.scope(state: \.editAvenue, action: \.editAvenue)) { store in
            NavigationStack { EditAvenueView(store: store) }
                .sheetLayout(height: 520)
        }
        .alert(
            "New Project",
            isPresented: Binding(
                get: { store.newProjectName != nil }, set: { if !$0 { store.send(.newProjectCancelled) } })
        ) {
            TextField("Name", text: Binding(get: { store.newProjectName ?? "" }, set: { store.newProjectName = $0 }))
            Button("Add") { store.send(.newProjectSubmitted) }
                .disabled((store.newProjectName ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
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
                    .accessibilityLabel("Name")
                TextField("Key", text: $store.key)
                    .accessibilityLabel("Key")
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

/// The form for changing an Avenue, or archiving or restoring it.
package struct EditAvenueView: View {
    @Bindable package var store: StoreOf<EditAvenueFeature>

    package init(store: StoreOf<EditAvenueFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            Section {
                TextField("Name", text: $store.name)
                    .accessibilityLabel("Name")
                // A menu draws its images in one color, so the row's icon
                // shows the Avenue's.
                Picker(selection: $store.color) {
                    ForEach(store.colorChoices, id: \.self) { Text($0.capitalized).tag($0) }
                } label: {
                    Label {
                        Text("Color")
                    } icon: {
                        Image(systemName: store.symbol).foregroundStyle(Color(avenue: store.color))
                    }
                }
                Picker("Symbol", selection: $store.symbol) {
                    ForEach(store.symbolChoices, id: \.self) { symbol in
                        Image(systemName: symbol).accessibilityLabel(symbol).tag(symbol)
                    }
                }
            }
            // The Inbox can't be archived.
            if !store.isInbox {
                Section {
                    if store.isArchived {
                        Button("Restore Avenue", systemImage: "arrow.uturn.backward") { store.send(.restoreTapped) }
                    } else {
                        Button("Archive Avenue", systemImage: "archivebox", role: .destructive) {
                            store.send(.archiveTapped)
                        }
                    }
                } footer: {
                    if !store.isArchived {
                        Text("Archiving hides the Avenue and its tasks. You can restore it from Archived.")
                    }
                }
            }
        }
        .navigationTitle("Edit Avenue")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { store.send(.cancelTapped) }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") { store.send(.saveTapped) }
                    .disabled(store.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .alert($store.scope(state: \.confirmArchive, action: \.confirmArchive))
        .alert($store.scope(state: \.alert, action: \.alert))
    }
}

#Preview {
    NavigationStack {
        AvenuesView(store: Store(initialState: AvenuesFeature.State()) { AvenuesFeature() })
    }
}

extension AvenuesView where Footer == EmptyView {
    package init(store: StoreOf<AvenuesFeature>) {
        self.init(store: store) { EmptyView() }
    }
}
