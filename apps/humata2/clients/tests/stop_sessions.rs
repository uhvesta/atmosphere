//! Stopping and starting a machine's sessions on the fakes (D260–D265): a
//! stop is kept on the machine, delivered when the machine is online, its
//! sessions read Stopped until Start, and Start of an undelivered stop only
//! cancels it. This Mac stops like any machine and leaves the others
//! running.

use std::sync::Arc;

use humata2_clients::machines::{FakeMachines, GhLogin, MachineDetail, MachineHealth, Machines, Reach, Started};
use humata2_clients::model::{MachineId, MachineStop, Request, RequestId, RequestKind, SessionId, Status, StoppedBy};
use humata2_clients::{Daemon, DaemonError, Event, FakeDaemon, Workspace, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn id(text: &str) -> MachineId {
    MachineId::new(text)
}

/// The standard workspace (this-mac and gpu-box online, ci-runner offline)
/// behind a daemon, and a machines client that goes through it.
fn fakes() -> (FakeDaemon, FakeMachines) {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    let machines = FakeMachines::new().through(daemon.clone());
    (daemon, machines)
}

/// Whether every session of `machine` that had a running process reads
/// Stopped, and the exited ones stay exited.
fn all_stopped(before: &Workspace, after: &Workspace, machine: &str) -> bool {
    statuses(before, machine).iter().zip(statuses(after, machine)).all(|(was, now)| match was {
        Status::Exited { .. } => now == *was,
        _ => now == Status::Stopped,
    })
}

/// The statuses of `machine`'s sessions that aren't archived.
fn statuses(workspace: &Workspace, machine: &str) -> Vec<Status> {
    workspace
        .sessions
        .iter()
        .filter(|session| session.machine == id(machine) && !session.archived)
        .map(|session| session.status)
        .collect()
}

fn stop_of(daemon: &FakeDaemon, machine: &str) -> Option<MachineStop> {
    daemon.workspace().machine(&id(machine)).and_then(|found| found.stop.clone())
}

#[test]
fn stopping_an_online_machine_delivers_it_and_its_sessions_read_stopped() {
    let (daemon, machines) = fakes();
    let before = daemon.workspace();
    assert!(!statuses(&before, "gpu-box").is_empty(), "the fixture has sessions on gpu-box");

    let stop = block_on(machines.stop_sessions(&id("gpu-box"))).expect("the stop is made");

    assert!(stop.delivered);
    assert_eq!(stop.by, StoppedBy::You);
    assert_eq!(stop_of(&daemon, "gpu-box"), Some(stop));
    let after = daemon.workspace();
    assert!(all_stopped(&before, &after, "gpu-box"));
    // Only that machine's sessions stop.
    assert_eq!(statuses(&after, "this-mac"), statuses(&before, "this-mac"));
    assert_eq!(statuses(&after, "ci-runner"), statuses(&before, "ci-runner"));
}

#[test]
fn a_stop_is_told_on_the_event_feed() {
    let (daemon, machines) = fakes();
    let mut events = block_on(daemon.events()).expect("events open");

    block_on(machines.stop_sessions(&id("gpu-box"))).expect("the stop is made");

    let mut machine_stopped = false;
    let mut session_stopped = false;
    while let Ok(event) = events.try_recv() {
        match event {
            Ok(Event::Machine(machine)) if machine.id == id("gpu-box") => {
                machine_stopped |= machine.stop.is_some();
            }
            Ok(Event::Session(session)) if session.machine == id("gpu-box") => {
                session_stopped |= session.status == Status::Stopped;
            }
            _ => {}
        }
    }
    assert!(machine_stopped, "the machine's change is sent");
    assert!(session_stopped, "its sessions' changes are sent");
}

#[test]
fn starting_brings_the_stopped_sessions_back() {
    let (daemon, machines) = fakes();
    block_on(machines.stop_sessions(&id("gpu-box"))).expect("the stop is made");

    let started = block_on(machines.start_sessions(&id("gpu-box"))).expect("the start is made");

    assert_eq!(started, Started { cancelled: false });
    assert_eq!(stop_of(&daemon, "gpu-box"), None);
    let after = daemon.workspace();
    assert!(statuses(&after, "gpu-box").iter().all(|status| *status != Status::Stopped));
}

#[test]
fn a_stop_of_an_offline_machine_is_kept_undelivered_and_start_only_cancels_it() {
    let (daemon, machines) = fakes();
    let before = daemon.workspace();

    let stop = block_on(machines.stop_sessions(&id("ci-runner"))).expect("the stop is kept");

    assert!(!stop.delivered);
    assert_eq!(stop_of(&daemon, "ci-runner"), Some(stop));
    // The machine hasn't heard of it, so its sessions are as they were.
    assert_eq!(statuses(&daemon.workspace(), "ci-runner"), statuses(&before, "ci-runner"));

    let started = block_on(machines.start_sessions(&id("ci-runner"))).expect("the stop is cancelled");
    assert_eq!(started, Started { cancelled: true });
    assert_eq!(stop_of(&daemon, "ci-runner"), None);
    assert_eq!(statuses(&daemon.workspace(), "ci-runner"), statuses(&before, "ci-runner"));
}

#[test]
fn this_mac_stops_like_any_machine_and_the_others_keep_running() {
    let (daemon, machines) = fakes();
    let before = daemon.workspace();
    assert!(!statuses(&before, "this-mac").is_empty(), "the fixture has sessions on this-mac");

    let stop = block_on(machines.stop_sessions(&id("this-mac"))).expect("the stop is made");

    assert!(stop.delivered);
    let after = daemon.workspace();
    assert!(all_stopped(&before, &after, "this-mac"));
    assert_eq!(statuses(&after, "gpu-box"), statuses(&before, "gpu-box"));
    assert_eq!(stop_of(&daemon, "gpu-box"), None);

    let started = block_on(machines.start_sessions(&id("this-mac"))).expect("the start is made");
    assert_eq!(started, Started { cancelled: false });
    assert!(statuses(&daemon.workspace(), "this-mac").iter().all(|status| *status != Status::Stopped));
}

#[test]
fn starting_a_machine_whose_sessions_run_is_refused() {
    let (daemon, machines) = fakes();
    let before = daemon.workspace();

    let result = block_on(machines.start_sessions(&id("gpu-box")));

    assert!(matches!(result, Err(DaemonError::Refused { .. })), "got {result:?}");
    assert_eq!(statuses(&daemon.workspace(), "gpu-box"), statuses(&before, "gpu-box"));
}

#[test]
fn stopping_a_stopped_machine_is_refused_and_keeps_the_first_stop() {
    let (daemon, machines) = fakes();
    let first = block_on(machines.stop_sessions(&id("gpu-box"))).expect("the stop is made");

    let again = block_on(machines.stop_sessions(&id("gpu-box")));

    assert!(matches!(again, Err(DaemonError::Refused { .. })), "got {again:?}");
    assert_eq!(stop_of(&daemon, "gpu-box"), Some(first));
}

#[test]
fn a_stop_of_a_machine_the_workspace_lacks_is_not_found() {
    let (_daemon, machines) = fakes();
    let stopped = block_on(machines.stop_sessions(&id("nowhere")));
    assert!(matches!(stopped, Err(DaemonError::NotFound { .. })), "got {stopped:?}");
    let started = block_on(machines.start_sessions(&id("nowhere")));
    assert!(matches!(started, Err(DaemonError::NotFound { .. })), "got {started:?}");
}

#[test]
fn a_scripted_failure_makes_no_stop() {
    let (daemon, machines) = fakes();
    let before = daemon.workspace();
    machines.fail("stop_sessions", DaemonError::Offline { machine: id("gpu-box") });

    let result = block_on(machines.stop_sessions(&id("gpu-box")));

    assert!(matches!(result, Err(DaemonError::Offline { .. })), "got {result:?}");
    assert_eq!(stop_of(&daemon, "gpu-box"), None);
    assert_eq!(statuses(&daemon.workspace(), "gpu-box"), statuses(&before, "gpu-box"));
}

fn detail(machine: &str) -> MachineDetail {
    MachineDetail {
        machine: id(machine),
        os: "Ubuntu 24.04".to_owned(),
        arch: "x86_64".to_owned(),
        address: format!("{machine}.lan"),
        reach: Reach::Ssh,
        daemon_version: "2.0.3".to_owned(),
        health: MachineHealth {
            gh: Some(GhLogin { user: "uhvesta".to_owned(), scopes: vec![] }),
            agents: vec![],
            disk_free: 11,
            disk_total: 500,
            variables: vec![],
            skills_problem: None,
            git_version: String::new(),
            gh_version: String::new(),
        },
        folders: vec![],
        sessions: vec![SessionId::new("s1")],
        rules: vec![],
        stop: None,
    }
}

#[test]
fn a_machines_detail_carries_its_stop_until_start() {
    // No daemon: the fake keeps the stop itself.
    let machines = FakeMachines::new();
    machines.set_detail(Ok(detail("devbox-1")));
    let running = block_on(machines.detail(&id("devbox-1"))).expect("the detail loads");
    assert_eq!(running.stop, None);

    let stop = block_on(machines.stop_sessions(&id("devbox-1"))).expect("the stop is made");
    let stopped = block_on(machines.detail(&id("devbox-1"))).expect("the detail loads");
    assert_eq!(stopped.stop, Some(stop));

    block_on(machines.start_sessions(&id("devbox-1"))).expect("the start is made");
    let started = block_on(machines.detail(&id("devbox-1"))).expect("the detail loads");
    assert_eq!(started.stop, None);
}

#[test]
fn a_stop_set_as_made_earlier_shows_in_the_detail_and_start_cancels_an_undelivered_one() {
    let machines = FakeMachines::new();
    machines.set_detail(Ok(detail("devbox-1")));
    let kept = MachineStop {
        stop_id: "stop-9".to_owned(),
        at: "14:02".to_owned(),
        by: StoppedBy::You,
        delivered: false,
    };
    machines.set_stop(&id("devbox-1"), Some(kept.clone()));

    let shown = block_on(machines.detail(&id("devbox-1"))).expect("the detail loads");
    assert_eq!(shown.stop, Some(kept));
    let started = block_on(machines.start_sessions(&id("devbox-1"))).expect("the stop is cancelled");
    assert_eq!(started, Started { cancelled: true });
}

#[test]
fn a_stopped_session_takes_no_typing() {
    assert!(!Status::Stopped.accepts_input());
    assert_ne!(Status::Stopped.label(), Status::Offline.label());
}

#[test]
fn who_stopped_is_named_by_the_machine_or_session_people_know() {
    let workspace = fixture::standard();
    let mac = workspace.machine(&id("this-mac")).map(|found| found.name.clone()).expect("this-mac exists");
    let login = workspace
        .session(&SessionId::new("s-login"))
        .map(|found| found.title.clone())
        .expect("s-login exists");

    let terminal = StoppedBy::Terminal { machine: id("this-mac") }.phrase(&workspace);
    let agent = StoppedBy::Agent { session: SessionId::new("s-login") }.phrase(&workspace);
    let you = StoppedBy::You.phrase(&workspace);

    assert!(terminal.contains(&mac), "{terminal}");
    assert!(agent.contains(&login), "{agent}");
    assert!(!you.contains(&mac) && !you.contains(&login), "{you}");
    // A machine nobody knows is named by its id rather than left blank.
    let unknown = StoppedBy::Terminal { machine: id("ma_9") }.phrase(&workspace);
    assert!(unknown.contains("ma_9"), "{unknown}");
}

/// A pending request from a session on `machine`, made from one the fixture has.
fn pending_on(daemon: &FakeDaemon, machine: &str, name: &str) -> RequestId {
    let workspace = daemon.workspace();
    let session = workspace
        .sessions
        .iter()
        .find(|session| session.machine == id(machine) && !session.archived)
        .map(|session| session.id.clone())
        .expect("the machine has a session");
    let template: Request = workspace
        .requests
        .iter()
        .find(|request| request.kind != RequestKind::Migrate)
        .cloned()
        .expect("the fixture has a request");
    let request = Request { id: RequestId::new(name), session, answer: None, ..template };
    let made = request.id.clone();
    daemon.put_request(request);
    made
}

fn has_request(daemon: &FakeDaemon, request: &RequestId) -> bool {
    daemon.workspace().requests.iter().any(|found| &found.id == request)
}

#[test]
fn a_delivered_stop_ends_its_sessions_pending_requests_and_leaves_other_machines() {
    let (daemon, machines) = fakes();
    let there = pending_on(&daemon, "gpu-box", "r-gpu");
    let elsewhere = pending_on(&daemon, "this-mac", "r-mac");

    let stop = block_on(machines.stop_sessions(&id("gpu-box"))).expect("the stop is made");

    assert!(stop.delivered);
    assert!(!has_request(&daemon, &there));
    assert!(has_request(&daemon, &elsewhere));
}

#[test]
fn an_undelivered_stop_keeps_the_pending_requests() {
    let (daemon, machines) = fakes();
    let kept = pending_on(&daemon, "ci-runner", "r-ci");

    let stop = block_on(machines.stop_sessions(&id("ci-runner"))).expect("the stop is made");

    assert!(!stop.delivered);
    assert!(has_request(&daemon, &kept));
}
