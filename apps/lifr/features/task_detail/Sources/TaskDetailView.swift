import ComposableArchitecture
import LifrCore
import LifrStoreClient
import LifrUI
import SwiftUI

#if os(iOS)
    import UIKit
#endif

package struct TaskDetailView: View {
    @Bindable package var store: StoreOf<TaskDetailFeature>
    @Environment(\.timeZone) private var timeZone
    @Environment(\.locale) private var locale
    @Environment(\.openURL) private var openURL
    @FocusState private var titleFocused: Bool
    /// The chip whose editor is open beside it.
    @State private var chipEditor: ChipEditor?

    private enum ChipEditor: Hashable {
        case start, due, labels
    }

    package init(store: StoreOf<TaskDetailFeature>) {
        self.store = store
    }

    package var body: some View {
        Form {
            fields
            chips
            details
            if !store.isSubtask {
                subtasks
            }
            activity
            manage
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
        .alert($store.scope(state: \.prompt, action: \.prompt))
        .alert($store.scope(state: \.alert, action: \.alert))
        .confirmationDialog($store.scope(state: \.statusPicker, action: \.statusPicker))
        .sheet(isPresented: $store.isMoving) {
            NavigationStack {
                Form { place }
                    .formStyle(.grouped)
                    .navigationTitle("Move to")
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) {
                            Button("Done") { store.isMoving = false }
                        }
                    }
            }
            .frame(minWidth: 360, minHeight: 240)
        }
        .sheet(
            isPresented: Binding(
                get: { store.repeatDraft != nil },
                set: { if !$0 { store.send(.repeatCustomCancelled) } })
        ) {
            RepeatEditor(
                draft: Binding(
                    get: { store.repeatDraft ?? RepeatDraft(anchor: store.repeatAnchor, from: nil) },
                    set: { store.send(.binding(.set(\.repeatDraft, $0))) }),
                save: { store.send(.repeatCustomSaved) },
                cancel: { store.send(.repeatCustomCancelled) })
        }
        .sheet(
            isPresented: Binding(
                get: { store.reminderDraft != nil },
                set: { if !$0 { store.send(.reminderCustomCancelled) } })
        ) {
            ReminderEditor(
                draft: Binding(
                    get: { store.reminderDraft ?? ReminderDraft(now: Date(), hasDueDate: false) },
                    set: { store.send(.binding(.set(\.reminderDraft, $0))) }),
                save: { store.send(.reminderCustomSaved) },
                cancel: { store.send(.reminderCustomCancelled) })
        }
        .task { store.send(.appeared) }
    }

    @ViewBuilder
    private var fields: some View {
        Section {
            TextField("Title", text: $store.title, prompt: Text("Title"), axis: .vertical)
                .titleField()
                .font(.title2.bold())
                .focused($titleFocused)
                .onSubmit { store.send(.titleSubmitted) }
                .submitLabel(.done)
            if let updatedBy = store.updatedBy {
                Text(updatedBy)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            StackedPicker(
                "Status", value: store.row.status.title,
                selection: Binding(get: { store.row.status }, set: { store.send(.statusPicked($0)) })
            ) {
                ForEach(TaskStatus.allCases, id: \.self) { status in
                    Text(status.title).tag(status)
                }
            }
            if store.canFocus {
                // A Live Activity, only for a task In Progress, and only
                // when asked for.
                if store.isFocusing {
                    Button("Stop Focusing", systemImage: "stop.circle") { store.send(.focusTapped) }
                } else if store.row.status == .inProgress {
                    Button("Focus on This", systemImage: "scope") { store.send(.focusTapped) }
                }
            }
            StackedPicker(
                "Priority", value: store.row.priority.title ?? String(localized: "None"),
                selection: Binding(get: { store.row.priority }, set: { store.send(.priorityPicked($0)) })
            ) {
                ForEach(Priority.allCases, id: \.self) { priority in
                    Text(priority.title ?? String(localized: "None")).tag(priority)
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
                        .keyboardShortcut(.cancelAction)
                    Spacer()
                    Button("Save") { store.send(.detailsSaved) }
                        .bold()
                        .keyboardShortcut(.return, modifiers: .command)
                }
                .buttonStyle(.borderless)
            } else {
                if store.details.isEmpty {
                    Button("Add a description") { store.send(.editDetailsTapped) }
                } else {
                    ForEach(Array(store.details.descriptionParts.enumerated()), id: \.offset) { _, part in
                        switch part {
                        case .text(let text):
                            Text(LocalizedStringKey(text))
                        case .item(let line, let title, let checked):
                            ChecklistItem(title: title, checked: checked) {
                                store.send(.checklistItemToggled(line: line))
                            }
                        }
                    }
                    Button("Edit description") { store.send(.editDetailsTapped) }
                }
            }
        }
    }

    /// The dates, repeat, reminders, place and labels, as chips that wrap.
    /// A chip opens its editor; one with nothing set offers to add it.
    private var chips: some View {
        Section {
            ChipFlow {
                dueChip
                startChip
                if !store.isSubtask {
                    repeatChip
                }
                ForEach(store.reminders) { reminderChip($0) }
                addReminderChip
                if !store.isSubtask {
                    avenueChip
                    projectChip
                }
                ForEach(store.labelChoices.filter { store.labelIDs.contains($0.id) }) { label in
                    Button {
                        chipEditor = .labels
                    } label: {
                        Chip(systemImage: "number", title: Text(label.name))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Label \(label.name)")
                    .accessibilityHint("Opens the labels")
                }
                labelsChip
            }
            .padding(.vertical, 4)
            if store.notificationsDenied {
                Button {
                    if let url = Self.notificationSettings { openURL(url) }
                } label: {
                    Label(
                        "Notifications are off for Lifr, so reminders won't ring here. Turn them on in Settings.",
                        systemImage: "bell.slash")
                }
                .foregroundStyle(.orange)
            }
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                if store.row.due?.isOverdue == true {
                    Label("Overdue", systemImage: "exclamationmark.circle").foregroundStyle(.red)
                }
                if !store.isSubtask {
                    Text("Moving a task moves its Sub tasks with it. In another Avenue it gets a new code.")
                }
            }
        }
    }

    private var dueChip: some View {
        let due = store.dueDate
        let overdue = store.row.due?.isOverdue == true
        return Button {
            if due == nil { store.send(.dueDatePicked(today(from: store.startDate, through: nil))) }
            chipEditor = .due
        } label: {
            if let due {
                if let time = store.dueTime {
                    Chip(
                        systemImage: "calendar",
                        title: Text("Due \(date(due, at: time), format: Self.dayAndTime)"),
                        tint: overdue ? .red : nil)
                } else {
                    Chip(
                        systemImage: "calendar",
                        title: Text("Due \(date(due, at: nil), format: .dateTime.weekday().month().day())"),
                        tint: overdue ? .red : nil)
                }
            } else {
                Chip(systemImage: "calendar", title: Text("Due"), isSet: false)
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(due == nil ? Text("Add a due date") : Text("Due"))
        .accessibilityValue(dueValue(due, overdue: overdue))
        .popover(isPresented: editing(.due)) {
            dateEditor {
                if let due {
                    datePicker("Due", due, from: store.startDate) { store.send(.dueDatePicked($0)) }
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
                        Button("Add a time") { store.send(.dueTimePicked(try? TimeOfDay("09:00"))) }
                    }
                    Button("Remove due date", systemImage: "trash", role: .destructive) {
                        chipEditor = nil
                        store.send(.dueDatePicked(nil))
                    }
                }
            }
        }
    }

    /// The due date VoiceOver reads, saying so when it's overdue.
    private func dueValue(_ due: CalendarDate?, overdue: Bool) -> Text {
        guard let due else { return Text("") }
        // Read in the screen's language and time zone, as the chip shows it.
        var format = dueFormat.locale(locale)
        format.timeZone = timeZone
        let when = date(due, at: store.dueTime).formatted(format)
        return overdue ? Text("\(when), overdue") : Text(when)
    }

    private var dueFormat: Date.FormatStyle {
        store.dueTime == nil ? .dateTime.weekday().month().day() : Self.dayAndTime
    }

    private static let dayAndTime: Date.FormatStyle = .dateTime.weekday().month().day().hour().minute()

    private var startChip: some View {
        let start = store.startDate
        return Button {
            if start == nil { store.send(.startDatePicked(today(from: nil, through: store.dueDate))) }
            chipEditor = .start
        } label: {
            if let start {
                Chip(
                    systemImage: "arrow.right.to.line",
                    title: Text("Starts \(date(start, at: nil), format: .dateTime.weekday().month().day())"))
            } else {
                Chip(systemImage: "arrow.right.to.line", title: Text("Start"), isSet: false)
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(start == nil ? Text("Add a start date") : Text("Start"))
        .accessibilityValue(
            start.map { Text(date($0, at: nil), format: .dateTime.weekday().month().day()) } ?? Text("")
        )
        .popover(isPresented: editing(.start)) {
            dateEditor {
                if let start {
                    datePicker("Start", start, through: store.dueDate) { store.send(.startDatePicked($0)) }
                    Button("Remove start date", systemImage: "trash", role: .destructive) {
                        chipEditor = nil
                        store.send(.startDatePicked(nil))
                    }
                }
            }
        }
    }

    private var repeatChip: some View {
        Menu {
            repeatChoices
        } label: {
            if let recurrence = store.recurrence {
                Chip(systemImage: "repeat", title: Text(recurrence.title(locale)))
            } else {
                Chip(systemImage: "repeat", title: Text("Repeat"), isSet: false)
            }
        }
        .chipMenu()
        .accessibilityLabel("Repeat")
        .accessibilityValue(store.recurrence?.title(locale) ?? String(localized: "Never"))
    }

    private func reminderChip(_ reminder: ReminderRow) -> some View {
        Menu {
            if let beforeDue = reminder.beforeDue {
                Text(beforeDue)
            }
            Button("Remove Reminder", systemImage: "xmark.circle", role: .destructive) {
                store.send(.reminderRemoved(reminder.id))
            }
        } label: {
            if let fireAt = reminder.fireAt {
                Chip(systemImage: "bell", title: Text(fireAt, format: Self.dayAndTime))
            } else {
                Chip(systemImage: "bell.slash", title: Text("Needs a due date"), tint: .secondary)
            }
        }
        .chipMenu()
        .accessibilityLabel("Reminder")
        .accessibilityValue(reminderValue(reminder))
    }

    private func reminderValue(_ reminder: ReminderRow) -> Text {
        let when =
            reminder.fireAt.map { Text($0, format: Self.dayAndTime) }
            ?? Text("Needs a due date")
        guard let beforeDue = reminder.beforeDue else { return when }
        return Text("\(when), \(beforeDue)")
    }

    private var addReminderChip: some View {
        Menu {
            ForEach(store.reminderChoices, id: \.self) { choice in
                Button(choice.title) { store.send(.reminderPicked(choice)) }
            }
            Button("Custom…") { store.send(.reminderCustomTapped) }
        } label: {
            Chip(systemImage: "bell", title: Text("Remind"), isSet: false)
        }
        .chipMenu()
        .accessibilityLabel("Add reminder")
    }

    private var avenueChip: some View {
        Menu {
            Picker("Avenue", selection: Binding(get: { store.avenueID }, set: { store.send(.avenuePicked($0)) })) {
                ForEach(store.avenueChoices) { Text($0.name).tag($0.id) }
            }
            .pickerStyle(.inline)
        } label: {
            Chip(systemImage: "square.stack", title: Text(avenueName))
        }
        .chipMenu()
        .accessibilityLabel("Avenue")
        .accessibilityValue(avenueName)
    }

    private var projectChip: some View {
        Menu {
            Picker(
                "Project", selection: Binding(get: { store.projectID }, set: { store.send(.projectPicked($0)) })
            ) {
                Text("None").tag(UUID?.none)
                ForEach(store.projectChoices) { Text($0.name).tag(UUID?.some($0.id)) }
                ForEach(store.otherProjectChoices) { group in
                    Section(group.avenue.name) {
                        ForEach(group.projects) { Text($0.name).tag(UUID?.some($0.id)) }
                    }
                }
            }
            .pickerStyle(.inline)
        } label: {
            if store.projectID == nil {
                Chip(systemImage: "folder", title: Text("Project"), isSet: false)
            } else {
                Chip(systemImage: "folder", title: Text(projectName))
            }
        }
        .chipMenu()
        .accessibilityLabel(store.projectID == nil ? Text("Add to a Project") : Text("Project"))
        .accessibilityValue(store.projectID == nil ? "" : projectName)
    }

    private var labelsChip: some View {
        Button {
            chipEditor = .labels
        } label: {
            Chip(systemImage: "number", title: Text("Label"), isSet: false)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Edit labels")
        .popover(isPresented: editing(.labels)) {
            #if os(iOS)
                NavigationStack {
                    labelList
                        .navigationTitle("Labels")
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbar {
                            ToolbarItem(placement: .confirmationAction) {
                                Button("Done") { chipEditor = nil }
                            }
                        }
                }
                .frame(minWidth: 320, minHeight: 360)
                .presentationDetents([.medium, .large])
            #else
                labelList.frame(minWidth: 280, minHeight: 320)
            #endif
        }
    }

    /// Every label, checked when the task has it, and a field for a new one.
    private var labelList: some View {
        List {
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
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(store.labelIDs.contains(label.id) ? .isSelected : [])
                }
                TextField("New label", text: $store.newLabelName)
                    .onSubmit { store.send(.newLabelSubmitted) }
                    .submitLabel(.done)
            }
        }
    }

    /// Whether `editor` is open, for a chip's popover.
    private func editing(_ editor: ChipEditor) -> Binding<Bool> {
        Binding(
            get: { chipEditor == editor },
            set: { open in
                if open {
                    chipEditor = editor
                } else if chipEditor == editor {
                    chipEditor = nil
                }
            })
    }

    /// A date chip's popover: a calendar and what can be changed with it,
    /// scrolling when large text makes it taller than the screen.
    private func dateEditor<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        let stack = VStack(alignment: .leading, spacing: 12) { content() }
            .buttonStyle(.borderless)
            .padding()
        return ViewThatFits(in: .vertical) {
            stack
            ScrollView { stack }
        }
        .frame(minWidth: 320)
        .presentationCompactAdaptation(.popover)
    }

    /// A calendar for a date that can't be before `lower` or after `upper`,
    /// so a task never starts after it's due.
    private func datePicker(
        _ title: LocalizedStringKey, _ day: CalendarDate, from lower: CalendarDate? = nil,
        through upper: CalendarDate? = nil, _ pick: @escaping (CalendarDate?) -> Void
    ) -> some View {
        DatePicker(
            title,
            selection: Binding(get: { date(day, at: nil) }, set: { pick(CalendarDate($0, in: timeZone)) }),
            in: (lower.map { date($0, at: nil) } ?? .distantPast)...(upper.map { date($0, at: nil) }
                ?? .distantFuture),
            displayedComponents: .date
        )
        .datePickerStyle(.graphical)
    }

    /// Today, kept within `lower` and `upper`, for a date being added.
    private func today(from lower: CalendarDate?, through upper: CalendarDate?) -> CalendarDate {
        var today = CalendarDate(Date(), in: timeZone)
        if let lower, today < lower { today = lower }
        if let upper, today > upper { today = upper }
        return today
    }

    @ViewBuilder
    private var repeatChoices: some View {
        ForEach(store.repeatPresets, id: \.self) { preset in
            Button(preset.title) { store.send(.repeatPicked(preset)) }
        }
        Button("Custom…") { store.send(.repeatCustomTapped) }
        if store.recurrence != nil {
            Divider()
            Button("Never", role: .destructive) { store.send(.repeatPicked(nil)) }
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

    private var avenueName: String {
        store.avenueChoices.first { $0.id == store.avenueID }?.name ?? ""
    }

    private var projectName: String {
        let projects = store.projectChoices + store.otherProjectChoices.flatMap(\.projects)
        return projects.first { $0.id == store.projectID }?.name ?? String(localized: "None")
    }

    private var place: some View {
        Section {
            StackedPicker(
                "Avenue", value: avenueName,
                selection: Binding(get: { store.avenueID }, set: { store.send(.avenuePicked($0)) })
            ) {
                ForEach(store.avenueChoices) { Text($0.name).tag($0.id) }
            }
            StackedPicker(
                "Project", value: projectName,
                selection: Binding(get: { store.projectID }, set: { store.send(.projectPicked($0)) })
            ) {
                Text("None").tag(UUID?.none)
                ForEach(store.projectChoices) { Text($0.name).tag(UUID?.some($0.id)) }
                ForEach(store.otherProjectChoices) { group in
                    Section(group.avenue.name) {
                        ForEach(group.projects) { Text($0.name).tag(UUID?.some($0.id)) }
                    }
                }
            }
        } footer: {
            Text("Moving a task moves its Sub tasks with it. In another Avenue it gets a new code.")
        }
    }

    /// Changing what kind of task this is, and the trash.
    private var manage: some View {
        Section {
            if store.isSubtask {
                Button("Turn into a Task", systemImage: "arrow.up.forward.square") { store.send(.parentPicked(nil)) }
            } else if !store.parentChoices.isEmpty {
                Menu {
                    ForEach(store.parentChoices) { choice in
                        Button(choice.name) { store.send(.parentPicked(choice.id)) }
                    }
                } label: {
                    Label("Make Sub task Of…", systemImage: "arrow.turn.down.right")
                }
            }
            Button("Move to Trash", systemImage: "trash", role: .destructive) { store.send(.trashTapped) }
        }
    }

    private var subtasks: some View {
        Section {
            ForEach(store.subtasks) { row in
                TaskRow(row) {
                    store.send(.subtaskToggled(row.id))
                } pick: {
                    store.send(.subtaskStatusNamed(row.id, $0))
                }
                .contentShape(Rectangle())
                .onTapGesture { store.send(.subtaskTapped(row.id)) }
                .accessibilityAction(named: "Open") { store.send(.subtaskTapped(row.id)) }
            }
            .onMove { store.send(.subtasksMoved($0, $1)) }
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

    /// Lifr's notification settings in the system's Settings.
    private static var notificationSettings: URL? {
        #if os(iOS)
            URL(string: UIApplication.openNotificationSettingsURLString)
        #else
            URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension")
        #endif
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
                    if store.editingCommentID == entry.id {
                        commentEditor
                    } else {
                        Text(
                            entry.isComment
                                ? LocalizedStringKey(entry.text) : LocalizedStringKey(stringLiteral: entry.text)
                        )
                        .font(entry.isComment ? .body : .callout)
                        .foregroundStyle(entry.isComment ? .primary : .secondary)
                        if entry.isEdited {
                            Text("Edited").font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                    if entry.canUndo {
                        Button("Revert") { store.send(.historyUndoTapped(entry.id)) }
                            .font(.caption)
                            .buttonStyle(.borderless)
                            .frame(maxWidth: .infinity, alignment: .trailing)
                    }
                }
                .accessibilityElement(children: store.editingCommentID == entry.id ? .contain : .combine)
                .accessibilityActions {
                    if entry.canUndo {
                        Button("Revert") { store.send(.historyUndoTapped(entry.id)) }
                    }
                    if entry.isMine {
                        Button("Edit") { store.send(.commentEditTapped(entry.id)) }
                        Button("Delete") { store.send(.commentDeleteTapped(entry.id)) }
                    }
                }
                .contextMenu {
                    if entry.isMine {
                        Button("Edit", systemImage: "pencil") { store.send(.commentEditTapped(entry.id)) }
                        Button("Delete", systemImage: "trash", role: .destructive) {
                            store.send(.commentDeleteTapped(entry.id))
                        }
                    }
                }
            }
            if store.hiddenTimelineCount > 0 {
                Button("Show All (\(store.hiddenTimelineCount) More)") { store.showsWholeTimeline = true }
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
        .alert($store.scope(state: \.commentDeletion, action: \.commentDeletion))
    }

    /// The comment being edited, with Save and Cancel.
    @ViewBuilder private var commentEditor: some View {
        if store.editingCommentID != nil {
            TextField("Comment", text: $store.editedComment, axis: .vertical)
                .onSubmit { store.send(.commentEditSaved) }
            HStack {
                Button("Cancel") { store.send(.commentEditCancelled) }
                Spacer()
                Button("Save") { store.send(.commentEditSaved) }
                    .disabled(store.editedComment.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .buttonStyle(.borderless)
            .font(.callout)
        }
    }
}

#Preview {
    let workspace = Workspace.sample(now: Date(), timeZone: .current)
    if let parent = workspace.tasks.values.first(where: { !workspace.subtasks(of: $0.id).isEmpty }),
        let state = TaskDetailFeature.State(id: parent.id, workspace: workspace, now: Date(), timeZone: .current)
    {
        NavigationStack {
            TaskDetailView(
                store: Store(initialState: state) {
                    TaskDetailFeature()
                } withDependencies: {
                    $0.store = .inMemory(workspace)
                })
        }
    } else {
        Text("The sample has no task with Sub tasks.")
    }
}

/// A picker whose chosen value wraps at accessibility text sizes, rather
/// than being cut to a few letters, under its label.
private struct StackedPicker<Selection: Hashable, Content: View>: View {
    let title: LocalizedStringKey
    let value: String
    @Binding var selection: Selection
    @ViewBuilder let content: () -> Content
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    init(
        _ title: LocalizedStringKey, value: String, selection: Binding<Selection>,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.title = title
        self.value = value
        _selection = selection
        self.content = content
    }

    var body: some View {
        if dynamicTypeSize.isAccessibilitySize {
            Menu {
                Picker(title, selection: $selection, content: content)
                    .pickerStyle(.inline)
                    .labelsHidden()
            } label: {
                VStack(alignment: .leading) {
                    Text(title).foregroundStyle(.primary)
                    HStack(alignment: .firstTextBaseline) {
                        Text(value)
                        Image(systemName: "chevron.up.chevron.down").imageScale(.small)
                    }
                    .foregroundStyle(.secondary)
                }
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .tint(.primary)
            .accessibilityLabel(Text(title))
            .accessibilityValue(value)
        } else {
            Picker(title, selection: $selection, content: content)
        }
    }
}

/// A checklist item from a description: tapping it checks or unchecks it.
private struct ChecklistItem: View {
    let title: String
    let checked: Bool
    let toggle: () -> Void

    var body: some View {
        Button(action: toggle) {
            Label {
                Text(LocalizedStringKey(title))
                    .strikethrough(checked)
                    .foregroundStyle(checked ? .secondary : .primary)
            } icon: {
                Image(systemName: checked ? "checkmark.square.fill" : "square")
                    .contentTransition(.symbolEffect(.replace))
            }
        }
        .buttonStyle(.plain)
        .accessibilityValue(checked ? Text("Checked") : Text("Not checked"))
    }
}
