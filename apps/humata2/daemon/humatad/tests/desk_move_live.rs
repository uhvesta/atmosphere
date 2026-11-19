//! D801 E through a real `humatad2`: moving a Desk session into a channel on
//! the same filesystem is a rename, so what runs in it keeps running. The
//! agent's and the shell's holders and programs keep their pids, the session
//! is filed in the channel, its folder is at the landing, and its old folder
//! name keeps resolving as a link to the new one while they run. A move cut
//! before the switch is undone at the next start by renaming the folder back
//! (the same holders, the same programs), and one cut after it is finished.

use humatad_test_common as common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{
    Build, Daemon, World, create_channel, finished, folder, get_session, holder_answers, last_start, shell_token,
    stored_tabs, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{Store, TabRecord};
use pb::SessionStatus as Status;
use tonic::transport::Channel;

/// The processes whose parent is one of `parents`, from `ps`, sorted.
fn children_of(parents: &[u32]) -> Vec<u32> {
    let listed = Command::new("/bin/ps").args(["-A", "-o", "pid=,ppid="]).output().expect("ps runs");
    let mut found: Vec<u32> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace().map(str::parse::<u32>);
            let (Some(Ok(pid)), Some(Ok(parent))) = (fields.next(), fields.next()) else {
                return None;
            };
            parents.contains(&parent).then_some(pid)
        })
        .collect();
    found.sort_unstable();
    found
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

async fn until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct Fixture {
    daemon: Daemon,
    channel: pb::Channel,
}

