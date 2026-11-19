//! Giving a context-only fork on another machine its own copy (FK-9): the
//! host relays GiveOwnCopy to the fork's machine, which copies the parent's
//! clones there onto a new branch and moves the agent; the host shows the
//! machine's steps, then the fork on its new branch. An offline machine is
//! refused at once and nothing changes.
//!
//! Waits are on the host's feeds (the rig's `wait_session`, `ended`).

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use humatad_test_common::upgrade;
use humatad_test_common::upgrade_rig;

use std::path::{Path, PathBuf};

use common::git;
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::Code;

const REPO: (&str, &str) = ("acme", "api");
const PARENT: &str = "sessions/cache-warmup/api";

fn fork_request(parent: &pb::SessionId, to: &pb::MachineId) -> pb::ForkSessionRequest {
    pb::ForkSessionRequest {
        idempotency_key: Some(pb::IdempotencyKey {
            value: format!("fork-{}", parent.value),
        }),
        parent: Some(parent.clone()),
        kind: pb::ForkKind::ContextOnly as i32,
        title: "Try it".to_owned(),
        machine: Some(to.clone()),
        ..Default::default()
    }
}

fn own_copy_request(fork: &pb::SessionId) -> pb::GiveOwnCopyRequest {
    pb::GiveOwnCopyRequest {
        idempotency_key: Some(pb::IdempotencyKey {
            value: format!("own-copy-{}", fork.value),
        }),
        fork: Some(fork.clone()),
    }
}

/// A box-1 session, and a context-only fork of it on box-1, both live.
async fn fork_on_box(rig: &Rig) -> (pb::Channel, pb::SessionId, pb::SessionId) {
    let one = Rig::box_id("box-1");
    let channel = rig
        .create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone()])
        .await;
    let parent = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&parent, "live", is_live).await;
    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .fork_session(fork_request(&parent, &one))
        .await
        .expect("the fork starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    let Some(pb::operation::Result::Session(fork)) = ended.result else {
        panic!("the fork names its session: {ended:?}");
    };
    rig.wait_session(&fork, "live on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s))
        .await;
    (channel, parent, fork)
}

/// The clone paths box-1's own records give `session`.
fn paths_on_box(rig: &Rig, session: &pb::SessionId) -> Vec<PathBuf> {
    rig.the("box-1")
        .store()
        .session_detail(&session.value)
        .unwrap()
        .unwrap_or_else(|| panic!("box-1 has no record of {}", session.value))
        .repos
        .into_iter()
        .map(|repo| repo.path)
        .collect()
}

fn branch_of(clone: &Path) -> String {
    git(&["-C", &clone.display().to_string(), "rev-parse", "--abbrev-ref", "HEAD"])
}

/// FK-9 on a box: the fork moves onto its own clones on box-1, on a branch
/// of its own, and the host shows that branch; the parent keeps its clone.
#[tokio::test(flavor = "multi_thread")]
async fn a_context_only_fork_on_a_box_gets_its_own_clones_there() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (channel, parent, fork) = fork_on_box(&rig).await;
    let parent_clone = rig.the("box-1").path(&channel, PARENT);
    assert_eq!(paths_on_box(&rig, &fork), paths_on_box(&rig, &parent));
    let before = rig.session(&parent).await;

    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .give_own_copy(own_copy_request(&fork))
        .await
        .expect("the copy starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(ended.kind, pb::OperationKind::GiveOwnCopy as i32);

    let own = paths_on_box(&rig, &fork);
    assert_eq!(own.len(), 1, "{own:?}");
    assert_ne!(own, paths_on_box(&rig, &parent), "box-1 moved the fork onto its own clones");
    let branch = branch_of(&own[0]);
    assert_ne!(branch, before.repos[0].branch, "on a branch of its own");
    assert_eq!(branch_of(&parent_clone), before.repos[0].branch, "the parent keeps its branch");

    let shown = rig
        .wait_session(&fork, "live on its own branch", |s| {
            is_live(s) && s.repos.first().is_some_and(|repo| repo.branch == branch)
        })
        .await;
    assert_eq!(shown.machine.as_ref(), Some(&Rig::box_id("box-1")));
    assert!(is_live(&rig.session(&parent).await), "the parent runs on");
}

/// An offline box: refused at once, and the fork stays in its parent's clones.
#[tokio::test(flavor = "multi_thread")]
async fn own_copy_of_a_fork_on_an_offline_box_is_refused_and_changes_nothing() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (_, parent, fork) = fork_on_box(&rig).await;
    rig.link_lost("box-1").await;

    let refused = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .give_own_copy(own_copy_request(&fork))
        .await
        .expect_err("an offline machine is refused at once");
    assert_eq!(refused.code(), Code::Unavailable, "{refused:?}");
    assert_eq!(paths_on_box(&rig, &fork), paths_on_box(&rig, &parent));
}

/// A machine on an older humatad2 refuses the relayed call as off its list
/// (D773): the host answers peer_version_unsupported naming the machine and
/// the Migrate fix, and the machine's Migrate item is in the Inbox.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_that_refuses_the_relayed_call_as_off_its_list_is_to_be_migrated() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    rig.restart_box_env(upgrade_rig::BOX, &[("HUMATA2_PEER_WITHHOLD", "GiveOwnCopy")])
        .await;
    let one = upgrade_rig::box_id();
    let channel = rig.create_channel("Payments", &[REPO], &[one.clone()]).await;
    let parent = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&parent, "live", is_live).await;
    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .fork_session(fork_request(&parent, &one))
        .await
        .expect("the fork starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    let Some(pb::operation::Result::Session(fork)) = ended.result else {
        panic!("the fork names its session: {ended:?}");
    };
    rig.wait_session(&fork, "live on gpu", |s| s.machine.as_ref() == Some(&one) && is_live(s))
        .await;

    let refused = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .give_own_copy(own_copy_request(&fork))
        .await
        .expect_err("the machine doesn't take the call");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    let error = common::error_of(&refused);
    assert!(
        matches!(error.kind, Some(pb::error::Kind::PeerVersionUnsupported(_))),
        "{error:?}"
    );
    assert!(error.fix.contains("Migrate gpu"), "{error:?}");
    upgrade_rig::daemon_item(&rig).await;
}

/// The same for another relayed method: a fork onto a machine that refuses
/// ForkSession as off its list fails as peer_version_unsupported, and the
/// machine's Migrate item is in the Inbox.
#[tokio::test(flavor = "multi_thread")]
async fn another_relayed_method_refused_as_off_the_list_also_posts_the_migrate_item() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    rig.restart_box_env(upgrade_rig::BOX, &[("HUMATA2_PEER_WITHHOLD", "ForkSession")])
        .await;
    let one = upgrade_rig::box_id();
    let channel = rig.create_channel("Payments", &[REPO], &[one.clone()]).await;
    let parent = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&parent, "live", is_live).await;
    let outcome = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .fork_session(fork_request(&parent, &one))
        .await;
    let error = match outcome {
        Err(refused) => common::error_of(&refused),
        Ok(started) => {
            let ended = rig.ended(&started.into_inner()).await;
            assert_eq!(ended.state, pb::OperationState::Failed as i32, "{ended:?}");
            ended.error.expect("the failed fork says why")
        }
    };
    assert!(
        matches!(error.kind, Some(pb::error::Kind::PeerVersionUnsupported(_))),
        "{error:?}"
    );
    upgrade_rig::daemon_item(&rig).await;
}
