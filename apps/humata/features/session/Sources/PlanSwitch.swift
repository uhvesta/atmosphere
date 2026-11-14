import ChatUI
import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// Switching an ordinary session between full access and plan mode from
/// the composer: the daemon keeps the change and makes it from the next
/// turn, and notes it in the chat.
package enum PlanSwitch {
    /// Whether `session` can be switched: its backend takes its tools anew
    /// each turn, as `commands` say, and it's neither a side chat nor a
    /// planning session with the code, which only ever read.
    package static func offered(for session: Session, commands: ChatCommands?) -> Bool {
        guard commands?.plan == true, !session.archived, !session.hostOffline else { return false }
        return !session.readOnly && session.access != .sideChat && !session.followsBase
    }
}

@CasePathable
package enum PlanSwitchAction: Sendable {
    case toggled
    /// The daemon answered with the session as it is now.
    case changed(Session, asked: AccessLevel)
    case failed(String)
}

extension SessionFeature {
    func planSwitch(_ state: inout State, _ action: PlanSwitchAction) -> Effect<Action> {
        switch action {
        case .toggled:
            guard !state.planSwitching, PlanSwitch.offered(for: state.session, commands: state.chatCommands) else {
                return .none
            }
            let asked: AccessLevel = state.session.access == .plan ? .full : .plan
            state.planSwitching = true
            let session = state.session.id
            // Sent at once, also while a turn runs: what that turn asks to
            // change is refused from now, and its tools follow next turn.
            return .run { send in
                let updated = try await daemon.update(session, SessionUpdate(access: asked))
                await send(.plan(.changed(updated, asked: asked)))
            } catch: { error, send in
                await send(.plan(.failed(error.localizedDescription)))
            }

        case .changed(let updated, let asked):
            state.planSwitching = false
            guard updated.access == asked else {
                // A humatad from before plan mode takes the request and
                // leaves the session as it was.
                let message =
                    state.session.host.map {
                        "Plan mode wasn’t switched: humatad on \($0) is older than this app. Install Humata on the "
                            + "host again from Settings › Environments, then switch it again."
                    }
                    ?? "Plan mode wasn’t switched: the Humata daemon that is running is older than this app. Quit "
                    + "and reopen Humata once no turn is running, then switch it again."
                state.alert = Self.planAlert(message)
                return .none
            }
            if state.session.access != updated.access { state.session.access = updated.access }
            return .none

        case .failed(let message):
            state.planSwitching = false
            state.alert = Self.planAlert(message)
            return .none
        }
    }

    private static func planAlert(_ message: String) -> AlertState<Never> {
        AlertState {
            TextState("Couldn’t switch plan mode")
        } message: {
            TextState(message)
        }
    }
}

extension PlanSwitch {
    /// The composer's Plan toggle, where the session can be switched; nil
    /// where it can't, so nothing is offered that won't work.
    @MainActor
    static func toggle(_ store: StoreOf<SessionFeature>) -> PlanToggle? {
        guard offered(for: store.session, commands: store.chatCommands) else { return nil }
        return PlanToggle(isOn: store.session.access == .plan, busy: store.planSwitching) {
            store.send(.plan(.toggled))
        }
    }
}
