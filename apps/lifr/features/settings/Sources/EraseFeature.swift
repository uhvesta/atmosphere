import ComposableArchitecture
import Foundation
import LifrStoreClient

/// Deletes every record once the person types the confirming word, so it
/// can't happen by a stray tap.
@Reducer
package struct EraseFeature {
    @ObservableState
    package struct State: Equatable {
        /// What the person typed so far.
        package var typed = ""
        package var isErasing = false
        /// Why the last try failed, until the next one.
        package var error: String?

        package init() {}

        /// The word to type, in the person's language.
        package static var word: String { String(localized: "Delete", comment: "The word typed to delete all data") }

        package var canErase: Bool {
            !isErasing
                && typed.trimmingCharacters(in: .whitespacesAndNewlines)
                    .compare(Self.word, options: [.caseInsensitive, .diacriticInsensitive]) == .orderedSame
        }
    }

    package enum Action: BindableAction, Equatable {
        case binding(BindingAction<State>)
        case eraseTapped
        case cancelTapped
        case erased(Result<Snapshot, EraseFailure>)
        case delegate(Delegate)

        @CasePathable
        package enum Delegate: Equatable {
            case erased(Snapshot)
        }
    }

    /// Why erasing failed, as the person reads it.
    package struct EraseFailure: Error, Equatable {
        package var reason: String
    }

    @Dependency(\.store) private var store
    @Dependency(\.dismiss) private var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .eraseTapped:
                guard state.canErase else { return .none }
                state.isErasing = true
                state.error = nil
                return .run { [store] send in
                    do {
                        await send(.erased(.success(try await store.eraseAll().snapshot)))
                    } catch {
                        await send(.erased(.failure(EraseFailure(reason: String(describing: error)))))
                    }
                }

            case .cancelTapped:
                return .run { [dismiss] _ in await dismiss() }

            case .erased(.success(let snapshot)):
                state.isErasing = false
                return .run { [dismiss] send in
                    await send(.delegate(.erased(snapshot)))
                    await dismiss()
                }

            case .erased(.failure(let failure)):
                state.isErasing = false
                state.error = failure.reason
                return .none

            case .binding, .delegate:
                return .none
            }
        }
    }
}
