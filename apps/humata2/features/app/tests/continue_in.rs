//! Continue in another channel… (CV-8) through the whole app: the menu
//! item, Escape, the keys the dialog holds, and the new session opening.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::model::{ChannelId, SessionId};
use humata2_clients::{Clients, Fakes, Workspace, fixture};
use humata2_session::{Delegate, MenuItem, SessionAction as S, SessionView};
use tca::{TestClock, TestStore};

/// An app on `workspace` showing session `id`, the daemon holding the same
/// workspace.
fn on_session(workspace: Workspace, id: &SessionId) -> (TestStore<App>, Fakes) {
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let workspace = Arc::new(workspace);
    let mut state = AppState::launch(&clients);
    state.workspace = workspace.clone();
    state.desk.workspace = workspace.clone();
    state.session.workspace = workspace;
    state.session.open = Some(id.clone());
    state.session.views.insert(id.clone(), SessionView::default());
    state.route = Route::Session(id.clone());
    (TestStore::new(state, App::new(clients)).non_exhaustive(), fakes)
}

#[test]
fn continuing_opens_the_new_session_in_the_other_channel() {
    let ledger = SessionId::new("ledger");
    let (mut store, fakes) = on_session(fixture::design(), &ledger);
    store.send(A::Session(S::MenuPick(MenuItem::ContinueInChannel)), |_| {});
    assert!(store.state().dialog_holds_keys(), "the dialog keeps the keys from the terminal");
    store.send(A::Session(S::ContinuePick(ChannelId::new("web"))), |_| {});
    store.send(A::Session(S::ConfirmContinue), |_| {});
    store.receive_matching(
        "the new session opens",
        |a| matches!(a, A::Session(S::Delegate(Delegate::OpenSession(_)))),
        |_| {},
    );
    let Route::Session(opened) = store.state().route.clone() else {
        panic!("the app shows a session");
    };
    assert_ne!(opened, ledger);
    let workspace = fakes.daemon.workspace();
    let new = workspace.session(&opened).expect("the daemon holds it");
    assert_eq!(new.channel, ChannelId::new("web"));
    assert_eq!(
        workspace.session(&ledger).map(|s| s.channel.clone()),
        Some(ChannelId::new("payments-rewrite")),
        "the session it came from stays in its channel"
    );
    assert!(!store.state().dialog_holds_keys());
}

#[test]
fn escape_closes_the_continue_dialog() {
    let id = SessionId::new("ledger");
    let (mut store, _) = on_session(fixture::design(), &id);
    store.send(A::Session(S::StartContinue), |_| {});
    assert!(store.state().session.continuing.is_some());
    store.send(A::Escape, |_| {});
    assert!(store.state().session.continuing.is_none());
    assert_eq!(store.state().route, Route::Session(id));
}

#[test]
fn a_desk_session_cant_be_continued_in_a_channel() {
    let mut workspace = fixture::design();
    let id = SessionId::new("desk-plan-q4");
    fixture::desk(&mut workspace);
    let (mut store, fakes) = on_session(workspace, &id);
    store.send(A::Session(S::MenuPick(MenuItem::ContinueInChannel)), |_| {});
    store.send(A::Session(S::StartContinue), |_| {});
    assert!(store.state().session.continuing.is_none());
    assert_eq!(fakes.daemon.last_continue(), None);
}