/// A Desk session whose agent and shell run: where it works, and the
/// holders and programs the move must leave be.
struct Running {
    id: pb::SessionId,
    folder: PathBuf,
    holders: Vec<u32>,
    programs: Vec<u32>,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        world.remote("acme", "api");
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[]).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        Self { daemon, channel }
    }

    async fn desk(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.daemon.channel().await)
    }

    fn store(&self) -> Store {
        Store::open(self.daemon.state.join("store")).unwrap()
    }

    fn tabs_of(&self, id: &pb::SessionId) -> Vec<TabRecord> {
        stored_tabs(&self.daemon.state).into_iter().filter(|tab| tab.session.as_deref() == Some(id.value.as_str())).collect()
    }

    /// Starts a Desk session, waits until its agent is up and opens a shell
    /// in it; the shell's attach is returned to keep the tab in use.
    async fn session(&self) -> (Running, common::Attach) {
        let started = self
            .desk()
            .await
            .start_desk_session(pb::StartDeskSessionRequest {
                title: "Plan Q4".to_owned(),
                access: Some(pb::DeskAccess { boards: true, channels_and_machines: false }),
                ..Default::default()
            })
            .await
            .expect("the Desk session starts")
            .into_inner();
        let id = started.session.expect("a start names its session");
        wait_status(&self.daemon, &id, Status::Working).await;
        let (_token, shell) = shell_token(&self.daemon, &id).await;
        let tabs = self.tabs_of(&id);
        assert_eq!(tabs.len(), 2, "an agent tab and a shell tab: {tabs:?}");
        let holders: Vec<u32> = tabs.iter().map(|tab| tab.holder_pid).collect();
        let programs = children_of(&holders);
        assert!(!programs.is_empty(), "the holders run programs");
        let folder = PathBuf::from(started.desk.and_then(|desk| desk.folder).expect("a start says where it works").path);
        (Running { id, folder, holders, programs }, shell)
    }

    async fn plan(&self, id: &pb::SessionId) -> pb::DeskMovePlan {
        self.desk()
            .await
            .plan_desk_move(pb::PlanDeskMoveRequest {
                session: Some(id.clone()),
                channel: self.channel.id.clone(),
                branch: String::new(),
            })
            .await
            .expect("the move is planned")
            .into_inner()
    }

    async fn start_move(&self, id: &pb::SessionId, plan: &pb::DeskMovePlan) -> Result<pb::Operation, tonic::Status> {
        self.desk()
            .await
            .move_desk_session_to_channel(pb::MoveDeskSessionToChannelRequest {
                session: Some(id.clone()),
                channel: self.channel.id.clone(),
                plan: plan.token.clone(),
                ..Default::default()
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// Waits until `id` isn't Moving, as a start that recovers it ends.
    async fn settled(&self, id: &pb::SessionId) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while get_session(&self.daemon, id).await.status == Status::Moving as i32 {
            assert!(Instant::now() < deadline, "the session is still moving");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Checks the holders and programs of `running` are the ones it started
    /// with, and still answer.
    fn assert_untouched(&self, running: &Running) {
        let tabs = self.tabs_of(&running.id);
        assert_eq!(
            tabs.iter().map(|tab| tab.holder_pid).collect::<Vec<_>>(),
            running.holders,
            "the holders are the same processes"
        );
        assert_eq!(children_of(&running.holders), running.programs, "so are the programs under them");
        assert!(tabs.iter().all(holder_answers), "the holders still answer");
    }
}

/// A Desk session with a running agent and a shell moves into a channel
/// without either stopping, and its old name resolves to the landing.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_moves_into_a_channel_with_its_agent_and_shell_running() {
    let fixture = Fixture::new().await;
    let (running, mut shell) = fixture.session().await;
    std::fs::write(running.folder.join("notes.md"), "# Q4\n").unwrap();
    let plan = fixture.plan(&running.id).await;
    let landing = PathBuf::from(&plan.folder);

    let started = fixture.start_move(&running.id, &plan).await.expect("a move that stops nothing needs no confirming");
    let moved = finished(&fixture.daemon, &started).await;

    assert_eq!(moved.state, pb::OperationState::Succeeded as i32, "{:?}", moved.error);
    let session = get_session(&fixture.daemon, &running.id).await;
    assert_eq!(session.channel, fixture.channel.id, "it is filed in the channel");
    let record = fixture.store().session(&running.id.value).unwrap().expect("the store has the session");
    assert_eq!(record.folder, landing, "its folder is the landing");
    assert!(fixture.store().desk(&running.id.value).unwrap().is_none(), "it left the Desk");
    assert_eq!(std::fs::read_to_string(landing.join("notes.md")).unwrap(), "# Q4\n");
    assert!(is_link(&running.folder), "the old name is a link");
    assert_eq!(std::fs::read_to_string(running.folder.join("notes.md")).unwrap(), "# Q4\n", "and it resolves");
    fixture.assert_untouched(&running);
    shell.type_text("echo alive-$((6*7))\r").await;
    shell.wait_for("alive-42").await;
}

/// A move cut after the rename and before the switch is undone at the next
/// start: the folder is renamed back, the session is a Desk session still,
/// and its holders and programs were never touched.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_cut_after_the_rename_is_renamed_back_at_the_next_start_with_everything_still_running() {
    let mut fixture = Fixture::new().await;
    let (running, _shell) = fixture.session().await;
    let plan = fixture.plan(&running.id).await;
    let landing = PathBuf::from(&plan.folder);
    fixture.daemon.restart_env(&[("HUMATA2_STOP_AT", "move-renamed")]).await;
    last_start(&fixture.daemon).await;

    // The daemon stops at the point, so the call may or may not answer.
    let _ = fixture.start_move(&running.id, &plan).await;
    fixture.daemon.stopped(Duration::from_secs(120)).await;
    fixture.daemon.restart_env(&[]).await;
    last_start(&fixture.daemon).await;

    fixture.settled(&running.id).await;
    until("the folder to be renamed back", || !is_link(&running.folder) && !landing.exists()).await;
    assert!(running.folder.is_dir(), "its Desk folder is where it was");
    assert!(fixture.store().desk(&running.id.value).unwrap().is_some(), "it is a Desk session still");
    assert_ne!(get_session(&fixture.daemon, &running.id).await.channel, fixture.channel.id);
    fixture.assert_untouched(&running);
}

/// A move cut after the switch is finished at the next start: the session
/// is in the channel at the landing, still running.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_cut_after_the_switch_is_finished_at_the_next_start_with_everything_still_running() {
    let mut fixture = Fixture::new().await;
    let (running, _shell) = fixture.session().await;
    let plan = fixture.plan(&running.id).await;
    let landing = PathBuf::from(&plan.folder);
    fixture.daemon.restart_env(&[("HUMATA2_STOP_AT", "move-switched")]).await;
    last_start(&fixture.daemon).await;

    let _ = fixture.start_move(&running.id, &plan).await;
    fixture.daemon.stopped(Duration::from_secs(120)).await;
    fixture.daemon.restart_env(&[]).await;
    last_start(&fixture.daemon).await;

    fixture.settled(&running.id).await;
    assert_eq!(get_session(&fixture.daemon, &running.id).await.channel, fixture.channel.id);
    let record = fixture.store().session(&running.id.value).unwrap().expect("the store has the session");
    assert_eq!(record.folder, landing);
    assert!(fixture.store().desk(&running.id.value).unwrap().is_none());
    assert!(landing.is_dir());
    fixture.assert_untouched(&running);
}
