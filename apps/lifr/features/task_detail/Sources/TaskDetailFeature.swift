import ComposableArchitecture
import Foundation
import LifrCore
import LifrFeedbackClient
import LifrRemindersClient
import LifrStoreClient
import LifrUI

/// One task, Task or Sub task: its fields, Sub tasks, comments and history.
/// Every field can be edited here; a Sub task opens in its own detail.
/// Status changes run Core's rules: Done or Won't Do with open Sub tasks ask
/// first, and a Sub task opening under a closed parent offers Undo.
@Reducer
package struct TaskDetailFeature {
    @ObservableState
    package struct State: Equatable {
        package let id: UUID
        /// The latest save shown, so an earlier one's late reply is ignored.
        package var saves = SaveOrder()
        package var row: TaskRowData
        package var details: String
        package var project: String?
        package var labels: [String]
        /// The title as typed; saved on submit.
        package var title: String
        /// The description being edited, or `nil` when it's only shown.
        package var detailsDraft: String?
        /// A description whose save was refused, to edit again instead of
        /// typing it over.
        package var refusedDetails: String?
        package var startDate: CalendarDate?
        package var dueDate: CalendarDate?
        package var dueTime: TimeOfDay?
        package var recurrence: Recurrence?
        /// The custom repeat being set up, while its sheet is open.
        package var repeatDraft: RepeatDraft?
        package var today: CalendarDate
        package var avenueID: UUID
        package var projectID: UUID?
        package var labelIDs: Set<UUID>
        /// Where the task can move: open Avenues, and the active Projects of
        /// the task's Avenue.
        package var avenueChoices: [Choice]
        package var projectChoices: [Choice]
        /// The active Projects of the other open Avenues, by Avenue, so a
        /// task moves straight into one in a single change.
        package var otherProjectChoices: [ProjectGroup]
        /// Every label, for the label menu.
        package var labelChoices: [Choice]
        /// Tasks this one can become a Sub task of: open top-level tasks in
        /// the same Avenue and Project. Empty for a Sub task, or a task
        /// with Sub tasks of its own, since Sub tasks don't nest.
        package var parentChoices: [Choice]
        package var newLabelName = ""
        package var createdAt: Date
        package var closedAt: Date?
        package var isSubtask: Bool
        package var subtasks: [TaskRowData]
        package var reminders: [ReminderRow]
        /// The custom reminder being set up, while its sheet is open.
        package var reminderDraft: ReminderDraft?
        /// Notifications are turned off for Lifr on this device, so its
        /// reminders won't ring here.
        package var notificationsDenied = false
        package var timeline: [TimelineEntry]
        package var showsCommentsOnly = false
        /// Whether the timeline shows every entry, not just the latest.
        package var showsWholeTimeline = false
        package var newSubtaskTitle = ""
        package var newComment = ""
        /// The comment being edited, and its text as typed.
        package var editingCommentID: UUID?
        package var editedComment = ""
        package var banner: Banner?
        @Presents package var subtask: TaskDetailFeature.State?

        /// The title and description as typed but not saved yet, as one
        /// edit, for saving when another task opens in this one's place.
        /// `nil` when there's nothing to save.
        package var unsavedEdit: Workspace.TaskEdit? {
            let typed = title.trimmingCharacters(in: .whitespacesAndNewlines)
            let title = typed.isEmpty || typed == row.title ? nil : typed
            let details = detailsDraft == details ? nil : detailsDraft
            guard title != nil || details != nil else { return nil }
            return Workspace.TaskEdit(title: title, details: details)
        }
        @Presents package var prompt: AlertState<StatusPrompt>?
        @Presents package var alert: AlertState<Never>?
        /// Asks before a comment is deleted.
        @Presents package var commentDeletion: AlertState<UUID>?
        /// What a Sub task's checkbox can move it to.
        @Presents package var statusPicker: ConfirmationDialogState<StatusPick>?

        /// `nil` when the task isn't in `workspace` or is in the trash.
        package init?(id: UUID, workspace: Workspace, now: Date, timeZone: TimeZone) {
            guard let task = workspace.tasks[id], task.deletedAt == nil else { return nil }
            self.id = id
            row = TaskRowData(task, in: workspace, now: now, timeZone: timeZone)
            details = task.details
            project = task.projectID.flatMap { workspace.projects[$0]?.name }
            labels = task.labelIDs.compactMap { workspace.labels[$0]?.name }.sorted()
            title = task.title
            startDate = task.startDate
            dueDate = task.dueDate
            dueTime = task.dueTime
            recurrence = task.recurrence
            today = CalendarDate(now, in: timeZone)
            avenueID = task.avenueID
            projectID = task.projectID
            labelIDs = task.labelIDs
            avenueChoices = workspace.avenues.values
                .filter { $0.archivedAt == nil || $0.id == task.avenueID }
                .sorted { $0.sortOrder < $1.sortOrder }
                .map { Choice(id: $0.id, name: $0.name) }
            projectChoices = workspace.projects.values
                .filter { $0.avenueID == task.avenueID && ($0.status == .active || $0.id == task.projectID) }
                .sorted { $0.sortOrder < $1.sortOrder }
                .map { Choice(id: $0.id, name: $0.name) }
            otherProjectChoices = workspace.avenues.values
                .filter { $0.archivedAt == nil && $0.id != task.avenueID }
                .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                .compactMap { avenue in
                    let projects = workspace.projects.values
                        .filter { $0.avenueID == avenue.id && $0.status == .active }
                        .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                        .map { Choice(id: $0.id, name: $0.name) }
                    return projects.isEmpty
                        ? nil : ProjectGroup(avenue: Choice(id: avenue.id, name: avenue.name), projects: projects)
                }
            labelChoices = workspace.labels.values
                .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
                .map { Choice(id: $0.id, name: $0.name) }
            createdAt = task.createdAt
            closedAt = task.closedAt
            isSubtask = task.isSubtask
            subtasks = workspace.subtasks(of: id).map {
                TaskRowData($0, in: workspace, now: now, timeZone: timeZone)
            }
            let hasSubtasks = !workspace.subtasks(of: id).isEmpty
            parentChoices =
                task.isSubtask || hasSubtasks
                ? []
                : workspace.tasks.values
                    .filter {
                        $0.id != task.id && $0.deletedAt == nil && $0.isOpen && !$0.isSubtask
                            && $0.avenueID == task.avenueID && $0.projectID == task.projectID
                    }
                    .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                    .map { Choice(id: $0.id, name: $0.title) }
            reminders = workspace.reminders(of: id, timeZone: timeZone).map { reminder in
                ReminderRow(reminder, fireAt: reminder.fireDate(for: task, in: timeZone))
            }
            timeline = workspace.timeline(of: id)
        }

        /// The day a repeat is read from: the due day, the start day, or today.
        package var repeatAnchor: CalendarDate { dueDate ?? startDate ?? today }

        package var repeatPresets: [RepeatPreset] { RepeatPreset.presets(for: repeatAnchor) }

        /// The reminders the person can add: before the due date only when
        /// there is one.
        package var reminderChoices: [ReminderChoice] {
            row.due == nil ? [.inOneHour, .tomorrowMorning] : ReminderChoice.allCases
        }

        /// What the timeline shows: everything, or comments only.
        package var visibleTimeline: [TimelineEntry] {
            let entries = showsCommentsOnly ? timeline.filter(\.isComment) : timeline
            guard !showsWholeTimeline else { return entries }
            return entries.enumerated()
                .filter { $0.offset < State.latestShown || $0.element.id == editingCommentID }
                .map(\.element)
        }

        /// How many of the timeline's entries Show All would add.
        package var hiddenTimelineCount: Int {
            guard !showsWholeTimeline else { return 0 }
            let entries = showsCommentsOnly ? timeline.filter(\.isComment) : timeline
            return entries.count - visibleTimeline.count
        }

        /// The latest entries the timeline shows before Show All.
        package static let latestShown = 3

        /// Shows `snapshot`'s records, keeping what the person is typing,
        /// unless later ones are shown already. False when the task is gone,
        /// such as moved to the trash.
        package mutating func refresh(from snapshot: Snapshot, now: Date, timeZone: TimeZone) -> Bool {
            guard saves.isLatest(snapshot) else { return true }
            guard let shown = State(id: id, workspace: snapshot.workspace, now: now, timeZone: timeZone) else {
                return false
            }
            // Changed in place, field by field, so the open detail redraws
            // only what the save or sync changed. Each field read from the
            // workspace is listed; the ones only the person sets stay.
            // Keep what's being typed over what was saved.
            if title == row.title { adopt(\.title, from: shown) }
            adopt(\.row, from: shown)
            adopt(\.details, from: shown)
            adopt(\.project, from: shown)
            adopt(\.labels, from: shown)
            adopt(\.startDate, from: shown)
            adopt(\.dueDate, from: shown)
            adopt(\.dueTime, from: shown)
            adopt(\.recurrence, from: shown)
            adopt(\.today, from: shown)
            adopt(\.avenueID, from: shown)
            adopt(\.projectID, from: shown)
            adopt(\.labelIDs, from: shown)
            adopt(\.avenueChoices, from: shown)
            adopt(\.projectChoices, from: shown)
            adopt(\.otherProjectChoices, from: shown)
            adopt(\.labelChoices, from: shown)
            adopt(\.parentChoices, from: shown)
            adopt(\.createdAt, from: shown)
            adopt(\.closedAt, from: shown)
            adopt(\.isSubtask, from: shown)
            adopt(\.subtasks, from: shown)
            adopt(\.reminders, from: shown)
            adopt(\.timeline, from: shown)
            if let editingCommentID, !timeline.contains(where: { $0.id == editingCommentID }) {
                self.editingCommentID = nil
                editedComment = ""
            }
            if prompt != nil { prompt = nil }
            if alert != nil { alert = nil }
            if statusPicker != nil { statusPicker = nil }
            if subtask?.refresh(from: snapshot, now: now, timeZone: timeZone) == false { subtask = nil }
            return true
        }

        /// Sets `field` to `other`'s value only when it differs, so the
        /// views reading it aren't told it changed.
        private mutating func adopt<Value: Equatable>(_ field: WritableKeyPath<Self, Value>, from other: Self) {
            if self[keyPath: field] != other[keyPath: field] { self[keyPath: field] = other[keyPath: field] }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case statusPicked(TaskStatus)
        case priorityPicked(Priority)
        case titleSubmitted
        case editDetailsTapped
        case detailsSaved
        case detailsCancelled
        case startDatePicked(CalendarDate?)
        case dueDatePicked(CalendarDate?)
        /// Due Today, Tomorrow or none from the Task menu: moves the start
        /// with the due date, as Reschedule does.
        case rescheduled(CalendarDate?)
        case dueTimePicked(TimeOfDay?)
        /// A preset, or `nil` for Never.
        case repeatPicked(RepeatPreset?)
        case repeatCustomTapped
        case repeatCustomSaved
        case repeatCustomCancelled
        case avenuePicked(UUID)
        case projectPicked(UUID?)
        case labelToggled(UUID)
        /// Makes the task a Sub task of this one, or a top-level task when
        /// nil.
        case parentPicked(UUID?)
        case trashTapped
        case newLabelSubmitted
        case subtaskTapped(UUID)
        case subtaskLoaded(UUID, Workspace)
        indirect case subtask(PresentationAction<Action>)
        case subtaskToggled(UUID)
        /// Shows the statuses for this task, as Change Status… on its
        /// reminder asks.
        case statusPickerRequested
        case addSubtaskSubmitted
        case commentSubmitted
        case commentEditTapped(UUID)
        case commentEditSaved
        case commentEditCancelled
        case commentDeleteTapped(UUID)
        case commentDeletion(PresentationAction<UUID>)
        /// Undo on a change in the Activity list.
        case historyUndoTapped(UUID)
        case reminderPicked(ReminderChoice)
        case reminderCustomTapped
        case reminderCustomSaved
        case reminderCustomCancelled
        case reminderRemoved(UUID)
        /// The detail is showing: checks whether notifications are on.
        case appeared
        case permissionChecked(ReminderPermission)
        /// The reminder was saved, but notifications are off on this device.
        case notificationsOff
        case saved(Result<Saved, any Error>)
        /// A save of what was typed was refused; it goes back in its field.
        case typedSaveFailed(Typed, any Error)
        case bannerUndoTapped
        case bannerDismissed
        case prompt(PresentationAction<StatusPrompt>)
        case alert(PresentationAction<Never>)
        case statusPicker(PresentationAction<StatusPick>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            /// The records changed; lists showing them should refresh.
            case saved(Snapshot)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.feedback) private var feedback
    @Dependency(\.reminders) private var reminders
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .statusPicked(let status):
                return setStatus(status, state.id)

            case .priorityPicked(let priority):
                return edit(state, Workspace.TaskEdit(priority: priority))

            case .titleSubmitted:
                let title = state.title.trimmingCharacters(in: .whitespacesAndNewlines)
                guard title != state.row.title else { return .none }
                guard !title.isEmpty else {
                    state.title = state.row.title
                    return .none
                }
                return edit(state, Workspace.TaskEdit(title: title))

            case .editDetailsTapped:
                state.detailsDraft = state.refusedDetails ?? state.details
                state.refusedDetails = nil
                return .none

            case .detailsSaved:
                guard let draft = state.detailsDraft else { return .none }
                state.detailsDraft = nil
                guard draft != state.details else { return .none }
                return save(restoring: .details(draft)) { [id = state.id] in
                    try await $0.edit(id, Workspace.TaskEdit(details: draft))
                }

            case .detailsCancelled:
                state.detailsDraft = nil
                return .none

            case .startDatePicked(let date):
                return edit(state, Workspace.TaskEdit(startDate: .some(date)))

            case .dueDatePicked(let date):
                // A time only makes sense with a date.
                let time: TimeOfDay?? = date == nil ? .some(nil) : nil
                return edit(state, Workspace.TaskEdit(dueDate: .some(date), dueTime: time))

            case .rescheduled(let date):
                // The start moves with it, as Reschedule on a row does.
                guard let date, let move = Workspace.TaskEdit.due(date, start: state.startDate, due: state.dueDate)
                else { return date == nil ? .send(.dueDatePicked(nil)) : .none }
                return edit(state, move)

            case .dueTimePicked(let time):
                return edit(state, Workspace.TaskEdit(dueTime: .some(time)))

            case .repeatPicked(let preset):
                return save { [id = state.id] in
                    try await $0.setRecurrence(preset?.rule, every: preset?.interval ?? 1, of: id)
                }

            case .repeatCustomTapped:
                state.repeatDraft = RepeatDraft(anchor: state.repeatAnchor, from: state.recurrence)
                return .none

            case .repeatCustomSaved:
                guard let draft = state.repeatDraft, draft.canSave else { return .none }
                state.repeatDraft = nil
                return save { [id = state.id] in try await $0.setRecurrence(draft.rule, every: draft.interval, of: id) }

            case .repeatCustomCancelled:
                state.repeatDraft = nil
                return .none

            case .avenuePicked(let avenueID):
                guard avenueID != state.avenueID else { return .none }
                return save { [id = state.id] in try await $0.move(id, toAvenue: avenueID, project: nil) }

            case .projectPicked(let projectID):
                guard projectID != state.projectID else { return .none }
                // A Project of another Avenue takes the task to that Avenue
                // too, in the same change.
                let avenueID =
                    state.otherProjectChoices.first { group in group.projects.contains { $0.id == projectID } }?
                    .avenue.id ?? state.avenueID
                return save { [id = state.id] in try await $0.move(id, toAvenue: avenueID, project: projectID) }

            case .labelToggled(let labelID):
                var labelIDs = state.labelIDs
                if labelIDs.remove(labelID) == nil { labelIDs.insert(labelID) }
                return edit(state, Workspace.TaskEdit(labelIDs: labelIDs))

            case .parentPicked(let parentID):
                return save { [id = state.id] in try await $0.setParent(parentID, of: id) }

            case .trashTapped:
                return save { [id = state.id] in try await $0.moveToTrash(id) }

            case .newLabelSubmitted:
                let name = state.newLabelName.trimmingCharacters(in: .whitespacesAndNewlines)
                state.newLabelName = ""
                guard !name.isEmpty else { return .none }
                return save(restoring: .label(name)) { [id = state.id] in try await $0.addLabel(named: name, to: id) }

            case .subtaskTapped(let id):
                return .run { [store] send in
                    await send(.subtaskLoaded(id, try await store.load()))
                } catch: { error, send in
                    await send(.saved(.failure(error)))
                }

            case .subtaskLoaded(let id, let workspace):
                state.subtask = State(id: id, workspace: workspace, now: now, timeZone: timeZone)
                return .none

            case .subtask(.presented(.delegate(.saved(let snapshot)))):
                _ = state.refresh(from: snapshot, now: now, timeZone: timeZone)
                return .send(.delegate(.saved(snapshot)))

            case .subtaskToggled(let id):
                guard let subtask = state.subtasks.first(where: { $0.id == id }) else { return .none }
                state.statusPicker = .statuses(for: id, title: subtask.title, status: subtask.status)
                return .none

            case .statusPickerRequested:
                state.statusPicker = .statuses(for: state.id, title: state.row.title, status: state.row.status)
                return .none

            case .statusPicker(.presented(let pick)):
                return setStatus(pick.status, pick.task)

            case .addSubtaskSubmitted:
                let title = state.newSubtaskTitle.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !title.isEmpty, !state.isSubtask else { return .none }
                state.newSubtaskTitle = ""
                return save(restoring: .subtask(title)) { [id = state.id] in
                    try await $0.createTask(Workspace.NewTask(title: title, parentID: id))
                }

            case .commentSubmitted:
                let body = state.newComment
                guard !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return .none }
                state.newComment = ""
                return save(restoring: .comment(body)) { [id = state.id] in try await $0.addComment(body, to: id) }

            case .commentEditTapped(let id):
                guard let entry = state.timeline.first(where: { $0.id == id && $0.isMine }) else { return .none }
                state.editingCommentID = id
                state.editedComment = entry.text
                return .none

            case .commentEditSaved:
                guard let id = state.editingCommentID else { return .none }
                let body = state.editedComment.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !body.isEmpty else { return .none }
                state.editingCommentID = nil
                guard body != state.timeline.first(where: { $0.id == id })?.text else { return .none }
                return save(restoring: .commentEdit(id, body)) { try await $0.editComment(id, body: body) }

            case .commentEditCancelled:
                state.editingCommentID = nil
                return .none

            case .commentDeleteTapped(let id):
                guard state.timeline.contains(where: { $0.id == id && $0.isMine }) else { return .none }
                state.commentDeletion = AlertState {
                    TextState("Delete this comment?")
                } actions: {
                    ButtonState(role: .destructive, action: id) { TextState("Delete") }
                    ButtonState(role: .cancel) { TextState("Cancel") }
                } message: {
                    TextState("It's deleted on all your devices, and can't be brought back.")
                }
                return .none

            case .commentDeletion(.presented(let id)):
                if state.editingCommentID == id { state.editingCommentID = nil }
                return save { try await $0.deleteComment(id) }

            case .historyUndoTapped(let eventID):
                return save { try await $0.revert(eventID) }

            case .reminderPicked(let choice):
                return addReminder(
                    choice.trigger(now: now, timeZone: timeZone), to: state.id, isOneOff: choice.isOneOff)

            case .reminderCustomTapped:
                state.reminderDraft = ReminderDraft(now: now, hasDueDate: state.dueDate != nil)
                return .none

            case .reminderCustomSaved:
                guard let draft = state.reminderDraft, draft.canSave(now: now) else { return .none }
                state.reminderDraft = nil
                return addReminder(draft.trigger, to: state.id)

            case .reminderCustomCancelled:
                state.reminderDraft = nil
                return .none

            case .appeared:
                return .run { [reminders] send in await send(.permissionChecked(await reminders.permission())) }

            case .permissionChecked(let permission):
                state.notificationsDenied = permission == .denied
                return .none

            case .reminderRemoved(let id):
                return save { try await $0.removeReminder(id) }

            case .notificationsOff:
                // Once the Reminders section says so, it needn't ask again.
                guard !state.notificationsDenied else { return .none }
                state.notificationsDenied = true
                state.alert = AlertState {
                    TextState("Notifications are off")
                } message: {
                    TextState(
                        """
                        The reminder is saved and rings on your other devices, but not on this one. \
                        To hear it here, turn on notifications for Lifr in Settings.
                        """)
                }
                return .none

            case .prompt(.presented(let answer)):
                guard let retry = answer.retry else { return .none }
                return save { try await $0.setStatus(retry.status, retry.task, true) }

            case .bannerUndoTapped:
                guard let change = state.banner?.undo else { return .none }
                state.banner = nil
                return save { try await $0.undo(change) }

            case .bannerDismissed:
                state.banner = nil
                return .none

            case .saved(.success(let saved)):
                let isLatest = saved.number >= state.saves.latest
                guard state.refresh(from: saved.snapshot, now: now, timeZone: timeZone) else {
                    return .send(.delegate(.saved(saved.snapshot)))
                }
                if let parentID = saved.change.reopenedParent, let parent = saved.workspace.tasks[parentID] {
                    state.banner = .reopened(parent, by: saved.change)
                }
                return isLatest ? .send(.delegate(.saved(saved.snapshot))) : .none

            case .typedSaveFailed(let typed, let error):
                state.restore(typed)
                return failed(&state, error)

            case .saved(.failure(let error)):
                return failed(&state, error)

            case .binding, .prompt, .alert, .statusPicker, .commentDeletion, .delegate, .subtask:
                return .none
            }
        }
        .ifLet(\.$subtask, action: \.subtask) { TaskDetailFeature() }
        .ifLet(\.$prompt, action: \.prompt)
        .ifLet(\.$alert, action: \.alert)
        .ifLet(\.$commentDeletion, action: \.commentDeletion)
        .ifLet(\.$statusPicker, action: \.statusPicker)
    }

    private func failed(_ state: inout State, _ error: any Error) -> Effect<Action> {
        let subtasks = state.subtasks
        if let prompt = AlertState.forRefusal(
            error, canShow: false, title: { id in subtasks.first { $0.id == id }?.title })
        {
            state.prompt = prompt
        } else {
            state.alert = .failed("Couldn't change the task", error)
        }
        return .run { [feedback] _ in await feedback(.warning) }
    }

    /// Saves the reminder, then asks for notifications if they haven't been
    /// asked for, and says so when they're off.
    private func addReminder(_ trigger: Reminder.Trigger, to id: UUID, isOneOff: Bool = false) -> Effect<Action> {
        .run { [store, reminders] send in
            do {
                await send(.saved(.success(try await store.addReminder(trigger, to: id, isOneOff: isOneOff))))
            } catch {
                await send(.saved(.failure(error)))
                return
            }
            if (try? await reminders.requestPermission()) != true {
                await send(.notificationsOff)
            }
        }
    }

    private func edit(_ state: State, _ edit: Workspace.TaskEdit) -> Effect<Action> {
        save { [id = state.id] in try await $0.edit(id, edit) }
    }

    private func setStatus(_ status: TaskStatus, _ id: UUID) -> Effect<Action> {
        save { [feedback] in
            let saved = try await $0.setStatus(status, id)
            await feedback(Feedback(status: status))
            return saved
        }
    }

    /// Saves what was typed, putting it back if the save is refused.
    private func save(
        restoring typed: Typed, _ operation: @escaping @Sendable (StoreClient) async throws -> Saved
    ) -> Effect<Action> {
        .run { [store] send in
            do {
                await send(.saved(.success(try await operation(store))))
            } catch {
                await send(.typedSaveFailed(typed, error))
            }
        }
    }

    private func save(_ operation: @escaping @Sendable (StoreClient) async throws -> Saved) -> Effect<Action> {
        .run { [store] send in
            do {
                await send(.saved(.success(try await operation(store))))
            } catch {
                await send(.saved(.failure(error)))
            }
        }
    }
}

