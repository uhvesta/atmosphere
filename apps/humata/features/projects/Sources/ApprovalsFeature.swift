import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Settings › Approvals: the grants that let other programs' admin actions
/// run without asking, each revocable, and what was asked lately.
@Reducer
package struct ApprovalsFeature {
    @ObservableState
    package struct State: Equatable {
        package var approvals = Approvals()
        package var loaded = false
        /// Why they couldn't be read or a grant revoked, and what to do.
        package var failure: String?
        /// When they were read, for how long each grant has left.
        package var now: Date?

        package init() {}
    }

    package enum Action {
        /// The tab appeared; reads them now and again every few seconds, as
        /// grants run out and actions run under them, until it goes.
        case task
        case loaded(Approvals)
        case loadFailed(String)
        case revokeTapped(ApprovalGrant.ID)
        case revokeFailed(String)
    }

    static let refreshInterval = Duration.seconds(5)

    @Dependency(\.daemon) var daemon
    @Dependency(\.continuousClock) var clock
    @Dependency(\.date) var date

    package init() {}

    package var body: some ReducerOf<Self> {
        Reduce { state, action in
            switch action {
            case .task:
                return .run { send in
                    while !Task.isCancelled {
                        await send(await load())
                        try await clock.sleep(for: Self.refreshInterval)
                    }
                }

            case .loaded(let approvals):
                let now = date.now.toTheMinute
                if state.now != now { state.now = now }
                if !state.loaded { state.loaded = true }
                if state.failure != nil { state.failure = nil }
                if state.approvals != approvals { state.approvals = approvals }
                return .none

            case .loadFailed(let message), .revokeFailed(let message):
                state.failure = message
                return .none

            case .revokeTapped(let id):
                // Gone at once; the daemon has stopped honouring it once this returns.
                state.approvals.grants.removeAll { $0.id == id }
                return .run { send in
                    try await daemon.revokeGrant(id)
                    await send(await load())
                } catch: { error, send in
                    // Read again first, so the reason stays shown.
                    await send(await load())
                    await send(.revokeFailed(error.localizedDescription))
                }
            }
        }
    }

    private func load() async -> Action {
        do {
            return .loaded(try await daemon.approvals())
        } catch {
            return .loadFailed(error.localizedDescription)
        }
    }
}
