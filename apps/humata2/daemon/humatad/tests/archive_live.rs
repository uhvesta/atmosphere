//! D801 F: archiving keeps a session's agent and shells running unless the
//! request says to end them, and an agent may only end them under the
//! machine-destroy switch.

mod common;

use std::collections::BTreeSet;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{
    Attach, Daemon, World, create_channel, finished, folder, get_session, shell_token, start_session,
    stored_tabs, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use pb::SessionStatus as Status;
use tonic::metadata::MetadataValue;
use tonic::{Code, Request};

const TOKEN_HEADER: &str = "humata-session-token";
const DEADLINE: Duration = Duration::from_secs(30);

struct Fixture {
    daemon: Daemon,
    channel: pb::Channel,
    id: pb::SessionId,
    token: String,
    /// Keeps the shell's tab open.
    _shell: Attach,
}

/// A session with a running agent and one shell.
async fn started() -> Fixture {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with_env(common::Build::Admin, dir, &refs, &[("HUMATA2_ARCHIVE_UNDO_MS", "0")]).await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    wait_running(&daemon, &id).await;
    let (token, shell) = shell_token(&daemon, &id).await;
    Fixture { daemon, channel, id, token, _shell: shell }
}

impl Fixture {
    /// The holders of the session's terminals (agent and shell), by pid.
    fn holders(&self) -> BTreeSet<u32> {
        stored_tabs(&self.daemon.state)
            .into_iter()
            .filter(|tab| tab.session.as_deref() == Some(self.id.value.as_str()))
            .map(|tab| tab.holder_pid)
            .collect()
    }

    fn answering(&self) -> usize {
        stored_tabs(&self.daemon.state)
            .iter()
            .filter(|tab| tab.session.as_deref() == Some(self.id.value.as_str()))
            .filter(|tab| common::holder_answers(tab))
            .count()
    }

    fn request(&self, end: Option<bool>, token: Option<&str>) -> Request<pb::ArchiveSessionRequest> {
        let mut request = Request::new(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(self.id.clone()),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: None,
            end,
        });
        if let Some(token) = token {
            request
                .metadata_mut()
                .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
        }
        request
    }

    async fn archive(&self, end: Option<bool>) -> pb::Operation {
        let started = self
            .daemon
            .session_service()
            .await
            .archive_session(self.request(end, None))
            .await
            .unwrap()
            .into_inner();
        finished(&self.daemon, &started).await
    }
}

/// The pids among `pids` that are still processes, from `ps`.
fn alive(pids: &BTreeSet<u32>) -> BTreeSet<u32> {
    let listed = Command::new("/bin/ps").args(["-A", "-o", "pid="]).output().unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .filter(|pid| pids.contains(pid))
        .collect()
}

/// The processes whose parent is one of `parents`: the programs the holders run.
fn programs_of(parents: &BTreeSet<u32>) -> BTreeSet<u32> {
    let listed = Command::new("/bin/ps").args(["-A", "-o", "pid=,ppid="]).output().unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse::<u32>().ok()?;
            let parent = fields.next()?.parse::<u32>().ok()?;
            parents.contains(&parent).then_some(pid)
        })
        .collect()
}

