import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrUI

/// A label in Settings › Labels, with how many tasks have it.
package struct LabelRow: Identifiable, Hashable, Sendable {
    package var id: UUID
    package var name: String
    package var color: String
    package var taskCount: Int
}

extension Workspace {
    /// Every label by name, with how many tasks outside the trash have it.
    package var labelRows: [LabelRow] {
        let live = tasks.values.filter { $0.deletedAt == nil }
        return labels.values
            .sorted { ($0.name.localizedLowercase, $0.id.uuidString) < ($1.name.localizedLowercase, $1.id.uuidString) }
            .map { label in
                LabelRow(
                    id: label.id, name: label.name, color: label.color,
                    taskCount: live.count { $0.labelIDs.contains(label.id) })
            }
    }
}

/// Settings › Labels: every label, each opening a sheet to rename, merge or
/// delete it.
@Reducer
package struct LabelsFeature {
    @ObservableState
    package struct State: Equatable {
        package var rows: [LabelRow]
        @Presents package var edit: EditLabelFeature.State?

        package init(workspace: Workspace) {
            rows = workspace.labelRows
        }

        mutating func show(_ snapshot: Snapshot) {
            rows = snapshot.workspace.labelRows
            if let id = edit?.id, !rows.contains(where: { $0.id == id }) { edit = nil }
        }
    }

    package enum Action {
        case rowTapped(UUID)
        case show(Snapshot)
        case edit(PresentationAction<EditLabelFeature.Action>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Snapshot)
        }
    }

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .rowTapped(let id):
                guard let row = state.rows.first(where: { $0.id == id }) else { return .none }
                state.edit = EditLabelFeature.State(row: row, others: state.rows.filter { $0.id != id })
                return .none

            case .show(let snapshot):
                state.show(snapshot)
                return .none

            case .edit(.presented(.delegate(.saved(let snapshot)))):
                state.show(snapshot)
                return .send(.delegate(.saved(snapshot)))

            case .edit, .delegate:
                return .none
            }
        }
        .ifLet(\.$edit, action: \.edit) { EditLabelFeature() }
    }
}

/// One label: rename it, merge it into another, or delete it. Merging and
/// deleting ask first, saying how many tasks change.
@Reducer
package struct EditLabelFeature {
    @ObservableState
    package struct State: Equatable {
        package let id: UUID
        package let original: String
        package let originalColor: String
        package let taskCount: Int
        package var name: String
        package var color: String
        /// The labels this one can merge into.
        package let others: [LabelRow]
        /// Why the last save failed, such as a name another label has.
        package var error: String?
        @Presents package var confirm: AlertState<Confirm>?

        /// The colors to pick from, with the label's own first if it's
        /// not one of them.
        package var colorChoices: [String] {
            Avenue.colors.contains(originalColor) ? Avenue.colors : [originalColor] + Avenue.colors
        }

        package init(row: LabelRow, others: [LabelRow]) {
            id = row.id
            original = row.name
            originalColor = row.color
            taskCount = row.taskCount
            name = row.name
            color = row.color
            self.others = others
        }
    }

    package enum Confirm: Equatable, Sendable {
        case delete
        case merge(UUID)
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case saveTapped
        case cancelTapped
        case deleteTapped
        case mergeTapped(UUID)
        case confirm(PresentationAction<Confirm>)
        case saved(Result<Saved, any Error>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Snapshot)
        }
    }

    @Dependency(\.store) private var store
    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .saveTapped:
                let name = state.name.trimmingCharacters(in: .whitespacesAndNewlines)
                guard name != state.original || state.color != state.originalColor else {
                    return .run { _ in await dismiss() }
                }
                return .run { [store, id = state.id, color = state.color] send in
                    await send(.saved(Result { try await store.edit(label: id, name: name, color: color) }))
                }

            case .cancelTapped:
                return .run { _ in await dismiss() }

            case .deleteTapped:
                let name = state.original
                let count = state.taskCount
                state.confirm = AlertState {
                    TextState("Delete “\(name)”?")
                } actions: {
                    ButtonState(role: .destructive, action: .delete) { TextState("Delete") }
                    ButtonState(role: .cancel) { TextState("Cancel") }
                } message: {
                    TextState(Self.deleteMessage(count))
                }
                return .none

            case .mergeTapped(let other):
                guard let into = state.others.first(where: { $0.id == other }) else { return .none }
                let name = state.original
                let count = state.taskCount
                state.confirm = AlertState {
                    TextState("Merge “\(name)” into “\(into.name)”?")
                } actions: {
                    ButtonState(action: .merge(other)) { TextState("Merge") }
                    ButtonState(role: .cancel) { TextState("Cancel") }
                } message: {
                    TextState(Self.mergeMessage(count, name, into.name))
                }
                return .none

            case .confirm(.presented(.delete)):
                return .run { [store, id = state.id] send in
                    await send(.saved(Result { try await store.merge(label: id, into: nil) }))
                }

            case .confirm(.presented(.merge(let other))):
                return .run { [store, id = state.id] send in
                    await send(.saved(Result { try await store.merge(label: id, into: other) }))
                }

            case .saved(.success(let saved)):
                return .run { send in
                    await send(.delegate(.saved(saved.snapshot)))
                    await dismiss()
                }

            case .saved(.failure(let error)):
                state.error = (error as? LifrError)?.description ?? error.localizedDescription
                return .none

            case .binding(\.name):
                state.error = nil
                return .none

            case .binding, .confirm, .delegate:
                return .none
            }
        }
        .ifLet(\.$confirm, action: \.confirm)
    }

    private static func deleteMessage(_ count: Int) -> String {
        switch count {
        case 0: String(localized: "No task has it. To bring it back, revert this from Activity in Settings.")
        case 1: String(localized: "1 task loses it. To put it back, revert this from Activity in Settings.")
        default: String(localized: "\(count) tasks lose it. To put it back, revert this from Activity in Settings.")
        }
    }

    private static func mergeMessage(_ count: Int, _ name: String, _ into: String) -> String {
        String(
            localized: "Its \(count) tasks get “\(into)” instead and “\(name)” is deleted. Revert in Activity to undo."
        )
    }
}
