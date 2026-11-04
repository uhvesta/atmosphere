import ComposableArchitecture
import Foundation
import LifrCore
import LifrRemindersClient
import LifrStoreClient
import LifrUI

/// One task, Task or Sub task: its fields, Sub tasks, comments and history.
/// Status changes run Core's rules: Done or Won't Do with open Sub tasks ask
/// first, and a Sub task opening under a closed parent offers Undo.
@Reducer
package struct TaskDetailFeature {
    @ObservableState
    package struct State: Equatable {
        package let id: UUID
        package var row: TaskRowData
        package var details: String
        package var project: String?
        package var labels: [String]
        package var createdAt: Date
        package var closedAt: Date?
        package var isSubtask: Bool
        package var subtasks: [TaskRowData]
        package var reminders: [ReminderRow]
        package var timeline: [TimelineEntry]
        package var showsCommentsOnly = false
        package var newSubtaskTitle = ""
        package var newComment = ""
        package var banner: Banner?
        @Presents package var prompt: ConfirmationDialogState<StatusPrompt>?
        @Presents package var alert: AlertState<Never>?

        /// `nil` when the task isn't in `workspace` or is in the trash.
        package init?(id: UUID, workspace: Workspace, now: Date, timeZone: TimeZone) {
            guard let task = workspace.tasks[id], task.deletedAt == nil else { return nil }
            self.id = id
            row = TaskRowData(task, in: workspace, now: now, timeZone: timeZone)
            details = task.details
            project = task.projectID.flatMap { workspace.projects[$0]?.name }
            labels = task.labelIDs.compactMap { workspace.labels[$0]?.name }.sorted()
            createdAt = task.createdAt
            closedAt = task.closedAt
            isSubtask = task.isSubtask
            subtasks = workspace.subtasks(of: id).map {
                TaskRowData($0, in: workspace, now: now, timeZone: timeZone)
            }
            reminders = workspace.reminders(of: id, timeZone: timeZone).map { reminder in
                ReminderRow(reminder, fireAt: reminder.fireDate(for: task, in: timeZone))
            }
            timeline = workspace.timeline(of: id)
        }

        /// The reminders the person can add: before the due date only when
        /// there is one.
        package var reminderChoices: [ReminderChoice] {
            row.due == nil ? [.inOneHour, .tomorrowMorning] : ReminderChoice.allCases
        }

        /// What the timeline shows: everything, or comments only.
        package var visibleTimeline: [TimelineEntry] {
            showsCommentsOnly ? timeline.filter(\.isComment) : timeline
        }

        /// Shows `workspace`, keeping what the person is typing. False when
        /// the task is gone, such as moved to the trash.
        package mutating func refresh(from workspace: Workspace, now: Date, timeZone: TimeZone) -> Bool {
            guard var refreshed = State(id: id, workspace: workspace, now: now, timeZone: timeZone) else {
                return false
            }
            refreshed.showsCommentsOnly = showsCommentsOnly
            refreshed.newSubtaskTitle = newSubtaskTitle
            refreshed.newComment = newComment
            refreshed.banner = banner
            self = refreshed
            return true
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case statusPicked(TaskStatus)
        case priorityPicked(Priority)
        case subtaskToggled(UUID)
        case addSubtaskSubmitted
        case commentSubmitted
        case reminderPicked(ReminderChoice)
        case reminderRemoved(UUID)
        /// The reminder was saved, but notifications are off on this device.
        case notificationsOff
        case saved(Result<Saved, any Error>)
        case bannerUndoTapped
        case bannerDismissed
        case prompt(PresentationAction<StatusPrompt>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            /// The records changed; lists showing them should refresh.
            case saved(Workspace)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.reminders) private var reminders
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .statusPicked(let status):
                return save { [id = state.id] in try await $0.setStatus(status, id) }

            case .priorityPicked(let priority):
                return save { [id = state.id] in try await $0.edit(id, Workspace.TaskEdit(priority: priority)) }

            case .subtaskToggled(let id):
                guard let subtask = state.subtasks.first(where: { $0.id == id }) else { return .none }
                let status: TaskStatus = subtask.status.isClosed ? .todo : .done
                return save { try await $0.setStatus(status, id) }

            case .addSubtaskSubmitted:
                let title = state.newSubtaskTitle.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !title.isEmpty, !state.isSubtask else { return .none }
                state.newSubtaskTitle = ""
                return save { [id = state.id] in
                    try await $0.createTask(Workspace.NewTask(title: title, parentID: id))
                }

            case .commentSubmitted:
                let body = state.newComment
                guard !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return .none }
                state.newComment = ""
                return save { [id = state.id] in try await $0.addComment(body, to: id) }

            case .reminderPicked(let choice):
                let trigger = choice.trigger(now: now, timeZone: timeZone)
                return .run { [id = state.id, store, reminders] send in
                    do {
                        await send(.saved(.success(try await store.addReminder(trigger, to: id))))
                    } catch {
                        await send(.saved(.failure(error)))
                        return
                    }
                    if (try? await reminders.requestPermission()) != true {
                        await send(.notificationsOff)
                    }
                }

            case .reminderRemoved(let id):
                return save { try await $0.removeReminder(id) }

            case .notificationsOff:
                state.alert = AlertState {
                    TextState("Notifications are off")
                } message: {
                    TextState(
                        "The reminder is saved and rings on your other devices, but not on this one. "
                            + "To hear it here, turn on notifications for Lifr in Settings.")
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
                guard state.refresh(from: saved.workspace, now: now, timeZone: timeZone) else {
                    return .send(.delegate(.saved(saved.workspace)))
                }
                if let parentID = saved.change.reopenedParent, let parent = saved.workspace.tasks[parentID] {
                    state.banner = .reopened(parent, by: saved.change)
                }
                return .send(.delegate(.saved(saved.workspace)))

            case .saved(.failure(let error)):
                let subtasks = state.subtasks
                if let prompt = ConfirmationDialogState.forRefusal(
                    error, canShow: false, title: { id in subtasks.first { $0.id == id }?.title })
                {
                    state.prompt = prompt
                } else {
                    state.alert = .failed("Couldn't change the task", error)
                }
                return .none

            case .binding, .prompt, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$prompt, action: \.prompt)
        .ifLet(\.$alert, action: \.alert)
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
