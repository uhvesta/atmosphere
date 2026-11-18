//! The palette's "Continue in another channel…" (CV-8) opens the session
//! menu's dialog over the open session; on a Desk session it does nothing.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::model::{ChannelId, SessionId};
use humata2_clients::{Clients, Fakes, Workspace, fixture};
use humata2_palette::{Command, Delegate, PaletteAction};
use humata2_session::{Delegate as SessionDelegate, SessionAction};
use tca::{TestClock, TestStore};

fn on_session(workspace: Workspace, id: &SessionId) -> (TestStore<App>, Fakes) {
    let (clients, fakes) = Clients::fake(workspace, Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.send(A::Switch(Route::Session(id.clone())), |_| {});
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
fn the_palette_opens_the_continue_dialog_and_it_continues() {
    let ledger = SessionId::new("ledger");
    let (mut store, fakes) = on_session(fixture::design(), &ledger);
    run(&mut store, Command::ContinueInChannel);
    assert!(store.state().session.continuing.is_some(), "the dialog opened");
    assert!(store.state().dialog_holds_keys());
    store.send(
        A::Session(SessionAction::ContinuePick(ChannelId::new("web"))),
        |_| {},
    );
    store.send(A::Session(SessionAction::ConfirmContinue), |_| {});
    store.receive_matching(
        "the new session opens",
        |a| matches!(a, A::Session(SessionAction::Delegate(SessionDelegate::OpenSession(_)))),
        |_| {},
    );
    let Route::Session(opened) = store.state().route.clone() else {
        panic!("the app shows a session");
    };
    assert_ne!(opened, ledger);
    assert_eq!(
        fakes.daemon.workspace().session(&opened).map(|s| s.channel.clone()),
        Some(ChannelId::new("web"))
    );
    store.skip_in_flight();
}

#[test]
fn on_a_desk_session_the_command_does_nothing() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let (mut store, fakes) = on_session(workspace, &SessionId::new("desk-plan-q4"));
    run(&mut store, Command::ContinueInChannel);
    assert!(store.state().session.continuing.is_none());
    assert_eq!(fakes.daemon.last_continue(), None);
    store.skip_in_flight();
}
