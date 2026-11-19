//! A push of several repositories reports each one's pull request and note
//! (F3): the operation's `pull_requests` lists every repo, not just the last.
//! Through a real `humatad2`; GitHub is a fake `gh` that logs every call and
//! answers per repository, as in tests/push_base.rs.

use humatad_test_common as common;

use std::path::{Path, PathBuf};

use common::{
    Daemon, World, create_channel, finished, folder, git, repo_ref, start_session, wait_running,
};
use humata2_grpc::humata::v1 as pb;

/// A `gh` that logs each call as one line in `gh.log` beside it. `pr list`
/// answers PR 3 targeting main for a repo whose `open-<name>` file exists, and
/// none otherwise; `pr create` answers PR 7's URL. The repo is the fourth
/// argument (`pr list --repo acme/api ...`).
const FAKE_GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
repo=$(basename "$4")
echo "$*" >> "$here/gh.log"
case "$1 $2" in
  "pr list")
    if [ -e "$here/open-$repo" ]; then
      echo "[{\"number\":3,\"url\":\"https://github.com/acme/$repo/pull/3\",\"baseRefName\":\"main\"}]"
    else
      echo '[]'
    fi ;;
  "pr create") echo "https://github.com/acme/$repo/pull/7" ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

struct Fixture {
    daemon: Daemon,
    channel: pb::Channel,
    gh: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let gh_dir = dir.path().join("gh");
        std::fs::create_dir_all(&gh_dir).unwrap();
        let gh = gh_dir.join("gh");
        std::fs::write(&gh, FAKE_GH).unwrap();
        std::fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let gh_text = gh.display().to_string();
        let daemon = Daemon::start_with_env(
            common::Build::Admin,
            dir,
            &refs,
            &[
                ("HUMATA2_GH", gh_text.as_str()),
                ("HUMATA2_ARCHIVE_UNDO_MS", "600000"),
            ],
        )
        .await;
        world.remote("acme", "api");
        world.remote("acme", "ledger");
        let (channel, _) =
            create_channel(&daemon, "Payments", &[("acme", "api"), ("acme", "ledger")]).await;
        Self {
            daemon,
            channel,
            gh,
        }
    }

    async fn session(&self, title: &str) -> pb::Session {
        let (id, _) = start_session(&self.daemon, &self.channel, title).await;
        wait_running(&self.daemon, &id).await
    }

    /// Makes `repo` have an open pull request (PR 3, targeting main).
    fn open_pull_request_for(&self, repo: &str) {
        std::fs::write(self.gh.with_file_name(format!("open-{repo}")), "").unwrap();
    }

    /// Puts the clone's HEAD on origin as `branch` and fetches, so the clone
    /// lists the branch as origin's.
    fn publish(&self, repo: &pb::SessionRepo, branch: &str) {
        git(&["-C", &repo.path, "push", "origin", &format!("HEAD:refs/heads/{branch}")]);
        git(&["-C", &repo.path, "fetch", "--quiet", "origin"]);
    }
}

fn clone_named<'a>(session: &'a pb::Session, name: &str) -> &'a pb::SessionRepo {
    session
        .repos
        .iter()
        .find(|repo| repo.repo.as_ref().is_some_and(|r| r.name == name))
        .unwrap_or_else(|| panic!("the session has no {name} clone: {session:?}"))
}

/// Writes `file` in the clone and commits it.
fn commit_file(repo: &pb::SessionRepo, file: &str) {
    std::fs::write(Path::new(&repo.path).join(file), "work\n").unwrap();
    git(&["-C", &repo.path, "add", "-A"]);
    git(&["-C", &repo.path, "commit", "-m", "Work"]);
}

/// A choice that opens a pull request against `base_branch` (empty: none).
fn choice(name: &str, base_branch: &str) -> pb::PushChoice {
    pb::PushChoice {
        repo: Some(repo_ref("acme", name)),
        commit_message: String::new(),
        open_pull_request: true,
        pr_title: "Retry ledger writes".to_owned(),
        pr_body: "Retries a write that timed out.".to_owned(),
        base_branch: base_branch.to_owned(),
    }
}

fn pull_request_of<'a>(operation: &'a pb::Operation, name: &str) -> &'a pb::PullRequestRef {
    operation
        .pull_requests
        .iter()
        .find(|pr| pr.repo == Some(repo_ref("acme", name)))
        .unwrap_or_else(|| panic!("{name} has no pull request in the result: {operation:?}"))
}

#[tokio::test]
async fn a_push_of_two_repos_reports_both_pull_requests_and_the_open_ones_note() {
    let fixture = Fixture::new().await;
    fixture.open_pull_request_for("api");
    let session = fixture.session("Retry writes").await;
    let id = session.id.clone().unwrap();
    let api = clone_named(&session, "api").clone();
    let ledger = clone_named(&session, "ledger").clone();
    fixture.publish(&api, "release/2.4");
    commit_file(&api, "retry.rs");
    commit_file(&ledger, "retry.sql");
    let mut sessions = fixture.daemon.session_service().await;
    let plan = sessions
        .plan_push(pb::PlanPushRequest {
            id: Some(id.clone()),
            spec: None,
        })
        .await
        .unwrap()
        .into_inner();

    let started = sessions
        .push_and_open_pull_requests(pb::PushAndOpenPullRequestsRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            repos: vec![choice("api", "release/2.4"), choice("ledger", "")],
            plan: plan.token,
            from_view: None,
        })
        .await
        .unwrap()
        .into_inner();
    let operation = finished(&fixture.daemon, &started).await;

    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    assert_eq!(operation.pull_requests.len(), 2, "{operation:?}");
    // The open one targets main and was not retargeted to the picked base.
    let api_pr = pull_request_of(&operation, "api");
    assert_eq!(api_pr.number, 3, "{api_pr:?}");
    assert!(!api_pr.note.is_empty(), "{api_pr:?}");
    // The new one has nothing to note.
    let ledger_pr = pull_request_of(&operation, "ledger");
    assert_eq!(ledger_pr.number, 7, "{ledger_pr:?}");
    assert!(ledger_pr.note.is_empty(), "{ledger_pr:?}");
}
