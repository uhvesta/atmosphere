import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrUI

/// One task: its fields, Sub tasks and status. Changing the status runs
/// Core's rules: Won't Do with open Sub tasks asks first, and reopening a Sub
/// task under a closed parent offers to undo.
@Reducer
package struct TaskDetailFeature {
    @ObservableState
    package struct State: Equatable {
        package let id: UUID
        package var row: TaskRowData
        package var details: String
        package var project: String?
        package var subtasks: [TaskRowData]
        @Presents package var alert: AlertState<Action.Alert>?

        /// `nil` when the task isn't in `workspace` or is in the trash.
        package init?(id: UUID, workspace: Workspace, now: Date, timeZone: TimeZone) {
            guard let task = workspace.tasks[id], task.deletedAt == nil else { return nil }
            self.id = id
            row = TaskRowData(task, in: workspace, now: now, timeZone: timeZone)
            details = task.details
            project = task.projectID.flatMap { workspace.projects[$0]?.name }
            subtasks = workspace.subtasks(of: id).map {
                TaskRowData($0, in: workspace, now: now, timeZone: timeZone)
            }
        }
    }

    package enum Action {
        case statusPicked(TaskStatus)
        case subtaskToggled(UUID)
        case saved(Result<Saved, any Error>)
        case alert(PresentationAction<Alert>)
        case delegate(Delegate)

        package enum Alert: Equatable {
            /// Mark Won't Do and close the open Sub tasks too.
            case closeSubtasksToo
            case undo(Change)
        }

        @CasePathable
        package enum Delegate {
            /// The records changed; lists showing them should refresh.
            case saved(Workspace)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .statusPicked(let status):
                return save { [id = state.id] in try await $0.setStatus(status, id, false) }

            case .subtaskToggled(let id):
                guard let subtask = state.subtasks.first(where: { $0.id == id }) else { return .none }
                let status: TaskStatus = subtask.status.isClosed ? .todo : .done
                return save { try await $0.setStatus(status, id, false) }

            case .alert(.presented(.closeSubtasksToo)):
                return save { [id = state.id] in try await $0.setStatus(.wontDo, id, true) }

            case .alert(.presented(.undo(let change))):
                return save { try await $0.undo(change) }

            case .saved(.success(let saved)):
                guard let refreshed = State(id: state.id, workspace: saved.workspace, now: now, timeZone: timeZone)
                else { return .send(.delegate(.saved(saved.workspace))) }
                state = refreshed
                if let parentID = saved.change.reopenedParent, let parent = saved.workspace.tasks[parentID] {
                    state.alert = .reopened(parent, change: saved.change)
                }
                return .send(.delegate(.saved(saved.workspace)))

            case .saved(.failure(LifrError.wontDoNeedsCascade(_, let open))):
                state.alert = .closeSubtasksToo(count: open.count)
                return .none

            case .saved(.failure(let error)):
                state.alert = .failed(error)
                return .none

            case .alert, .delegate:
                return .none
            }
        }
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

extension AlertState where Action == TaskDetailFeature.Action.Alert {
    static func closeSubtasksToo(count: Int) -> Self {
        AlertState {
            TextState("Close \(count) open Sub task(s) too?")
        } actions: {
            ButtonState(role: .destructive, action: .closeSubtasksToo) { TextState("Mark All Won't Do") }
            ButtonState(role: .cancel) { TextState("Cancel") }
        } message: {
            TextState("Marking this task Won't Do also marks its open Sub tasks Won't Do.")
        }
    }

    static func reopened(_ parent: LifrTask, change: Change) -> Self {
        AlertState {
            TextState("\"\(parent.title)\" is open again")
        } actions: {
            ButtonState(action: .undo(change)) { TextState("Undo") }
            ButtonState(role: .cancel) { TextState("OK") }
        } message: {
            TextState("A closed task can't have open Sub tasks, so it moved back to In Progress.")
        }
    }

    static func failed(_ error: any Error) -> Self {
        AlertState {
            TextState("Couldn't change the task")
        } actions: {
            ButtonState(role: .cancel) { TextState("OK") }
        } message: {
            TextState((error as? LifrError)?.description ?? error.localizedDescription)
        }
    }
}
