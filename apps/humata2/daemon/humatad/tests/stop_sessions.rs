//! Stop all sessions on a machine, and start them again (STOP, D260–D265),
//! through a real `humatad2` on the host: a stop ends every process the
//! machine's sessions started and keeps them stopped across daemon
//! restarts; a start brings them back as after a shutdown, with a restart
//! report that names the stop. A stop or a start cut at any of its fault
//! points, by a crash or by a power cut, still ends stopped or running,
//! with nothing lost.
//!
//! The daemon restarts itself (same process id) after a stop and after a
//! start, so the waits here ask the socket until the new run answers with
//! what only it can say (`sessions_stopped.ended`, `after_stop`); a call
//! that fails while the daemon is between runs is asked again.

mod common;

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, World, as_script, create_channel, create_script_token, folder,
    get_session, holder_answers, humata_as_agent, last_start, machine_id, open, pending_requests,
    shell_token, start_request, start_session, stored_tabs, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use rustix::process::{Pid, Signal, kill_process};
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Code, Request, Status};

/// How long a stop's sweep or a start's bring-back may take on a loaded
/// machine: a safety net, never a time anything is expected to take.
const LONG: Duration = Duration::from_secs(90);

const TOKEN_HEADER: &str = "humata-session-token";

/// The fault points of a stop, in the order a stop reaches them.
const STOP_POINTS: [&str; 7] = [
    "stop.record.written",
    "stop.exec",
    "stop.programs.hungup",
    "stop.holders.signalled",
    "stop.inbox.withdrawn",
    "stop.holders.gone",
    "stop.record.settled",
];

/// The fault points of a start, in the order a start reaches them.
const START_POINTS: [&str; 4] = [
    "start.record.renamed",
    "start.exec",
    "start.report.written",
    "start.record.removed",
];

// ---------------------------------------------------------------------------
// Processes.

/// Whether process `pid` still runs. One that has ended and waits to be
/// reaped (a zombie) doesn't.
fn alive(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let state = String::from_utf8_lossy(&output.stdout);
    let state = state.trim();
    !state.is_empty() && !state.starts_with('Z')
}

/// The running processes of the world's test agent, as `ps` lines: its
/// script lives in the daemon's own folder, so no other test's agent
/// matches.
fn agent_processes(world: &World) -> Vec<String> {
    let folder = world
        .root
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the world's folder has a name");
    let script = format!("{folder}/agent.sh");
    let output = Command::new("ps")
        .args(["-A", "-ww", "-o", "pid=", "-o", "stat=", "-o", "args="])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.contains(&script))
        .filter(|line| {
            // pid, state, then the command.
            !line.split_whitespace().nth(1).is_some_and(|state| state.starts_with('Z'))
        })
        .map(str::to_owned)
        .collect()
}

/// The restart report folders under the daemon's state.
fn report_folders(state: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(state.join("restarts")) else {
        return Vec::new();
    };
    let mut folders: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    folders
}

