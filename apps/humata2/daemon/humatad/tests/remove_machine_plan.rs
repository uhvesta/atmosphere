//! ChannelService.PlanRemoveMachine (CH-8, LD-10, MV-9): before a machine
//! leaves a channel, the plan lists that machine's live sessions with their
//! uncommitted and unpushed work, and a token for RemoveMachine. It changes
//! nothing. Through a real `humatad2`; needs FORK's archive (T3) and seam s4,
//! and moves up from tests/pending with them.

use humatad_test_common as common;

use std::path::PathBuf;

use common::{
    Daemon, World, create_channel, finished, folder, machine_id, start_session,
};
use humata2_grpc::humata::v1 as pb;

struct Fixture {
    daemon: Daemon,
    _world: World,
    channel: pb::Channel,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        // The archive's undo window never closes by itself here, so the
        // archived session's folder is still on disk during the plan.
        let daemon = Daemon::start_with_env(
            common::Build::Admin,
            dir,
            &refs,
            &[("HUMATA2_ARCHIVE_UNDO_MS", "600000")],
        )
        .await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        Self {
            daemon,
            _world: world,
            channel,
        }
    }

    fn channel_id(&self) -> pb::ChannelId {
        self.channel.id.clone().unwrap()
    }

    async fn session(&self, title: &str) -> pb::Session {
        let (id, _) = start_session(&self.daemon, &self.channel, title).await;
        common::wait_running(&self.daemon, &id).await
    }

    async fn plan(
        &self,
        channel: pb::ChannelId,
        machine: pb::MachineId,
    ) -> Result<pb::RemoveChannelMachinePlan, tonic::Status> {
        self.daemon
            .channel_service()
            .await
            .plan_remove_machine(pb::PlanRemoveChannelMachineRequest {
                channel: Some(channel),
                machine: Some(machine),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    async fn archive(&self, id: &pb::SessionId) {
        let mut sessions = self.daemon.session_service().await;
        let plan = sessions
            .plan_archive(pb::PlanArchiveRequest {
                id: Some(id.clone()),
            })
            .await
            .unwrap()
            .into_inner();
        let started = sessions
            .archive_session(pb::ArchiveSessionRequest {
                idempotency_key: None,
                id: Some(id.clone()),
                choice: pb::ArchiveChoice::Archive as i32,
                plan: plan.token,
                end: Some(true),
            })
            .await
            .unwrap()
            .into_inner();
        let operation = finished(&self.daemon, &started).await;
        assert_eq!(
            operation.state,
            pb::OperationState::Succeeded as i32,
            "{operation:?}"
        );
    }
}

fn clone_of(session: &pb::Session) -> PathBuf {
    PathBuf::from(&session.repos[0].path)
}

/// Every work-tree file under `root` with its contents, sorted. `.git` is
/// left out: `git status` may refresh the index's stat cache, which isn't a
/// change to the work.
fn snapshot(root: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
            if kind.is_dir() {
                if path.file_name().is_some_and(|name| name == ".git") {
                    continue;
                }
                stack.push(path);
            } else if kind.is_file() {
                let bytes = std::fs::read(&path).unwrap();
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}

#[tokio::test]
async fn the_plan_lists_live_sessions_with_their_work() {
    let fixture = Fixture::new().await;
    let dirty = fixture.session("Dirty").await;
    std::fs::write(clone_of(&dirty).join("wip.txt"), "wip\n").unwrap();
    let clean = fixture.session("Clean").await;
    let archived = fixture.session("Archived").await;
    fixture.archive(archived.id.as_ref().unwrap()).await;

    let machine = machine_id(&fixture.daemon).await;
    let plan = fixture
        .plan(fixture.channel_id(), machine.clone())
        .await
        .expect("the plan for this machine");

    assert_eq!(plan.channel, Some(fixture.channel_id()));
    assert_eq!(plan.machine, Some(machine));
    assert!(plan.token.as_ref().is_some_and(|t| !t.value.is_empty()), "{plan:?}");
    let work = |session: &pb::Session| {
        plan.sessions
            .iter()
            .find(|w| w.session == session.id)
            .map(|w| (w.dirty_files, w.unpushed_commits))
    };
    assert_eq!(work(&dirty), Some((1, 0)), "{plan:?}");
    assert_eq!(work(&clean), Some((0, 0)), "{plan:?}");
    assert_eq!(work(&archived), None, "an archived session isn't listed: {plan:?}");
    assert_eq!(plan.sessions.len(), 2, "{plan:?}");
}

#[tokio::test]
async fn each_plan_has_its_own_token() {
    let fixture = Fixture::new().await;
    fixture.session("One").await;
    let machine = machine_id(&fixture.daemon).await;
    let first = fixture.plan(fixture.channel_id(), machine.clone()).await.unwrap();
    let second = fixture.plan(fixture.channel_id(), machine).await.unwrap();
    assert_ne!(first.token, second.token);
}

#[tokio::test]
async fn a_machine_not_in_the_channel_is_refused() {
    let fixture = Fixture::new().await;
    let stranger = pb::MachineId {
        value: "ma_00000000000000000000000000".to_owned(),
    };
    let status = fixture
        .plan(fixture.channel_id(), stranger)
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::FailedPrecondition, "{status:?}");
}

#[tokio::test]
async fn an_unknown_channel_is_not_found() {
    let fixture = Fixture::new().await;
    let machine = machine_id(&fixture.daemon).await;
    let unknown = pb::ChannelId {
        value: "ch_00000000000000000000000000".to_owned(),
    };
    let status = fixture.plan(unknown, machine).await.unwrap_err();
    assert_eq!(status.code(), tonic::Code::NotFound, "{status:?}");
}

#[tokio::test]
async fn the_plan_changes_nothing() {
    let fixture = Fixture::new().await;
    let dirty = fixture.session("Dirty").await;
    std::fs::write(clone_of(&dirty).join("wip.txt"), "wip\n").unwrap();
    let clone = clone_of(&dirty);
    let before = snapshot(&clone);

    let machine = machine_id(&fixture.daemon).await;
    fixture.plan(fixture.channel_id(), machine).await.unwrap();

    assert_eq!(snapshot(&clone), before);
    let after = common::get_session(&fixture.daemon, dirty.id.as_ref().unwrap()).await;
    assert!(common::is_running(&after), "the session runs on: {:?}", after.status());
}
