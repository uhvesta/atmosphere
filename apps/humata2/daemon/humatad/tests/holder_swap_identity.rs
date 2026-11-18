//! SEC-15 across a holder swap (D784): when a session's agent is resumed in
//! a new holder (Resume after a stop, Undo of End for good, revival after a
//! restart), the resumed agent's own tools can use the session's token while
//! the resume is still being checked, and the resume finishes. Found live in
//! VERIFY 0.2.3: the tab still named the old holder until the resume was
//! checked, so the agent's MCP child was refused and its token revoked.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Build, Daemon, World, binary, create_channel, finished, folder, get_session, start_session};
use humata2_grpc::humata::v1 as pb;
use tonic::Request;
use tonic::metadata::MetadataValue;

/// Well inside the 2 minutes the live restore hung for.
const QUICK: Duration = Duration::from_secs(45);

/// The test agent, which on a resume calls `humata whoami` with its
/// session's token before it says it is up, and prints all the while so the
/// resume can't be checked as settled until the call has its answer. It
/// leaves the token in `token` and the answer in `whoami.json`.
fn agent_script(conv: &Path, socket: &Path) -> String {
    format!(
        r#"CONV_DIR='{conv}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume)
    f="$CONV_DIR/$id.jsonl"
    printf '%s' "$HUMATA_SESSION_TOKEN" > "$CONV_DIR/token.part"; mv "$CONV_DIR/token.part" "$CONV_DIR/token"
    ( HUMATA_SOCKET='{socket}' '{humata}' --json whoami > "$CONV_DIR/whoami.part" 2>/dev/null; mv "$CONV_DIR/whoami.part" "$CONV_DIR/whoami.json" ) &
    while [ ! -e "$CONV_DIR/whoami.json" ]; do printf .; sleep 0.05; done
    echo ;;
esac
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#,
        conv = conv.display(),
        socket = socket.display(),
        humata = binary("HUMATA").display(),
    )
}

struct Fixture {
    daemon: Daemon,
    world: World,
    channel: pb::Channel,
}

