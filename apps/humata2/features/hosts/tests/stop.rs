//! Stop all sessions… and Start sessions (frames 72–76, D260–D265) on the
//! machine's page and the Hosts page, against the fakes: nothing is sent
//! before its confirm, a stop shows as Sessions stopped after the page
//! reloads, a stop the machine never got is cancelled without a confirm,
//! and while stopped the page refuses Open a shell and New session.

use std::sync::Arc;
use std::time::Duration;

use humata2_clients::machines::{
    Call, FakeMachines, GhLogin, MachineDetail, MachineHealth, Reach, Started,
};
use humata2_clients::model::{MachineId, MachineStop, SessionId, StoppedBy};
use humata2_clients::{Clients, DaemonError, fixture};
use humata2_hosts::page::{Confirm, MenuPick, Refused, SessionsChange};
use humata2_hosts::{
    Hosts, HostsAction as H, HostsState, MachinePage, MachinePageAction as A, MachinePageState,
};
use tca::{TestClock, TestStore};

fn id(text: &str) -> MachineId {
    MachineId::new(text)
}

fn detail(machine: &str, reach: Reach) -> MachineDetail {
    MachineDetail {
        machine: id(machine),
        os: "Ubuntu 24.04".to_owned(),
        arch: "x86_64".to_owned(),
        address: format!("{machine}.lan"),
        reach,
        daemon_version: "2.0.3".to_owned(),
        health: MachineHealth {
            gh: Some(GhLogin { user: "uhvesta".to_owned(), scopes: vec!["repo".to_owned()] }),
            agents: vec![],
            disk_free: 11,
            disk_total: 500,
            variables: vec![],
            skills_problem: None,
            git_version: "2.43.0".to_owned(),
            gh_version: "2.83.0".to_owned(),
        },
        folders: vec![],
        sessions: vec![SessionId::new("s1")],
        rules: vec![],
        stop: None,
    }
}

fn kept_stop(delivered: bool) -> MachineStop {
    MachineStop {
        stop_id: "stop-9".to_owned(),
        at: "14:02".to_owned(),
        by: StoppedBy::You,
        delivered,
    }
}

