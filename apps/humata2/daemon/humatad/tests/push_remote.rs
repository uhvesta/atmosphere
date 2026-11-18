//! Push & PR for a session on another machine (F3). The host relays PlanPush
//! and PushAndOpenPullRequests to the machine, which reads its own clones and
//! runs its own git and gh; the host's operation ends with the machine's pull
//! requests. A host and one box on the multi-machine rig, with a fake `gh`
//! (each box is an admin daemon; the rig gives it `HUMATA2_GH`).

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::path::{Path, PathBuf};

use common::{git, repo_ref};
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");

/// A `gh` that logs each call as one line in `gh.log` beside it, finds no
/// open pull request and answers a create with PR 7's URL.
const FAKE_GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
echo "$*" >> "$here/gh.log"
case "$1 $2" in
  "pr list") echo '[]' ;;
  "pr create") echo "https://github.com/acme/api/pull/7" ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

fn fake_gh(dir: &Path) -> PathBuf {
    let gh = dir.join("gh");
    std::fs::write(&gh, FAKE_GH).unwrap();
    std::fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    gh
}

fn api_of(plan: &pb::PushPlan) -> &pb::push_plan::Repo {
    plan.repos
        .iter()
        .find(|repo| repo.repo == Some(repo_ref(REPO.0, REPO.1)))
        .unwrap_or_else(|| panic!("the plan has no api repo: {plan:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn planning_and_pushing_a_box_session_runs_on_the_box() {
    let tools = tempfile::tempdir().unwrap();
    let gh = fake_gh(tools.path());
    // The box's daemon reads HUMATA2_GH; the rig gives it to that daemon's
    // process alone, so no environment of this test process changes.
    let gh_text = gh.display().to_string();
    let rig = Rig::start_seen_with_env(
        &["box-1"],
        &[REPO],
        &[("HUMATA2_GH", gh_text.as_str())],
    )
    .await;
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
    // The file exists in the box's clone only.
    std::fs::write(clone.join("refund.rs"), "fn round() {}\n").unwrap();
    let mut sessions = pb::session_service_client::SessionServiceClient::new(rig.channel().await);

    let dirty = sessions
        .plan_push(pb::PlanPushRequest {
            id: Some(id.clone()),
            spec: None,
        })
        .await
        .expect("the host plans a push of a box's session")
        .into_inner();

    let branch = git(&["-C", &clone_text, "rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(api_of(&dirty).branch, branch, "{dirty:?}");
    assert!(api_of(&dirty).dirty_files > 0, "{dirty:?}");

    // The box has no git identity of its own, so the commit is made here.
    git(&["-C", &clone_text, "add", "-A"]);
    git(&["-C", &clone_text, "commit", "-m", "Round refunds half up"]);
    let committed = sessions
        .plan_push(pb::PlanPushRequest {
            id: Some(id.clone()),
            spec: None,
        })
        .await
        .expect("the host plans again")
        .into_inner();
    assert_eq!(api_of(&committed).unpushed_commits, 1, "{committed:?}");

    let started = sessions
        .push_and_open_pull_requests(pb::PushAndOpenPullRequestsRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            repos: vec![pb::PushChoice {
                repo: Some(repo_ref(REPO.0, REPO.1)),
                commit_message: String::new(),
                open_pull_request: true,
                pr_title: "Round refunds half up".to_owned(),
                pr_body: String::new(),
                base_branch: String::new(),
            }],
            plan: committed.token,
            from_view: None,
        })
        .await
        .expect("the host starts the box's push")
        .into_inner();
    let ended = rig.ended(&started).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{ended:?}");
    match ended.pull_requests.as_slice() {
        [pr] => {
            assert_eq!(pr.number, 7, "{pr:?}");
            assert_eq!(pr.repo, Some(repo_ref(REPO.0, REPO.1)));
        }
        other => panic!("one pull request was expected: {other:?}"),
    }
    let bare = rig.world.remotes.join("acme").join("api.git");
    let pushed = std::process::Command::new("git")
        .args(["--git-dir", &bare.display().to_string(), "rev-parse", "--verify", "-q"])
        .arg(format!("refs/heads/{branch}"))
        .output()
        .unwrap();
    assert!(pushed.status.success(), "the box's branch reached origin");
    let calls = std::fs::read_to_string(tools.path().join("gh.log")).unwrap_or_default();
    assert!(
        calls.lines().any(|call| call.starts_with("pr create")),
        "the box ran gh itself: {calls:?}"
    );
}
