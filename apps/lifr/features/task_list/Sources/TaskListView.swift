import ChatUI
import ComposableArchitecture
import LifrCore
import LifrTaskDetail
import LifrUI
import SwiftUI

#if os(macOS)
    import AppKit
#endif

/// A task list in sections, after an optional header such as Today's way
/// into Procrastinated. Swipe right to start a task; swipe left for the
/// query's other actions.
package struct TaskListView<Header: View>: View {
    @Bindable package var store: StoreOf<TaskListFeature>
    private let header: Header
    private var emptyAction: (title: LocalizedStringKey, perform: () -> Void)?
    private var emptyBelowHeader = false

    package init(store: StoreOf<TaskListFeature>, @ViewBuilder header: () -> Header) {
        self.store = store
        self.header = header()
    }

    /// A button under the empty list's message, such as Clear Filters, or
    /// none when `title` is nil.
    package func emptyAction(_ title: LocalizedStringKey?, perform: @escaping () -> Void) -> Self {
        var view = self
        view.emptyAction = title.map { ($0, perform) }
        return view
    }

    /// Shows the empty list's message under the header rather than over
    /// the whole list, for a header the person works in, such as a
    /// Project's fields.
    package func emptyBelowHeader(_ below: Bool = true) -> Self {
        var view = self
        view.emptyBelowHeader = below
        return view
    }

    package var body: some View {
        List {
            header
            if emptyBelowHeader, store.workspace != nil, store.isEmpty {
                empty.listRowBackground(Color.clear)
            }
            ForEach(store.sections) { section in
                Section(section.title) {
                    ForEach(section.rows) { row in
                        TaskListRow(row: row, note: section.notes[row.id], store: store)
                    }
                    .onMove(
                        perform: store.canReorder
                            ? { store.send(.moved(section: section.id, from: $0, to: $1)) } : nil)
                }
            }
        }
        .overlay {
            if store.isSearching, store.isEmpty {
                ContentUnavailableView.search(text: store.searchText)
            } else if !emptyBelowHeader, store.workspace != nil, store.isEmpty {
                empty
            }
        }
        .safeAreaInset(edge: .top) {
            if let banner = store.banner {
                BannerView(banner, undo: { store.send(.bannerUndoTapped) }, dismiss: { store.send(.bannerDismissed) })
            }
        }
        .safeAreaInset(edge: .bottom) {
            if store.isSelecting {
                BulkBar(store: store)
            }
        }
        .task { await store.send(.task).finish() }
        #if os(iOS)
            .refreshable { await store.send(.pulled).finish() }
        #endif
        .toolbar {
            ToolbarItem(placement: .secondaryAction) {
                Menu {
                    Picker("Sort", selection: Binding(get: { store.sort }, set: { store.send(.sortPicked($0)) })) {
                        ForEach(store.sorts, id: \.self) { Text($0.title).tag($0) }
                    }
                } label: {
                    Label("Sort", systemImage: "arrow.up.arrow.down")
                }
            }
            ToolbarItem(placement: .secondaryAction) {
                Button(
                    store.isSelecting ? "Done Selecting" : "Select Tasks",
                    systemImage: store.isSelecting ? "checkmark.circle.fill" : "checkmark.circle"
                ) {
                    store.send(.selectTapped)
                }
                .disabled(!store.isSelecting && store.isEmpty)
            }
        }
        .modifier(SearchField(store: store))
        .modifier(TaskListPresentations(store: store))
    }
}

/// The search field of a list that searches, such as the Logbook.
private struct SearchField: ViewModifier {
    var store: StoreOf<TaskListFeature>

    func body(content: Content) -> some View {
        if store.query.searches {
            content.searchable(text: Binding(get: { store.searchText }, set: { store.send(.searchChanged($0)) }))
        } else {
            content
        }
    }
}

extension TaskListView {
    private var empty: some View {
        ContentUnavailableView {
            Label(store.query.empty.title, systemImage: store.query.empty.systemImage)
        } description: {
            Text(store.query.empty.description)
        } actions: {
            if let emptyAction {
                Button(emptyAction.title, action: emptyAction.perform)
            }
        }
    }
}

extension TaskListView where Header == EmptyView {
    package init(store: StoreOf<TaskListFeature>) {
        self.init(store: store) { EmptyView() }
    }
}

