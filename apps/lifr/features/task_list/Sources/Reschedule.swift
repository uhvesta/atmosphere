import ComposableArchitecture
import Foundation
import LifrCore

/// Picking a new due date for one task.
@Reducer
package struct RescheduleFeature {
    @ObservableState
    package struct State: Equatable {
        package var id: UUID
        package var title: String
        package var date: Date

        package init(id: UUID, title: String, date: Date) {
            self.id = id
            self.title = title
            self.date = date
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case saveTapped
        case cancelTapped
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            case picked(UUID, CalendarDate)
        }
    }

    @Dependency(\.dismiss) private var dismiss
    @Dependency(\.timeZone) private var timeZone

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .saveTapped:
                let picked = CalendarDate(state.date, in: timeZone)
                return .run { [id = state.id, dismiss] send in
                    await send(.delegate(.picked(id, picked)))
                    await dismiss()
                }
            case .cancelTapped:
                return .run { [dismiss] _ in await dismiss() }
            case .binding, .delegate:
                return .none
            }
        }
    }
}
