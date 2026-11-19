//! A push on a box's session that the host restarts in the middle of still
//! ends (F3, persistence). The host's operation is kept across the restart
//! under its own id, follows the box's operation to its end and ends with the
//! box's pull request, instead of hanging in Running or vanishing. The box's
//! `gh` waits on a file until the test releases it, so the push is mid-way
//! when the host is killed. Every wait is on a pushed change or on a
//! condition read with a deadline.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::path::Path;
use std::time::Duration;

use common::{git, repo_ref};
use humata2_grpc::humata::v1 as pb;
use remote_rig::{LONG, Rig, is_live};
use tokio::time::Instant;

const REPO: (&str, &str) = ("acme", "api");

/// A `gh` that logs each call as one line in `gh.log` beside it, finds no
/// open pull request, and on `pr create` marks `create-started` and waits
/// until `release` exists (at most two minutes, so a failed test leaves no
/// process behind) before it answers with PR 7's URL.
const FAKE_GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
echo "$*" >> "$here/gh.log"
case "$1 $2" in
  "pr list") echo '[]' ;;
  "pr create")
    : > "$here/create-started"
    waited=0
    while [ ! -e "$here/release" ] && [ "$waited" -lt 1200 ]; do
      sleep 0.1
      waited=$((waited + 1))
    done
    echo "https://github.com/acme/api/pull/7" ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

fn fake_gh(dir: &Path) {
    let gh = dir.join("gh");
    std::fs::write(&gh, FAKE_GH).unwrap();
    std::fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

/// Returns once the host's operation shows the push at a pull request step,
/// which the box reached only after the host had kept the push on disk.
async fn until_opening_pull_request(rig: &Rig, started: &pb::Operation) {
    let mut watch = pb::operation_service_client::OperationServiceClient::new(rig.channel().await)
        .watch_operation(pb::WatchOperationRequest {
            id: started.id.clone(),
            from_version: 0,
        })
        .await
        .expect("the push can be watched")
        .into_inner();
    let deadline = Instant::now() + LONG;
    let mut last = started.clone();
    loop {
        let opening = last
            .steps
            .iter()
            .any(|step| step.key.starts_with("open_pr") && step.state == pb::StepState::Running as i32);
        if opening {
            return;
        }
        match tokio::time::timeout_at(deadline, watch.message()).await {
            Ok(Ok(Some(next))) => last = next,
            other => panic!("the push never reached its pull request step ({other:?}); it last was {last:?}"),
        }
    }
}

/// How many pushes the host keeps on disk for a restart.
fn kept(host_state: &Path) -> usize {
    std::fs::read_dir(host_state.join("carry").join("remote_archive").join("push"))
        .map(|entries| entries.flatten().count())
        .unwrap_or(0)
}

/// A box's session with one committed change, and a host that can ask for its
/// push.
struct Pushing {
    rig: Rig,
    tools: tempfile::TempDir,
    id: pb::SessionId,
    plan: Option<pb::PlanToken>,
}

impl Pushing {
    /// `host_env` is the host's environment for its first run.
    async fn start(host_env: &[(&str, &str)]) -> Self {
        let tools = tempfile::tempdir().unwrap();
        fake_gh(tools.path());
        // The box's daemon gets HUMATA2_GH from the rig, which keeps it for
        // that process alone; only the host restarts, so the box keeps it.
        let gh_text = tools.path().join("gh").display().to_string();
        let mut rig =
            Rig::start_seen_with_env(&["box-1"], &[REPO], &[("HUMATA2_GH", gh_text.as_str())]).await;
        let one = Rig::box_id("box-1");
        let channel = rig
            .create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone()])
            .await;
        let id = rig.start_on(&channel, &one, "Fix refund rounding").await;
        rig.wait_session(&id, "live", is_live).await;
        let clone = rig
            .the("box-1")
            .path(&channel, "sessions/fix-refund-rounding/api");
        let clone_text = clone.display().to_string();
        std::fs::write(clone.join("refund.rs"), "fn round() {}\n").unwrap();
        // The box has no git identity of its own, so the commit is made here.
        git(&["-C", &clone_text, "add", "-A"]);
        git(&["-C", &clone_text, "commit", "-m", "Round refunds half up"]);
        let mut sessions = pb::session_service_client::SessionServiceClient::new(rig.channel().await);
        let plan = sessions
            .plan_push(pb::PlanPushRequest {
                id: Some(id.clone()),
                spec: None,
            })
            .await
            .expect("the host plans a push of a box's session")
            .into_inner()
            .token;
        if !host_env.is_empty() {
            rig.restart_host(host_env).await;
        }
        Self { rig, tools, id, plan }
    }

    /// Asks the host to push and open the pull request, under `key`.
    async fn ask(&self, key: Option<&str>) -> Result<pb::Operation, tonic::Status> {
        let mut sessions = pb::session_service_client::SessionServiceClient::new(self.rig.channel().await);
        sessions
            .push_and_open_pull_requests(pb::PushAndOpenPullRequestsRequest {
                idempotency_key: key.map(|value| pb::IdempotencyKey { value: value.to_owned() }),
                id: Some(self.id.clone()),
                repos: vec![pb::PushChoice {
                    repo: Some(repo_ref(REPO.0, REPO.1)),
                    commit_message: String::new(),
                    open_pull_request: true,
                    pr_title: "Round refunds half up".to_owned(),
                    pr_body: String::new(),
                    base_branch: String::new(),
                }],
                plan: self.plan.clone(),
                from_view: None,
            })
            .await
            .map(tonic::Response::into_inner)
    }

    fn release(&self) {
        std::fs::write(self.tools.path().join("release"), "").unwrap();
    }

    /// How many times the box's `gh` was asked to create a pull request.
    fn creates(&self) -> usize {
        let calls = std::fs::read_to_string(self.tools.path().join("gh.log")).unwrap_or_default();
        calls.lines().filter(|call| call.starts_with("pr create")).count()
    }

    /// The host's operation `id`, as it is now.
    async fn operation(&self, id: &pb::OperationId) -> Result<pb::Operation, tonic::Status> {
        pb::operation_service_client::OperationServiceClient::new(self.rig.channel().await)
            .get_operation(pb::GetOperationRequest { id: Some(id.clone()) })
            .await
            .map(tonic::Response::into_inner)
    }
}