/// The detail, date picker, prompt and alert a task list presents. Apply it
/// to any view showing the list's rows, such as the board.
package struct TaskListPresentations: ViewModifier {
    @Bindable package var store: StoreOf<TaskListFeature>

    package init(store: StoreOf<TaskListFeature>) {
        self.store = store
    }

    package func body(content: Content) -> some View {
        content
            .navigationDestination(item: $store.scope(state: \.detail, action: \.detail)) { store in
                TaskDetailView(store: store)
            }
            .sheet(item: $store.scope(state: \.reschedule, action: \.reschedule)) { store in
                RescheduleView(store: store)
                    .sheetLayout()
            }
            .alert($store.scope(state: \.prompt, action: \.prompt))
            .confirmationDialog($store.scope(state: \.statusPicker, action: \.statusPicker))
            .alert($store.scope(state: \.alert, action: \.alert))
            .alert($store.scope(state: \.bulkPrompt, action: \.bulkPrompt))
    }
}

/// One row with its swipe actions and context menu.
package struct TaskListRow: View {
    package var row: TaskRowData
    package var note: String?
    package var store: StoreOf<TaskListFeature>

    package init(row: TaskRowData, note: String? = nil, store: StoreOf<TaskListFeature>) {
        self.row = row
        self.note = note
        self.store = store
    }

    private var isPicked: Bool { store.selection.contains(row.id) }

    package var body: some View {
        HStack(spacing: 10) {
            if store.isSelecting {
                Image(systemName: isPicked ? "checkmark.circle.fill" : "circle")
                    .font(.title3)
                    .foregroundStyle(isPicked ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                    .accessibilityHidden(true)
            }
            VStack(alignment: .leading, spacing: 4) {
                TaskRow(row) { store.send(.toggled(row.id)) }
                if let note {
                    Text(note)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                }
            }
        }
        .contentShape(Rectangle())
        .onTapGesture { tapped() }
        .accessibilityAddTraits(store.isSelecting && isPicked ? .isSelected : [])
        // Onto a day in the calendar, to move it there. A list in manual
        // order drags rows to reorder them instead.
        .modifier(DraggableTask(id: row.id, isOn: !store.canReorder))
        .accessibilityElement(children: .combine)
        .accessibilityHint(note ?? "")
        .accessibilityAction { store.send(.rowTapped(row.id)) }
        .swipeActions(edge: .leading, allowsFullSwipe: false) {
            if !store.isSelecting, row.status != .inProgress {
                Button(row.status == .procrastinated ? "Restart" : "Start") {
                    store.send(.rowAction(row.id, .status(.inProgress)))
                }
                .tint(.blue)
            }
        }
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            ForEach(trailingActions, id: \.self) { action in
                Button(action.title) { store.send(.rowAction(row.id, action)) }
                    .tint(action.tint)
            }
        }
        .contextMenu {
            ForEach(TaskStatus.allCases.filter { $0 != row.status }, id: \.self) { status in
                Button(status.title) { store.send(.rowAction(row.id, .status(status))) }
            }
            Divider()
            Button("Reschedule…") { store.send(.rowAction(row.id, .reschedule)) }
        }
    }

    private var trailingActions: [RowAction] {
        store.isSelecting ? [] : store.query.trailingActions.filter { $0 != .status(row.status) }
    }

    /// Opens the task, or picks it while picking. On the Mac, ⌘-click picks
    /// it too, and ⇧-click picks the rows from the last one picked.
    private func tapped() {
        #if os(macOS)
            let flags = NSEvent.modifierFlags
            if flags.contains(.shift) {
                store.send(.selectionExtended(to: row.id))
                return
            }
            if flags.contains(.command) {
                store.send(.selectionToggled(row.id))
                return
            }
        #endif
        store.send(.rowTapped(row.id))
    }
}

/// What can be done to the picked tasks: shown under the list while
/// picking.
private struct BulkBar: View {
    let store: StoreOf<TaskListFeature>