/// `machine`'s page, loaded, its sessions running.
fn page(machine: &str, reach: Reach) -> (TestStore<MachinePage>, FakeMachines) {
    let fake = FakeMachines::new();
    fake.set_detail(Ok(detail(machine, reach)));
    let mut store =
        TestStore::new(MachinePageState::default(), MachinePage::new(Arc::new(fake.clone())))
            .non_exhaustive();
    store.send(A::Show(id(machine)), |_| {});
    store.receive_matching("loaded", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
    (store, fake)
}

/// devbox-1's page, loaded with `stop` already made.
fn stopped_page(stop: MachineStop) -> (TestStore<MachinePage>, FakeMachines) {
    let fake = FakeMachines::new();
    fake.set_detail(Ok(detail("devbox-1", Reach::Ssh)));
    fake.set_stop(&id("devbox-1"), Some(stop));
    let mut store =
        TestStore::new(MachinePageState::default(), MachinePage::new(Arc::new(fake.clone())))
            .non_exhaustive();
    store.send(A::Show(id("devbox-1")), |_| {});
    store.receive_matching("loaded", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
    (store, fake)
}

fn stops_sent(fake: &FakeMachines) -> usize {
    fake.calls().iter().filter(|call| matches!(call, Call::StopSessions(_))).count()
}

fn starts_sent(fake: &FakeMachines) -> usize {
    fake.calls().iter().filter(|call| matches!(call, Call::StartSessions(_))).count()
}

fn reloaded(store: &mut TestStore<MachinePage>) {
    store.receive_matching("reloaded", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
}

/// Whether the page handed anything to its parent before the timeout.
fn delegated(store: &mut TestStore<MachinePage>) -> bool {
    store
        .try_receive_matching("a delegate", |a| matches!(a, A::Delegate(_)), |_| {})
        .is_ok()
}

#[test]
fn stop_all_sessions_asks_first_and_sends_nothing_until_confirmed() {
    let (mut store, fake) = page("devbox-1", Reach::Ssh);
    store.send(A::OpenMenu, |_| {});
    store.send(A::PickMenu(MenuPick::StopSessions), |_| {});

    assert!(!store.state().menu_open);
    assert_eq!(store.state().confirm, Some(Confirm::Stop));
    store.settle();
    assert_eq!(stops_sent(&fake), 0);
}

#[test]
fn cancelling_the_stop_confirm_keeps_the_sessions_running() {
    let (mut store, fake) = page("devbox-1", Reach::Ssh);
    store.send(A::PickMenu(MenuPick::StopSessions), |_| {});
    store.send(A::DismissConfirm, |_| {});

    assert_eq!(store.state().confirm, None);
    store.settle();
    assert_eq!(stops_sent(&fake), 0);
    assert_eq!(store.state().stop(), None);
}

#[test]
fn confirming_the_stop_shows_sessions_stopped_after_the_reload() {
    let (mut store, fake) = page("devbox-1", Reach::Ssh);
    store.send(A::PickMenu(MenuPick::StopSessions), |_| {});
    store.send(A::ConfirmSessions, |_| {});

    assert_eq!(store.state().confirm, None);
    assert_eq!(store.state().changing, Some(SessionsChange::Stop));
    store.receive_matching("stopped", |a| matches!(a, A::SessionsStopped(_, Ok(_))), |_| {});
    reloaded(&mut store);

    assert_eq!(stops_sent(&fake), 1);
    assert_eq!(store.state().changing, None);
    assert_eq!(store.state().error, None);
    let stop = store.state().stop().cloned().expect("the page shows the stop");
    assert!(stop.delivered);
    assert_eq!(stop.by, StoppedBy::You);
}

#[test]
fn this_mac_is_stopped_the_same_way() {
    let (mut store, fake) = page("this-mac", Reach::Local);
    store.send(A::PickMenu(MenuPick::StopSessions), |_| {});
    assert_eq!(store.state().confirm, Some(Confirm::Stop));
    store.send(A::ConfirmSessions, |_| {});
    store.receive_matching("stopped", |a| matches!(a, A::SessionsStopped(_, Ok(_))), |_| {});
    reloaded(&mut store);

    assert_eq!(fake.calls().iter().find(|call| matches!(call, Call::StopSessions(_))),
        Some(&Call::StopSessions(id("this-mac"))));
    assert!(store.state().stop().is_some());
}

#[test]
fn a_failed_stop_keeps_the_page_and_says_why() {
    let (mut store, fake) = page("devbox-1", Reach::Ssh);
    fake.fail("stop_sessions", DaemonError::Offline { machine: id("devbox-1") });
    store.send(A::PickMenu(MenuPick::StopSessions), |_| {});
    store.send(A::ConfirmSessions, |_| {});
    store.receive_matching("failed", |a| matches!(a, A::SessionsStopped(_, Err(_))), |_| {});

    assert_eq!(store.state().changing, None);
    assert!(store.state().error.is_some());
    assert!(store.state().detail.is_some());
    assert_eq!(store.state().stop(), None);
}

#[test]
fn a_stopped_machine_offers_no_second_stop() {
    let (mut store, fake) = stopped_page(kept_stop(true));
    store.send(A::OpenMenu, |_| {});
    store.send(A::PickMenu(MenuPick::StopSessions), |_| {});

    assert!(!store.state().menu_open);
    assert_eq!(store.state().confirm, None);
    store.settle();
    assert_eq!(stops_sent(&fake), 0);
}

#[test]
fn start_sessions_asks_first_then_brings_the_sessions_back() {
    let (mut store, fake) = stopped_page(kept_stop(true));
    store.send(A::StartSessions, |_| {});
    assert_eq!(store.state().confirm, Some(Confirm::Start));
    store.settle();
    assert_eq!(starts_sent(&fake), 0);

    store.send(A::ConfirmSessions, |_| {});
    assert_eq!(store.state().changing, Some(SessionsChange::Start));
    store.receive(A::SessionsStarted(id("devbox-1"), Ok(Started { cancelled: false })), |_| {});
    reloaded(&mut store);

    assert_eq!(starts_sent(&fake), 1);
    assert_eq!(store.state().changing, None);
    assert_eq!(store.state().stop(), None);
}

#[test]
fn cancelling_the_start_confirm_keeps_the_stop() {
    let (mut store, fake) = stopped_page(kept_stop(true));
    store.send(A::StartSessions, |_| {});
    store.send(A::DismissConfirm, |_| {});

    assert_eq!(store.state().confirm, None);
    store.settle();
    assert_eq!(starts_sent(&fake), 0);
    assert_eq!(store.state().stop(), Some(&kept_stop(true)));
}

#[test]
fn start_sessions_does_nothing_while_the_sessions_run() {
    let (mut store, fake) = page("devbox-1", Reach::Ssh);
    store.send(A::StartSessions, |_| {});
    store.send(A::CancelStop, |_| {});

    assert_eq!(store.state().confirm, None);
    assert_eq!(store.state().changing, None);
    store.settle();
    assert_eq!(starts_sent(&fake), 0);
}

#[test]
fn a_stop_the_machine_never_got_is_cancelled_without_a_confirm() {
    let (mut store, fake) = stopped_page(kept_stop(false));
    store.send(A::CancelStop, |_| {});

    assert_eq!(store.state().confirm, None);
    assert_eq!(store.state().changing, Some(SessionsChange::CancelStop));
    store.receive(A::SessionsStarted(id("devbox-1"), Ok(Started { cancelled: true })), |_| {});
    reloaded(&mut store);

    assert_eq!(starts_sent(&fake), 1);
    assert_eq!(store.state().changing, None);
    assert_eq!(store.state().stop(), None);
}

#[test]
fn cancel_stop_does_nothing_once_the_machine_got_the_stop() {
    let (mut store, fake) = stopped_page(kept_stop(true));
    store.send(A::CancelStop, |_| {});

    assert_eq!(store.state().changing, None);
    store.settle();
    assert_eq!(starts_sent(&fake), 0);
    assert_eq!(store.state().stop(), Some(&kept_stop(true)));
}

#[test]
fn a_failed_start_keeps_the_stop_and_says_why() {
    let (mut store, fake) = stopped_page(kept_stop(true));
    fake.fail("start_sessions", DaemonError::Offline { machine: id("devbox-1") });
    store.send(A::StartSessions, |_| {});
    store.send(A::ConfirmSessions, |_| {});
    store.receive_matching("failed", |a| matches!(a, A::SessionsStarted(_, Err(_))), |_| {});

    assert_eq!(store.state().changing, None);
    assert!(store.state().error.is_some());
    assert_eq!(store.state().stop(), Some(&kept_stop(true)));
}

#[test]
fn while_stopped_a_shell_or_new_session_is_refused_and_says_start_first() {
    for action in [A::OpenShell, A::NewSession] {
        let (mut store, _fake) = stopped_page(kept_stop(true));
        store.send(action, |_| {});
        assert_eq!(store.state().refused, Some(Refused::SessionsStopped));
        assert!(!delegated(&mut store), "nothing goes to the parent");
    }
}

#[test]
fn opening_a_stopped_session_still_goes_to_the_parent() {
    // Its transcript stays readable; only typing is refused (D264).
    let (mut store, _fake) = stopped_page(kept_stop(true));
    store.send(A::OpenSession(SessionId::new("s1")), |_| {});
    assert!(delegated(&mut store));
    assert_eq!(store.state().refused, None);
}

// The Hosts page: a stopped machine's row's one action (frame 74).

/// The Hosts page on the design workspace with devbox-1's sessions
/// stopped as `stop`.
fn hosts(stop: Option<MachineStop>) -> (TestStore<Hosts>, Clients, humata2_clients::Fakes) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    fakes.machines.set_stop(&id("devbox-1"), stop);
    let workspace = Arc::new(fakes.daemon.workspace());
    let mut store = TestStore::new(HostsState::default(), Hosts::new(clients.clone()))
        .with_timeout(Duration::from_secs(30))
        .non_exhaustive();
    store.send(H::Show(workspace), |_| {});
    store.skip_received();
    (store, clients, fakes)
}

fn hosts_starts(fakes: &humata2_clients::Fakes) -> usize {
    fakes.machines.calls().iter().filter(|call| matches!(call, Call::StartSessions(_))).count()
}

#[test]
fn the_hosts_row_starts_a_stopped_machine_after_its_confirm() {
    let (mut store, _clients, fakes) = hosts(Some(kept_stop(true)));
    store.send(H::StartSessions(id("devbox-1")), |_| {});
    assert_eq!(store.state().start_confirm, Some(id("devbox-1")));
    store.settle();
    assert_eq!(hosts_starts(&fakes), 0);

    store.send(H::ConfirmStart, |_| {});
    assert_eq!(store.state().start_confirm, None);
    assert!(store.state().starting.contains(&id("devbox-1")));
    store.receive_matching(
        "started",
        |a| matches!(a, H::SessionsStarted(machine, Ok(_)) if *machine == id("devbox-1")),
        |_| {},
    );

    assert!(store.state().starting.is_empty());
    assert!(!store.state().errors.contains_key(&id("devbox-1")));
    let devbox = fakes.daemon.workspace().machine(&id("devbox-1")).cloned();
    assert_eq!(devbox.and_then(|machine| machine.stop), None);
}

#[test]
fn cancelling_the_hosts_start_confirm_keeps_the_stop() {
    let (mut store, _clients, fakes) = hosts(Some(kept_stop(true)));
    store.send(H::StartSessions(id("devbox-1")), |_| {});
    store.send(H::DismissStart, |_| {});

    assert_eq!(store.state().start_confirm, None);
    store.settle();
    assert_eq!(hosts_starts(&fakes), 0);
    let devbox = fakes.daemon.workspace().machine(&id("devbox-1")).cloned();
    assert_eq!(devbox.and_then(|machine| machine.stop), Some(kept_stop(true)));
}

#[test]
fn the_hosts_page_offers_no_start_for_a_machine_whose_sessions_run() {
    let (mut store, _clients, fakes) = hosts(None);
    store.send(H::StartSessions(id("devbox-1")), |_| {});

    assert_eq!(store.state().start_confirm, None);
    store.settle();
    assert_eq!(hosts_starts(&fakes), 0);
}

#[test]
fn a_failed_start_from_hosts_shows_on_the_machines_row() {
    let (mut store, _clients, fakes) = hosts(Some(kept_stop(true)));
    fakes.machines.fail("start_sessions", DaemonError::Offline { machine: id("devbox-1") });
    store.send(H::StartSessions(id("devbox-1")), |_| {});
    store.send(H::ConfirmStart, |_| {});
    store.receive_matching(
        "failed",
        |a| matches!(a, H::SessionsStarted(_, Err(_))),
        |_| {},
    );

    assert!(store.state().starting.is_empty());
    assert!(store.state().errors.contains_key(&id("devbox-1")));
}
