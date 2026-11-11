import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient

/// Names a search to keep it, or renames, re-symbols or deletes a saved
/// one. Deleting asks first.
@Reducer
package struct SavedFilterEditor {
    @ObservableState
    package struct State: Equatable {
        /// `nil` while saving a new one.
        package let id: UUID?
        package let filter: TaskFilter
        package var name: String
        package var symbol: String
        /// Why the last save failed.
        package var error: String?
        @Presents package var confirm: AlertState<Confirm>?

        /// Saving `filter` as a new saved filter.
        package init(saving filter: TaskFilter) {
            id = nil
            self.filter = filter
            name = filter.text.trimmingCharacters(in: .whitespacesAndNewlines)
            symbol = SavedFilter.defaultSymbol
        }

        /// Editing `saved`.
        package init(editing saved: SavedFilter) {
            id = saved.id
            filter = saved.filter
            name = saved.name
            symbol = saved.symbol
        }

        /// Whether this saves a new saved filter rather than editing one.
        package var isNew: Bool { id == nil }

        package var canSave: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    package enum Confirm: Equatable, Sendable {
        case delete
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case saveTapped
        case cancelTapped
        case deleteTapped
        case confirm(PresentationAction<Confirm>)
        case saved(Result<Saved, any Error>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case saved(Snapshot)
            /// The saved filter is gone.
            case deleted(Snapshot)
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
                guard state.canSave else { return .none }
                let (name, symbol, filter) = (state.name, state.symbol, state.filter)
                return .run { [store, id = state.id] send in
                    await send(
                        .saved(
                            Result {
                                if let id {
                                    try await store.edit(savedFilter: id, name: name, symbol: symbol, filter: filter)
                                } else {
                                    try await store.saveFilter(filter, name: name, symbol: symbol)
                                }
                            }))
                }

            case .cancelTapped:
                return .run { _ in await dismiss() }

            case .deleteTapped:
                let name = state.name
                state.confirm = AlertState {
                    TextState("Delete “\(name)”?")
                } actions: {
                    ButtonState(role: .destructive, action: .delete) { TextState("Delete") }
                    ButtonState(role: .cancel) { TextState("Cancel") }
                } message: {
                    TextState("Your tasks stay as they are. Activity in Settings can bring the filter back.")
                }
                return .none

            case .confirm(.presented(.delete)):
                guard let id = state.id else { return .none }
                return .run { [store] send in
                    do {
                        let saved = try await store.delete(savedFilter: id)
                        await send(.delegate(.deleted(saved.snapshot)))
                        await dismiss()
                    } catch {
                        await send(.saved(.failure(error)))
                    }
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
}
