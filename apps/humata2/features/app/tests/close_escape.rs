//! D784: Escape cancels the close-tab dialog and the machine page's running
//! confirmations, leaving the page as it was.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::model::{Agent, ChannelId, MachineId, SessionId};
use humata2_clients::{Clients, RunningHolder, fixture};
use humata2_session::{CloseConfirm, CloseKind};
use tca::{TestClock, TestStore};

fn store(route: Route, set: impl FnOnce(&mut AppState)) -> TestStore<App> {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.route = route;
    set(&mut state);
    TestStore::new(state, App::new(clients)).non_exhaustive()
}

fn holder() -> RunningHolder {
    RunningHolder {
        tab: Some("tab-1".to_owned()),
        session: None,
        title: String::new(),
        channel: None::<ChannelId>,
        desk: false,
        agent: Agent::Shell,
        pid: Some(1),
        folder: "~/scratch".to_owned(),
        started_unix_ms: None,
        attached: false,
        machine: MachineId::new("devbox-1"),
        stopped: false,
        agent_tab: false,
        archived: false,
        paused_for_move: false,
    }
}

fn store_adopting() -> TestStore<App> {
    store(Route::Machine(MachineId::new("devbox-1")), |state| {
        state.machine_page.running.adopting = Some(holder());
    })
}

#[test]
fn escape_cancels_the_close_dialog() {
    let id = SessionId::new("ledger");
    let mut store = store(Route::Session(id.clone()), |state| {
        state.session.closing = Some(CloseConfirm {
        session: id.clone(),
        group: 0,
        index: 0,
        tab: 0,
        kind: CloseKind::Agent,
        title: Some("Ledger".to_owned()),
        agent: "Claude Code".to_owned(),
        machine: "This Mac".to_owned(),
        always_keep_running: false,
        sending: None,
        error: None,
        });
    });
    store.send(A::Escape, |_| {});
    assert!(store.state().session.closing.is_none());
    assert_eq!(store.state().route, Route::Session(id));
}

#[test]
fn escape_cancels_end_for_good_and_adopt_on_the_machine_page() {
    let mut store = store(Route::Machine(MachineId::new("devbox-1")), |state| {
        state.machine_page.running.confirm_end = Some(holder());
    });
    store.send(A::Escape, |_| {});
    assert!(store.state().machine_page.running.confirm_end.is_none());
    let mut store = store_adopting();
    store.send(A::Escape, |_| {});
    assert!(store.state().machine_page.running.adopting.is_none());
}

#[test]
fn the_close_dialog_and_running_confirms_hold_the_keys_from_a_terminal() {
    let id = SessionId::new("ledger");
    let mut held = store(Route::Session(id.clone()), |state| {
        state.session.closing = Some(CloseConfirm {
            session: id.clone(),
            group: 0,
            index: 0,
            tab: 0,
            kind: CloseKind::Agent,
            title: Some("Ledger".to_owned()),
            agent: "Claude Code".to_owned(),
            machine: "This Mac".to_owned(),
            always_keep_running: false,
            sending: None,
            error: None,
        });
    });
    assert!(held.state().dialog_holds_keys(), "the close dialog holds the keys");
    held.send(A::Escape, |_| {});
    assert!(!held.state().dialog_holds_keys(), "Escape gives them back");

    let mut held = store(Route::Machine(MachineId::new("devbox-1")), |state| {
        state.machine_page.running.confirm_end = Some(holder());
    });
    assert!(held.state().dialog_holds_keys(), "End for good… holds the keys");
    held.send(A::Escape, |_| {});
    assert!(!held.state().dialog_holds_keys());

    let mut held = store_adopting();
    assert!(held.state().dialog_holds_keys(), "Adopt… holds the keys");
    held.send(A::Escape, |_| {});
    assert!(!held.state().dialog_holds_keys());
}
