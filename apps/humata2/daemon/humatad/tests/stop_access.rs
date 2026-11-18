//! Who may stop or start a machine's sessions (STOP, D263), through a real
//! `humatad2`: an agent only when its channel's rule allows it, and never
//! a start of the user's stop; a script only when its token lists the
//! capability; the user always. `humata stop` typed on the machine itself,
//! outside every Humata session and with no credential, needs no login;
//! `humata start` always does.
//!
//! The daemon restarts itself after a stop and after a start, so the
//! waits ask the socket until the new run says it is stopped or started.

mod common;

use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, World, as_script, binary, create_channel, create_script_token,
    error_of, folder, get_session, humata, humata_as_agent, pending_requests, shell_token,
    start_session, wait_for_path, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Code, Request, Status};

/// A safety net for a stop's sweep or a start's bring-back.
const LONG: Duration = Duration::from_secs(90);

const TOKEN_HEADER: &str = "humata-session-token";

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

/// The stop the daemon's current run says it is under, once the sweep ran.
async fn wait_stopped(daemon: &Daemon) -> pb::SessionsStopped {
    let deadline = Instant::now() + LONG;
    loop {
        let stopped = try_info(daemon)
            .await
            .and_then(|info| info.last_start)
            .and_then(|start| start.sessions_stopped)
            .filter(|stopped| stopped.ended.is_some());
        if let Some(stopped) = stopped {
            return stopped;
        }
        assert!(Instant::now() < deadline, "the daemon never said its sessions are stopped");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits until the daemon has started its sessions again after a stop.
async fn wait_started(daemon: &Daemon) {
    let deadline = Instant::now() + LONG;
    loop {
        let start = try_info(daemon).await.and_then(|info| info.last_start);
        if start.is_some_and(|start| start.after_stop.is_some()) {
            return;
        }
        assert!(Instant::now() < deadline, "the daemon never started its sessions again");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Whether the daemon's current run says its sessions are stopped. Asks
/// until a run answers.
async fn is_stopped(daemon: &Daemon) -> bool {
    let deadline = Instant::now() + LONG;
    loop {
        if let Some(info) = try_info(daemon).await {
            return info.last_start.is_some_and(|start| start.sessions_stopped.is_some());
        }
        assert!(Instant::now() < deadline, "the daemon never answered");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn daemon_service(daemon: &Daemon) -> pb::daemon_service_client::DaemonServiceClient<Channel> {
    pb::daemon_service_client::DaemonServiceClient::new(daemon.channel().await)
}

fn stop_request(stop_id: &str) -> pb::StopSessionsRequest {
    pb::StopSessionsRequest {
        stop_id: stop_id.to_owned(),
        reason: "agents running wild".to_owned(),
        via: pb::StopVia::App as i32,
        ..Default::default()
    }
}

/// The user, in the app, stops the sessions and waits for the sweep.
async fn user_stops(daemon: &Daemon, stop_id: &str) -> pb::SessionsStopped {
    daemon_service(daemon)
        .await
        .stop_sessions(stop_request(stop_id))
        .await
        .expect("the user's stop is taken");
    let stopped = wait_stopped(daemon).await;
    assert_eq!(stopped.stop_id, stop_id);
    stopped
}

// ---------------------------------------------------------------------------
// A host with one agent session in Payments.

struct Fixture {
    daemon: Daemon,
    _world: World,
    channel: pb::Channel,
    id: pb::SessionId,
    /// The session token the session's shell tab was given: its agent's identity.
    token: String,
    /// The attach on that shell tab.
    shell: Attach,
}

impl Fixture {
    /// On the admin daemon: every call without a credential is the user.
    async fn new() -> Self {
        Self::on(Build::Admin).await
    }

    /// On a daemon that takes only this test process for the user: a
    /// `humata` it starts is judged as the release daemon would.
    async fn judged_as_released() -> Self {
        Self::on(Build::AdminForThisProcess).await
    }

    async fn on(build: Build) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        world.remote("acme", "api");
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(build, dir, &refs, &[]).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{setup:?}");
        let (id, _) = start_session(&daemon, &channel, "Fix login").await;
        wait_running(&daemon, &id).await;
        let (token, shell) = shell_token(&daemon, &id).await;
        Self { daemon, _world: world, channel, id, token, shell }
    }

    fn channel_id(&self) -> pb::ChannelId {
        self.channel.id.clone().expect("the channel has an id")
    }

    /// A request that carries the session token: the agent's call.
    fn as_agent<T>(&self, message: T) -> Request<T> {
        let mut request = Request::new(message);
        request
            .metadata_mut()
            .insert(TOKEN_HEADER, MetadataValue::try_from(self.token.as_str()).unwrap());
        request
    }

    /// The user sets Payments' rule for stopping and starting sessions.
    async fn set_rule(&self, decision: pb::Decision) {
        pb::access_service_client::AccessServiceClient::new(self.daemon.channel().await)
            .set_channel_rule(pb::SetChannelRuleRequest {
                channel: Some(self.channel_id()),
                capability: pb::Capability::MachineSessionsStop as i32,
                decision: decision as i32,
                ..Default::default()
            })
            .await
            .expect("the user sets the rule");
    }

    async fn agent_stops(&self, stop_id: &str) -> Result<pb::StopSessionsResponse, Status> {
        let mut request = stop_request(stop_id);
        request.via = pb::StopVia::Cli as i32;
        daemon_service(&self.daemon)
            .await
            .stop_sessions(self.as_agent(request))
            .await
            .map(tonic::Response::into_inner)
    }

    async fn agent_starts(&self) -> Result<pb::StartSessionsResponse, Status> {
        daemon_service(&self.daemon)
            .await
            .start_sessions(self.as_agent(pb::StartSessionsRequest {}))
            .await
            .map(tonic::Response::into_inner)
    }

    /// Nothing was stopped: the daemon isn't stopped, the session still
    /// runs and the Inbox holds no request.
    async fn assert_nothing_stopped(&self) {
        assert!(!is_stopped(&self.daemon).await, "the sessions were stopped");
        let session = get_session(&self.daemon, &self.id).await;
        assert_ne!(session.status(), pb::SessionStatus::MachineStopped);
        assert!(pending_requests(&self.daemon).await.is_empty(), "the refusal left an Inbox request");
    }

    /// `humata --json <args>` typed into the session's shell tab with its
    /// session token taken out of the environment: a person typing in a
    /// Humata session, with no credential. The exit code.
    async fn humata_in_tab_without_token(&mut self, args: &str) -> i32 {
        let code = self.daemon.dir.path().join("in-tab-no-token.code");
        let partial = self.daemon.dir.path().join("in-tab-no-token.part");
        self.shell
            .type_text(&format!(
                "env -u HUMATA_SESSION_TOKEN -u HUMATA_TOKEN HUMATA_SOCKET='{}' '{}' --json {args} \
                 >/dev/null 2>&1; echo $? > '{}'; mv '{}' '{}'\r",
                self.daemon.socket.display(),
                binary("HUMATA").display(),
                partial.display(),
                partial.display(),
                code.display(),
            ))
            .await;
        wait_for_path(&code).await;
        std::fs::read_to_string(&code)
            .expect("the exit code was written")
            .trim()
            .parse()
            .expect("the exit code is a number")
    }
}

fn denied_by_never_rule(status: &Status) {
    assert_eq!(status.code(), Code::PermissionDenied, "{status:?}");
    match error_of(status).kind {
        Some(pb::error::Kind::Denied(denied)) => {
            assert_eq!(denied.source(), pb::denied::Source::NeverRule, "{denied:?}");
            assert_eq!(denied.capability(), pb::Capability::MachineSessionsStop, "{denied:?}");
        }
        other => panic!("expected a Never rule's denial, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 1. An agent or a script whose rule doesn't allow it.

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_under_the_default_rule_is_denied_a_stop_at_once_and_nothing_stops() {
    let fix = Fixture::new().await;
    let refused = fix
        .agent_stops("7b1e0c52-1f0d-4f43-9d8e-5a1c2b3d4e01")
        .await
        .expect_err("Never denies an agent's stop");
    denied_by_never_rule(&refused);
    fix.assert_nothing_stopped().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_humata_stop_under_the_default_rule_exits_denied_with_a_fix() {
    let fix = Fixture::new().await;
    let (code, json) =
        humata_as_agent(&fix.daemon, &fix.token, &["stop", "--reason", "loop"]).await;
    assert_eq!(code, 4, "{json}");
    assert_eq!(json["error"]["kind"], "denied", "{json}");
    assert!(json["error"]["fix"].as_str().is_some_and(|fix| !fix.is_empty()), "{json}");
    fix.assert_nothing_stopped().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_whose_token_lacks_the_capability_is_refused_a_stop_and_nothing_stops() {
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
    let refused = daemon_service(&fix.daemon)
        .await
        .stop_sessions(as_script(stop_request("7b1e0c52-1f0d-4f43-9d8e-5a1c2b3d4e02"), &script))
        .await
        .expect_err("the token doesn't list stopping sessions");
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    fix.assert_nothing_stopped().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rule_of_ask_puts_an_agents_stop_in_the_inbox_and_stops_nothing_meanwhile() {
    let fix = Fixture::new().await;
    fix.set_rule(pb::Decision::Ask).await;
    fix.agent_stops("7b1e0c52-1f0d-4f43-9d8e-5a1c2b3d4e03")
        .await
        .expect_err("the stop waits for the user's answer");
    assert_eq!(pending_requests(&fix.daemon).await.len(), 1, "one request waits in the Inbox");
    assert!(!is_stopped(&fix.daemon).await, "an unanswered request stopped the sessions");
}

// ---------------------------------------------------------------------------
// 2. An agent whose rule allows it.

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_whose_rule_allows_it_stops_its_machine_itself_included_and_is_named_as_the_stopper() {
    let fix = Fixture::new().await;
    fix.set_rule(pb::Decision::Allow).await;
    let stop_id = "7b1e0c52-1f0d-4f43-9d8e-5a1c2b3d4e04";
    // The reply may be lost as the daemon goes down with the agent's session.
    if let Err(status) = fix.agent_stops(stop_id).await {
        assert_ne!(status.code(), Code::PermissionDenied, "the allowed stop was refused: {status:?}");
    }
    let stopped = wait_stopped(&fix.daemon).await;
    assert_eq!(stopped.stop_id, stop_id);
    let by = stopped.by.expect("the stop says who made it");
    assert_eq!(by.kind(), pb::StoppedByKind::Agent, "{by:?}");
    assert_eq!(by.session, fix.id.value, "{by:?}");
    let session = get_session(&fix.daemon, &fix.id).await;
    assert_eq!(session.status(), pb::SessionStatus::MachineStopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_whose_token_lists_stopping_sessions_stops_the_machine_and_is_named_a_script() {
    let fix = Fixture::new().await;
    let (_, script) = create_script_token(
        &fix.daemon,
        "ci",
        &[
            pb::Capability::GlobalRead,
            pb::Capability::ChannelRead,
            pb::Capability::MachineSessionsStop,
        ],
        &[&fix.channel_id()],
        0,
    )
    .await
    .expect("the user makes a script token");
    let stop_id = "7b1e0c52-1f0d-4f43-9d8e-5a1c2b3d4e20";
    // The reply may be lost as the daemon goes down.
    if let Err(status) = daemon_service(&fix.daemon)
        .await
        .stop_sessions(as_script(stop_request(stop_id), &script))
        .await
    {
        assert_ne!(status.code(), Code::PermissionDenied, "the allowed stop was refused: {status:?}");
    }
    let stopped = wait_stopped(&fix.daemon).await;
    assert_eq!(stopped.stop_id, stop_id);
    let by = stopped.by.expect("the stop says who made it");
    assert_eq!(by.kind(), pb::StoppedByKind::Script, "{by:?}");
    assert!(by.session.is_empty(), "a script has no session: {by:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_whose_rule_allows_it_still_cannot_start_the_users_stop() {
    let fix = Fixture::new().await;
    fix.set_rule(pb::Decision::Allow).await;
    user_stops(&fix.daemon, "7b1e0c52-1f0d-4f43-9d8e-5a1c2b3d4e05").await;

    fix.agent_starts().await.expect_err("only the user starts the user's stop");
    assert!(is_stopped(&fix.daemon).await, "the agent started the user's stop");

    // The user still can.
    daemon_service(&fix.daemon)
        .await
        .start_sessions(pb::StartSessionsRequest {})
        .await
        .expect("the user starts the sessions");
    wait_started(&fix.daemon).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn under_the_default_rule_an_agent_cannot_start_a_stopped_machine() {
    let fix = Fixture::new().await;
    user_stops(&fix.daemon, "7b1e0c52-1f0d-4f43-9d8e-5a1c2b3d4e06").await;
    let refused = fix.agent_starts().await.expect_err("an agent can't start sessions");
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    assert!(is_stopped(&fix.daemon).await);
}

// ---------------------------------------------------------------------------
// 3. `humata stop` and `humata start` typed on the machine.

#[tokio::test(flavor = "multi_thread")]
async fn humata_stop_typed_outside_every_session_with_no_login_stops_the_machine() {
    let fix = Fixture::judged_as_released().await;
    let daemon = &fix.daemon;
    let (code, json) = tokio::task::block_in_place(|| {
        humata(daemon, &["stop", "--reason", "from a terminal"])
    });
    assert_eq!(code, 0, "{json}");
    let stop_id = json["result"]["stopped"]["stop_id"]
        .as_str()
        .expect("the reply names the stop")
        .to_owned();
    let stopped = wait_stopped(daemon).await;
    assert_eq!(stopped.stop_id, stop_id);
    assert_eq!(stopped.reason, "from a terminal");
    let by = stopped.by.expect("the stop says who made it");
    assert_eq!(by.kind(), pb::StoppedByKind::Terminal, "{by:?}");
    assert_eq!(by.via(), pb::StopVia::Cli, "{by:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn humata_start_with_no_login_is_refused_and_the_machine_stays_stopped() {
    let fix = Fixture::judged_as_released().await;
    user_stops(&fix.daemon, "7b1e0c52-1f0d-4f43-9d8e-5a1c2b3d4e07").await;
    let daemon = &fix.daemon;
    let (code, json) = tokio::task::block_in_place(|| humata(daemon, &["start"]));
    assert_eq!(code, 5, "start needs the user: {json}");
    assert!(json["error"]["fix"].as_str().is_some_and(|fix| !fix.is_empty()), "{json}");
    assert!(is_stopped(daemon).await, "a start with no login started the sessions");
}

#[tokio::test(flavor = "multi_thread")]
async fn humata_stop_typed_inside_a_session_without_its_token_is_refused() {
    let mut fix = Fixture::judged_as_released().await;
    let code = fix
        .humata_in_tab_without_token("stop --reason 'from inside'")
        .await;
    assert_ne!(code, 0, "a stop from inside a session needs a login or a rule");
    fix.assert_nothing_stopped().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_carrying_an_unusable_session_token_is_refused_not_taken_as_a_terminal() {
    let fix = Fixture::judged_as_released().await;
    let daemon = &fix.daemon;
    let (code, json) =
        humata_as_agent(daemon, "not-a-real-token", &["stop", "--reason", "forged"]).await;
    assert_ne!(code, 0, "{json}");
    fix.assert_nothing_stopped().await;
}
