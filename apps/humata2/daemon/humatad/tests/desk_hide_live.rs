//! D801 G: Hide never ends a Desk session's agent. Hide is a view action: the
//! app hides the terminal tab and sends no call (features/session close.rs,
//! Keep running). Only End and End for good call CloseDeskSession, and an
//! agent may not call it at all (desk_service.rs `user_only`). Each test
//! checks the agent's holder keeps its pid, its program keeps its pid, and
//! the holder keeps answering.

mod common;

use std::process::Command;

use common::{
    Build, Daemon, World, agent_tab, folder, get_session, holder_answers, is_running, shell_token, stored_tabs,
    wait_status,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::TabRecord;
use pb::SessionStatus as Status;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

const TOKEN_HEADER: &str = "humata-session-token";

/// The agent's holder record, and the pid of the program it runs.
struct Running {
    holder: TabRecord,
    program: u32,
}

async fn desks(daemon: &Daemon) -> pb::desk_service_client::DeskServiceClient<Channel> {
    pb::desk_service_client::DeskServiceClient::new(daemon.channel().await)
}

/// Starts a Desk session whose agent is up.
async fn start(daemon: &Daemon, title: &str) -> pb::SessionId {
    let started = desks(daemon)
        .await
        .start_desk_session(pb::StartDeskSessionRequest {
            title: title.to_owned(),
            access: Some(pb::DeskAccess {
                boards: true,
                channels_and_machines: false,
            }),
            ..Default::default()
        })
        .await
        .expect("the Desk session starts")
        .into_inner();
    let id = started.session.expect("a start names its session");
    wait_status(daemon, &id, Status::Working).await;
    id
}

/// The pid of the one process whose parent is `holder`: the holder's program.
fn program_of(holder: u32) -> Option<u32> {
    let listed = Command::new("/bin/ps").args(["-A", "-o", "pid=,ppid="]).output().ok()?;
    String::from_utf8_lossy(&listed.stdout).lines().find_map(|line| {
        let mut fields = line.split_whitespace().map(str::parse::<u32>);
        match (fields.next(), fields.next()) {
            (Some(Ok(pid)), Some(Ok(parent))) if parent == holder => Some(pid),
            _ => None,
        }
    })
}

async fn running(daemon: &Daemon, id: &pb::SessionId) -> Running {
    let tab = agent_tab(daemon, id).await;
    let holder = stored_tabs(&daemon.state)
        .into_iter()
        .find(|record| record.id == tab.value)
        .expect("the agent's tab is stored");
    let program = program_of(holder.holder_pid).expect("the agent's holder runs its program");
    Running { holder, program }
}

/// Asserts the agent is the one `before` names, still running, with its
/// holder answering, and the session is open on the Desk.
async fn assert_untouched(daemon: &Daemon, id: &pb::SessionId, before: &Running) {
    let after = running(daemon, id).await;
    assert_eq!(after.holder.holder_pid, before.holder.holder_pid, "the agent's holder was replaced");
    assert_eq!(after.program, before.program, "the agent's program was restarted or ended");
    assert!(holder_answers(&after.holder), "the agent's holder stopped answering");
    assert!(is_running(&get_session(daemon, id).await), "the agent stopped running");
    let listed = desks(daemon)
        .await
        .list_desk_sessions(pb::ListDeskSessionsRequest::default())
        .await
        .expect("the Desk lists its sessions")
        .into_inner()
        .sessions;
    let desk = listed
        .iter()
        .find(|desk| desk.session.as_ref() == Some(id))
        .expect("the Desk still lists the session");
    assert!(desk.closed_at.is_none(), "the session was closed on the Desk");
}

#[tokio::test]
async fn hiding_a_desk_session_keeps_its_agent_running() {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[]).await;
    let id = start(&daemon, "Plan Q4").await;
    let before = running(&daemon, &id).await;

    // Hide sends no RPC, so the app's side of it is only the reads it makes
    // after hiding the tab: the Desk list and the session.
    assert_untouched(&daemon, &id, &before).await;
}

#[tokio::test]
async fn an_agent_cannot_close_a_desk_session_and_nothing_ends() {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[]).await;
    let id = start(&daemon, "Plan Q4").await;
    let before = running(&daemon, &id).await;
    // An agent's caller: the session token its own shell was given. The
    // destroy switch is off by default and never reaches this call.
    let (token, _shell) = shell_token(&daemon, &id).await;

    let mut request = Request::new(pb::CloseDeskSessionRequest {
        session: Some(id.clone()),
        ..Default::default()
    });
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token.as_str()).expect("a token is ASCII"));
    let refused = desks(&daemon)
        .await
        .close_desk_session(request)
        .await
        .expect_err("an agent's CloseDeskSession is refused");
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");

    assert_untouched(&daemon, &id, &before).await;
}