/// Something to pick by name: an Avenue, a Project or a label.
package struct Choice: Identifiable, Hashable, Sendable {
    package var id: UUID
    package var name: String
}

/// An Avenue's Projects, to pick one by name.
package struct ProjectGroup: Identifiable, Hashable, Sendable {
    package var avenue: Choice
    package var projects: [Choice]

    package var id: UUID { avenue.id }
}

/// Text typed into the detail whose save was refused.
package enum Typed: Equatable, Sendable {
    case subtask(String)
    case comment(String)
    case commentEdit(UUID, String)
    case details(String)
    case label(String)
}

extension TaskDetailFeature.State {
    /// Puts refused text back where it was typed, unless something new has
    /// been typed there since.
    mutating func restore(_ typed: Typed) {
        switch typed {
        case .subtask(let title):
            if newSubtaskTitle.isEmpty { newSubtaskTitle = title }
        case .comment(let body):
            if newComment.isEmpty { newComment = body }
        case .commentEdit(let id, let body):
            guard editingCommentID == nil, timeline.contains(where: { $0.id == id }) else { return }
            editingCommentID = id
            editedComment = body
        case .details(let draft):
            if detailsDraft == nil { refusedDetails = draft }
        case .label(let name):
            if newLabelName.isEmpty { newLabelName = name }
        }
    }
}
