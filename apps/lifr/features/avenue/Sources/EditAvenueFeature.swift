import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskDetail

/// Changing an Avenue's name, color and symbol, or archiving or restoring
/// it. Archiving an Avenue with open tasks asks first, since they close as
/// Won't Do.
@Reducer
package struct EditAvenueFeature {
    /// The colors an Avenue can have, by stored name.
    package static let colors = Avenue.colors
    /// The symbols an Avenue can have.
    package static let symbols = [
        "circle", "heart", "briefcase", "house", "book", "dumbbell", "leaf", "dollarsign.circle", "person.2",
        "paintbrush", "airplane", "graduationcap", "cart", "gamecontroller", "music.note", "star",
    ]

    @ObservableState
    package struct State: Equatable {
        package let id: UUID
        /// The latest save shown, so an earlier one's late reply is ignored.
        package var saves = SaveOrder()
        package var name: String
        package var color: String
        package var symbol: String
        package var isArchived: Bool
        /// The Inbox can't be archived, so it isn't offered.
        package var isInbox: Bool
        /// Open top-level tasks, which archiving closes.
        package var openCount: Int
        @Presents package var confirmArchive: AlertState<Action.ConfirmArchive>?
        @Presents package var alert: AlertState<Never>?

        /// `nil` when the Avenue isn't in `workspace`.
        package init?(id: UUID, workspace: Workspace) {
            guard let avenue = workspace.avenues[id] else { return nil }
            self.id = id
            name = avenue.name
            color = avenue.color
            symbol = avenue.symbol
            isArchived = avenue.archivedAt != nil
            isInbox = avenue.isInbox
            openCount = workspace.tasks.values.count {
                $0.avenueID == id && $0.deletedAt == nil && $0.isOpen && !$0.isSubtask
            }
        }

        /// The symbols to offer: the usual ones, and the Avenue's own if it
        /// was set elsewhere, such as by the assistant.
        package var symbolChoices: [String] { Self.including(symbol, in: EditAvenueFeature.symbols) }

        package var colorChoices: [String] { Self.including(color, in: EditAvenueFeature.colors) }

        private static func including(_ value: String, in choices: [String]) -> [String] {
            choices.contains(value) ? choices : [value] + choices
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case saveTapped
        case cancelTapped
        case archiveTapped
        case restoreTapped
        case confirmArchive(PresentationAction<ConfirmArchive>)
        case saved(Result<Saved, any Error>)
        case alert(PresentationAction<Never>)
        case delegate(Delegate)

        package enum ConfirmArchive: Equatable { case closeOpenTasks }

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
                let edit = Workspace.AvenueEdit(
                    name: state.name.trimmingCharacters(in: .whitespacesAndNewlines), color: state.color,
                    symbol: state.symbol)
                return save { [id = state.id] in try await $0.edit(avenue: id, edit) }

            case .cancelTapped:
                return .run { [dismiss] _ in await dismiss() }

            case .archiveTapped:
                guard state.openCount > 0 else {
                    return save { [id = state.id] in try await $0.archive(avenue: id, openTasks: nil) }
                }
                let open = counted(state.openCount, "open task", "open tasks")
                state.confirmArchive = AlertState {
                    TextState("Archive \(state.name)?")
                } actions: {
                    ButtonState(role: .destructive, action: .closeOpenTasks) {
                        TextState("Close \(open) as Won't Do and Archive")
                    }
                    ButtonState(role: .cancel) { TextState("Cancel") }
                } message: {
                    TextState(
                        "It still has \(open). To keep them, move them to another Avenue first, then archive.")
                }
                return .none

            case .confirmArchive(.presented(.closeOpenTasks)):
                return save { [id = state.id] in try await $0.archive(avenue: id, openTasks: .closeAsWontDo) }

            case .restoreTapped:
                return save { [id = state.id] in try await $0.restore(avenue: id) }

            case .saved(.success(let saved)):
                let isLatest = state.saves.isLatest(saved)
                return .run { [dismiss] send in
                    if isLatest {
                        await send(.delegate(.saved(saved.snapshot)))
                    }
                    await dismiss()
                }

            case .saved(.failure(let error)):
                state.alert = .failed("Couldn't change the Avenue", error)
                return .none

            case .binding, .confirmArchive, .alert, .delegate:
                return .none
            }
        }
        .ifLet(\.$confirmArchive, action: \.confirmArchive)
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
