import ComposableArchitecture
import Foundation
import HumataDaemonClient

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
        /// Whether installing stopped Humata's daemon and left it stopped.
        /// The copy in Applications then starts its own when it opens, and
        /// this one isn't run over its login item.
        package var daemonStopped = false

        package init(survey: InstallSurvey) {
            self.survey = survey
        }
    }

    package enum Phase: Equatable {
        /// Says what would happen, and waits for the person.
        case offering
        /// Before anything is copied: `others` are open and would be quit,
        /// or, when `busy`, sessions on this Mac are in a turn that
        /// stopping the daemon ends. Waits for the person to go on or not.
        case confirming(others: [RunningCopy], busy: Bool)
        /// At work: checking, copying, quitting other copies, stopping the
        /// daemon, putting the copy in place, or opening it.
        case working(Step)
        /// Why it stopped, and what to do; the copy in Applications is as
        /// it was unless the failure says otherwise.
        case failed(String)
    }

    package enum Step: Equatable, Sendable {
        case checking
        case copying
        case quittingOthers
        case stoppingDaemon
        case replacing
        case opening
    }

    package enum Action {
        /// Installs this copy in Applications and opens it from there.
        case installTapped
        /// What installing would interrupt: other open copies, and whether
        /// a session on this Mac is in a turn.
        case checked(others: [RunningCopy], busy: Bool)
        /// Goes on with the install, quitting the other copies and ending
        /// the turns it was asked about.
        case confirmTapped
        /// Doesn't install, to wait for the turns or keep the other copies.
        case cancelTapped
        /// Opens the copy in Applications.
        case openInstalledTapped
        /// Leaves Applications alone and opens this copy's window.
        case runFromHereTapped
        /// Quits this copy, where running it from here isn't offered.
        case quitTapped
        case stepped(Step)
        /// Whether the daemon is stopped now, by the install.
        case daemonStopped(Bool)
        /// This copy is in Applications now.
        case swapped
        case failed(String)
    }

    @Dependency(\.installer) var installer
    @Dependency(\.daemonService) var service
    @Dependency(\.daemon) var daemon
    @Dependency(\.dismiss) var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .installTapped:
                guard !state.isWorking, state.offersInstall else { return .none }
                state.phase = .working(.checking)
                return .run { send in
                    let others = await installer.otherCopies()
                    // No daemon answering has no turns to end.
                    let sessions = (try? await daemon.sessions(nil)) ?? []
                    await send(.checked(others: others, busy: AppFeature.anyBusy(sessions)))
                }

            case .checked(let others, let busy):
                guard state.phase == .working(.checking) else { return .none }
                guard others.isEmpty, !busy else {
                    state.phase = .confirming(others: others, busy: busy)
                    return .none
                }
                state.phase = .working(.copying)
                return install(state.survey, quitting: [])

            case .confirmTapped:
                guard case .confirming(let others, _) = state.phase else { return .none }
                state.phase = .working(.copying)
                return install(state.survey, quitting: others)

            case .cancelTapped:
                guard case .confirming = state.phase else { return .none }
                state.phase = .offering
                return .none

            case .openInstalledTapped:
                guard !state.isWorking, state.opensInstalled else { return .none }
                state.phase = .working(.opening)
                let destination = state.survey.destination
                return .run { _ in
                    try await installer.open(destination)
                    await installer.quit()
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .runFromHereTapped:
                guard !state.isWorking, state.offersRunFromHere else { return .none }
                return .run { _ in await dismiss() }

            case .quitTapped:
                guard !state.isWorking else { return .none }
                return .run { _ in await installer.quit() }

            case .stepped(let step):
                state.phase = .working(step)
                return .none

            case .daemonStopped(let stopped):
                state.daemonStopped = stopped
                return .none

            case .swapped:
                state.survey.installed = state.survey.version
                return .none

            case .failed(let message):
                state.phase = .failed(message)
                return .none
            }
        }
    }

    /// Copies first, so a copy that fails leaves the daemon running and
    /// Applications as it was; only then quits `others`, stops the daemon,
    /// swaps the copy in, and opens it. A failure once the daemon is
    /// stopped says so; with nothing in Applications, the daemon that ran
    /// is started again, while one installed starts its own when opened.
    private func install(_ survey: InstallSurvey, quitting others: [RunningCopy]) -> Effect<Action> {
        .run { send in
            let staged = try await installer.stage(survey.source, survey.destination)
            var stopped = false
            do {
                // Only a copy signed by this app's own team is trusted
                // without Gatekeeper asking again.
                if let team = survey.team, await installer.team(staged) == team {
                    await installer.clearQuarantine(staged)
                }
                if !others.isEmpty {
                    await send(.stepped(.quittingOthers))
                    try await installer.quitCopies(others)
                }
                await send(.stepped(.stoppingDaemon))
                stopped = try await service.stop()
                if stopped { await send(.daemonStopped(true)) }
                await send(.stepped(.replacing))
                try await installer.swap(staged, survey.destination)
            } catch {
                await installer.discard(staged)
                guard stopped else { throw error }
                // Never this copy's daemon over an installed copy's login
                // item: that copy, opened, starts its own.
                guard survey.installed == nil else {
                    throw InstallerError.stoppedDaemon(restarted: false, underlying: error)
                }
                let restarted = (try? await service.start()) != nil
                if restarted { await send(.daemonStopped(false)) }
                throw InstallerError.stoppedDaemon(restarted: restarted, underlying: error)
            }
            await send(.swapped)
            await send(.stepped(.opening))
            do {
                try await installer.open(survey.destination)
            } catch {
                guard stopped else { throw error }
                throw InstallerError.stoppedDaemon(restarted: false, underlying: error)
            }
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

    /// Whether this copy can be installed: there's nothing in Applications,
    /// or an older copy.
    package var offersInstall: Bool {
        if case .install = survey.offer { return true }
        return false
    }

    /// Whether the copy in Applications is offered to open: it's as new as
    /// this one or newer, or is this one now, or the install stopped the
    /// daemon and left that copy there, which starts its own when opened.
    package var opensInstalled: Bool {
        if case .openInstalled = survey.offer { return true }
        return daemonStopped && survey.installed != nil
    }

    /// Whether running this copy from where it is is offered. Not beside a
    /// newer copy in Applications, whose daemon this one's would replace
    /// with one that can't open its store; nor once the install stopped the
    /// daemon of a copy that's there, whose login item it would take.
    package var offersRunFromHere: Bool {
        guard let installed = survey.installed else { return true }
        return installed <= survey.version && !daemonStopped
    }
}