async fn wait_ended(fixture: &Fixture) {
    let deadline = Instant::now() + DEADLINE;
    while fixture.answering() > 0 {
        assert!(Instant::now() < deadline, "the terminals never ended");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn archiving_keeps_the_agent_and_shell_running_and_lists_them_under_their_session() {
    let fixture = started().await;
    let holders = fixture.holders();
    assert!(holders.len() >= 2, "an agent and a shell run: {holders:?}");
    let programs = programs_of(&holders);
    assert!(!programs.is_empty(), "the holders run programs");

    let operation = fixture.archive(None).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    assert_eq!(get_session(&fixture.daemon, &fixture.id).await.status, Status::Archived as i32);

    // Past the undo window (0 ms): its folder is kept while it runs.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(fixture.holders(), holders, "the terminals' holders changed");
    assert_eq!(fixture.answering(), holders.len());
    assert_eq!(programs_of(&holders), programs, "a program was restarted or ended");
    assert_eq!(alive(&holders), holders);
    let session = get_session(&fixture.daemon, &fixture.id).await;
    assert!(
        std::path::Path::new(&session.repos[0].path).exists(),
        "the folder of a running archived session was freed"
    );

    // The machine page still names the holders by their session.
    let listed = pb::machine_service_client::MachineServiceClient::new(fixture.daemon.channel().await)
        .list_running_holders(pb::ListRunningHoldersRequest { machine: None })
        .await
        .unwrap()
        .into_inner()
        .holders;
    for pid in &holders {
        let holder = listed.iter().find(|holder| holder.pid == *pid).expect("the holder is listed");
        assert_eq!(holder.session.as_ref(), Some(&fixture.id), "listed as an orphan");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_user_can_archive_and_end_the_terminals() {
    let fixture = started().await;
    let holders = fixture.holders();
    let programs = programs_of(&holders);

    let operation = fixture.archive(Some(true)).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    assert_eq!(get_session(&fixture.daemon, &fixture.id).await.status, Status::Archived as i32);
    wait_ended(&fixture).await;
    let deadline = Instant::now() + DEADLINE;
    let all: BTreeSet<u32> = holders.union(&programs).copied().collect();
    while !alive(&all).is_empty() {
        assert!(Instant::now() < deadline, "a holder or its program still runs");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_needs_the_destroy_switch_to_archive_and_end() {
    let fixture = started().await;
    // The agent may archive its own session; ending terminals is the switch.
    pb::access_service_client::AccessServiceClient::new(fixture.daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: fixture.channel.id.clone(),
            capability: pb::Capability::OwnSessionEnd as i32,
            decision: pb::Decision::Allow as i32,
            ..Default::default()
        })
        .await
        .expect("the user lets the agent archive its own session");
    let holders = fixture.holders();
    let programs = programs_of(&holders);

    let refused = fixture
        .daemon
        .session_service()
        .await
        .archive_session(fixture.request(Some(true), Some(&fixture.token)))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_ne!(get_session(&fixture.daemon, &fixture.id).await.status, Status::Archived as i32);
    assert_eq!(fixture.answering(), holders.len(), "a refused call ended a terminal");
    assert_eq!(programs_of(&holders), programs);

    // Without end it archives and nothing ends.
    let started = fixture
        .daemon
        .session_service()
        .await
        .archive_session(fixture.request(None, Some(&fixture.token)))
        .await
        .expect("an agent archives its own session and keeps it running")
        .into_inner();
    let operation = finished(&fixture.daemon, &started).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    assert_eq!(fixture.answering(), holders.len());
    assert_eq!(programs_of(&holders), programs);
}

/// A cut right after an archive that ends its terminals is recorded: the
/// restart that finishes it still ends them (the choice is kept with the
/// archive), so the session's folder is freed.
#[tokio::test(flavor = "multi_thread")]
async fn an_archive_that_ends_its_terminals_cut_once_recorded_ends_them_at_restart() {
    let mut fixture = started().await;
    let holders = fixture.holders();
    let folder = std::path::PathBuf::from(&get_session(&fixture.daemon, &fixture.id).await.repos[0].path);
    fixture.daemon.restart_env(&[("HUMATA2_STOP_AT", "archive.recorded")]).await;
    common::last_start(&fixture.daemon).await;
    let _ = fixture
        .daemon
        .session_service()
        .await
        .archive_session(fixture.request(Some(true), None))
        .await;
    let deadline = Instant::now() + DEADLINE;
    while std::os::unix::net::UnixStream::connect(&fixture.daemon.socket).is_ok() {
        assert!(Instant::now() < deadline, "the daemon never stopped once the archive was recorded");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    fixture.daemon.restart().await;
    common::last_start(&fixture.daemon).await;

    assert_eq!(get_session(&fixture.daemon, &fixture.id).await.status, Status::Archived as i32);
    wait_ended(&fixture).await;
    let deadline = Instant::now() + DEADLINE;
    while !alive(&holders).is_empty() || folder.exists() {
        assert!(Instant::now() < deadline, "a holder still runs or the folder was never freed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
