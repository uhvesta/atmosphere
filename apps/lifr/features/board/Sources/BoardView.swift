import ComposableArchitecture
import LifrAvenue
import LifrCore
import LifrTaskList
import LifrUI
import SwiftUI
import UniformTypeIdentifiers

/// The board: a column per status, each a stack of cards. Drag a card to
/// another column to set its status, or use its context menu; in manual
/// order, drag it onto another card in its column to reorder.
package struct BoardView: View {
    @Bindable package var store: StoreOf<BoardFeature>

    package init(store: StoreOf<BoardFeature>) {
        self.store = store
    }

    package var body: some View {
        ScrollView(.horizontal) {
            HStack(alignment: .top, spacing: 12) {
                ForEach(store.columns) { column in
                    BoardColumn(column: column, store: store)
                }
            }
            .padding()
        }
        .navigationTitle(store.title)
        .toolbar {
            ToolbarItem(placement: .secondaryAction) {
                Menu {
                    Picker(
                        "Sort",
                        selection: Binding(get: { store.list.sort }, set: { store.send(.list(.sortPicked($0))) })
                    ) {
                        ForEach(store.list.sorts, id: \.self) { Text($0.title).tag($0) }
                    }
                } label: {
                    Label("Sort", systemImage: "arrow.up.arrow.down")
                }
            }
        }
        .task { await store.send(.list(.task)).finish() }
        .modifier(TaskListPresentations(store: store.scope(state: \.list, action: \.list)))
    }
}

private struct BoardColumn: View {
    var column: TaskSection
    var store: StoreOf<BoardFeature>
    @State private var isTargeted = false

    private var status: TaskStatus { TaskStatus(rawValue: column.id) ?? .todo }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("\(column.title.uppercased()) (\(column.rows.count))")
                .font(.caption.bold())
                .foregroundStyle(.secondary)
                .accessibilityAddTraits(.isHeader)
                .accessibilityLabel(
                    column.rows.count == 1
                        ? Text("\(column.title), 1 task") : Text("\(column.title), \(column.rows.count) tasks"))
            ForEach(column.rows) { row in
                BoardCard(row: row, note: column.notes[row.id], store: store)
            }
            if status == .todo {
                Button("Add Task", systemImage: "plus") { store.send(.addTapped) }
                    .buttonStyle(.borderless)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 8)
            }
            Spacer(minLength: 40)
        }
        .frame(width: 200, alignment: .topLeading)
        .frame(maxHeight: .infinity, alignment: .top)
        .padding(8)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(isTargeted ? Color.accentColor.opacity(0.15) : Color.secondary.opacity(0.08))
        )
        .dropDestination(for: String.self) { ids, _ in
            let dropped = ids.compactMap(UUID.init(uuidString:))
            for id in dropped {
                store.send(.dropped(id, on: status))
            }
            return !dropped.isEmpty
        } isTargeted: {
            isTargeted = $0
        }
    }
}

private struct BoardCard: View {
    var row: TaskRowData
    var note: String?
    var store: StoreOf<BoardFeature>
    @Environment(\.locale) private var locale

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                TaskCodeLabel(row.code)
                Spacer()
                PriorityBadge(row.priority)
            }
            Text(row.title).font(.callout)
            if let note {
                Text(note).font(.caption).foregroundStyle(.orange)
            }
            if row.subtasks != nil || row.due != nil {
                HStack(spacing: 6) {
                    if let progress = row.subtasks {
                        Text("\(progress.closed)/\(progress.total)")
                            .accessibilityLabel("\(progress.closed) of \(progress.total) Sub tasks closed")
                    }
                    if let due = row.due {
                        Text(due.text(locale))
                            .foregroundStyle(due.isOverdue ? .red : .secondary)
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 8).fill(.background))
        .contentShape(Rectangle())
        .onTapGesture { store.send(.list(.rowTapped(row.id))) }
        .draggable(row.id.uuidString)
        // In manual order, onto another card in its column to take its
        // place; onto a card in another column to move there.
        .dropDestination(for: String.self) { ids, _ in
            let dropped = ids.compactMap(UUID.init(uuidString:))
            for id in dropped {
                store.send(.droppedCard(id, onto: row.id))
            }
            return !dropped.isEmpty
        }
        .accessibilityElement(children: .combine)
        .accessibilityAction { store.send(.list(.rowTapped(row.id))) }
        .contextMenu {
            ForEach(BoardFeature.columns.filter { $0.rawValue != row.status.rawValue }, id: \.self) { status in
                Button("Move to \(status.title)") { store.send(.dropped(row.id, on: status)) }
            }
        }
    }
}

#Preview {
    let workspace = Workspace.sample(now: Date(), timeZone: .current)
    Group {
        if let avenue = workspace.avenues.values.first(where: { $0.key.description == "HEALTH" }) {
            NavigationStack {
                BoardView(
                    store: Store(initialState: BoardFeature.State(scope: .avenue(avenue.id), title: avenue.name)) {
                        BoardFeature()
                    } withDependencies: {
                        $0.store = .inMemory(workspace)
                    })
            }
        } else {
            Text("The sample has no HEALTH Avenue.")
        }
    }
    .frame(width: 1100, height: 600)
}