/// The ids of the pushes the host keeps on disk.
fn kept_ids(host_state: &Path) -> Vec<String> {
    std::fs::read_dir(host_state.join("carry").join("remote_archive").join("push"))
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.path().file_stem().map(|stem| stem.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

/// Waits until the host keeps no push.
async fn until_nothing_kept(host_state: &Path) {
    let deadline = Instant::now() + LONG;
    while kept(host_state) != 0 {
        assert!(Instant::now() < deadline, "the host still keeps the ended push");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_on_a_box_session_ends_with_the_boxs_pull_request_after_the_host_restarts() {
    let mut push = Pushing::start(&[]).await;
    let started = push.ask(None).await.expect("the host starts the box's push");
    until_opening_pull_request(&push.rig, &started).await;
    assert_eq!(kept(&push.rig.host.state), 1, "the host keeps the push it is following");

    push.rig.restart_host(&[]).await;

    // The host knows the push again, under its id, and it still runs: the
    // box's gh has not been released.
    let id = started.id.clone().expect("an operation has an id");
    let restored = push
        .operation(&id)
        .await
        .expect("the host still knows the push after its restart");
    assert_eq!(restored.state, pb::OperationState::Running as i32, "{restored:?}");

    push.release();
    let ended = push.rig.ended(&started).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{ended:?}");
    match ended.pull_requests.as_slice() {
        [pr] => {
            assert_eq!(pr.number, 7, "{pr:?}");
            assert_eq!(pr.repo, Some(repo_ref(REPO.0, REPO.1)));
        }
        other => panic!("one pull request was expected: {other:?}"),
    }
    // The box pushed once: the host followed it, it did not ask again.
    assert_eq!(push.creates(), 1);
    // The record goes once the operation has ended.
    until_nothing_kept(&push.rig.host.state).await;
}

/// The host keeps the push, then stops before the box is asked. The restart
/// can't tell whether the box heard it, so it doesn't ask again: the push
/// ends Failed, saying to look at the box and push again, and the box makes
/// no pull request.
#[tokio::test(flavor = "multi_thread")]
async fn a_push_the_host_kept_but_never_sent_ends_failed_after_the_host_restarts() {
    let mut push = Pushing::start(&[("HUMATA2_STOP_AT", "push_remote.kept")]).await;
    // The host stops while it answers, so the call may fail.
    let _ = push.ask(None).await;
    push.rig.host_stopped().await;
    let ids = kept_ids(&push.rig.host.state);
    let [id] = ids.as_slice() else {
        panic!("the host kept one push before it stopped: {ids:?}");
    };

    push.rig.restart_host(&[]).await;

    let operation = pb::Operation {
        id: Some(pb::OperationId { value: id.clone() }),
        ..Default::default()
    };
    let ended = push.rig.ended(&operation).await;
    assert_eq!(ended.state, pb::OperationState::Failed as i32, "{ended:?}");
    assert_eq!(push.creates(), 0, "the box was never asked to open a pull request");
    until_nothing_kept(&push.rig.host.state).await;
}

/// A client that lost the answer asks again after the host's restart with
/// the same key: it gets the same operation, and the box makes one pull
/// request.
#[tokio::test(flavor = "multi_thread")]
async fn the_same_push_asked_again_after_the_host_restarts_is_the_same_operation() {
    let mut push = Pushing::start(&[]).await;
    let started = push.ask(Some("push-once")).await.expect("the host starts the box's push");
    until_opening_pull_request(&push.rig, &started).await;

    push.rig.restart_host(&[]).await;
    let again = push
        .ask(Some("push-once"))
        .await
        .expect("the host answers a repeated key after its restart");
    assert_eq!(again.id, started.id, "the repeated key names the push it started");

    push.release();
    let ended = push.rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{ended:?}");
    assert_eq!(ended.pull_requests.len(), 1, "{ended:?}");
    assert_eq!(push.creates(), 1, "the repeated key made no second pull request");
}
