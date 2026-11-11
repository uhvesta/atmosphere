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

    /// A day picked in one tap, counted from today.
    package enum Shortcut: CaseIterable, Hashable, Sendable {
        case today, tomorrow
        /// The coming Saturday, or today at the weekend.
        case thisWeekend
        /// The coming Monday.
        case nextWeek

        package var title: String {
            switch self {
            case .today: String(localized: "Today")
            case .tomorrow: String(localized: "Tomorrow")
            case .thisWeekend: String(localized: "This Weekend")
            case .nextWeek: String(localized: "Next Week")
            }
        }

        package var symbol: String {
            switch self {
            case .today: "star"
            case .tomorrow: "sunrise"
            case .thisWeekend: "sofa"
            case .nextWeek: "arrow.right.to.line"
            }
        }

        package func day(from today: CalendarDate) -> CalendarDate {
            // Weekdays run 1 for Sunday through 7 for Saturday.
            switch self {
            case .today: today
            case .tomorrow: today.adding(days: 1)
            case .thisWeekend: today.weekday == 1 ? today : today.adding(days: 7 - today.weekday)
            case .nextWeek: today.adding(days: (8 - today.weekday) % 7 + 1)
            }
        }
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case shortcutPicked(Shortcut)
        case clearTapped
        case saveTapped
        case cancelTapped
        case delegate(Delegate)

        @CasePathable
        package enum Delegate {
            /// The new due date, or `nil` for none.
            case picked(UUID, CalendarDate?)
        }
    }

    @Dependency(\.dismiss) private var dismiss
    @Dependency(\.timeZone) private var timeZone
    @Dependency(\.date.now) private var now

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .shortcutPicked(let shortcut):
                return pick(shortcut.day(from: CalendarDate(now, in: timeZone)), state)
            case .clearTapped:
                return pick(nil, state)
            case .saveTapped:
                return pick(CalendarDate(state.date, in: timeZone), state)
            case .cancelTapped:
                return .run { [dismiss] _ in await dismiss() }
            case .binding, .delegate:
                return .none
            }
        }
    }

    private func pick(_ day: CalendarDate?, _ state: State) -> Effect<Action> {
        .run { [id = state.id, dismiss] send in
            await send(.delegate(.picked(id, day)))
            await dismiss()
        }
    }
}
