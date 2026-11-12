import ComposableArchitecture
import HumataDaemonClient
import HumataProjects

/// What the person chose on closing an item that still has sessions.
package enum CloseAlert: Equatable {
    case stopAndArchive(Item.ID, ItemStatus)
    case keep(Item.ID, ItemStatus)
}

extension AlertState where Action == CloseAlert {
    /// Asks whether to stop and archive the `sessions` unarchived sessions on
    /// `item` as it moves to `status`, a closed one.
    static func closing(_ item: Item, to status: ItemStatus, sessions: Int) -> Self {
        AlertState {
            TextState("Move \(item.title) to \(status.title)?")
        } actions: {
            ButtonState(action: .stopAndArchive(item.id, status)) {
                TextState(sessions == 1 ? "Stop and Archive 1 Session" : "Stop and Archive \(sessions) Sessions")
            }
            ButtonState(action: .keep(item.id, status)) {
                TextState("Keep Them")
            }
            ButtonState(role: .cancel) {
                TextState("Cancel")
            }
        } message: {
            TextState(
                "Its sessions keep running and asking for approval until they're archived. "
                    + "An archived session can be unarchived later.")
        }
    }
}

/// Stops each of `sessions` in a turn, then archives them all.
func stopAndArchive(_ sessions: [Session], with daemon: DaemonClient) async throws {
    for session in sessions where session.state == .working || session.state == .waitingForApproval {
        try await daemon.abort(session.id)
    }
    for session in sessions {
        _ = try await daemon.update(session.id, SessionUpdate(archived: true))
    }
}
