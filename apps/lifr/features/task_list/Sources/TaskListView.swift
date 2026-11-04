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

    package init(store: StoreOf<TaskListFeature>, @ViewBuilder header: () -> Header) {
        self.store = store
        self.header = header()
    }

    package var body: some View {
        List {
            header
            ForEach(store.sections) { section in
                Section(section.title) {
                    ForEach(section.rows) { row in
                        TaskListRow(row: row, note: section.notes[row.id], store: store)
                    }
                }
            }
            if store.query.adds != nil, !store.isEmpty {
                Button {
                    store.send(.addTaskTapped)
                } label: {
                    Label("Add task", systemImage: "plus.circle.fill")
                }
            }
        }
        .overlay {
            if store.workspace != nil, store.isEmpty {
                ContentUnavailableView {
                    Label(store.query.empty.title, systemImage: store.query.empty.systemImage)
                } description: {
                    Text(store.query.empty.description)
                } actions: {
                    if store.query.adds != nil {
                        Button("Add task") { store.send(.addTaskTapped) }
                            .buttonStyle(.borderedProminent)
                    }
                }
            }
        }
        .safeAreaInset(edge: .top) {
            if let banner = store.banner {
                BannerView(banner, undo: { store.send(.bannerUndoTapped) }, dismiss: { store.send(.bannerDismissed) })
            }
        }
        .task { await store.send(.task).finish() }
        .modifier(TaskListPresentations(store: store))
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
            }
            .confirmationDialog($store.scope(state: \.prompt, action: \.prompt))
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
