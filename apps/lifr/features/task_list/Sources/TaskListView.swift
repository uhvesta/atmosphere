import ChatUI
import ComposableArchitecture
import LifrCore
import LifrTaskDetail
import LifrUI
import SwiftUI

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
        .task { await store.send(.task).finish() }
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

    package var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            TaskRow(row) { store.send(.toggled(row.id)) }
            if let note {
                Text(note)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture { store.send(.rowTapped(row.id)) }
        // Onto a day in the calendar, to move it there. A list in manual
        // order drags rows to reorder them instead.
        .modifier(DraggableTask(id: row.id, isOn: !store.canReorder))
        .accessibilityElement(children: .combine)
        .accessibilityHint(note ?? "")
        .accessibilityAction { store.send(.rowTapped(row.id)) }
        .swipeActions(edge: .leading, allowsFullSwipe: false) {
            if row.status != .inProgress {
                Button(row.status == .procrastinated ? "Restart" : "Start") {
                    store.send(.rowAction(row.id, .status(.inProgress)))
                }
                .tint(.blue)
            }
        }
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            ForEach(store.query.trailingActions.filter { $0 != .status(row.status) }, id: \.self) { action in
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
                DatePicker("Due", selection: $store.date, displayedComponents: .date)
                    .datePickerStyle(.graphical)
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
