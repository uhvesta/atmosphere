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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// The row VoiceOver is on, so it can move to a neighbour when the row
    /// leaves the list instead of jumping to the top.
    @AccessibilityFocusState private var focusedRow: UUID?
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
            #if os(iOS)
                if store.showsSwipeTip {
                    GestureTipCard(
                        symbol: "hand.draw", title: "Swipe a task",
                        message: "Swipe right to start it, or left to close it or pick another status."
                    ) {
                        store.send(.swipeTipClosed, animation: .default)
                    }
                }
            #endif
            if emptyBelowHeader, store.workspace != nil, store.isEmpty {
                empty.listRowBackground(Color.clear)
            }
            if store.workspace == nil, store.loadError == nil {
                PlaceholderRows()
            }
            ForEach(store.sections) { section in
                Section(section.title) {
                    ForEach(section.rows) { row in
                        TaskListRow(row: row, note: section.notes[row.id], store: store)
                            .accessibilityFocused($focusedRow, equals: row.id)
                    }
                    .onMove(
                        perform: store.canReorder
                            ? { store.send(.moved(section: section.id, from: $0, to: $1)) } : nil)
                }
            }
        }
        .overlay {
            if let loadError = store.loadError {
                ContentUnavailableView {
                    Label("Couldn't load your tasks", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(loadError)
                } actions: {
                    Button("Try Again") { store.send(.retryTapped) }
                }
            } else if store.isSearching, store.isEmpty {
                ContentUnavailableView.search(text: store.searchText)
            } else if !emptyBelowHeader, store.workspace != nil, store.isEmpty {
                empty
            }
        }
        .safeAreaInset(edge: .top) {
            if let banner = store.banner {
                BannerView(banner, undo: { store.send(.bannerUndoTapped) }, dismiss: { store.send(.bannerDismissed) })
                    .transition(reduceMotion ? .opacity : .move(edge: .top).combined(with: .opacity))
            }
        }
        .onChange(of: store.sections) { old, new in
            // Only while VoiceOver is on a row, which is rarely.
            guard let row = focusedRow, let next = new.focus(after: row, leaving: old) else { return }
            focusedRow = next
        }
        // Rows move in, out and between sections, a sync's changes too.
        .animation(reduceMotion ? .easeInOut(duration: 0.2) : .snappy(duration: 0.25), value: store.sections)
        .animation(reduceMotion ? .easeInOut(duration: 0.2) : .spring, value: store.banner)
        .safeAreaInset(edge: .bottom) {
            if store.isSelecting {
                BulkBar(store: store)
                    .reportsBottomBar()
            }
        }
        .task { await store.send(.task).finish() }
        #if os(iOS)
            .refreshable { await store.send(.pulled).finish() }
            // The spinner goes either way, so say it worked.
            .sensoryFeedback(.success, trigger: store.syncs)
            .onChange(of: store.syncs) { AccessibilityNotification.Announcement(String(localized: "Synced")).post() }
        #else
            .onKeyPress(keys: [.upArrow, .downArrow]) { press in
                store.send(.cursorMoved(up: press.key == .upArrow, extending: press.modifiers.contains(.shift)))
                return .handled
            }
            .onKeyPress(.return) {
                store.send(.openCursorTapped)
                return .handled
            }
            .onKeyPress(.space) {
                store.send(.cursorStatusTapped)
                return .handled
            }
            .onKeyPress(keys: [.delete, .deleteForward]) { _ in
                store.send(.trashKeyPressed)
                return .handled
            }
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
            ToolbarItem(placement: .secondaryAction) {
                Button("Copy List", systemImage: "doc.on.doc") { store.send(.copyListTapped) }
                    .disabled(store.isEmpty)
            }
            ToolbarItem(placement: .secondaryAction) {
                ShareLink(item: store.markdown) { Label("Share List", systemImage: "square.and.arrow.up") }
                    .disabled(store.isEmpty)
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
            Label(store.emptyMessage.title, systemImage: store.emptyMessage.systemImage)
        } description: {
            Text(store.emptyMessage.description)
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
    @Environment(\.droppedText) private var droppedText
    @Environment(\.opensInNewWindow) private var opensInNewWindow
    /// Whether something dragged is over the row, to drop onto it.
    @State private var isTargeted = false

    package init(row: TaskRowData, note: String? = nil, store: StoreOf<TaskListFeature>) {
        self.row = row
        self.note = note
        self.store = store
    }

    private var isPicked: Bool { store.selection.contains(row.id) }

    /// Whether the keyboard is on this row. Only a row that's the last
    /// picked or the open task can be, so the rest skip looking.
    private var isCursor: Bool {
        guard store.selectionAnchor == row.id || store.detail?.id == row.id else { return false }
        return store.cursor == row.id
    }

    package var body: some View {
        HStack(spacing: 10) {
            if store.isSelecting {
                Image(systemName: isPicked ? "checkmark.circle.fill" : "circle")
                    .font(.title3)
                    .foregroundStyle(isPicked ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                    .accessibilityHidden(true)
            }
            VStack(alignment: .leading, spacing: 4) {
                TaskRow(row) {
                    store.send(.toggled(row.id))
                } pick: {
                    store.send(.rowAction(row.id, .status($0)))
                }
                if let note {
                    Text(note)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                }
            }
        }
        .contentShape(Rectangle())
        #if os(macOS)
            // Where ↑ ↓, Return, Space and ⌫ act.
            .listRowBackground(
                isCursor && !store.isSelecting
                    ? RoundedRectangle(cornerRadius: 6).fill(.tint.opacity(0.15)) : nil)
        #endif
        .onTapGesture { tapped() }
        .accessibilityAddTraits(store.isSelecting && isPicked ? .isSelected : [])
        // Onto a day in the calendar, to move it there. A list in manual
        // order drags rows to reorder them instead.
        .modifier(DraggableTask(row: row, isOn: !store.canReorder))
        // Another task dropped onto this one becomes its Sub task. Text
        // from another app goes where the list sends it, as if dropped
        // beside the row.
        .dropDestination(for: String.self) { items, _ in
            guard !OrderDrag.isAmong(items) else { return false }
            let ids = items.compactMap(UUID.init(uuidString:))
            if !ids.isEmpty, ids.count == items.count {
                store.send(.droppedOnto(row.id, tasks: ids))
                return true
            }
            guard let droppedText, !items.isEmpty else { return false }
            droppedText(items.joined(separator: "\n"))
            return true
        } isTargeted: {
            isTargeted = $0
        }
        // Outlines the row a drop would land on, so onto a row reads
        // apart from between two.
        .overlay {
            if isTargeted {
                RoundedRectangle(cornerRadius: 6).strokeBorder(.tint, lineWidth: 2).padding(-4)
                    .accessibilityHidden(true)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityHint(note ?? "")
        .accessibilityAction { store.send(.rowTapped(row.id)) }
        // What swipes and drags do, by name, for VoiceOver and Voice Control.
        .accessibilityActions {
            if !store.isSelecting {
                Button("Reschedule") { store.send(.rowAction(row.id, .reschedule)) }
                if store.canReorder {
                    Button("Move Up") { store.send(.movedOnePlace(row.id, up: true)) }
                    Button("Move Down") { store.send(.movedOnePlace(row.id, up: false)) }
                }
                Button("Move to Trash") { store.send(.trashRowTapped(row.id)) }
            }
        }
        .swipeActions(edge: .leading, allowsFullSwipe: false) {
            if !store.isSelecting, row.status != .inProgress {
                Button(
                    row.status == .procrastinated ? "Restart" : "Start",
                    systemImage: row.status == .procrastinated ? "arrow.clockwise" : "play"
                ) {
                    store.send(.rowAction(row.id, .status(.inProgress)))
                }
                .tint(.blue)
            }
        }
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            ForEach(trailingActions, id: \.self) { action in
                Button(action.title, systemImage: action.systemImage) { store.send(.rowAction(row.id, action)) }
                    .tint(action.tint)
            }
        }
        .contextMenu {
            ForEach(TaskStatus.allCases.filter { $0 != row.status }, id: \.self) { status in
                Button(status.title) { store.send(.rowAction(row.id, .status(status))) }
            }
            Divider()
            Button("Reschedule…") { store.send(.rowAction(row.id, .reschedule)) }
            Button("Copy as Markdown") { store.send(.copyTapped(row.id)) }
            if let opensInNewWindow {
                Button("Open in New Window") { opensInNewWindow(row.id) }
            }
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
        #if os(macOS)
            // One row like a Mac's toolbar: titled menus while they fit,
            // icons when the list is narrow.
            HStack(spacing: 14) {
                count(picked).fixedSize()
                Divider().frame(height: 16)
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 14) { actions }.labelStyle(.titleAndIcon).fixedSize()
                    HStack(spacing: 10) { actions }.labelStyle(.iconOnly).fixedSize()
                }
                .menuStyle(.borderlessButton)
                .disabled(picked == 0)
                Spacer(minLength: 0)
                pickAll.fixedSize()
                done.fixedSize()
            }
            .buttonStyle(.borderless)
            .padding(.horizontal)
            .padding(.vertical, 8)
            .background(.bar)
        #else
            VStack(spacing: 8) {
                HStack {
                    count(picked)
                    Spacer()
                    pickAll
                    done
                }
                HStack { actions }
                    .labelStyle(.iconOnly)
                    .disabled(picked == 0)
            }
            .buttonStyle(.borderless)
            .padding(.horizontal)
            .padding(.vertical, 10)
            .background(.bar)
        #endif
    }

    private func count(_ picked: Int) -> some View {
        Text("^[\(picked) task](inflect: true)")
            .font(.subheadline.weight(.semibold))
            .accessibilityLabel("^[\(picked) task](inflect: true) picked")
    }

    private var pickAll: some View {
        Button(store.selection.count == store.rows.count ? "Deselect All" : "Select All") {
            store.send(.selectAllTapped)
        }
    }

    private var done: some View {
        Button("Done Selecting") { store.send(.selectTapped) }
            .bold()
    }

    /// Spreads the actions across the iPhone's bar; the Mac's sit together.
    @ViewBuilder private var gap: some View {
        #if os(iOS)
            Spacer()
        #endif
    }

    @ViewBuilder private var actions: some View {
        Menu("Status", systemImage: "circle.dashed") {
            ForEach(TaskStatus.allCases, id: \.self) { status in
                Button(status.title) { store.send(.bulk(.status(status))) }
            }
        }
        gap
        Menu("Due", systemImage: "calendar") {
            Button("Today") { store.send(.bulkDue(days: 0)) }
            Button("Tomorrow") { store.send(.bulkDue(days: 1)) }
            Button("In a Week") { store.send(.bulkDue(days: 7)) }
            Divider()
            Button("No Due Date") { store.send(.bulkDue(days: nil)) }
        }
        gap
        Menu("Priority", systemImage: "flag") {
            ForEach(Priority.allCases, id: \.self) { priority in
                Button(priority.title ?? String(localized: "None")) { store.send(.bulk(.priority(priority))) }
            }
        }
        gap
        Menu("Label", systemImage: "tag") {
            ForEach(labels) { label in
                Button("#\(label.name)") { store.send(.bulk(.addLabel(label.id))) }
            }
        }
        .disabled(labels.isEmpty)
        gap
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
        gap
        Button("Move to Trash", systemImage: "trash", role: .destructive) { store.send(.bulk(.trash)) }
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
    var row: TaskRowData
    var isOn: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        if isOn {
            content.draggable(row.id.uuidString) {
                // Lifted off the list while it moves: tilted, with a shadow.
                TaskRow(row) {}
                    .padding(12)
                    .frame(width: 320, alignment: .leading)
                    .background(.background, in: .rect(cornerRadius: 10))
                    .rotationEffect(.degrees(reduceMotion ? 0 : 2))
                    .shadow(color: .black.opacity(0.2), radius: 8, y: 4)
                    .padding(12)
            }
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

    var systemImage: String {
        switch self {
        case .status(.done): "checkmark"
        case .status(.wontDo): "xmark"
        case .status(.procrastinated): "hourglass"
        case .status(.inProgress): "play"
        case .status(.todo): "circle"
        case .reschedule: "calendar"
        case .pickStatus: "ellipsis.circle"
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
                        Label("No Due Date", systemImage: "xmark.circle")
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

/// Rows shaped like tasks while the records load, so the list isn't blank.
private struct PlaceholderRows: View {
    var body: some View {
        Section {
            ForEach(0..<3, id: \.self) { index in
                HStack(spacing: 12) {
                    Image(systemName: "circle").font(.title3)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(index == 1 ? "A task with a longer title" : "A task title")
                        Text("ABC-1").font(.caption)
                    }
                }
                .redacted(reason: .placeholder)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Loading tasks")
    }
}

extension EnvironmentValues {
    /// Where text another app drops onto a task's row goes; with none,
    /// the row turns it down.
    @Entry package var droppedText: (@MainActor (String) -> Void)?
    /// Opens a task in a window of its own; with none, as on iPhone, rows
    /// don't offer it.
    @Entry package var opensInNewWindow: (@MainActor (UUID) -> Void)?
}
