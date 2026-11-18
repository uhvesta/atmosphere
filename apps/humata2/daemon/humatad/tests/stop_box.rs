//! Stop all sessions on a box from the host, and start them again (STOP,
//! D261, D262): the host reaches the box over its link and over a one-off
//! SSH at once, so a box whose link hangs is still stopped; a box reachable
//! neither way gets the stop the next time it links, once only, even after
//! the host restarts. On the box itself, `humatad2 stop` stops it even when
//! its daemon doesn't answer.
//!
//! A real host and a real box daemon on the multi-machine rig. The box's
//! daemon is the admin build for this test process, so the test reads the
//! box's own state as its user; the host relays and runs SSH as shipped.
//! The box restarts itself (same process id) after a stop and after a
//! start, so the waits ask its socket until the new run answers with what
//! only it can say.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Daemon, binary, holder_answers};
use humata2_grpc::humata::v1 as pb;
use humata2_store::TabRecord;
use hyper_util::rt::TokioIo;
use remote_rig::{LONG, Remote, Rig, is_live};
use rustix::process::{Pid, Signal, kill_process, kill_process_group};
use tokio::net::UnixStream;
use tonic::Status;
use tonic::transport::{Channel, Endpoint, Uri};

const REPO: (&str, &str) = ("acme", "api");
const BOX: &str = "devbox";

// ---------------------------------------------------------------------------
// Processes.

/// Whether process `pid` still runs. One that has ended and waits to be
/// reaped (a zombie) doesn't; one paused with SIGSTOP does.
fn alive(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let state = String::from_utf8_lossy(&output.stdout);
    let state = state.trim();
    !state.is_empty() && !state.starts_with('Z')
}

/// The running processes of `remote`'s test agent, as `ps` lines: its
/// script lives in the box's own home folder, so no other agent matches.
fn agent_processes(remote: &Remote) -> Vec<String> {
    let script = format!("{}/agent.sh", remote.home().display());
    let output = Command::new("ps")
        .args(["-A", "-ww", "-o", "pid=", "-o", "stat=", "-o", "args="])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.contains(&script))
        .filter(|line| !line.split_whitespace().nth(1).is_some_and(|state| state.starts_with('Z')))
        .map(str::to_owned)
        .collect()
}

/// Waits until `done` holds, asking again every 50 ms; `what` names it in
/// the failure.
async fn until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + LONG;
    while !done() {
        assert!(Instant::now() < deadline, "{what} never happened within {LONG:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A link paused with SIGSTOP: it stays up and answers nothing, as a link
/// flooded by the box's agents or stuck in the network does. Dropping it
/// ends the link's processes, which are no use to anyone once paused; the
/// host dials again by itself.
struct Frozen(Pid);

impl Frozen {
    fn link_of(remote: &Remote) -> Self {
        let group = remote
            .link_group()
            .unwrap_or_else(|| panic!("{}'s link left no process group; is its link up?", remote.name));
        kill_process_group(group, Signal::STOP).expect("the link's processes can be paused");
        Self(group)
    }
}

impl Drop for Frozen {
    fn drop(&mut self) {
        let _ = kill_process_group(self.0, Signal::KILL);
    }
}

// ---------------------------------------------------------------------------
// The box's own daemon, asked across its own restarts.

/// GetServerInfo, or `None` while the daemon is between two runs of itself.
async fn try_info(daemon: &Daemon) -> Option<pb::ServerInfo> {
    let socket = daemon.socket.clone();
    let channel = Endpoint::try_from("http://[::]:50051")
        .ok()?
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let socket = socket.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(socket).await?)) }
        }))
        .await
        .ok()?;
    pb::daemon_service_client::DaemonServiceClient::new(channel)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .ok()
        .map(tonic::Response::into_inner)
}

