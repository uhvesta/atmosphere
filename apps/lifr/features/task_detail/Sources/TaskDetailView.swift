import ComposableArchitecture
import LifrCore
import LifrStoreClient
import LifrUI
import SwiftUI

package struct TaskDetailView: View {
    @Bindable package var store: StoreOf<TaskDetailFeature>
    @Environment(\.timeZone) private var timeZone
    @FocusState private var titleFocused: Bool

    package init(store: StoreOf<TaskDetailFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            fields
            details
            dates
            if !store.isSubtask {
                place
            }
            labels
            if !store.isSubtask {
                subtasks
            }
            reminders
            activity
        }
        .safeAreaInset(edge: .top) {
            if let banner = store.banner {
                BannerView(banner, undo: { store.send(.bannerUndoTapped) }, dismiss: { store.send(.bannerDismissed) })
            }
        }
        .navigationTitle(store.row.code)
        #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
        #endif
        .navigationDestination(item: $store.scope(state: \.subtask, action: \.subtask)) { store in
            TaskDetailView(store: store)
        }
        .onChange(of: titleFocused) { _, focused in
            if !focused { store.send(.titleSubmitted) }
        }
        .confirmationDialog($store.scope(state: \.prompt, action: \.prompt))
        .alert($store.scope(state: \.alert, action: \.alert))
    }

    @ViewBuilder
    private var fields: some View {
        Section {
            TextField("Title", text: $store.title, axis: .vertical)
                .font(.title2.bold())
                .focused($titleFocused)
                .onSubmit { store.send(.titleSubmitted) }
                .submitLabel(.done)
            Picker("Status", selection: Binding(get: { store.row.status }, set: { store.send(.statusPicked($0)) })) {
                ForEach(TaskStatus.allCases, id: \.self) { status in
                    Text(status.title).tag(status)
                }
            }
            Picker(
                "Priority", selection: Binding(get: { store.row.priority }, set: { store.send(.priorityPicked($0)) })
            ) {
                ForEach(Priority.allCases, id: \.self) { priority in
                    Text(priority.title ?? "None").tag(priority)
                }
            }
            LabeledContent("Created") { Text(store.createdAt, format: .dateTime.month().day().year()) }
            if let closedAt = store.closedAt {
                LabeledContent("Closed") { Text(closedAt, format: .dateTime.month().day().year()) }
            }
        }
    }

    @ViewBuilder
    private var details: some View {
        Section("Description") {
            if store.detailsDraft != nil {
                TextEditor(text: Binding(get: { store.detailsDraft ?? "" }, set: { store.detailsDraft = $0 }))
                    .frame(minHeight: 120)
                HStack {
                    Button("Cancel") { store.send(.detailsCancelled) }
                    Spacer()
                    Button("Save") { store.send(.detailsSaved) }
                        .bold()
                }
                .buttonStyle(.borderless)
            } else {
                if store.details.isEmpty {
                    Button("Add a description") { store.send(.editDetailsTapped) }
                } else {
                    Text(LocalizedStringKey(store.details))
                    Button("Edit description") { store.send(.editDetailsTapped) }
                }
            }
        }
    }

    @ViewBuilder
    private var dates: some View {
        Section("Dates") {
            dateRow("Due", store.dueDate) { store.send(.dueDatePicked($0)) }
            if let due = store.dueDate {
                if let time = store.dueTime {
                    HStack {
                        DatePicker(
                            "Time",
                            selection: Binding(
                                get: { self.date(due, at: time) },
                                set: { store.send(.dueTimePicked(TimeOfDay($0, in: timeZone))) }),
                            displayedComponents: .hourAndMinute)
                        clear("Remove time") { store.send(.dueTimePicked(nil)) }
                    }
                } else {
                    Button("Add a time") {
                        store.send(.dueTimePicked(TimeOfDay(self.date(due, at: nil) + 9 * 3600, in: timeZone)))
                    }
                }
            }
            dateRow("Start", store.startDate) { store.send(.startDatePicked($0)) }
        }
    }

    /// A date with a picker and a remove button, or a button that sets it
    /// to today.
    @ViewBuilder
    private func dateRow(
        _ title: String, _ day: CalendarDate?, _ pick: @escaping (CalendarDate?) -> Void
    ) -> some View {
        if let day {
            HStack {
                DatePicker(
                    title,
                    selection: Binding(get: { date(day, at: nil) }, set: { pick(CalendarDate($0, in: timeZone)) }),
                    displayedComponents: .date)
                clear("Remove \(title.lowercased()) date") { pick(nil) }
            }
        } else {
            Button("Add a \(title.lowercased()) date") { pick(CalendarDate(Date(), in: timeZone)) }
        }
    }

    private func clear(_ label: String, _ action: @escaping () -> Void) -> some View {
        Button(label, systemImage: "xmark.circle.fill", action: action)
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .foregroundStyle(.secondary)
    }

    /// Midnight of `day`, plus `time`, in the person's time zone.
    private func date(_ day: CalendarDate, at time: TimeOfDay?) -> Date {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        let parts = DateComponents(
            year: day.year, month: day.month, day: day.day, hour: time?.hour ?? 0, minute: time?.minute ?? 0)
        return calendar.date(from: parts) ?? Date()
    }

    private var place: some View {
        Section {
            Picker("Avenue", selection: Binding(get: { store.avenueID }, set: { store.send(.avenuePicked($0)) })) {
                ForEach(store.avenueChoices) { Text($0.name).tag($0.id) }
            }
            Picker(
                "Project", selection: Binding(get: { store.projectID }, set: { store.send(.projectPicked($0)) })
            ) {
                Text("None").tag(UUID?.none)
                ForEach(store.projectChoices) { Text($0.name).tag(UUID?.some($0.id)) }
            }
        } footer: {
            Text("Moving a task moves its Sub tasks with it.")
        }
    }

    private var labels: some View {
        Section("Labels") {
            ForEach(store.labelChoices) { label in
                Button {
                    store.send(.labelToggled(label.id))
                } label: {
                    HStack {
                        Text("#\(label.name)")
                        Spacer()
                        if store.labelIDs.contains(label.id) {
                            Image(systemName: "checkmark")
                        }
                    }
                }
                .accessibilityAddTraits(store.labelIDs.contains(label.id) ? .isSelected : [])
            }
            TextField("New label", text: $store.newLabelName)
                .onSubmit { store.send(.newLabelSubmitted) }
                .submitLabel(.done)
        }
        .tint(.primary)
    }

    private var subtasks: some View {
        Section {
            ForEach(store.subtasks) { row in
                TaskRow(row) { store.send(.subtaskToggled(row.id)) }
                    .contentShape(Rectangle())
                    .onTapGesture { store.send(.subtaskTapped(row.id)) }
                    .accessibilityAction(named: "Open") { store.send(.subtaskTapped(row.id)) }
            }
            TextField("Add Sub task", text: $store.newSubtaskTitle)
                .onSubmit { store.send(.addSubtaskSubmitted) }
                .submitLabel(.done)
        } header: {
            HStack {
                Text("Sub tasks")
                Spacer()
                if let progress = store.row.subtasks {
                    Text("\(progress.closed)/\(progress.total)")
                        .accessibilityLabel("\(progress.closed) of \(progress.total) closed")
                }
            }
        }
    }

    private var reminders: some View {
        Section("Reminders") {
            ForEach(store.reminders) { reminder in
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        if let fireAt = reminder.fireAt {
                            Text(fireAt, format: .dateTime.weekday().month().day().hour().minute())
                        } else {
                            Text("Needs a due date")
                        }
                        if let beforeDue = reminder.beforeDue {
                            Text(beforeDue).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    Button("Remove", systemImage: "xmark.circle.fill") {
                        store.send(.reminderRemoved(reminder.id))
                    }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
            }
            Menu {
                ForEach(store.reminderChoices, id: \.self) { choice in
                    Button(choice.title) { store.send(.reminderPicked(choice)) }
                }
            } label: {
                Label("Add reminder", systemImage: "bell.badge")
            }
        }
    }

    private var activity: some View {
        Section {
            Picker("Show", selection: $store.showsCommentsOnly) {
                Text("All").tag(false)
                Text("Comments").tag(true)
            }
            .pickerStyle(.segmented)
            ForEach(store.visibleTimeline) { entry in
                VStack(alignment: .leading, spacing: 2) {
                    HStack {
                        Text(entry.author).font(.caption.bold())
                        Spacer()
                        Text(entry.date, format: .dateTime.month().day().hour().minute())
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Text(
                        entry.isComment ? LocalizedStringKey(entry.text) : LocalizedStringKey(stringLiteral: entry.text)
                    )
                    .font(entry.isComment ? .body : .callout)
                    .foregroundStyle(entry.isComment ? .primary : .secondary)
                    if entry.canUndo {
                        Button("Undo") { store.send(.historyUndoTapped(entry.id)) }
                            .font(.caption)
                            .buttonStyle(.borderless)
                            .frame(maxWidth: .infinity, alignment: .trailing)
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityActions {
                    if entry.canUndo {
                        Button("Undo") { store.send(.historyUndoTapped(entry.id)) }
                    }
                }
            }
            HStack {
                TextField("Add a comment", text: $store.newComment, axis: .vertical)
                    .onSubmit { store.send(.commentSubmitted) }
                Button {
                    store.send(.commentSubmitted)
                } label: {
                    Image(systemName: "paperplane.fill")
                }
                .accessibilityLabel("Send comment")
                .disabled(store.newComment.isEmpty)
            }
        } header: {
            Text("Activity")
        }
    }
}

#Preview {
    let workspace = Workspace.sample(now: Date(), timeZone: .current)
    let parent = workspace.tasks.values.first { !workspace.subtasks(of: $0.id).isEmpty }!
    NavigationStack {
        TaskDetailView(
            store: Store(
                initialState: TaskDetailFeature.State(
                    id: parent.id, workspace: workspace, now: Date(), timeZone: .current)!
            ) {
                TaskDetailFeature()
            } withDependencies: {
                $0.store = .inMemory(workspace)
            })
    }
}
