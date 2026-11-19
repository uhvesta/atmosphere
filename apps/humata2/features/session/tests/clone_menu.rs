//! The Info panel's clone rows (PL-10): Reveal in Finder, Open terminal here
//! and Copy path.
use humata2_clients::{Clients, Fakes, fixture, model::SessionId};
use humata2_session::{SessionAction as A, SessionState, Sessions};
use std::sync::Arc;
use tca::{TestClock, TestStore};

const PATH: &str = "/home/dev/Humata/pay/sessions/two/api";

fn store(open: &str) -> (TestStore<Sessions>, Fakes) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let state = SessionState {
        workspace: Arc::new(fakes.daemon.workspace()),
        open: Some(SessionId::new(open)),
        ..Default::default()
    };
    (
        TestStore::new(state, Sessions::new(clients)).non_exhaustive(),
        fakes,
    )
}

#[test]
fn the_menu_opens_on_one_clone_and_closes() {
    let (mut store, _) = store("ledger");
    store.send(A::CloneMenu(Some(PATH.to_owned())), |_| {});
    assert_eq!(store.state().clone_menu.as_deref(), Some(PATH));
    store.send(A::CloneMenu(None), |_| {});
    assert_eq!(store.state().clone_menu, None);
}

#[test]
fn copy_path_puts_the_clones_absolute_path_on_the_clipboard() {
    let (mut store, fakes) = store("ledger");
    store.send(A::CloneMenu(Some(PATH.to_owned())), |_| {});
    store.send(A::CopyClonePath(PATH.to_owned()), |_| {});
    store.skip_received();
    assert_eq!(fakes.clipboard.copied().as_deref(), Some(PATH));
    assert_eq!(store.state().clone_menu, None);
}

#[test]
fn reveal_asks_finder_for_the_clones_path() {
    let (mut store, fakes) = store("ledger");
    store.send(A::RevealClone("/Users/me/Humata/pay/sessions/one/api".to_owned()), |_| {});
    store.skip_received();
    assert_eq!(fakes.reveal.revealed(), ["/Users/me/Humata/pay/sessions/one/api"]);
}

#[test]
fn open_terminal_here_starts_a_new_shell_tab_in_the_clones_folder() {
    let (mut store, fakes) = store("ledger");
    let id = SessionId::new("ledger");
    let before = store.state().workspace.session(&id).map_or(0, |session| session.shells.len());
    store.send(
        A::OpenTerminalIn {
            session: id.clone(),
            folder: PATH.to_owned(),
        },
        |_| {},
    );
    store.receive_matching("the terminal opened", |action| matches!(action, A::TerminalOpened { result: Ok(_), .. }), |_| {});
    let after = store.state().workspace.session(&id).map_or(0, |session| session.shells.len());
    assert_eq!(after, before + 1);
    assert!(fakes.daemon.calls().iter().any(|call| call.contains(PATH)));
}

#[test]
fn a_session_the_daemon_does_not_know_shows_an_error_and_no_tab() {
    let (mut store, _) = store("ledger");
    let missing = SessionId::new("not-a-session");
    store.send(
        A::OpenTerminalIn {
            session: missing,
            folder: PATH.to_owned(),
        },
        |_| {},
    );
    store.receive_matching("the terminal failed", |action| matches!(action, A::TerminalOpened { result: Err(_), .. }), |_| {});
    assert!(store.state().error.is_some() || store.state().refused.is_some());
}
