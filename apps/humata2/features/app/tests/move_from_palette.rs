//! MV-1/MV-2 from the palette: picking a machine for a channel session
//! asks the daemon to move it, through the session's own Move, rather than
//! saying it can't. A refusal shows on the session; nothing is toasted as
//! done.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::Operation;
use humata2_clients::model::{MachineId, SessionId};
use humata2_clients::{Clients, Fakes, Workspace, fixture};
use humata2_palette::{Command, Delegate, PaletteAction};
use humata2_session::SessionAction;
use tca::{TestClock, TestStore};

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

/// An online machine other than the one `session` runs on.
fn elsewhere(workspace: &Workspace, session: &SessionId) -> MachineId {
    let from = workspace
        .session(session)
        .map(|s| s.machine.clone())
        .unwrap_or_else(|| panic!("the design workspace has {session}"));
    workspace
        .machines
        .iter()
        .find(|m| m.online && m.id != from)
        .map(|m| m.id.clone())
        .unwrap_or_else(|| panic!("another machine is online"))
}

fn opened_on_ledger() -> (TestStore<App>, Fakes, MachineId) {
    let workspace = fixture::design();
    let to = elsewhere(&workspace, &ledger());
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    // The host's workspace arrives as it does at launch, so every feature sees it.
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.send(A::Switch(Route::Session(ledger())), |_| {});
    store.settle();
    (store, fakes, to)
}

fn move_to(store: &mut TestStore<App>, to: &MachineId) {
    store.send(
        A::Palette(PaletteAction::Delegate(Delegate::Run(Command::MoveTo {
            session: ledger(),
            machine: to.clone(),
        }))),
        |_| {},
    );
    store.receive_matching(
        "the daemon's answer to the move",
        |a| matches!(a, A::Session(SessionAction::Moved(_))),
        |_| {},
    );
}

#[test]
fn moving_a_channel_session_from_the_palette_asks_the_daemon() {
    let (mut store, fakes, to) = opened_on_ledger();
    let toasts = store.state().toasts.len();
    move_to(&mut store, &to);
    let moved = fakes
        .daemon
        .workspace()
        .session(&ledger())
        .map(|s| s.machine.clone());
    assert_eq!(moved, Some(to), "the daemon moved the session");
    assert_eq!(store.state().toasts.len(), toasts, "no “can't move” toast");
    assert!(store.state().session.moving.is_none(), "the Move finished");
    store.skip_in_flight();
}

#[test]
fn a_refused_move_shows_why_on_the_session() {
    let (mut store, fakes, to) = opened_on_ledger();
    fakes
        .daemon
        .fail_next(Operation::MoveSession, "the machine is out of disk");
    move_to(&mut store, &to);
    assert!(
        store.state().session.error.is_some(),
        "the session says why the move failed"
    );
    store.skip_in_flight();
}
