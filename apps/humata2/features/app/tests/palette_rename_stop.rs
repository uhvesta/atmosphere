//! The palette's Rename and Stop the agent act on the open session (SE-7,
//! FC-17): Rename opens the dialog on its title, Stop ends its agent.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::model::{SessionId, Status};
use humata2_clients::{Clients, Fakes, fixture};
use humata2_palette::{Command, Delegate, PaletteAction};
use humata2_session::SessionAction;
use tca::{TestClock, TestStore};

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

fn on_ledger() -> (TestStore<App>, Fakes) {
    let (clients, fakes) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.send(A::Switch(Route::Session(ledger())), |_| {});
    store.settle();
    (store, fakes)
}

fn run(store: &mut TestStore<App>, command: Command) {
    store.send(
        A::Palette(PaletteAction::Delegate(Delegate::Run(command))),
        |_| {},
    );
}

#[test]
fn rename_opens_the_dialog_on_the_sessions_title() {
    let (mut store, fakes) = on_ledger();
    run(&mut store, Command::RenameSession);
    let title = fakes
        .daemon
        .workspace()
        .session(&ledger())
        .map(|s| s.title.clone())
        .expect("the daemon has the session");
    let draft = store.state().session.renaming.clone().expect("the dialog opened");
    assert_eq!(draft.title, title);
}

#[test]
fn stop_ends_the_agent_and_keeps_the_session() {
    let (mut store, fakes) = on_ledger();
    run(&mut store, Command::StopAgent);
    store.receive_matching(
        "the daemon's answer",
        |a| matches!(a, A::Session(SessionAction::Done(Ok(())))),
        |_| {},
    );
    let session = fakes
        .daemon
        .workspace()
        .session(&ledger())
        .cloned()
        .expect("the session stays");
    assert_eq!(session.status, Status::AgentStopped);
    assert!(!session.archived);
}