/// The `restart.json` of the report in `folder`, if it has one that reads.
fn restart_json(folder: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(folder.join("restart.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether `json` (a `restart.json`) says its start came after stop `stop_id`.
fn names_stop(json: &serde_json::Value, stop_id: &str) -> bool {
    let after = &json["after_stop"];
    after.is_object() && after.to_string().contains(stop_id)
}

// ---------------------------------------------------------------------------
// The daemon, asked across its own restarts.

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

/// Waits until the daemon says its sessions are stopped by stop `stop_id`
/// and the sweep has run; returns that start and the stop.
async fn wait_stopped(daemon: &Daemon, stop_id: &str) -> (pb::LastStart, pb::SessionsStopped) {
    let deadline = Instant::now() + LONG;
    loop {
        let start = try_info(daemon).await.and_then(|info| info.last_start);
        let swept = start
            .as_ref()
            .and_then(|start| start.sessions_stopped.clone())
            .filter(|stopped| stopped.ended.is_some());
        if let (Some(start), Some(stopped)) = (start, swept) {
            assert_eq!(stopped.stop_id, stop_id, "the daemon is stopped by another stop");
            return (start, stopped);
        }
        assert!(Instant::now() < deadline, "the daemon never said its sessions are stopped");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits until the daemon has started its sessions again after a stop and
/// brought everything back; returns that start.
async fn wait_started(daemon: &Daemon) -> pb::LastStart {
    let deadline = Instant::now() + LONG;
    loop {
        let start = try_info(daemon).await.and_then(|info| info.last_start);
        if let Some(start) = start.filter(|start| start.after_stop.is_some()) {
            return start;
        }
        assert!(Instant::now() < deadline, "the daemon never started its sessions again");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn daemon_service(daemon: &Daemon) -> pb::daemon_service_client::DaemonServiceClient<Channel> {
    pb::daemon_service_client::DaemonServiceClient::new(daemon.channel().await)
}

fn stop_request(stop_id: &str, reason: &str) -> pb::StopSessionsRequest {
    pb::StopSessionsRequest {
        stop_id: stop_id.to_owned(),
        reason: reason.to_owned(),
        via: pb::StopVia::App as i32,
        ..Default::default()
    }
}

/// The user stops the machine's sessions; the reply.
async fn try_stop(daemon: &Daemon, stop_id: &str, reason: &str) -> Result<pb::StopSessionsResponse, Status> {
    daemon_service(daemon)
        .await
        .stop_sessions(stop_request(stop_id, reason))
        .await
        .map(tonic::Response::into_inner)
}

/// The user starts the machine's sessions again; the reply.
async fn try_start(daemon: &Daemon) -> Result<pb::StartSessionsResponse, Status> {
    daemon_service(daemon)
        .await
        .start_sessions(pb::StartSessionsRequest {})
        .await
        .map(tonic::Response::into_inner)
}

/// Stops the sessions and waits for the sweep; returns the stopped start
/// and the stop as the sweep left it.
async fn stop_and_wait(daemon: &Daemon, stop_id: &str) -> (pb::LastStart, pb::SessionsStopped) {
    let reply = try_stop(daemon, stop_id, "agents running wild").await.expect("the stop is taken");
    let stopped = reply.stopped.expect("the reply carries the stop");
    assert_eq!(stopped.stop_id, stop_id);
    wait_stopped(daemon, stop_id).await
}

// ---------------------------------------------------------------------------
// A host with one agent session and one more shell tab in it.

struct Fixture {
    daemon: Daemon,
    world: World,
    channel: pb::Channel,
    id: pb::SessionId,
    /// The session's conversation before any stop.
    conversation: String,
    agent_tab: pb::TabId,
    shell_tab: pb::TabId,
    /// The process id of the shell tab's shell.
    shell_pid: u32,
    /// The session token the shell tab's shell was given: an agent's identity.
    token: String,
    /// The attach on the shell tab, to type into it before a stop.
    shell: Attach,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_env(&[]).await
    }

    /// As [`Self::new`], on a daemon started with `env` (`HUMATA2_STOP_AT`
    /// and the like get the fault build, and stay through the daemon's own
    /// restarts of itself).
    async fn with_env(env: &[(&str, &str)]) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        Self::in_world(dir, world, env).await
    }

    /// A fixture whose daemon stops at `point` the way `cut` says.
    async fn cut_at(point: &str, cut: Cut) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        // The daemon keeps its state in `<dir>/state` (common::Daemon).
        let env = cut_env(&dir.path().join("state"), &world.channels, dir.path(), point, cut);
        let refs: Vec<(&str, &str)> = env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
        Self::in_world(dir, world, &refs).await
    }

    async fn in_world(dir: tempfile::TempDir, world: World, env: &[(&str, &str)]) -> Self {
        world.remote("acme", "api");
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, env).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{setup:?}");
        let (id, _) = start_session(&daemon, &channel, "Fix login").await;
        let session = wait_running(&daemon, &id).await;
        let agent_tab = common::agent_tab(&daemon, &id).await;
        // The agent's program runs, not only its holder.
        Attach::start(&daemon, open(&agent_tab, 0)).await.wait_for("agent up").await;

        let (token, mut shell) = shell_token(&daemon, &id).await;
        let shell_tab = daemon
            .terminals()
            .await
            .list_tabs(pb::ListTabsRequest { session: Some(id.clone()) })
            .await
            .unwrap()
            .into_inner()
            .tabs
            .into_iter()
            .find(|tab| tab.kind == pb::tab::Kind::Shell as i32)
            .expect("the session has a shell tab")
            .id
            .unwrap();
        let shell_pid = shell_pid(&mut shell).await;

        Self {
            daemon,
            world,
            channel,
            id,
            conversation: session.conversation_id,
            agent_tab,
            shell_tab,
            shell_pid,
            token,
            shell,
        }
    }

    fn channel_id(&self) -> pb::ChannelId {
        self.channel.id.clone().expect("the channel has an id")
    }

    /// Says `line` to the agent and waits for its answer.
    async fn say(&self, line: &str) {
        let mut attach = Attach::start(&self.daemon, open(&self.agent_tab, 0)).await;
        attach.type_text(&format!("{line}\r")).await;
        attach.wait_for(&format!("got: {line}")).await;
    }

    /// What the agent's conversation file holds.
    fn heard(&self) -> String {
        std::fs::read_to_string(self.world.conversation(&self.conversation)).unwrap_or_default()
    }

    /// A request that carries the shell tab's session token: an agent's call.
    fn as_agent<T>(&self, message: T) -> Request<T> {
        let mut request = Request::new(message);
        request
            .metadata_mut()
            .insert(TOKEN_HEADER, MetadataValue::try_from(self.token.as_str()).unwrap());
        request
    }

    /// The agent asks the user a question; the call's end.
    async fn agent_asks(&self, question: &str) -> Result<pb::WaitResponse, Status> {
        self.daemon
            .inbox_service()
            .await
            .ask_user(self.as_agent(pb::AskUserRequest {
                question: question.to_owned(),
                ..Default::default()
            }))
            .await
            .map(tonic::Response::into_inner)
    }

    /// Nothing the machine's sessions started still runs: no holder that
    /// `before` (the tabs as stored before the stop) names answers or
    /// lives, the agent's program and the shell are gone, and the session
    /// reads stopped with its machine.
    async fn assert_nothing_runs(&self, before: &[humata2_store::TabRecord], stopped: &pb::SessionsStopped) {
        let ended = stopped.ended.as_ref().expect("the sweep says what it ended");
        assert_eq!(ended.left, 0, "{ended:?}");
        let mut tabs = before.to_vec();
        tabs.extend(stored_tabs(&self.daemon.state));
        for tab in &tabs {
            assert!(!holder_answers(tab), "the holder of tab {} still answers", tab.id);
            assert!(!alive(tab.holder_pid), "holder {} of tab {} still runs", tab.holder_pid, tab.id);
        }
        let agents = agent_processes(&self.world);
        assert!(agents.is_empty(), "the agent still runs: {agents:?}");
        assert!(!alive(self.shell_pid), "the shell {} still runs", self.shell_pid);
        let session = get_session(&self.daemon, &self.id).await;
        assert_eq!(session.status(), pb::SessionStatus::MachineStopped);
    }

    /// The session's agent runs on the conversation it had before the
    /// stop, with what it heard then, and still listens.
    async fn assert_agent_is_back(&self) {
        let session = wait_running(&self.daemon, &self.id).await;
        assert_eq!(session.conversation_id, self.conversation);
        assert!(!agent_processes(&self.world).is_empty(), "the agent's program runs again");
        self.say("after").await;
        assert!(self.heard().contains("after"));
    }

    /// The session runs as it did before a stop that never took hold.
    async fn assert_still_running(&self, before: &[humata2_store::TabRecord]) {
        for tab in before {
            assert!(holder_answers(tab), "the holder of tab {} no longer answers", tab.id);
        }
        assert!(alive(self.shell_pid), "the shell {} no longer runs", self.shell_pid);
        self.assert_agent_is_back().await;
    }
}

/// The process id of the shell `attach` types into.
async fn shell_pid(attach: &mut Attach) -> u32 {
    // The shell prints `shellpid=<pid>.`; the line as typed never holds
    // `shellpid=`, so the echo of the typing can't be taken for it.
    attach.type_text("echo \"shell\"\"pid=$$.\"\r").await;
    loop {
        let found = {
            let screen = String::from_utf8_lossy(&attach.screen);
            screen
                .split("shellpid=")
                .nth(1)
                .and_then(|rest| rest.split_once('.'))
                .and_then(|(digits, _)| digits.parse::<u32>().ok())
        };
        if let Some(pid) = found {
            return pid;
        }
        attach.read().await.expect("the shell tab's output goes on");
    }
}

/// How an attach to `tab` is refused: at the call, or as the stream's end.
async fn attach_refusal(daemon: &Daemon, tab: &pb::TabId) -> Status {
    match Attach::try_start(daemon, open(tab, 0)).await {
        Err(status) => status,
        Ok(mut attach) => attach.wait_for_error().await,
    }
}

// ---------------------------------------------------------------------------
// 1. A stop ends everything.

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_ends_every_agent_and_shell_the_sessions_started_and_writes_no_restart_report() {
    let fix = Fixture::new().await;
    last_start(&fix.daemon).await;
    let before = stored_tabs(&fix.daemon.state);
    assert_eq!(before.len(), 2, "an agent tab and a shell tab: {before:?}");
    assert!(before.iter().all(holder_answers));
    assert!(!agent_processes(&fix.world).is_empty(), "the agent's program runs before the stop");
    assert!(alive(fix.shell_pid));
    let reports = report_folders(&fix.daemon.state);

    let (start, stopped) = stop_and_wait(&fix.daemon, "5d0c1f6e-7a54-4c0f-9a52-2f3b8f6f0a01").await;

    let ended = stopped.ended.clone().expect("the sweep says what it ended");
    assert_eq!((ended.agents, ended.shells), (1, 1), "{ended:?}");
    fix.assert_nothing_runs(&before, &stopped).await;
    // A stopped start brings nothing back and writes no report.
    assert!(start.report.is_empty(), "{:?}", start.report);
    assert!(start.resumed.is_empty(), "{:?}", start.resumed);
    assert!(start.after_stop.is_none());
    assert_eq!(report_folders(&fix.daemon.state), reports);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_attached_when_the_sessions_stop_is_told_to_connect_again() {
    let mut fix = Fixture::new().await;
    try_stop(&fix.daemon, "0b6f2c1e-3d4a-4e5f-8a9b-1c2d3e4f5a6b", "agents running wild")
        .await
        .expect("the stop is taken");
    let replied = Instant::now();
    // The daemon ends the attach itself as it leaves, rather than holding
    // its leaving up until the connection is cut, 3 s on.
    let ended = fix.shell.wait_for_error().await;
    assert_eq!(ended.code(), Code::Unavailable, "{ended:?}");
    let after = replied.elapsed();
    assert!(after < Duration::from_millis(2500), "the attach ended {after:?} after the stop");
    wait_stopped(&fix.daemon, "0b6f2c1e-3d4a-4e5f-8a9b-1c2d3e4f5a6b").await;
}

// ---------------------------------------------------------------------------
// 2. While stopped, no agent or script is served and nothing is started.

#[tokio::test(flavor = "multi_thread")]
async fn while_stopped_agents_and_scripts_are_refused_and_nothing_can_be_typed_attached_or_started() {
    let fix = Fixture::new().await;
    let (_, script) = create_script_token(
        &fix.daemon,
        "ci",
        &[pb::Capability::GlobalRead, pb::Capability::ChannelRead],
        &[&fix.channel_id()],
        0,
    )
    .await
    .expect("the user makes a script token");
    // Before the stop each of these is served.
    let (code, json) = humata_as_agent(&fix.daemon, &fix.token, &["channels"]).await;
    assert_eq!(code, 0, "{json}");
    let listed = fix
        .daemon
        .channel_service()
        .await
        .list_channels(as_script(pb::ListChannelsRequest::default(), &script))
        .await;
    assert!(listed.is_ok(), "{listed:?}");

    stop_and_wait(&fix.daemon, "5d0c1f6e-7a54-4c0f-9a52-2f3b8f6f0a02").await;

    // The agent, over the CLI and over gRPC.
    let (code, json) = humata_as_agent(&fix.daemon, &fix.token, &["channels"]).await;
    assert!(code != 0 && code != 12, "the agent's call was served or left waiting: exit {code}, {json}");
    let refused = fix
        .daemon
        .channel_service()
        .await
        .list_channels(fix.as_agent(pb::ListChannelsRequest::default()))
        .await
        .expect_err("an agent is refused while stopped");
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    let refused = fix.agent_asks("May I go on?").await.expect_err("an agent can't ask while stopped");
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    assert!(pending_requests(&fix.daemon).await.is_empty());
    // A script.
    let refused = fix
        .daemon
        .channel_service()
        .await
        .list_channels(as_script(pb::ListChannelsRequest::default(), &script))
        .await
        .expect_err("a script is refused while stopped");
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");

    // The user can't attach to or type into a stopped tab either.
    for tab in [&fix.agent_tab, &fix.shell_tab] {
        let refused = attach_refusal(&fix.daemon, tab).await;
        assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    }
    let refused = fix
        .daemon
        .terminals()
        .await
        .r#type(pb::TypeRequest {
            session: Some(fix.id.clone()),
            tab: Some(fix.agent_tab.clone()),
            text: "keep going\r".to_owned(),
            when: pb::type_request::When::Now as i32,
            ..Default::default()
        })
        .await
        .expect_err("typing is refused while stopped");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(!fix.heard().contains("keep going"));

    // And no new session starts.
    let machine = machine_id(&fix.daemon).await;
    let refused = fix
        .daemon
        .session_service()
        .await
        .start_session(start_request(&fix.channel_id(), &machine, "Another"))
        .await
        .expect_err("a session can't start while stopped");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(agent_processes(&fix.world).is_empty());
}

// ---------------------------------------------------------------------------
// 3. The Inbox.

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_withdraws_the_agents_pending_inbox_request_as_stopped_with_its_machine() {
    let fix = Fixture::new().await;
    let asked = fix.agent_asks("Shall I drop the table?").await.expect("the agent asks");
    assert_eq!(asked.state(), pb::RequestState::Pending);
    let request = asked.id.expect("the request has an id");
    assert_eq!(pending_requests(&fix.daemon).await, vec![request.clone()]);

    stop_and_wait(&fix.daemon, "5d0c1f6e-7a54-4c0f-9a52-2f3b8f6f0a03").await;

    let after = fix
        .daemon
        .inbox_service()
        .await
        .get_request(pb::GetRequestRequest { id: Some(request) })
        .await
        .expect("the user still reads the request")
        .into_inner();
    assert_eq!(after.state(), pb::RequestState::Withdrawn);
    assert_eq!(after.withdraw_reason(), pb::WithdrawReason::MachineStopped);
    assert!(pending_requests(&fix.daemon).await.is_empty());
}

// ---------------------------------------------------------------------------
// 4. A daemon restart while stopped.

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_killed_and_started_again_while_stopped_stays_stopped_and_starts_nothing() {
    const STOP: &str = "5d0c1f6e-7a54-4c0f-9a52-2f3b8f6f0a04";
    let mut fix = Fixture::new().await;
    let before = stored_tabs(&fix.daemon.state);
    let (_, first) = stop_and_wait(&fix.daemon, STOP).await;
    let reports = report_folders(&fix.daemon.state);

    fix.daemon.restart().await;

    let (start, stopped) = wait_stopped(&fix.daemon, STOP).await;
    assert_eq!(stopped.stopped_at_unix_ms, first.stopped_at_unix_ms);
    assert_eq!(stopped.reason, first.reason);
    fix.assert_nothing_runs(&before, &stopped).await;
    assert!(start.report.is_empty(), "{:?}", start.report);
    assert!(start.resumed.is_empty(), "{:?}", start.resumed);
    assert_eq!(report_folders(&fix.daemon.state), reports);
}

// ---------------------------------------------------------------------------
// 5. Start.

#[tokio::test(flavor = "multi_thread")]
async fn start_brings_the_sessions_back_as_after_a_shutdown_with_a_report_naming_the_stop() {
    const STOP: &str = "5d0c1f6e-7a54-4c0f-9a52-2f3b8f6f0a05";
    let mut fix = Fixture::new().await;
    fix.say("one").await;
    // The typed line holds `kept-''line`; only the shell's output holds this.
    fix.shell.type_text("echo kept-''line\r").await;
    fix.shell.wait_for("kept-line").await;
    stop_and_wait(&fix.daemon, STOP).await;
    let reports = report_folders(&fix.daemon.state);

    try_start(&fix.daemon).await.expect("the start is taken");

    let start = wait_started(&fix.daemon).await;
    let after_stop = start.after_stop.clone().expect("the start names its stop");
    assert_eq!(after_stop.stop.expect("the stop it came after").stop_id, STOP);
    assert!(start.sessions_stopped.is_none());
    assert!(start.not_brought_back.is_empty(), "{:?}", start.not_brought_back);
    assert!(start.resumed.contains(&fix.id), "{:?}", start.resumed);
    // The agent is back on its conversation, resumed, with what it heard.
    let session = wait_running(&fix.daemon, &fix.id).await;
    assert_eq!(session.conversation_id, fix.conversation);
    let mut agent = Attach::start(&fix.daemon, open(&fix.agent_tab, 0)).await;
    agent.wait_for("agent up --resume").await;
    assert!(fix.heard().contains("one"));
    fix.say("two").await;
    assert!(fix.heard().contains("two"));
    // The shell tab is back with what it printed before the stop, and works.
    let mut shell = Attach::start(&fix.daemon, open(&fix.shell_tab, 0)).await;
    shell.wait_for("kept-line").await;
    shell.type_text("echo bac''k\r").await;
    shell.wait_for("back").await;
    // The start wrote a report of its own, and it names the stop.
    let report = Path::new(&start.report);
    assert!(report.is_dir(), "{:?}", start.report);
    assert_eq!(report_folders(&fix.daemon.state).len(), reports.len() + 1);
    let json = restart_json(report).expect("the report has a restart.json");
    assert!(names_stop(&json, STOP), "{json}");
}

// ---------------------------------------------------------------------------
// 6. A program that won't end.

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_that_ignores_hangup_and_terminate_is_killed_by_the_stop() {
    let mut fix = Fixture::new().await;
    let before = stored_tabs(&fix.daemon.state);
    // The typed line holds `trap''ped`; the shell prints this once the trap is set.
    fix.shell.type_text("trap '' HUP TERM; echo trap''ped\r").await;
    fix.shell.wait_for("trapped").await;

    let (_, stopped) = stop_and_wait(&fix.daemon, "5d0c1f6e-7a54-4c0f-9a52-2f3b8f6f0a06").await;

    let ended = stopped.ended.clone().expect("the sweep says what it ended");
    assert!(ended.killed >= 1, "{ended:?}");
    assert_eq!(ended.left, 0, "{ended:?}");
    fix.assert_nothing_runs(&before, &stopped).await;
}

// ---------------------------------------------------------------------------
// 7. Asked twice, or at the wrong time.

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_stop_acts_once_and_a_start_with_nothing_stopped_is_refused() {
    const STOP: &str = "5d0c1f6e-7a54-4c0f-9a52-2f3b8f6f0a07";
    let fix = Fixture::new().await;
    let before = stored_tabs(&fix.daemon.state);

    let refused = try_start(&fix.daemon).await.expect_err("nothing is stopped yet");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    fix.assert_still_running(&before).await;

    let (_, first) = stop_and_wait(&fix.daemon, STOP).await;
    let again = try_stop(&fix.daemon, STOP, "asked again, in other words")
        .await
        .expect("the repeated stop is answered")
        .stopped
        .expect("the reply carries the stop");

    // The repeat is the first stop, not another one.
    assert_eq!(again.stop_id, first.stop_id);
    assert_eq!(again.stopped_at_unix_ms, first.stopped_at_unix_ms);
    assert_eq!(again.reason, first.reason);
    let (_, still) = wait_stopped(&fix.daemon, STOP).await;
    assert_eq!(still.stopped_at_unix_ms, first.stopped_at_unix_ms);
    fix.assert_nothing_runs(&before, &still).await;
}

// ---------------------------------------------------------------------------
// 8. Cuts: the daemon killed, or the machine's power cut, at each fault
//    point of a stop and of a start.

/// How a cut at a fault point ends the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cut {
    /// The daemon is killed there; its holders and the programs they run
    /// carry on.
    Crash,
    /// The power goes there: whatever the daemon and the holders wrote and
    /// didn't sync is undone (humata2_durable's simulation, on the faults
    /// builds), and every process the machine ran dies with them.
    Power,
}

/// The environment that stops a daemon at `point` the way `cut` says,
/// for a daemon whose state is `state`, whose channels are `channels`,
/// and whose test folder is `dir`. It stays through the daemon's own
/// restarts of itself, which run in the same process.
///
/// The power-cut simulation keeps what wasn't synced in the process that
/// wrote it, so a write a stop leaves unsynced before it restarts itself
/// counts as kept: a pass here says nothing of those writes.
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

/// The power cut reaches the holder of `tab` if it still answers: SIGUSR2
/// makes the faults holder drop what it hadn't synced and die. Waits on
/// its socket closing, not on a time.
async fn cut_holder(tab: &humata2_store::TabRecord) {
    let Ok(mut connection) = UnixStream::connect(&tab.holder_socket).await else {
        return;
    };
    let Some(pid) = i32::try_from(tab.holder_pid).ok().and_then(Pid::from_raw) else {
        return;
    };
    if kill_process(pid, Signal::USR2).is_err() {
        // It ended on its own between the connect and the signal.
        return;
    }
    let gone = tokio::time::timeout(LONG, async {
        let mut buffer = [0u8; 4096];
        while matches!(connection.read(&mut buffer).await, Ok(read) if read > 0) {}
    })
    .await;
    assert!(gone.is_ok(), "the holder of tab {} never died at the power cut", tab.id);
}

/// What the power cut takes after the daemon: every holder of `tabs` (and
/// of the tabs stored now) that still answers, then every process that ran
/// under one of them, as the machine going dark would.
async fn power_off_sessions(fix: &Fixture, tabs: &[humata2_store::TabRecord]) {
    let mut tabs = tabs.to_vec();
    tabs.extend(stored_tabs(&fix.daemon.state));
    tabs.sort_by_key(|tab| tab.holder_pid);
    tabs.dedup_by_key(|tab| tab.holder_pid);
    let programs: Vec<u32> = tabs
        .iter()
        .filter(|tab| alive(tab.holder_pid))
        .flat_map(|tab| descendants(tab.holder_pid))
        .collect();
    for tab in &tabs {
        cut_holder(tab).await;
    }
    for program in programs {
        if let Some(pid) = i32::try_from(program).ok().and_then(Pid::from_raw) {
            // One that ended with its holder is fine.
            let _ = kill_process(pid, Signal::KILL);
        }
    }
}

/// A stop cut at `point` by `cut`, then the daemon started again. It ends
/// stopped with nothing running, and a start then brings everything back;
/// or, only where `may_still_run` (the cut came before the stop was
/// recorded), it ends running: as before after a crash, brought back as
/// after any outage after a power cut.
async fn a_stop_cut_at(point: &str, may_still_run: bool, cut: Cut) {
    const STOP: &str = "5d0c1f6e-7a54-4c0f-9a52-2f3b8f6f0a08";
    assert!(STOP_POINTS.contains(&point), "{point} is not a stop's fault point");
    let mut fix = Fixture::cut_at(point, cut).await;
    fix.say("one").await;
    let before = stored_tabs(&fix.daemon.state);

    // The daemon may die under the call.
    let _ = try_stop(&fix.daemon, STOP, "agents running wild").await;
    fix.daemon.stopped(LONG).await;
    if cut == Cut::Power {
        power_off_sessions(&fix, &before).await;
    }
    fix.daemon.restart().await;

    let start = last_start(&fix.daemon).await;
    let Some(stopped) = start.sessions_stopped.clone() else {
        assert!(may_still_run, "{point}: the stop was recorded, yet the sessions were started again");
        match cut {
            Cut::Crash => fix.assert_still_running(&before).await,
            Cut::Power => {
                assert!(start.not_brought_back.is_empty(), "{point}: {:?}", start.not_brought_back);
                fix.assert_agent_is_back().await;
            }
        }
        assert!(fix.heard().contains("one"), "{point}");
        return;
    };
    assert_eq!(stopped.stop_id, STOP, "{point}");
    fix.assert_nothing_runs(&before, &stopped).await;
    assert!(start.report.is_empty(), "{point}: {:?}", start.report);

    // Nothing was lost: a start brings it all back.
    try_start(&fix.daemon).await.expect("the start is taken");
    let back = wait_started(&fix.daemon).await;
    assert!(back.not_brought_back.is_empty(), "{point}: {:?}", back.not_brought_back);
    fix.assert_agent_is_back().await;
    assert!(fix.heard().contains("one"), "{point}");
}

/// A start cut at `point` by `cut`, then the daemon started again: the
/// sessions are back and running, and a restart report names the stop.
async fn a_start_cut_at(point: &str, cut: Cut) {
    const STOP: &str = "5d0c1f6e-7a54-4c0f-9a52-2f3b8f6f0a09";
    assert!(START_POINTS.contains(&point), "{point} is not a start's fault point");
    let mut fix = Fixture::new().await;
    fix.say("one").await;
    stop_and_wait(&fix.daemon, STOP).await;
    // The fault build, still stopped: only the start can reach `point`.
    let env = cut_env(&fix.daemon.state, &fix.world.channels, fix.daemon.dir.path(), point, cut);
    let refs: Vec<(&str, &str)> = env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
    fix.daemon.restart_env(&refs).await;
    wait_stopped(&fix.daemon, STOP).await;

    // The daemon may die under the call.
    let _ = try_start(&fix.daemon).await;
    fix.daemon.stopped(LONG).await;
    if cut == Cut::Power {
        power_off_sessions(&fix, &[]).await;
    }
    fix.daemon.restart().await;

    let start = last_start(&fix.daemon).await;
    assert!(start.sessions_stopped.is_none(), "{point}: the sessions are stopped again");
    assert!(start.not_brought_back.is_empty(), "{point}: {:?}", start.not_brought_back);
    fix.assert_agent_is_back().await;
    assert!(fix.heard().contains("one"), "{point}");
    let named = report_folders(&fix.daemon.state)
        .iter()
        .filter_map(|folder| restart_json(folder))
        .any(|json| names_stop(&json, STOP));
    assert!(named, "{point}: no restart report names the stop");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_cut_as_its_record_is_written_ends_stopped_or_still_running_with_nothing_lost() {
    a_stop_cut_at("stop.record.written", true, Cut::Crash).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_cut_as_the_daemon_restarts_itself_ends_stopped_with_nothing_lost() {
    a_stop_cut_at("stop.exec", false, Cut::Crash).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_cut_once_the_programs_are_hung_up_on_ends_stopped_with_nothing_lost() {
    a_stop_cut_at("stop.programs.hungup", false, Cut::Crash).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_cut_once_the_holders_are_signalled_ends_stopped_with_nothing_lost() {
    a_stop_cut_at("stop.holders.signalled", false, Cut::Crash).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_cut_once_the_inbox_is_withdrawn_ends_stopped_with_nothing_lost() {
    a_stop_cut_at("stop.inbox.withdrawn", false, Cut::Crash).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_cut_once_the_holders_are_gone_ends_stopped_with_nothing_lost() {
    a_stop_cut_at("stop.holders.gone", false, Cut::Crash).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_cut_as_its_record_is_settled_ends_stopped_with_nothing_lost() {
    a_stop_cut_at("stop.record.settled", false, Cut::Crash).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_cut_once_its_record_is_renamed_ends_running_with_a_report_naming_the_stop() {
    a_start_cut_at("start.record.renamed", Cut::Crash).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_cut_as_the_daemon_restarts_itself_ends_running_with_a_report_naming_the_stop() {
    a_start_cut_at("start.exec", Cut::Crash).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_cut_once_its_report_is_written_ends_running_with_a_report_naming_the_stop() {
    a_start_cut_at("start.report.written", Cut::Crash).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_cut_once_its_record_is_removed_ends_running_with_a_report_naming_the_stop() {
    a_start_cut_at("start.record.removed", Cut::Crash).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_with_the_power_cut_as_its_record_is_written_ends_stopped_or_still_running_with_nothing_lost() {
    a_stop_cut_at("stop.record.written", true, Cut::Power).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_with_the_power_cut_as_the_daemon_restarts_itself_ends_stopped_with_nothing_lost() {
    a_stop_cut_at("stop.exec", false, Cut::Power).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_with_the_power_cut_once_the_programs_are_hung_up_on_ends_stopped_with_nothing_lost() {
    a_stop_cut_at("stop.programs.hungup", false, Cut::Power).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_with_the_power_cut_once_the_holders_are_signalled_ends_stopped_with_nothing_lost() {
    a_stop_cut_at("stop.holders.signalled", false, Cut::Power).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_with_the_power_cut_once_the_inbox_is_withdrawn_ends_stopped_with_nothing_lost() {
    a_stop_cut_at("stop.inbox.withdrawn", false, Cut::Power).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_with_the_power_cut_once_the_holders_are_gone_ends_stopped_with_nothing_lost() {
    a_stop_cut_at("stop.holders.gone", false, Cut::Power).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_with_the_power_cut_as_its_record_is_settled_ends_stopped_with_nothing_lost() {
    a_stop_cut_at("stop.record.settled", false, Cut::Power).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_with_the_power_cut_once_its_record_is_renamed_ends_running_with_a_report_naming_the_stop() {
    a_start_cut_at("start.record.renamed", Cut::Power).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_with_the_power_cut_as_the_daemon_restarts_itself_ends_running_with_a_report_naming_the_stop() {
    a_start_cut_at("start.exec", Cut::Power).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_with_the_power_cut_once_its_report_is_written_ends_running_with_a_report_naming_the_stop() {
    a_start_cut_at("start.report.written", Cut::Power).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_with_the_power_cut_once_its_record_is_removed_ends_running_with_a_report_naming_the_stop() {
    a_start_cut_at("start.record.removed", Cut::Power).await;
}

// ---------------------------------------------------------------------------
// 9. A terminal query whose answer the holder never confirms.

/// The bytes of a terminal query that asks where the cursor is (ESC[6n).
const CURSOR_QUERY: &[u8] = b"\x1b[6n";

/// The longest a stop may take with a holder that never confirms. A stop
/// with a stopped holder takes a few seconds (the grace of each step), so
/// this only catches a hang.
const STOP_GUARD: Duration = Duration::from_secs(30);

/// Sends `sig` to process `pid`.
fn send_signal(pid: u32, sig: Signal) {
    let pid = i32::try_from(pid).ok().and_then(Pid::from_raw).expect("a process id");
    kill_process(pid, sig).expect("the test can signal its own holder");
}

/// Resumes a process the test stopped, when the test ends, so a failing
/// assertion leaves nothing stopped behind.
struct Resumed(u32);

impl Drop for Resumed {
    fn drop(&mut self) {
        // A pid that no longer names a process is fine.
        if let Some(pid) = i32::try_from(self.0).ok().and_then(Pid::from_raw) {
            let _ = kill_process(pid, Signal::CONT);
        }
    }
}

/// A shell's terminal query (ESC[6n) asks where its cursor is. The answer
/// is written to the holder and waits there to be confirmed. The holder is
/// stopped as soon as the query shows on the attach, so that confirmation
/// may never come. The stop must still end the shell and the holder rather
/// than wait for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_finishes_while_a_terminal_reply_waits_on_its_holder() {
    const STOP: &str = "5d0c1f6e-7a54-4c0f-9a52-2f3b8f6f0a10";
    let mut fix = Fixture::new().await;
    let before = stored_tabs(&fix.daemon.state);
    let holder = before
        .iter()
        .find(|tab| tab.id == fix.shell_tab.value)
        .expect("the shell tab is stored")
        .holder_pid;

    // `printf` makes the escape byte; the typed line holds only the backslash form.
    fix.shell.type_text("printf '\\033[6n'\r").await;
    fix.shell
        .wait_until(|attach| {
            attach
                .screen
                .windows(CURSOR_QUERY.len())
                .any(|window| window == CURSOR_QUERY)
        })
        .await;
    send_signal(holder, Signal::STOP);
    let _resumed = Resumed(holder);

    let (_, stopped) = tokio::time::timeout(STOP_GUARD, async {
        try_stop(&fix.daemon, STOP, "agents running wild")
            .await
            .expect("the stop is taken");
        wait_stopped(&fix.daemon, STOP).await
    })
    .await
    .expect("the stop finished within the guard, though the holder never confirmed the reply");

    let ended = stopped.ended.clone().expect("the sweep says what it ended");
    assert!(ended.killed >= 1, "the stopped holder can only end by SIGKILL: {ended:?}");
    fix.assert_nothing_runs(&before, &stopped).await;
}

