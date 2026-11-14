import ComposableArchitecture
import Foundation

/// The pane a Developer ID copy of Humata shows, in its window, when it's
/// opened from outside Applications: it installs itself there and opens
/// from there, opens the copy there if that's as new or newer, or runs
/// from where it is. The daemon isn't started until the person chooses.
@Reducer
package struct InstallerFeature {
    @ObservableState
    package struct State: Equatable {
        package var survey: InstallSurvey
        package var phase = Phase.offering

        package init(survey: InstallSurvey) {
            self.survey = survey
        }
    }

    package enum Phase: Equatable {
        /// Says what would happen, and waits for the person.
        case offering
        /// At work: copying, stopping the daemon, putting the copy in
        /// place, or opening it.
        case working(Step)
        /// Why it stopped, and what to do; the copy in Applications is as
        /// it was unless the failure says otherwise.
        case failed(String)
    }

    package enum Step: Equatable, Sendable {
        case copying
        case stoppingDaemon
        case replacing
        case opening
    }

    package enum Action {
        /// Installs this copy in Applications and opens it from there.
        case installTapped
        /// Opens the copy in Applications, as new as this one or newer.
        case openInstalledTapped
        /// Leaves Applications alone and opens this copy's window.
        case runFromHereTapped
        case stepped(Step)
        case failed(String)
    }

    @Dependency(\.installer) var installer
    @Dependency(\.daemonService) var service
    @Dependency(\.dismiss) var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .installTapped:
                guard !state.isWorking, case .install = state.survey.offer else { return .none }
                state.phase = .working(.copying)
                return install(state.survey)

            case .openInstalledTapped:
                guard !state.isWorking, case .openInstalled = state.survey.offer else { return .none }
                state.phase = .working(.opening)
                let destination = state.survey.destination
                return .run { _ in
                    try await installer.open(destination)
                    await installer.quit()
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .runFromHereTapped:
                guard !state.isWorking else { return .none }
                return .run { _ in await dismiss() }

            case .stepped(let step):
                state.phase = .working(step)
                return .none

            case .failed(let message):
                state.phase = .failed(message)
                return .none
            }
        }
    }

    /// Copies first, so a copy that fails leaves the daemon running and
    /// Applications as it was; only then stops the daemon, swaps the copy
    /// in, and opens it.
    private func install(_ survey: InstallSurvey) -> Effect<Action> {
        .run { send in
            let staged = try await installer.stage(survey.source, survey.destination)
            do {
                // Only a copy signed by this app's own team is trusted
                // without Gatekeeper asking again.
                if let team = survey.team, await installer.team(staged) == team {
                    await installer.clearQuarantine(staged)
                }
                await send(.stepped(.stoppingDaemon))
                try await service.stop()
                await send(.stepped(.replacing))
                try await installer.swap(staged, survey.destination)
            } catch {
                await installer.discard(staged)
                throw error
            }
            await send(.stepped(.opening))
            try await installer.open(survey.destination)
            await installer.quit()
        } catch: { error, send in
            await send(.failed(error.localizedDescription))
        }
    }
}

extension InstallerFeature.State {
    package var isWorking: Bool {
        if case .working = phase { return true }
        return false
    }
}