/// Waits until the box says its sessions are stopped by `stop_id` and its
/// sweep has run; returns the stop as the sweep left it.
async fn wait_box_stopped(daemon: &Daemon, stop_id: &str) -> pb::SessionsStopped {
    let deadline = Instant::now() + LONG;
    loop {
        let swept = try_info(daemon)
            .await
            .and_then(|info| info.last_start)
            .and_then(|start| start.sessions_stopped)
            .filter(|stopped| stopped.ended.is_some());
        if let Some(stopped) = swept {
            assert_eq!(stopped.stop_id, stop_id, "the box is stopped by another stop");
            return stopped;
        }
        assert!(Instant::now() < deadline, "the box never said its sessions are stopped");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits until the box has started its sessions again after stop
/// `stop_id`; returns that start.
async fn wait_box_started(daemon: &Daemon, stop_id: &str) -> pb::LastStart {
    let deadline = Instant::now() + LONG;
    loop {
        let start = try_info(daemon).await.and_then(|info| info.last_start);
        let after = start
            .as_ref()
            .and_then(|start| start.after_stop.as_ref())
            .and_then(|after| after.stop.as_ref())
            .map(|stop| stop.stop_id.clone());
        if let (Some(start), Some(after)) = (start, after) {
            assert_eq!(after, stop_id, "the box started again after another stop");
            return start;
        }
        assert!(Instant::now() < deadline, "the box never started its sessions again");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Whether the box holds a stop record (D261: `<state>/stop/this.json`):
/// written before a stop is answered, so a stop taken shows here at once.
fn box_has_stop_record(remote: &Remote) -> bool {
    remote.daemon().state.join("stop").join("this.json").exists()
}

/// Whether one of the box's restart reports says it came after `stop_id`.
fn box_report_names(remote: &Remote, stop_id: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(remote.daemon().state.join("restarts")) else {
        return false;
    };
    entries.flatten().any(|entry| {
        std::fs::read_to_string(entry.path().join("restart.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|json| json["after_stop"].is_object() && json["after_stop"].to_string().contains(stop_id))
    })
}

// ---------------------------------------------------------------------------
// The host.

async fn machines(rig: &Rig) -> pb::machine_service_client::MachineServiceClient<Channel> {
    pb::machine_service_client::MachineServiceClient::new(rig.channel().await)
}

/// The user stops `name`'s sessions from the host; the reply.
async fn stop_box(rig: &Rig, name: &str, stop_id: &str) -> Result<pb::StopMachineSessionsResponse, Status> {
    machines(rig)
        .await
        .stop_machine_sessions(pb::StopMachineSessionsRequest {
            machine: Some(Rig::box_id(name)),
            stop_id: stop_id.to_owned(),
            reason: "agents running wild".to_owned(),
            via: pb::StopVia::App as i32,
        })
        .await
        .map(tonic::Response::into_inner)
}

/// The user starts `name`'s sessions again from the host; the reply.
async fn start_box(rig: &Rig, name: &str) -> Result<pb::StartMachineSessionsResponse, Status> {
    machines(rig)
        .await
        .start_machine_sessions(pb::StartMachineSessionsRequest {
            machine: Some(Rig::box_id(name)),
        })
        .await
        .map(tonic::Response::into_inner)
}

/// The machine `name` as the host lists it.
async fn host_machine(rig: &Rig, name: &str) -> pb::Machine {
    machines(rig)
        .await
        .list_machines(pb::ListMachinesRequest::default())
        .await
        .expect("the host lists its machines")
        .into_inner()
        .machines
        .into_iter()
        .find(|machine| machine.name == name)
        .unwrap_or_else(|| panic!("the host doesn't list {name}"))
}

/// The machine `name` once the host shows `want` of it.
async fn wait_host_machine(rig: &Rig, name: &str, what: &str, want: impl Fn(&pb::Machine) -> bool) -> pb::Machine {
    let deadline = Instant::now() + LONG;
    loop {
        let machine = host_machine(rig, name).await;
        if want(&machine) {
            return machine;
        }
        assert!(Instant::now() < deadline, "the host never showed {name} {what}; it shows {machine:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Whether the host shows `machine`'s sessions stopped by `stop_id`, and
/// whether that stop reached it.
fn shows_stop(machine: &pb::Machine, stop_id: &str, delivered: bool) -> bool {
    machine.sessions_stopped.as_ref().is_some_and(|stop| stop.stop_id == stop_id) && machine.stop_delivered == delivered
}

// ---------------------------------------------------------------------------
// A host and a box with one agent session on the box.

struct Fixture {
    rig: Rig,
    id: pb::SessionId,
    /// The box's tabs as stored before any stop.
    before: Vec<TabRecord>,
}

impl Fixture {
    async fn new() -> Self {
        Self::on(Rig::start_seen(&[BOX], &[REPO]).await).await
    }

    /// As [`Self::new`], the host started with `env` (its first start only).
    async fn with_host_env(env: &[(&str, &str)]) -> Self {
        Self::on(Rig::start_seen_with_host_env(&[BOX], &[REPO], env).await).await
    }

    async fn on(rig: Rig) -> Self {
        let on = Rig::box_id(BOX);
        let channel = rig.create_channel("Payments", &[REPO], &[on.clone()]).await;
        let id = rig.start_on(&channel, &on, "Fix login").await;
        rig.wait_session(&id, "live", is_live).await;
        let remote = rig.the(BOX);
        until("the box's agent program starting", || !agent_processes(remote).is_empty()).await;
        let before = remote.tabs();
        assert!(!before.is_empty(), "the box stores the session's tabs");
        assert!(before.iter().all(holder_answers), "every holder on the box answers before the stop");
        Self { rig, id, before }
    }

    fn remote(&self) -> &Remote {
        self.rig.the(BOX)
    }

    /// Nothing the box's sessions started still runs, and the sweep says
    /// it left nothing.
    fn assert_box_ended(&self, stopped: &pb::SessionsStopped) {
        let ended = stopped.ended.as_ref().expect("the sweep says what it ended");
        assert_eq!(ended.left, 0, "{ended:?}");
        assert!(ended.agents >= 1, "the box's agent counts as ended: {ended:?}");
        for tab in &self.before {
            assert!(!holder_answers(tab), "the holder of tab {} on the box still answers", tab.id);
            assert!(!alive(tab.holder_pid), "holder {} of tab {} on the box still runs", tab.holder_pid, tab.id);
        }
        let agents = agent_processes(self.remote());
        assert!(agents.is_empty(), "the box's agent still runs: {agents:?}");
    }

    /// As [`Self::assert_box_ended`] for a stop cut short on its way: the
    /// sweep may count fewer agents than it ended, but leaves nothing.
    fn assert_box_quiet(&self, stopped: &pb::SessionsStopped) {
        let ended = stopped.ended.as_ref().expect("the sweep says what it ended");
        assert_eq!(ended.left, 0, "{ended:?}");
        for tab in &self.before {
            assert!(!holder_answers(tab), "the holder of tab {} on the box still answers", tab.id);
            assert!(!alive(tab.holder_pid), "holder {} of tab {} on the box still runs", tab.holder_pid, tab.id);
        }
        let agents = agent_processes(self.remote());
        assert!(agents.is_empty(), "the box's agent still runs: {agents:?}");
    }

    /// The box was never stopped: no stop record, every holder from before
    /// still answers, and the host doesn't show the session stopped. A
    /// linked box's session reads live; an unlinked one's reads offline.
    async fn assert_box_still_running(&self, linked: bool) {
        assert!(!box_has_stop_record(self.remote()), "the box holds a stop record");
        for tab in &self.before {
            assert!(holder_answers(tab), "the holder of tab {} on the box no longer answers", tab.id);
        }
        assert!(!agent_processes(self.remote()).is_empty(), "the box's agent still runs");
        let session = self.rig.session(&self.id).await;
        assert_ne!(
            session.status,
            pb::SessionStatus::MachineStopped as i32,
            "the host shows the session stopped before the box got the stop"
        );
        if linked {
            assert!(is_live(&session), "the session still reads live");
        }
    }
}

// ---------------------------------------------------------------------------
// 1. A linked box.

#[tokio::test(flavor = "multi_thread")]
async fn a_host_stops_a_linked_boxs_sessions_and_starts_them_again_with_a_report_naming_the_stop() {
    if !common::in_this_shard(0) {
        return;
    }
    let fix = Fixture::new().await;
    let stop_id = "0b6a3c1e-4d2f-4b8a-9e51-7c3d2a1f0b01";

    let reply = stop_box(&fix.rig, BOX, stop_id).await.expect("the stop is taken");
    assert!(reply.delivered, "a linked box gets the stop");
    assert_eq!(reply.stopped.expect("the reply carries the stop").stop_id, stop_id);

    let stopped = wait_box_stopped(fix.remote().daemon(), stop_id).await;
    assert_eq!(stopped.by.as_ref().expect("the stop names who asked").kind(), pb::StoppedByKind::User);
    fix.assert_box_ended(&stopped);
    wait_host_machine(&fix.rig, BOX, "with its sessions stopped", |machine| shows_stop(machine, stop_id, true)).await;
    fix.rig
        .wait_session(&fix.id, "stopped with its machine", |session| {
            session.status == pb::SessionStatus::MachineStopped as i32
        })
        .await;

    start_box(&fix.rig, BOX).await.expect("the start is taken");

    wait_box_started(fix.remote().daemon(), stop_id).await;
    fix.rig.wait_session(&fix.id, "live again", is_live).await;
    until("the box's agent program running again", || !agent_processes(fix.remote()).is_empty()).await;
    assert!(box_report_names(fix.remote(), stop_id), "the box's restart report names the stop");
    wait_host_machine(&fix.rig, BOX, "with its sessions running", |machine| machine.sessions_stopped.is_none()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_the_box_has_already_been_started_from_is_not_applied_again() {
    if !common::in_this_shard(1) {
        return;
    }
    let fix = Fixture::new().await;
    let stop_id = "0b6a3c1e-4d2f-4b8a-9e51-7c3d2a1f0b02";
    stop_box(&fix.rig, BOX, stop_id).await.expect("the stop is taken");
    wait_box_stopped(fix.remote().daemon(), stop_id).await;
    start_box(&fix.rig, BOX).await.expect("the start is taken");
    wait_box_started(fix.remote().daemon(), stop_id).await;
    fix.rig.wait_session(&fix.id, "live again", is_live).await;
    let before = fix.remote().tabs();

    // The same stop arriving late at the box, as an SSH that lost the race
    // to the link would bring it after the user's Start.
    let _ = pb::daemon_service_client::DaemonServiceClient::new(fix.remote().daemon().channel().await)
        .stop_sessions(pb::StopSessionsRequest {
            stop_id: stop_id.to_owned(),
            reason: String::new(),
            via: pb::StopVia::Ssh as i32,
            ..Default::default()
        })
        .await;

    assert!(!box_has_stop_record(fix.remote()), "the late stop was applied again");
    for tab in &before {
        assert!(holder_answers(tab), "the holder of tab {} on the box no longer answers", tab.id);
    }
}

// ---------------------------------------------------------------------------
// 2. A box whose link hangs.

#[tokio::test(flavor = "multi_thread")]
async fn a_box_whose_link_hangs_is_still_stopped_through_its_one_off_ssh() {
    if !common::in_this_shard(2) {
        return;
    }
    let fix = Fixture::new().await;
    let stop_id = "0b6a3c1e-4d2f-4b8a-9e51-7c3d2a1f0b03";
    // Declared after the fixture, so the paused link ends before the rig.
    let _frozen = Frozen::link_of(fix.remote());

    let reply = stop_box(&fix.rig, BOX, stop_id).await.expect("the stop is taken");

    // The link answered nothing, so only the SSH can have carried it.
    assert!(reply.delivered, "a box reachable over SSH gets the stop");
    let stopped = wait_box_stopped(fix.remote().daemon(), stop_id).await;
    fix.assert_box_ended(&stopped);
    assert!(shows_stop(&host_machine(&fix.rig, BOX).await, stop_id, true));
}

// ---------------------------------------------------------------------------
// 3. A box reachable neither way.

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_that_reaches_a_box_neither_way_is_kept_across_a_host_restart_and_lands_once_when_the_box_links_back() {
    if !common::in_this_shard(3) {
        return;
    }
    let mut fix = Fixture::new().await;
    let stop_id = "0b6a3c1e-4d2f-4b8a-9e51-7c3d2a1f0b04";
    // Every SSH to the box, its link and the one-off alike, now waits.
    fix.rig.link_lost(BOX).await;

    let reply = stop_box(&fix.rig, BOX, stop_id).await.expect("the stop is kept");
    assert!(!reply.delivered, "nothing reached the box");
    assert_eq!(reply.stopped.expect("the reply carries the stop").stop_id, stop_id);
    assert!(shows_stop(&host_machine(&fix.rig, BOX).await, stop_id, false));
    fix.assert_box_still_running(false).await;

    fix.rig.restart_host_unlinked(&[]).await;
    assert!(
        shows_stop(&host_machine(&fix.rig, BOX).await, stop_id, false),
        "the host still holds the undelivered stop after its restart"
    );

    fix.rig.link_back(BOX).await;
    let stopped = wait_box_stopped(fix.remote().daemon(), stop_id).await;
    fix.assert_box_ended(&stopped);
    wait_host_machine(&fix.rig, BOX, "with the stop delivered", |machine| shows_stop(machine, stop_id, true)).await;

    // Once only: started again, the box stays running through another link.
    start_box(&fix.rig, BOX).await.expect("the start is taken");
    wait_box_started(fix.remote().daemon(), stop_id).await;
    fix.rig.wait_session(&fix.id, "live again", is_live).await;
    let running = fix.remote().tabs();
    fix.rig.link_lost(BOX).await;
    fix.rig.link_back(BOX).await;
    assert!(!box_has_stop_record(fix.remote()), "the box was stopped again");
    for tab in &running {
        assert!(holder_answers(tab), "the holder of tab {} on the box no longer answers", tab.id);
    }
    assert!(host_machine(&fix.rig, BOX).await.sessions_stopped.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn starting_a_box_whose_stop_was_never_delivered_cancels_the_stop() {
    if !common::in_this_shard(4) {
        return;
    }
    let fix = Fixture::new().await;
    let stop_id = "0b6a3c1e-4d2f-4b8a-9e51-7c3d2a1f0b05";
    fix.rig.link_lost(BOX).await;
    let reply = stop_box(&fix.rig, BOX, stop_id).await.expect("the stop is kept");
    assert!(!reply.delivered);

    let started = start_box(&fix.rig, BOX).await.expect("the start is taken");

    assert!(started.cancelled, "the start only cancelled the stop the box never got");
    assert!(host_machine(&fix.rig, BOX).await.sessions_stopped.is_none());
    fix.rig.link_back(BOX).await;
    fix.assert_box_still_running(true).await;
}

// ---------------------------------------------------------------------------
// 4. `humatad2 stop` on the box, its daemon not answering.

/// Runs `humatad2 stop` on the box as its user at a terminal would, on the
/// box daemon's state and socket; its exit and what it printed.
async fn humatad2_stop_on(remote: &Remote, stop_id: &str) -> std::process::Output {
    humatad2_stop_run(remote, stop_id, "HUMATAD2_RELEASE", &[]).await
}

/// As [`humatad2_stop_on`], with the build `binary_variable` names and `env`
/// added to its environment.
async fn humatad2_stop_run(
    remote: &Remote,
    stop_id: &str,
    binary_variable: &str,
    env: &[(String, String)],
) -> std::process::Output {
    let mut command = Command::new(binary(binary_variable));
    command
        .args(["stop", "--id", stop_id, "--state"])
        .arg(&remote.daemon().state)
        .arg("--socket")
        .arg(&remote.daemon().socket)
        .env("HOME", remote.home())
        .envs(env.iter().map(|(name, value)| (name, value)));
    let run = tokio::task::spawn_blocking(move || command.output());
    tokio::time::timeout(LONG, run)
        .await
        .expect("`humatad2 stop` ends")
        .expect("the run finished")
        .expect("`humatad2 stop` runs")
}

#[tokio::test(flavor = "multi_thread")]
async fn humatad2_stop_on_a_box_whose_daemon_does_not_answer_leaves_it_stopped_once_the_daemon_is_started_again() {
    if !common::in_this_shard(5) {
        return;
    }
    let mut fix = Fixture::new().await;
    let stop_id = "0b6a3c1e-4d2f-4b8a-9e51-7c3d2a1f0b06";
    let daemon = fix.remote().daemon().pid().expect("the box's daemon runs");
    // Paused: its socket takes the connection and answers nothing.
    kill_process(Pid::from_raw(i32::try_from(daemon).unwrap()).unwrap(), Signal::STOP)
        .expect("the box's daemon can be paused");

    let output = humatad2_stop_on(fix.remote(), stop_id).await;

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!alive(daemon), "the daemon that didn't answer still runs");
    let holders: Vec<u32> = fix.before.iter().map(|tab| tab.holder_pid).collect();
    until("every holder on the box ending", || holders.iter().all(|pid| !alive(*pid))).await;

    // The box's service manager starts its daemon again, into the sweep.
    fix.rig.restart_box_env(BOX, &[]).await;
    let stopped = wait_box_stopped(fix.remote().daemon(), stop_id).await;
    assert_eq!(
        stopped.by.as_ref().expect("the stop names who asked").kind(),
        pb::StoppedByKind::Terminal
    );
    fix.assert_box_ended(&stopped);
    // The host learns of a stop made on the box when the box links.
    wait_host_machine(&fix.rig, BOX, "with the box's own stop", |machine| shows_stop(machine, stop_id, true)).await;
}

// ---------------------------------------------------------------------------
// 5. Cut short on the way (D504): the host, or `humatad2 stop` on the box,
// ends at each point of a stop or start that crosses machines. Once things
// run again, the box is stopped once by the stop, the host says so, and a
// Start brings the session back on its conversation.

/// How a run is cut short at a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cut {
    /// The process is killed there; whatever else runs carries on.
    Crash,
    /// The power goes there: what the process wrote and didn't sync is
    /// undone (humata2_durable's simulation, on the faults builds), and on
    /// the box every process dies with it.
    Power,
}

/// The environment that ends a run at `point` the way `cut` says, for a
/// state folder `state`, channels folder `channels` and test folder `dir`.
fn cut_env(state: &Path, channels: &Path, dir: &Path, point: &str, cut: Cut) -> Vec<(String, String)> {
    let mut env = vec![("HUMATA2_STOP_AT".to_owned(), point.to_owned())];
    if cut == Cut::Power {
        let roots = std::env::join_paths([state, channels]).expect("the test folders' paths hold no ':'");
        env.push(("HUMATA2_POWER_CUT_ROOTS".to_owned(), roots.to_string_lossy().into_owned()));
        // On the same file system as the roots, as the simulation needs.
        env.push(("HUMATA2_POWER_CUT_SPARE".to_owned(), dir.join("spare").display().to_string()));
    }
    env
}

/// The host's environment that ends it at `point`; the rig fills in the
/// host's own folders.
fn host_cut_env(point: &str, cut: Cut) -> Vec<(&'static str, String)> {
    let mut env = vec![("HUMATA2_STOP_AT", point.to_owned())];
    if cut == Cut::Power {
        env.push(("HUMATA2_POWER_CUT_ROOTS", "{state}:{channels}".to_owned()));
        env.push(("HUMATA2_POWER_CUT_SPARE", "{dir}/spare".to_owned()));
    }
    env
}

/// The processes under `pid`: its children, theirs, and so on.
fn descendants(pid: u32) -> Vec<u32> {
    let output = Command::new("ps")
        .args(["-A", "-o", "pid=", "-o", "ppid="])
        .output()
        .expect("ps runs");
    let pairs: Vec<(u32, u32)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
        })
        .collect();
    let mut found = Vec::new();
    let mut parents = vec![pid];
    while let Some(parent) = parents.pop() {
        for &(child, of) in &pairs {
            if of == parent && !found.contains(&child) {
                found.push(child);
                parents.push(child);
            }
        }
    }
    found
}

/// Kills `pid` at once, if it still runs.
fn kill_now(pid: u32) {
    if let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) {
        let _ = kill_process(pid, Signal::KILL);
    }
}

/// The power goes on the box: every holder its sessions had, what they
/// ran, and its agent die at once. Its daemon goes when the rig restarts
/// it.
fn power_off_box(fix: &Fixture) {
    let mut holders: Vec<u32> = fix.before.iter().map(|tab| tab.holder_pid).collect();
    holders.extend(fix.remote().tabs().iter().map(|tab| tab.holder_pid));
    holders.sort_unstable();
    holders.dedup();
    let programs: Vec<u32> = holders.iter().flat_map(|&holder| descendants(holder)).collect();
    for &holder in &holders {
        kill_now(holder);
    }
    for program in programs {
        kill_now(program);
    }
    for line in agent_processes(fix.remote()) {
        if let Some(pid) = line.split_whitespace().next().and_then(|pid| pid.parse().ok()) {
            kill_now(pid);
        }
    }
}

/// The stop id of cut test `n`.
fn cut_stop_id(n: u8) -> String {
    format!("0b6a3c1e-4d2f-4b8a-9e51-7c3d2a1f0b{n:02}")
}

/// The box has started again after `stop_id`: the session is live on
/// `conversation`, the box's report names the stop, and the host shows the
/// box's sessions running.
async fn assert_started_again(fix: &Fixture, stop_id: &str, conversation: &str) {
    wait_box_started(fix.remote().daemon(), stop_id).await;
    let session = fix.rig.wait_session(&fix.id, "live again", is_live).await;
    assert_eq!(session.conversation_id, conversation, "the session came back on another conversation");
    until("the box's agent program running again", || !agent_processes(fix.remote()).is_empty()).await;
    assert!(box_report_names(fix.remote(), stop_id), "the box's restart report names the stop");
    wait_host_machine(&fix.rig, BOX, "with its sessions running", |machine| machine.sessions_stopped.is_none()).await;
}

/// The host ends at `point` while it stops the box; started again, it
/// gets the stop to the box once and shows it delivered.
async fn host_cut_while_stopping(point: &str, cut: Cut, n: u8) {
    let env = host_cut_env(point, cut);
    let env: Vec<(&str, &str)> = env.iter().map(|(name, value)| (*name, value.as_str())).collect();
    let mut fix = Fixture::with_host_env(&env).await;
    let stop_id = cut_stop_id(n);
    let conversation = fix.rig.session(&fix.id).await.conversation_id;

    // The host ends on the way, so the stop may get no answer.
    let _ = stop_box(&fix.rig, BOX, &stop_id).await;
    fix.rig.host_stopped().await;
    fix.rig.restart_host(&[]).await;

    let stopped = wait_box_stopped(fix.remote().daemon(), &stop_id).await;
    assert_eq!(stopped.by.as_ref().expect("the stop names who asked").kind(), pb::StoppedByKind::User);
    fix.assert_box_ended(&stopped);
    wait_host_machine(&fix.rig, BOX, "with the stop delivered", |machine| shows_stop(machine, &stop_id, true)).await;

    start_box(&fix.rig, BOX).await.expect("the start is taken");
    assert_started_again(&fix, &stop_id, &conversation).await;
}

/// The host ends at `point` while it starts a stopped box; started again,
/// it shows the box running, and the box is.
async fn host_cut_while_starting(point: &str, cut: Cut, n: u8) {
    let env = host_cut_env(point, cut);
    let env: Vec<(&str, &str)> = env.iter().map(|(name, value)| (*name, value.as_str())).collect();
    let mut fix = Fixture::with_host_env(&env).await;
    let stop_id = cut_stop_id(n);
    let conversation = fix.rig.session(&fix.id).await.conversation_id;
    let reply = stop_box(&fix.rig, BOX, &stop_id).await.expect("the stop is taken");
    assert!(reply.delivered, "a linked box gets the stop");
    wait_box_stopped(fix.remote().daemon(), &stop_id).await;

    let _ = start_box(&fix.rig, BOX).await;
    fix.rig.host_stopped().await;
    fix.rig.restart_host(&[]).await;

    assert_started_again(&fix, &stop_id, &conversation).await;
}

/// `humatad2 stop` on the box, its daemon not answering, ends at `point`;
/// once the box's daemon is started again, the box is stopped by the stop,
/// the host learns of it, and a Start brings the session back.
async fn humatad2_stop_cut(point: &str, cut: Cut, n: u8) {
    let mut fix = Fixture::new().await;
    let stop_id = cut_stop_id(n);
    let conversation = fix.rig.session(&fix.id).await.conversation_id;
    let remote = fix.remote();
    let daemon = remote.daemon().pid().expect("the box's daemon runs");
    // Paused: its socket takes the connection and answers nothing.
    let paused = i32::try_from(daemon).ok().and_then(Pid::from_raw).expect("a process id");
    kill_process(paused, Signal::STOP).expect("the box's daemon can be paused");
    let state = remote.daemon().state.clone();
    let dir = state.parent().expect("the box's state is in its test folder").to_owned();
    let env = cut_env(&state, &remote.channels(), &dir, point, cut);

    let output = humatad2_stop_run(remote, &stop_id, "HUMATAD2_FAULTS", &env).await;

    assert!(!output.status.success(), "`humatad2 stop` ran on past {point}");
    if cut == Cut::Power {
        power_off_box(&fix);
    }
    // The box's service manager starts its daemon again, into the sweep.
    fix.rig.restart_box_env(BOX, &[]).await;
    let stopped = wait_box_stopped(fix.remote().daemon(), &stop_id).await;
    assert_eq!(
        stopped.by.as_ref().expect("the stop names who asked").kind(),
        pb::StoppedByKind::Terminal
    );
    fix.assert_box_quiet(&stopped);
    wait_host_machine(&fix.rig, BOX, "with the box's own stop", |machine| shows_stop(machine, &stop_id, true)).await;

    start_box(&fix.rig, BOX).await.expect("the start is taken");
    assert_started_again(&fix, &stop_id, &conversation).await;
}

macro_rules! cut_tests {
    ($run:ident: $($name:ident => $point:literal, $cut:ident, $n:literal;)*) => {$(
        #[tokio::test(flavor = "multi_thread")]
        async fn $name() {
            $run($point, Cut::$cut, $n).await;
        }
    )*};
}

cut_tests! { host_cut_while_stopping:
    a_host_that_crashes_once_its_copy_of_a_stop_is_saved_stops_the_box_once_back => "stop.remote.copy_written", Crash, 7;
    a_host_that_loses_power_once_its_copy_of_a_stop_is_saved_stops_the_box_once_back => "stop.remote.copy_written", Power, 8;
    a_host_that_crashes_once_the_box_took_a_stop_shows_it_delivered_once_back => "stop.remote.sent", Crash, 9;
    a_host_that_loses_power_once_the_box_took_a_stop_shows_it_delivered_once_back => "stop.remote.sent", Power, 10;
    a_host_that_crashes_once_a_stop_is_marked_delivered_keeps_it_once_back => "stop.remote.delivered", Crash, 11;
    a_host_that_loses_power_once_a_stop_is_marked_delivered_keeps_it_once_back => "stop.remote.delivered", Power, 12;
}

cut_tests! { host_cut_while_starting:
    a_host_that_crashes_once_it_told_the_box_to_start_shows_it_running_once_back => "start.remote.sent", Crash, 13;
    a_host_that_loses_power_once_it_told_the_box_to_start_shows_it_running_once_back => "start.remote.sent", Power, 14;
    a_host_that_crashes_once_its_copy_of_a_stop_is_dropped_shows_the_box_running_once_back => "start.remote.copy_removed", Crash, 15;
    a_host_that_loses_power_once_its_copy_of_a_stop_is_dropped_shows_the_box_running_once_back => "start.remote.copy_removed", Power, 16;
}

cut_tests! { humatad2_stop_cut:
    humatad2_stop_that_crashes_once_its_record_is_written_leaves_the_box_stopped => "stop.remote.cli.record_written", Crash, 17;
    humatad2_stop_that_loses_power_once_its_record_is_written_leaves_the_box_stopped => "stop.remote.cli.record_written", Power, 18;
    humatad2_stop_that_crashes_once_the_daemon_is_ended_leaves_the_box_stopped => "stop.remote.cli.daemon_killed", Crash, 19;
    humatad2_stop_that_loses_power_once_the_daemon_is_ended_leaves_the_box_stopped => "stop.remote.cli.daemon_killed", Power, 20;
    humatad2_stop_that_crashes_once_the_holders_are_signalled_leaves_the_box_stopped => "stop.remote.cli.holders_signalled", Crash, 21;
    humatad2_stop_that_loses_power_once_the_holders_are_signalled_leaves_the_box_stopped => "stop.remote.cli.holders_signalled", Power, 22;
}

