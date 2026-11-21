//! D784 on the fake daemon: a machine lists its running holders in order;
//! a shell stops, resumes and ends by its tab; an agent's tab is refused;
//! an offline machine refuses everything; a failure can be scripted.

use std::sync::Arc;

use humata2_clients::model::{Agent, MachineId, SessionId, Status};
use humata2_clients::{Daemon, DaemonError, FakeDaemon, HolderTab, Operation, RunningHolder, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn mac() -> MachineId {
    MachineId::new("this-mac")
}

fn holder(tab: &str, session: Option<&str>, title: &str, agent: Agent, started: i64) -> RunningHolder {
    RunningHolder {
        tab: Some(tab.to_owned()),
        session: session.map(SessionId::new),
        title: title.to_owned(),
        channel: None,
        desk: false,
        agent,
        pid: Some(4000),
        folder: "/work/sample".to_owned(),
        started_unix_ms: Some(started),
        attached: false,
        machine: mac(),
        stopped: false,
        agent_tab: false,
        archived: false,
        paused_for_move: false,
    }
}

/// An agent session of this Mac in the standard workspace.
fn agent_session(daemon: &FakeDaemon) -> SessionId {
    match daemon
        .workspace()
        .sessions
        .iter()
        .find(|session| session.machine == mac() && session.agent != Agent::Shell && !session.archived)
    {
        Some(session) => session.id.clone(),
        None => panic!("the standard workspace has an agent session on this Mac"),
    }
}

fn seeded() -> (FakeDaemon, SessionId) {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    let session = agent_session(&daemon);
    let mut stopped = holder("unused", Some("sample-stopped"), "Alpha stopped", Agent::Codex, 0);
    stopped.tab = None;
    stopped.pid = None;
    stopped.started_unix_ms = None;
    stopped.stopped = true;
    daemon.set_running_holders(vec![
        holder("orphan-late", None, "", Agent::Copilot, 300),
        holder("zulu", Some(session.as_str()), "Zulu", Agent::Claude, 100),
        stopped,
        holder("orphan-early", None, "", Agent::Shell, 200),
        holder("alpha", Some("sample-alpha"), "Alpha", Agent::Shell, 50),
        RunningHolder {
            machine: MachineId::new("gpu-box"),
            ..holder("elsewhere", None, "", Agent::Shell, 10)
        },
    ]);
    (daemon, session)
}

fn tabs(holders: &[RunningHolder]) -> Vec<Option<&str>> {
    holders.iter().map(|holder| holder.tab.as_deref()).collect()
}

#[test]
fn a_machine_lists_its_running_sessions_then_stopped_ones_then_orphans_by_start() {
    let (daemon, _) = seeded();
    let listed = block_on(daemon.running_holders(&mac())).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        tabs(&listed),
        vec![Some("alpha"), Some("zulu"), None, Some("orphan-early"), Some("orphan-late")]
    );
}

#[test]
fn a_stopped_shell_resumes_and_an_ended_one_is_gone() {
    let (daemon, _) = seeded();
    let tab = HolderTab::Listed("orphan-early".to_owned());
    block_on(daemon.stop_running_holder(&mac(), &tab)).unwrap_or_else(|error| panic!("{error}"));
    let listed = block_on(daemon.running_holders(&mac())).unwrap_or_else(|error| panic!("{error}"));
    let stopped = listed.iter().find(|holder| holder.tab.as_deref() == Some("orphan-early"));
    assert_eq!(stopped.map(|holder| (holder.stopped, holder.pid)), Some((true, None)));

    block_on(daemon.resume_running_holder(&mac(), &tab)).unwrap_or_else(|error| panic!("{error}"));
    let listed = block_on(daemon.running_holders(&mac())).unwrap_or_else(|error| panic!("{error}"));
    let resumed = listed.iter().find(|holder| holder.tab.as_deref() == Some("orphan-early"));
    assert_eq!(resumed.map(|holder| holder.stopped), Some(false));

    block_on(daemon.end_running_holder(&mac(), &tab)).unwrap_or_else(|error| panic!("{error}"));
    let listed = block_on(daemon.running_holders(&mac())).unwrap_or_else(|error| panic!("{error}"));
    assert!(listed.iter().all(|holder| holder.tab.as_deref() != Some("orphan-early")));
}

#[test]
fn an_agents_tab_is_refused_and_stays_listed() {
    let (daemon, session) = seeded();
    for tab in [
        HolderTab::Listed("zulu".to_owned()),
        HolderTab::Session { session: session.clone(), index: 0 },
    ] {
        let ended = block_on(daemon.end_running_holder(&mac(), &tab));
        assert!(matches!(ended, Err(DaemonError::Refused { .. })), "{tab:?}: {ended:?}");
        let stopped = block_on(daemon.stop_running_holder(&mac(), &tab));
        assert!(matches!(stopped, Err(DaemonError::Refused { .. })), "{tab:?}: {stopped:?}");
    }
    let listed = block_on(daemon.running_holders(&mac())).unwrap_or_else(|error| panic!("{error}"));
    assert!(listed.iter().any(|holder| holder.tab.as_deref() == Some("zulu") && !holder.stopped));
}

#[test]
fn an_offline_machine_refuses_to_list_stop_or_end() {
    let (daemon, _) = seeded();
    let offline = MachineId::new("ci-runner");
    let tab = HolderTab::Listed("anything".to_owned());
    assert!(matches!(block_on(daemon.running_holders(&offline)), Err(DaemonError::Offline { .. })));
    assert!(matches!(block_on(daemon.stop_running_holder(&offline, &tab)), Err(DaemonError::Offline { .. })));
    assert!(matches!(block_on(daemon.end_running_holder(&offline, &tab)), Err(DaemonError::Offline { .. })));
}

#[test]
fn a_scripted_failure_leaves_the_shell_running() {
    let (daemon, _) = seeded();
    daemon.fail_next(Operation::EndRunningHolder, "the holder would not end");
    let tab = HolderTab::Listed("orphan-late".to_owned());
    assert!(block_on(daemon.end_running_holder(&mac(), &tab)).is_err());
    let listed = block_on(daemon.running_holders(&mac())).unwrap_or_else(|error| panic!("{error}"));
    assert!(listed.iter().any(|holder| holder.tab.as_deref() == Some("orphan-late")));
}

#[test]
fn an_archived_session_restores_stopped() {
    let (daemon, session) = seeded();
    let plan = block_on(daemon.plan_archive(&session)).unwrap_or_else(|error| panic!("{error}"));
    let _ = plan;
    block_on(daemon.archive(&session, false)).unwrap_or_else(|error| panic!("{error}"));
    block_on(daemon.restore_session_stopped(&session)).unwrap_or_else(|error| panic!("{error}"));
    let restored = daemon.workspace().session(&session).map(|found| (found.archived, found.status));
    assert_eq!(restored, Some((false, Status::Stopped)));
}
