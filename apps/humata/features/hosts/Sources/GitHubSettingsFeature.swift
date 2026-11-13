import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Settings › GitHub: whether this Mac reads open pull requests on its own
/// every few minutes, or only when a task's Refresh asks, and how many
/// minutes apart. Each change is saved at once; one that isn't goes back.
@Reducer
package struct GitHubSettingsFeature {
    @ObservableState
    package struct State: Equatable {
        /// Nil until first read.
        package var refresh: PullRefresh?
        /// A change is being saved.
        package var saving = false
        /// Custom… is picked, so the minutes are typed in.
        package var custom = false
        /// The minutes being typed, saved on Return.
        package var customMinutes = ""
        /// Why it couldn't be read or saved, and what to do about it.
        package var failure: String?

        package init(refresh: PullRefresh? = nil) {
            self.refresh = refresh
        }

        /// The minutes offered before Custom….
        package static let minuteChoices = [1, 2, 5, 10, 15, 30, 60]

        /// The interval picked: one offered, or Custom… for any other.
        package var every: Every {
            guard let minutes = refresh?.everyMinutes, !custom, Self.minuteChoices.contains(minutes) else {
                return .custom
            }
            return .minutes(minutes)
        }
    }

    package enum Every: Hashable, Sendable {
        case minutes(Int)
        case custom
    }

    package enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case loaded(PullRefresh)
        case loadFailed(String)
        case saved(PullRefresh)
        /// Not saved; the setting goes back to `was`.
        case saveFailed(was: PullRefresh, String)
        case automaticPicked(Bool)
        case everyPicked(Every)
        /// Return in the minutes field.
        case customMinutesSubmitted
    }

    @Dependency(\.daemon) var daemon

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding:
                return .none

            case .task:
                state.failure = nil
                return .run { send in
                    await send(.loaded(try await daemon.pullRefresh()))
                } catch: { error, send in
                    await send(.loadFailed(error.localizedDescription))
                }

            case .loaded(let refresh):
                state.refresh = refresh
                state.custom = !State.minuteChoices.contains(refresh.everyMinutes)
                state.customMinutes = "\(refresh.everyMinutes)"
                return .none

            case .loadFailed(let message):
                state.failure = String(
                    localized: """
                        Couldn't read how pull requests are followed: \(message) Check that Humata's daemon is \
                        running, then try again.
                        """)
                return .none

            case .saved(let refresh):
                state.saving = false
                state.failure = nil
                state.refresh = refresh
                state.customMinutes = "\(refresh.everyMinutes)"
                return .none

            case .saveFailed(let was, let message):
                state.saving = false
                state.refresh = was
                state.custom = !State.minuteChoices.contains(was.everyMinutes)
                state.customMinutes = "\(was.everyMinutes)"
                state.failure = String(
                    localized: """
                        Couldn't save the change, so it's back as it was: \(message) Check that Humata's daemon \
                        is running, then pick again.
                        """)
                return .none

            case .automaticPicked(let automatic):
                guard var refresh = state.refresh else { return .none }
                refresh.automatic = automatic
                return save(&state, refresh)

            case .everyPicked(.minutes(let minutes)):
                guard var refresh = state.refresh else { return .none }
                state.custom = false
                refresh.everyMinutes = minutes
                return save(&state, refresh)

            case .everyPicked(.custom):
                // Nothing changes until minutes are typed in.
                state.custom = true
                state.customMinutes = state.refresh.map { "\($0.everyMinutes)" } ?? ""
                return .none

            case .customMinutesSubmitted:
                guard var refresh = state.refresh else { return .none }
                let typed = state.customMinutes.trimmingCharacters(in: .whitespaces)
                guard let minutes = Int(typed), PullRefresh.minutes.contains(minutes) else {
                    state.failure = String(
                        localized: "Type a whole number of minutes from 1 to 1440 (a day); nothing was changed.")
                    return .none
                }
                refresh.everyMinutes = minutes
                return save(&state, refresh)
            }
        }
    }

    private func save(_ state: inout State, _ refresh: PullRefresh) -> Effect<Action> {
        guard !state.saving, let was = state.refresh, refresh != was else { return .none }
        state.saving = true
        state.failure = nil
        // Shown at once; put back if it isn't saved.
        state.refresh = refresh
        return .run { send in
            await send(.saved(try await daemon.setPullRefresh(refresh)))
        } catch: { error, send in
            await send(.saveFailed(was: was, error.localizedDescription))
        }
    }
}
