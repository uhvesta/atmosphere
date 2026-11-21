//! A channel's Desk through the whole app (CD-2, CD-4): the sidebar's Desk
//! row opens the Desk tab, and closing a Desk session from its own page
//! asks first and goes back to that tab.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route, channel};
use humata2_clients::model::{ChannelId, SessionId};
use humata2_clients::{Fakes, Workspace, fixture};
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

fn launch_without_desk() -> (TestStore<App>, Fakes) {
    let workspace: Workspace = fixture::design();
    let (clients, fakes) = humata2_clients::fake_clients(workspace, Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.settle();
    (store, fakes)
}

fn infra() -> ChannelId {
    ChannelId::new("infra")
}

fn receive_primary_started(store: &mut TestStore<App>) {
    store.receive_matching(
        "primary desk started",
        |a| matches!(a, A::Desk(D::PrimaryChannelStarted { result: Ok(_), .. })),
        |_| {},
    );
    store.receive_matching(
        "workspace reloaded",
        |a| matches!(a, A::Loaded(Ok(_))),
        |_| {},
    );
    store.skip_received();
}

#[test]
fn opening_a_channels_desk_tab_auto_starts_a_primary_desk_and_reuses_it() {
    let (mut store, fakes) = launch_without_desk();
    assert!(store.state().desk.primary_channel_desk(&payments()).is_none());

    // Navigating to a channel defaults to Tab::Desk and auto-starts its primary Desk.
    store.send(A::Switch(Route::Channel(payments())), |_| {});
    receive_primary_started(&mut store);

    // Stays on the channel's Desk tab (does not hijack route to Route::Session).
    assert_eq!(store.state().route, Route::Channel(payments()));
    assert_eq!(
        store
            .state()
            .channel_pages
            .get(&payments())
            .map(|v| v.tab)
            .unwrap_or_default(),
        channel::Tab::Desk
    );
    let primary_id = store
        .state()
        .active_channel_desk()
        .expect("primary Desk session should be active on Tab::Desk");
    let record = store
        .state()
        .desk
        .channel_record(&primary_id)
        .expect("channel Desk record exists");
    assert_eq!(record.channel, payments());
    assert_eq!(record.target, None);
    assert_eq!(store.state().desk.channel_rows(&payments()).len(), 1);
    assert_eq!(
        fakes.daemon.workspace().desk.channel_sessions.len(),
        1,
        "exactly one Desk session was created"
    );

    // Switching away to Sessions tab and back to Desk reuses the same session.
    store.send(
        A::Channel(channel::Action::Tab(channel::Tab::Sessions)),
        |_| {},
    );
    assert_eq!(store.state().active_channel_desk(), None);
    store.send(A::Channel(channel::Action::Tab(channel::Tab::Desk)), |_| {});
    store.settle();

    assert_eq!(store.state().active_channel_desk(), Some(primary_id.clone()));
    assert_eq!(
        fakes.daemon.workspace().desk.channel_sessions.len(),
        1,
        "revisiting reuses the existing primary Desk without spawning a duplicate"
    );
}

#[test]
fn closing_a_primary_channel_desk_from_full_session_stays_closed_and_does_not_reopen() {
    let (mut store, fakes) = launch_without_desk();

    // Open channel (defaults to Tab::Desk, auto-starts primary Desk).
    store.send(A::Switch(Route::Channel(payments())), |_| {});
    receive_primary_started(&mut store);
    let primary_id = store
        .state()
        .active_channel_desk()
        .expect("primary Desk started");

    // Open as full session.
    store.send(A::Desk(D::Open(primary_id.clone())), |_| {});
    store.receive_matching(
        "open full session delegated",
        |a| matches!(a, A::Desk(D::Delegate(humata2_desk::Delegate::OpenSession(_)))),
        |_| {},
    );
    store.settle();
    assert_eq!(store.state().route, Route::Session(primary_id.clone()));

    // Close from full session view.
    store.send(A::Desk(D::AskClose(primary_id.clone())), |_| {});
    assert!(store.state().desk.channel_close.is_some());
    store.send(A::Desk(D::ConfirmCloseChannel), |_| {});
    store.receive_matching(
        "close finished",
        |a| matches!(a, A::Desk(D::Changed { result: Ok(()), .. })),
        |_| {},
    );
    store.receive_matching(
        "workspace reloaded",
        |a| matches!(a, A::Loaded(Ok(_))),
        |_| {},
    );
    store.skip_received();

    // Must return to the channel's Desk tab, stay Closed, and NOT reopen as a
    // full session or respawn a new primary Desk.
    assert_eq!(store.state().route, Route::Channel(payments()));
    assert_eq!(
        store.state().desk.channel_status.get(&payments()),
        Some(&humata2_desk::ChannelDeskStatus::Closed)
    );
    assert_eq!(store.state().active_channel_desk(), None);
    assert!(fakes.daemon.workspace().desk.channel_sessions.is_empty());
}

#[test]
fn switching_between_channels_keeps_each_channels_desk_scoped() {
    let (mut store, fakes) = launch_without_desk();

    store.send(A::OpenChannelDesk(payments()), |_| {});
    receive_primary_started(&mut store);
    let payments_desk = store
        .state()
        .active_channel_desk()
        .expect("payments primary Desk started");

    store.send(A::OpenChannelDesk(infra()), |_| {});
    receive_primary_started(&mut store);
    let infra_desk = store
        .state()
        .active_channel_desk()
        .expect("infra primary Desk started");

    assert_ne!(payments_desk, infra_desk);
    assert_eq!(
        store.state().desk.channel_record(&payments_desk).map(|r| &r.channel),
        Some(&payments())
    );
    assert_eq!(
        store.state().desk.channel_record(&infra_desk).map(|r| &r.channel),
        Some(&infra())
    );
    assert_eq!(fakes.daemon.workspace().desk.channel_sessions.len(), 2);

    // Returning to payments reattaches to payments_desk with 0 new spawns.
    store.send(A::OpenChannelDesk(payments()), |_| {});
    store.settle();
    assert_eq!(store.state().active_channel_desk(), Some(payments_desk));
    assert_eq!(fakes.daemon.workspace().desk.channel_sessions.len(), 2);
}

#[test]
fn rapid_duplicate_open_channel_desk_only_starts_one_session() {
    let (mut store, fakes) = launch_without_desk();
    store.send(A::OpenChannelDesk(payments()), |_| {});
    assert_eq!(
        store.state().desk.channel_status.get(&payments()),
        Some(&humata2_desk::ChannelDeskStatus::Starting)
    );
    // Second dispatch before the first settles is deduplicated.
    store.send(A::OpenChannelDesk(payments()), |_| {});
    store.send(A::Channel(channel::Action::Tab(channel::Tab::Desk)), |_| {});
    receive_primary_started(&mut store);

    assert_eq!(store.state().desk.channel_rows(&payments()).len(), 1);
    assert_eq!(fakes.daemon.workspace().desk.channel_sessions.len(), 1);
}

#[test]
fn channel_desk_shows_no_agent_status_when_all_agents_are_off() {
    let (mut store, fakes) = launch_without_desk();
    let all_off: Vec<humata2_clients::AgentSwitch> = humata2_clients::model::Agent::ALL
        .into_iter()
        .filter(|a| *a != humata2_clients::model::Agent::Shell)
        .map(|a| humata2_clients::AgentSwitch {
            kind: a.name().to_lowercase(),
            name: a.name().to_owned(),
            agent: Some(a),
            enabled: false,
            install_when_missing: false,
            can_install: false,
            install_command: None,
            asks_in_inbox: false,
        })
        .collect();
    store.send(
        A::Settings(humata2_settings::SettingsAction::Agents(
            humata2_settings::AgentsAction::Loaded(Ok(all_off)),
        )),
        |_| {},
    );

    store.send(A::OpenChannelDesk(payments()), |_| {});
    store.settle();

    assert_eq!(
        store.state().desk.channel_status.get(&payments()),
        Some(&humata2_desk::ChannelDeskStatus::NoAgent)
    );
    assert!(fakes.daemon.workspace().desk.channel_sessions.is_empty());

    store.send(A::OpenAgentSettings, |_| {});
    assert_eq!(store.state().route, Route::Settings);
    assert_eq!(
        store.state().settings.section,
        humata2_settings::Section::Agents
    );
}

#[test]
fn channel_desk_shows_no_machine_status_when_all_machines_are_offline() {
    let mut workspace: Workspace = fixture::design();
    for m in &mut workspace.machines {
        m.online = false;
    }
    let (clients, fakes) = humata2_clients::fake_clients(workspace, Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.settle();

    store.send(A::OpenChannelDesk(payments()), |_| {});
    store.settle();

    assert_eq!(
        store.state().desk.channel_status.get(&payments()),
        Some(&humata2_desk::ChannelDeskStatus::NoMachine)
    );
    assert!(fakes.daemon.workspace().desk.channel_sessions.is_empty());
}
