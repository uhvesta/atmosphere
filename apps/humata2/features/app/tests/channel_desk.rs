//! A channel's Desk through the whole app (CD-2, CD-4): the sidebar's Desk
//! row opens the Desk tab, and closing a Desk session from its own page
//! asks first and goes back to that tab.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route, channel};
use humata2_clients::model::{ChannelId, SessionId};
use humata2_clients::{Clients, Fakes, Workspace, fixture};
use humata2_desk::DeskAction as D;
use tca::{TestClock, TestStore};

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

fn launch() -> (TestStore<App>, Fakes) {
    let mut workspace: Workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    let (clients, fakes) = humata2_clients::fake_clients(workspace, Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.settle();
    (store, fakes)
}

#[test]
fn the_sidebars_desk_row_opens_the_channel_on_its_desk_tab() {
    let (mut store, _fakes) = launch();
    let rows = store
        .state()
        .sidebar
        .channel_desks
        .get(&payments())
        .map(Vec::len);
    assert_eq!(rows, Some(2), "the sidebar lists the channel's open Desk sessions");

    store.send(A::OpenChannelDesk(payments()), |_| {});
    assert_eq!(store.state().route, Route::Channel(payments()));
    assert_eq!(
        store.state().channel_pages.get(&payments()).map(|view| view.tab),
        Some(channel::Tab::Desk)
    );
    assert_eq!(store.state().desk.channel_rows(&payments()).len(), 2);
    assert!(
        store.state().workspace.sessions.iter().filter(|session| session.channel == payments()).all(
            |session| !store.state().desk.channel_rows(&payments()).iter().any(|row| row.session.id == session.id)
        ),
        "Desk sessions are not in the Sessions tree"
    );
}

#[test]
fn closing_from_its_own_page_asks_first_and_returns_to_the_desk_tab() {
    let (mut store, fakes) = launch();
    let id = SessionId::new("cdesk-triage");
    store.send(A::Switch(Route::Session(id.clone())), |_| {});
    store.send(A::Desk(D::AskClose(id.clone())), |_| {});
    assert!(store.state().desk.channel_close.is_some(), "it asks first");
    assert!(
        fakes.daemon.workspace().desk.channel_sessions.contains_key(&id),
        "nothing closed yet"
    );

    store.send(A::Desk(D::ConfirmCloseChannel), |_| {});
    store.receive_matching(
        "the close finished",
        |a| matches!(a, A::Desk(D::Changed { result: Ok(()), .. })),
        |_| {},
    );
    // The reload it asks for, and the sidebar that follows it.
    for _ in 0..8 {
        store.skip_received();
    }
    assert!(!fakes.daemon.workspace().desk.channel_sessions.contains_key(&id));
    assert_eq!(store.state().route, Route::Channel(payments()));
    assert_eq!(
        store.state().channel_pages.get(&payments()).map(|view| view.tab),
        Some(channel::Tab::Desk)
    );
    assert_eq!(
        store.state().sidebar.channel_desks.get(&payments()).map(Vec::len),
        Some(1),
        "the sidebar row follows"
    );
}

#[test]
fn a_sessions_menu_opens_the_new_desk_dialog_with_that_session_as_the_target() {
    let (mut store, _fakes) = launch();
    let ledger = SessionId::new("ledger");
    store.send(A::NewDeskSessionFor(ledger.clone()), |_| {});
    let Some(form) = store.state().desk.channel_new.clone() else {
        panic!("the dialog opens")
    };
    assert_eq!(form.channel, payments());
    assert!(form.on_session);
    assert_eq!(form.target(), Some(&ledger));
}

#[test]
fn a_desk_session_has_no_new_desk_session_for_itself() {
    let (mut store, _fakes) = launch();
    store.send(A::NewDeskSessionFor(SessionId::new("cdesk-triage")), |_| {});
    assert!(store.state().desk.channel_new.is_none());
}

#[test]
fn the_rule_link_opens_the_channels_agent_access_from_any_page() {
    let (mut store, _fakes) = launch();
    store.send(A::Switch(Route::Session(SessionId::new("cdesk-triage"))), |_| {});
    store.send(A::OpenChannelAccess(payments()), |_| {});
    assert_eq!(store.state().route, Route::Channel(payments()));
    assert_eq!(
        store.state().channel_pages.get(&payments()).map(|view| view.tab),
        Some(channel::Tab::Access)
    );
}

#[test]
fn opening_a_session_lists_the_desk_sessions_that_look_after_it() {
    let (mut store, _fakes) = launch();
    let ledger = SessionId::new("ledger");
    store.send(A::Switch(Route::Session(ledger.clone())), |_| {});
    store.settle();
    let ids: Vec<SessionId> = store
        .state()
        .desk
        .targeting_rows(&ledger)
        .iter()
        .map(|row| row.session.id.clone())
        .collect();
    assert_eq!(ids, vec![SessionId::new("cdesk-backfill")]);
}
