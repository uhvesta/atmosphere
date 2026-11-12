import ComposableArchitecture
import HumataDaemonClient
import HumataProjects

/// What the person chose in an alert about an item: on closing it with
/// sessions, or deleting it.
package enum BoardAlert: Equatable {
    case stopAndArchive(Item.ID, ItemStatus)
    case keep(Item.ID, ItemStatus)
    case delete(Item.ID)
}

extension AlertState where Action == BoardAlert {
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

extension AlertState where Action == BoardAlert {
    /// Confirms deleting `item`.
    static func deleting(_ item: Item) -> Self {
        AlertState {
            TextState("Delete \(item.title)?")
        } actions: {
            ButtonState(role: .destructive, action: .delete(item.id)) {
                TextState("Delete")
            }
            ButtonState(role: .cancel) {
                TextState("Cancel")
            }
        } message: {
            TextState(
                "Its archived sessions move to the project. Board History can bring it back. "
                    + "One with subtasks or unarchived sessions can't be deleted until they're gone.")
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
