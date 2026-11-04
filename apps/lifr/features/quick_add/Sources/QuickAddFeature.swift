import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskDetail

/// Adds a task in a few taps: a title, an Avenue (the last one used), and
/// optionally a Project, a due day and a priority. In Sub task mode the
/// first task added stays open and the next ones become its Sub tasks.
@Reducer
package struct QuickAddFeature {
    /// When the new task is due.
    package enum Due: String, CaseIterable, Hashable, Sendable {
        case none, today, tomorrow, pick

        package var title: String {
            switch self {
            case .none: "None"
            case .today: "Today"
            case .tomorrow: "Tomorrow"
            case .pick: "Pick"
            }
        }
    }

    package struct Choice: Identifiable, Hashable, Sendable {
        package var id: UUID
        package var name: String

        package init(id: UUID, name: String) {
            self.id = id
            self.name = name
        }
    }

    @ObservableState
    package struct State: Equatable {
        package var title = ""
        package var avenueID: UUID?
        package var projectID: UUID?
        package var due = Due.none
        package var pickedDate: Date
        package var priority = Priority.none
        package var subtaskMode = false
        /// The task Sub tasks are added to, once Sub task mode added it.
        package var parent: Choice?
        /// Titles added under `parent` so far.
        package var addedSubtasks: [String] = []
        package var error: String?
        package var avenues: [Choice]
        package var projects: [UUID: [Choice]]
        /// The Avenue last added to, on this device.
        @Shared(.appStorage("lifrQuickAddLastAvenue")) package var lastAvenue = ""

        package init(workspace: Workspace, now: Date) {
            pickedDate = now
            avenues = workspace.avenues.values
                .filter { $0.archivedAt == nil }
                .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) }
                .map { Choice(id: $0.id, name: $0.name) }
            projects = Dictionary(
                grouping: workspace.projects.values
                    .filter { $0.status == .active }
                    .sorted { ($0.sortOrder, $0.id.uuidString) < ($1.sortOrder, $1.id.uuidString) },
                by: \.avenueID
            ).mapValues { $0.map { Choice(id: $0.id, name: $0.name) } }
            let last = UUID(uuidString: lastAvenue)
            avenueID = avenues.first { $0.id == last }?.id ?? avenues.first?.id
        }

        /// Projects in the picked Avenue.
        package var avenueProjects: [Choice] {
            avenueID.flatMap { projects[$0] } ?? []
        }

        package var canAdd: Bool {
            !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && (parent != nil || avenueID != nil)
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case addTapped
        case doneTapped
        case cancelTapped
        case added(Result<Saved, any Error>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Workspace)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.dismiss) private var dismiss
    @Dependency(\.date.now) private var now
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
            .onChange(of: \.avenueID) { _, _ in
                Reduce { state, _ in
                    if let project = state.projectID, !state.avenueProjects.contains(where: { $0.id == project }) {
                        state.projectID = nil
                    }
                    return .none
                }
            }
        Reduce { state, action in
            switch action {
            case .addTapped:
                guard state.canAdd else { return .none }
                var new = Workspace.NewTask(title: state.title.trimmingCharacters(in: .whitespacesAndNewlines))
                if let parent = state.parent {
                    new.parentID = parent.id
                } else {
                    new.avenueID = state.avenueID
                    new.projectID = state.projectID
                    new.priority = state.priority
                    new.dueDate = dueDate(state)
                }
                state.error = nil
                return .run { [store, new] send in
                    await send(.added(Result { try await store.createTask(new) }))
                }

            case .added(.success(let saved)):
                let created = saved.change.previous.first { $0.value == nil }.flatMap { saved.workspace.tasks[$0.key] }
                if state.parent != nil {
                    state.addedSubtasks.append(state.title.trimmingCharacters(in: .whitespacesAndNewlines))
                    state.title = ""
                    return .send(.delegate(.saved(saved.workspace)))
                }
                if let avenueID = state.avenueID {
                    state.$lastAvenue.withLock { $0 = avenueID.uuidString }
                }
                if state.subtaskMode, let created {
                    state.parent = Choice(id: created.id, name: created.title)
                    state.title = ""
                    return .send(.delegate(.saved(saved.workspace)))
                }
                return .run { [dismiss] send in
                    await send(.delegate(.saved(saved.workspace)))
                    await dismiss()
                }

            case .added(.failure(let error)):
                state.error = (error as? LifrError)?.description ?? error.localizedDescription
                return .none

            case .doneTapped, .cancelTapped:
                return .run { [dismiss] _ in await dismiss() }

            case .binding, .delegate:
                return .none
            }
        }
    }

    private func dueDate(_ state: State) -> CalendarDate? {
        switch state.due {
        case .none: nil
        case .today: CalendarDate(now, in: timeZone)
        case .tomorrow: CalendarDate(now + 24 * 60 * 60, in: timeZone)
        case .pick: CalendarDate(state.pickedDate, in: timeZone)
        }
    }
}
