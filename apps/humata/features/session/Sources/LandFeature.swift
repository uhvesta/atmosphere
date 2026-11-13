import ComposableArchitecture
import Foundation
import HumataDaemonClient

/// Landing a session: its work committed onto a new branch in each
/// repository it changed, and pushed or opened as pull requests on GitHub.
@Reducer
package struct LandFeature {
    @ObservableState
    package struct State: Equatable {
        package var sessionID: String
        package var branch: String
        /// Its first line titles the pull requests.
        package var message: String
        package var openPullRequests = true
        /// Only push, without pull requests.
        package var push = false
        package var landing = false
        /// The session's turn is running, so its work isn't done to land:
        /// what's committed could be half of an edit.
        package var turnRunning = false
        package var landed: Landed?
        package var error: String?

        package init(sessionID: String, title: String) {
            self.sessionID = sessionID
            branch = Self.branch(for: title)
            message = title
        }

        /// A branch name git takes, from the session's or task's title.
        package static func branch(for title: String) -> String {
            let words = title.lowercased().split { !$0.isLetter && !$0.isNumber }
            let name = words.prefix(6).joined(separator: "-")
            return name.isEmpty ? "humata-work" : name
        }

        package var canLand: Bool {
            !landing && landed == nil && cantLand == nil
        }

        /// Why landing can't start yet, and what to do about it; nil when
        /// it can, or while it lands.
        var cantLand: String? {
            if turnRunning {
                return String(localized: "The agent is still working. Wait for its turn to end, or stop it, then land.")
            }
            if branch.trimmingCharacters(in: .whitespaces).isEmpty {
                return String(localized: "Name the branch to land on.")
            }
            if message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                return String(localized: "Write the commit message.")
            }
            return nil
        }
    }

    package enum Action: BindableAction, Equatable {
        case binding(BindingAction<State>)
        case landTapped
        case landed(Landed)
        case failed(String)
        case doneTapped
    }

    @Dependency(\.daemon) var daemon
    @Dependency(\.dismiss) var dismiss

    package init() {}

    package var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding:
                return .none

            case .landTapped:
                guard state.canLand else { return .none }
                state.landing = true
                state.error = nil
                let session = state.sessionID
                let request = LandRequest(
                    branch: state.branch.trimmingCharacters(in: .whitespaces),
                    message: state.message.trimmingCharacters(in: .whitespacesAndNewlines),
                    push: state.push || state.openPullRequests, openPullRequests: state.openPullRequests)
                return .run { send in
                    await send(.landed(try await daemon.land(session, request)))
                } catch: { error, send in
                    await send(.failed(error.localizedDescription))
                }

            case .landed(let landed):
                state.landing = false
                state.landed = landed
                return .none

            case .failed(let message):
                state.landing = false
                state.error = message
                return .none

            case .doneTapped:
                return .run { _ in await dismiss() }
            }
        }
    }
}