    var body: some View {
        let picked = store.selection.count
        let none = picked == 0
        VStack(spacing: 8) {
            HStack {
                Text(picked == 1 ? "1 task" : "\(picked) tasks")
                    .font(.subheadline.weight(.semibold))
                    .accessibilityLabel(picked == 1 ? "1 task picked" : "\(picked) tasks picked")
                Spacer()
                Button(store.selection.count == store.rows.count ? "Deselect All" : "Select All") {
                    store.send(.selectAllTapped)
                }
                Button("Done") { store.send(.selectTapped) }
                    .bold()
            }
            HStack {
                Menu("Status", systemImage: "circle.dashed") {
                    ForEach(TaskStatus.allCases, id: \.self) { status in
                        Button(status.title) { store.send(.bulk(.status(status))) }
                    }
                }
                Spacer()
                Menu("Due", systemImage: "calendar") {
                    Button("Today") { store.send(.bulkDue(days: 0)) }
                    Button("Tomorrow") { store.send(.bulkDue(days: 1)) }
                    Button("In a Week") { store.send(.bulkDue(days: 7)) }
                    Divider()
                    Button("No Due Date") { store.send(.bulkDue(days: nil)) }
                }
                Spacer()
                Menu("Priority", systemImage: "flag") {
                    ForEach(Priority.allCases, id: \.self) { priority in
                        Button(priority.title ?? String(localized: "None")) { store.send(.bulk(.priority(priority))) }
                    }
                }
                Spacer()
                Menu("Label", systemImage: "tag") {
                    ForEach(labels) { label in
                        Button("#\(label.name)") { store.send(.bulk(.addLabel(label.id))) }
                    }
                }
                .disabled(labels.isEmpty)
                Spacer()
                Menu("Move", systemImage: "folder") {
                    ForEach(places, id: \.avenue.id) { place in
                        Button(place.avenue.name) {
                            store.send(.bulk(.move(avenue: place.avenue.id, project: nil)))
                        }
                        ForEach(place.projects) { project in
                            Button("\(place.avenue.name) › \(project.name)") {
                                store.send(.bulk(.move(avenue: place.avenue.id, project: project.id)))
                            }
                        }
                    }
                }
                Spacer()
                Button("Move to Trash", systemImage: "trash", role: .destructive) { store.send(.bulk(.trash)) }
            }
            .labelStyle(.iconOnly)
            .disabled(none)
        }
        .buttonStyle(.borderless)
        .padding(.horizontal)
        .padding(.vertical, 10)
        .background(.bar)
    }

    private var labels: [LifrCore.Label] {
        (store.workspace.map { Array($0.labels.values) } ?? [])
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    /// Each open Avenue with its active Projects, in order.
    private var places: [(avenue: Avenue, projects: [Project])] {
        guard let workspace = store.workspace else { return [] }
        return workspace.avenues.values
            .filter { $0.archivedAt == nil }
            .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
            .map { avenue in
                let projects = workspace.projects.values
                    .filter { $0.avenueID == avenue.id && $0.status == .active }
                    .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                return (avenue, projects)
            }
    }
}

/// Lets a row be dragged out as its task's ID, when `isOn`.
private struct DraggableTask: ViewModifier {
    var id: UUID
    var isOn: Bool

    func body(content: Content) -> some View {
        if isOn {
            content.draggable(id.uuidString)
        } else {
            content
        }
    }
}

extension RowAction {
    var title: String {
        switch self {
        case .status(let status): status.title
        case .reschedule: String(localized: "Reschedule")
        case .pickStatus: String(localized: "Status")
        }
    }

    var tint: Color {
        switch self {
        case .status(.done): .green
        case .status(.wontDo): .gray
        case .status(.procrastinated): .orange
        case .status(.inProgress): .blue
        case .status(.todo): .secondary
        case .reschedule: .indigo
        case .pickStatus: .purple
        }
    }
}

struct RescheduleView: View {
    @Bindable var store: StoreOf<RescheduleFeature>

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    ForEach(RescheduleFeature.Shortcut.allCases, id: \.self) { shortcut in
                        Button {
                            store.send(.shortcutPicked(shortcut))
                        } label: {
                            Label(shortcut.title, systemImage: shortcut.symbol)
                        }
                    }
                    Button(role: .destructive) {
                        store.send(.clearTapped)
                    } label: {
                        Label("No due date", systemImage: "xmark.circle")
                    }
                }
                Section {
                    DatePicker("Due", selection: $store.date, displayedComponents: .date)
                        .datePickerStyle(.graphical)
                }
            }
            .navigationTitle(store.title)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { store.send(.cancelTapped) }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { store.send(.saveTapped) }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}