impl Fixture {
    /// A daemon that judges every caller but this test as the release
    /// daemon does, so the process-tree check is live.
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with(Build::AdminForThisProcess, dir, &refs).await;
        std::fs::write(world.root.join("agent.sh"), agent_script(&world.conv, &daemon.socket)).unwrap();
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        Self { daemon, world, channel }
    }

    async fn session(&self) -> pb::SessionId {
        let (id, _) = start_session(&self.daemon, &self.channel, "Swap").await;
        common::wait_running(&self.daemon, &id).await;
        id
    }

    fn answer(&self) -> PathBuf {
        self.world.conv.join("whoami.json")
    }

    /// What the resumed agent's `humata whoami` printed.
    async fn whoami(&self) -> serde_json::Value {
        common::wait_for_path(&self.answer()).await;
        let text = std::fs::read_to_string(self.answer()).unwrap();
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
    }

    fn token(&self) -> String {
        std::fs::read_to_string(self.world.conv.join("token")).unwrap()
    }

    async fn stop(&self, id: &pb::SessionId) {
        self.daemon
            .session_service()
            .await
            .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(id.clone()) })
            .await
            .unwrap();
        common::wait_status(&self.daemon, id, pb::SessionStatus::Exited).await;
    }

    async fn resume(&self, id: &pb::SessionId) -> pb::Operation {
        let started = self
            .daemon
            .session_service()
            .await
            .resume_session(pb::ResumeSessionRequest {
                idempotency_key: None,
                id: Some(id.clone()),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        finished(&self.daemon, &started).await
    }

    /// End for good: archived, its agent stopped.
    async fn end(&self, id: &pb::SessionId) {
        let plan = self
            .daemon
            .session_service()
            .await
            .plan_archive(pb::PlanArchiveRequest { id: Some(id.clone()) })
            .await
            .unwrap()
            .into_inner();
        let started = self
            .daemon
            .session_service()
            .await
            .archive_session(pb::ArchiveSessionRequest {
                idempotency_key: None,
                id: Some(id.clone()),
                choice: pb::ArchiveChoice::Archive as i32,
                plan: plan.token,
            })
            .await
            .unwrap()
            .into_inner();
        succeeded(&finished(&self.daemon, &started).await);
    }

    async fn restore(&self, id: &pb::SessionId) -> pb::Operation {
        let started = self
            .daemon
            .session_service()
            .await
            .restore_session(pb::RestoreSessionRequest {
                idempotency_key: None,
                id: Some(id.clone()),
                machine: None,
                stopped: false,
            })
            .await
            .unwrap()
            .into_inner();
        finished(&self.daemon, &started).await
    }

    /// The token used by this test's process, outside every holder's tree.
    async fn used_outside(&self, token: &str) -> tonic::Status {
        let mut request = Request::new(pb::ListTabsRequest { session: None });
        request
            .metadata_mut()
            .insert("humata-session-token", MetadataValue::try_from(token).unwrap());
        self.daemon.terminals().await.list_tabs(request).await.unwrap_err()
    }
}

fn succeeded(operation: &pb::Operation) {
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
}

fn is_agent(me: &serde_json::Value) {
    assert_eq!(me["result"]["tier"], "agent", "the resumed agent's own call was refused: {me}");
}

/// Resume after a stop: a new holder in place of the stopped one.
#[tokio::test(flavor = "multi_thread")]
async fn a_resumed_agent_can_use_its_token_from_the_new_holder() {
    let fixture = Fixture::new().await;
    let id = fixture.session().await;
    fixture.stop(&id).await;

    let began = Instant::now();
    let resumed = fixture.resume(&id).await;

    succeeded(&resumed);
    assert!(began.elapsed() < QUICK, "the resume took {:?}", began.elapsed());
    is_agent(&fixture.whoami().await);
    common::wait_running(&fixture.daemon, &id).await;
}

/// Undo of End for good that brings it back running: a new holder for an
/// agent whose old one was ended.
#[tokio::test(flavor = "multi_thread")]
async fn a_restored_agent_can_use_its_token_and_the_restore_finishes() {
    let fixture = Fixture::new().await;
    let id = fixture.session().await;
    fixture.end(&id).await;

    let began = Instant::now();
    let restored = fixture.restore(&id).await;

    succeeded(&restored);
    assert!(began.elapsed() < QUICK, "the restore took {:?}", began.elapsed());
    is_agent(&fixture.whoami().await);
    let back = common::wait_running(&fixture.daemon, &id).await;
    assert!(back.archive.is_none(), "{back:?}");
}

/// Revival after a reboot: the daemon starts again, finds the holder gone
/// and resumes the agent in a new one.
#[tokio::test(flavor = "multi_thread")]
async fn a_revived_agent_can_use_its_token_after_a_restart() {
    let mut fixture = Fixture::new().await;
    let id = fixture.session().await;

    fixture.daemon.kill();
    common::kill_holders(&common::stored_tabs(&fixture.daemon.state)).await;
    fixture.daemon.restart().await;

    is_agent(&fixture.whoami().await);
    common::last_start(&fixture.daemon).await;
    common::wait_running(&fixture.daemon, &id).await;
}

/// The check stays strict after a swap: the new holder's token used from
/// outside its tree is refused.
#[tokio::test(flavor = "multi_thread")]
async fn after_a_swap_the_token_is_still_refused_outside_the_new_holder() {
    let fixture = Fixture::new().await;
    let id = fixture.session().await;
    fixture.stop(&id).await;
    succeeded(&fixture.resume(&id).await);
    is_agent(&fixture.whoami().await);

    let refused = fixture.used_outside(&fixture.token()).await;

    let error = common::error_of(&refused);
    assert!(matches!(error.kind, Some(pb::error::Kind::IdentityMismatch(_))), "{refused:?}");
    assert_eq!(get_session(&fixture.daemon, &id).await.id, Some(id));
}
