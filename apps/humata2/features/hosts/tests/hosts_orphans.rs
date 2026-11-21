//! D784, frame 28: a Hosts row's "N sessions" counts the orphan holders
//! running on that machine too. The page reads each online machine's
//! holders when that machine first shows as online, keeps what it read
//! when a later read fails, and reads nothing from an offline machine.

use std::sync::Arc;
use std::time::Duration;

use humata2_clients::model::{Agent, MachineId, SessionId};
use humata2_clients::{Clients, FakeDaemon, RunningHolder, Workspace, fixture};
use humata2_hosts::{Hosts, HostsAction as A, HostsState};
use tca::{TestClock, TestStore};

fn devbox() -> MachineId {
    MachineId::new("devbox-1")
}

fn holder(session: Option<&str>, stopped: bool) -> RunningHolder {
    RunningHolder {
        tab: Some(format!("tab-{}", session.unwrap_or("orphan"))),
        session: session.map(SessionId::new),
        title: String::new(),
        channel: None,
        desk: false,
        agent: Agent::Shell,
        pid: (!stopped).then_some(4000),
        folder: "~/scratch/probe".to_owned(),
        started_unix_ms: (!stopped).then_some(100),
        attached: false,
        machine: devbox(),
        stopped,
        agent_tab: false,
        archived: false,
        paused_for_move: false,
    }
}

/// The Hosts page on the design workspace, its daemon listing `holders`.
fn page(holders: Vec<RunningHolder>) -> (TestStore<Hosts>, FakeDaemon, Arc<Workspace>) {
    let workspace = Arc::new(fixture::design());
    let (clients, fakes) = humata2_clients::fake_clients((*workspace).clone(), Arc::new(TestClock::new()));
    fakes.daemon.set_running_holders(holders);
    let store = TestStore::new(HostsState::default(), Hosts::new(clients))
        .with_timeout(Duration::from_secs(30))
        .non_exhaustive();
    (store, fakes.daemon, workspace)
}

#[test]
fn an_online_machines_running_orphans_are_counted() {
    let (mut store, _daemon, workspace) = page(vec![holder(None, false), holder(None, false), holder(Some("ledger"), false)]);
    store.send(A::Show(workspace), |_| {});
    store.receive_matching("Holders", |action| matches!(action, A::Holders(machine, Ok(_)) if *machine == devbox()), |_| {});
    assert_eq!(store.state().orphans.get(&devbox()), Some(&2), "two orphans, the session's holder is not one");
}

#[test]
fn a_stopped_orphan_is_not_counted() {
    let (mut store, _daemon, workspace) = page(vec![holder(None, true)]);
    store.send(A::Show(workspace), |_| {});
    store.receive_matching("Holders", |action| matches!(action, A::Holders(machine, Ok(_)) if *machine == devbox()), |_| {});
    assert_eq!(store.state().orphans.get(&devbox()).copied().unwrap_or(0), 0);
}

#[test]
fn a_failed_read_keeps_the_last_count() {
    let (mut store, _daemon, _workspace) = page(Vec::new());
    store.send(A::Holders(devbox(), Ok(vec![holder(None, false)])), |_| {});
    store.send(A::Holders(devbox(), Err(humata2_clients::DaemonError::Offline { machine: devbox() })), |_| {});
    assert_eq!(store.state().orphans.get(&devbox()), Some(&1));
}
