import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskDetail

/// A task in the trash as its row shows it.
package struct TrashRow: Hashable, Identifiable, Sendable {
    package var id: UUID
    package var title: String
    package var code: String
    package var avenueColor: String?
    /// Sub tasks trashed with it, which come back with it.
    package var subtaskCount: Int
    /// Whole days until it's deleted for good, rounded up: 0 once it's due.
    package var daysLeft: Int

    package init(id: UUID, title: String, code: String, avenueColor: String?, subtaskCount: Int, daysLeft: Int) {
        self.id = id
        self.title = title
        self.code = code
        self.avenueColor = avenueColor
        self.subtaskCount = subtaskCount
        self.daysLeft = daysLeft
    }
}

extension Workspace {
    /// The trash, newest first.
    package func trashRows(now: Date) -> [TrashRow] {
        trash().map { trashed in
            let task = trashed.task
            return TrashRow(
                id: task.id, title: task.title, code: displayCode(of: task),
                avenueColor: avenues[task.avenueID]?.color,
                subtaskCount: tasks.values.count { $0.parentID == task.id && $0.deletedAt == task.deletedAt },
                daysLeft: max(0, Int((trashed.purgedAt.timeIntervalSince(now) / (24 * 60 * 60)).rounded(.up))))
        }
    }
}

/// Trashed tasks, each deleted for good 30 days after it was trashed, to
/// restore one or empty the lot.
@Reducer
package struct TrashFeature {
    @ObservableState
    package struct State: Equatable {
        package var rows: [TrashRow] = []
        /// The latest records shown. See `SaveOrder`.
        package var saves = SaveOrder()
        @Presents package var confirmation: ConfirmationDialogState<Confirmation>?
        @Presents package var alert: AlertState<Never>?

        package init() {}

        package init(workspace: Workspace, now: Date) {
            rows = workspace.trashRows(now: now)
        }

        mutating func show(_ snapshot: Snapshot, now: Date) {
            guard saves.isLatest(snapshot.number) else { return }
            rows = snapshot.workspace.trashRows(now: now)
        }
    }

    package enum Confirmation: Equatable, Sendable {
        case empty
    }

    package enum Action {
        /// The records changed elsewhere.
        case show(Snapshot)
        case restoreTapped(UUID)
        case emptyTapped
        case confirmation(PresentationAction<Confirmation>)
        case saved(Result<Saved, any Error>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            /// The trash changed the records; other screens should show them.
            case saved(Snapshot)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.date.now) private var now

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .show(let snapshot):
                state.show(snapshot, now: now)
                return .none

            case .restoreTapped(let id):
                return save { try await $0.restore(id) }

            case .emptyTapped:
                guard !state.rows.isEmpty else { return .none }
                let count = state.rows.count + state.rows.reduce(0) { $0 + $1.subtaskCount }
                let delete = AttributedString(localized: "Delete ^[\(count) Task](inflect: true)")
                state.confirmation = ConfirmationDialogState(titleVisibility: .visible) {
                    TextState("Empty Trash?")
                } actions: {
                    ButtonState(role: .destructive, action: .empty) {
                        TextState(String(delete.characters))
                    }
                    ButtonState(role: .cancel) { TextState("Cancel") }
                } message: {
                    TextState("They're deleted from all your devices, and this can't be undone.")
                }
                return .none

            case .confirmation(.presented(.empty)):
                return save { try await $0.emptyTrash() }

            case .saved(.success(let saved)):
                let isLatest = saved.number >= state.saves.latest
                state.show(saved.snapshot, now: now)
                return isLatest ? .send(.delegate(.saved(saved.snapshot))) : .none

            case .saved(.failure(let error)):
                state.alert = .failed("Couldn't change the trash", error)
                return .none

            case .confirmation, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$confirmation, action: \.confirmation)
        .ifLet(\.$alert, action: \.alert)
    }

    private func save(_ operation: @escaping @Sendable (StoreClient) async throws -> Saved) -> Effect<Action> {
        .run { [store] send in
            await send(.saved(Result { try await operation(store) }))
        }
    }
}
