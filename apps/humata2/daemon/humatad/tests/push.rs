//! PlanPush, PushAndOpenPullRequests (MV-12, GRPC §7 PUSH_AND_OPEN_PR) and
//! Commit, push and open a PR, then archive (MV-10), through a real
//! `humatad2`. GitHub is a fake `gh` (admin builds read `HUMATA2_GH`) that
//! logs its arguments and answers `pr list` and `pr create`; the remotes are
//! the world's bare repositories. Moves up from tests/pending with FORK's
//! push batch.

mod common;

use std::path::{Path, PathBuf};

use common::{
    Daemon, World, create_channel, finished, folder, get_session, git, repo_ref, start_session,
};
use humata2_grpc::humata::v1 as pb;
use pb::SessionStatus as Status;

/// A `gh` that logs each call as one line in `gh.log` beside it. `pr list`
/// answers no open PRs unless `open-pr` exists (then PR 3); `pr create`
/// answers PR 7's URL; with `fail` present every call fails as signed out.
const FAKE_GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
echo "$*" >> "$here/gh.log"
if [ -e "$here/fail" ]; then
  echo "To get started with GitHub CLI, please run: gh auth login" >&2
  exit 4
fi
case "$1 $2" in
  "pr list")
    if [ -e "$here/open-pr" ]; then
      echo '[{"number":3,"url":"https://github.com/acme/api/pull/3"}]'
    else
      echo '[]'
    fi ;;
  "pr create") echo "https://github.com/acme/api/pull/7" ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

struct Fixture {
    daemon: Daemon,
    world: World,
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
            world,
            channel,
            gh,
        }
    }

    async fn session(&self, title: &str) -> pb::Session {
        let (id, _) = start_session(&self.daemon, &self.channel, title).await;
        common::wait_running(&self.daemon, &id).await
    }

    fn gh_calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.gh.with_file_name("gh.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn gh_flag(&self, name: &str) {
        std::fs::write(self.gh.with_file_name(name), "").unwrap();
    }

    async fn plan(&self, id: &pb::SessionId) -> pb::PushPlan {
        self.daemon
            .session_service()
            .await
            .plan_push(pb::PlanPushRequest {
                id: Some(id.clone()),
            })
            .await
            .unwrap()
            .into_inner()
    }

    async fn push(
        &self,
        id: &pb::SessionId,
        repos: Vec<pb::PushChoice>,
        plan: Option<pb::PlanToken>,
    ) -> Result<pb::Operation, tonic::Status> {
        let started = self
            .daemon
            .session_service()
            .await
            .push_and_open_pull_requests(pb::PushAndOpenPullRequestsRequest {
                idempotency_key: None,
                id: Some(id.clone()),
                repos,
                plan,
                from_view: None,
            })
            .await?
            .into_inner();
        Ok(finished(&self.daemon, &started).await)
    }

    fn remote_has(&self, name: &str, branch: &str, file: &str) -> bool {
        let bare = self.world.remotes.join("acme").join(format!("{name}.git"));
        std::process::Command::new("git")
            .args(["--git-dir", &bare.display().to_string(), "cat-file", "-e"])
            .arg(format!("{branch}:{file}"))
            .status()
            .unwrap()
            .success()
    }

    fn remote_branch_exists(&self, name: &str, branch: &str) -> bool {
        let bare = self.world.remotes.join("acme").join(format!("{name}.git"));
        std::process::Command::new("git")
            .args(["--git-dir", &bare.display().to_string(), "rev-parse", "--verify", "-q"])
            .arg(format!("refs/heads/{branch}"))
            .output()
            .unwrap()
            .status
            .success()
    }
}

fn clone_named<'a>(session: &'a pb::Session, name: &str) -> &'a pb::SessionRepo {
    session
        .repos
        .iter()
        .find(|repo| repo.repo.as_ref().is_some_and(|r| r.name == name))
        .unwrap_or_else(|| panic!("the session has no {name} clone: {session:?}"))
}

fn path_of(repo: &pb::SessionRepo) -> &Path {
    Path::new(&repo.path)
}

fn choice(name: &str, commit_message: &str, open_pull_request: bool) -> pb::PushChoice {
    pb::PushChoice {
        repo: Some(repo_ref("acme", name)),
        commit_message: commit_message.to_owned(),
        open_pull_request,
        pr_title: if open_pull_request {
            "Retry ledger writes".to_owned()
        } else {
            String::new()
        },
        pr_body: if open_pull_request {
            "Retries a write that timed out.".to_owned()
        } else {
            String::new()
        },
    }
}

fn state(operation: &pb::Operation) -> pb::OperationState {
    pb::OperationState::try_from(operation.state).unwrap()
}

#[tokio::test]
async fn the_plan_lists_each_repo_with_its_branch_and_totals() {
    let fixture = Fixture::new().await;
    let session = fixture.session("Retry writes").await;
    let api = clone_named(&session, "api");
    std::fs::write(path_of(api).join("retry.rs"), "fn retry() {}\nfn again() {}\n").unwrap();

    let plan = fixture.plan(session.id.as_ref().unwrap()).await;

    assert_eq!(plan.id, session.id);
    assert!(plan.token.as_ref().is_some_and(|t| !t.value.is_empty()), "{plan:?}");
    assert_eq!(plan.repos.len(), 2, "{plan:?}");
    let listed = |name: &str| {
        plan.repos
            .iter()
            .find(|repo| repo.repo.as_ref().is_some_and(|r| r.name == name))
            .unwrap_or_else(|| panic!("{name} isn't in the plan: {plan:?}"))
    };
    let api_plan = listed("api");
    assert_eq!(api_plan.branch, api.branch);
    assert_eq!(api_plan.dirty_files, 1);
    assert_eq!(api_plan.unpushed_commits, 0);
    assert_eq!(api_plan.added, 2);
    assert!(api_plan.existing.is_none());
    let ledger_plan = listed("ledger");
    assert_eq!((ledger_plan.dirty_files, ledger_plan.added), (0, 0));
}

#[tokio::test]
async fn the_plan_names_an_open_pull_request() {
    let fixture = Fixture::new().await;
    fixture.gh_flag("open-pr");
    let session = fixture.session("Retry writes").await;
    let plan = fixture.plan(session.id.as_ref().unwrap()).await;
    let api_plan = plan
        .repos
        .iter()
        .find(|repo| repo.repo.as_ref().is_some_and(|r| r.name == "api"))
        .unwrap();
    assert_eq!(api_plan.existing.as_ref().map(|pr| pr.number), Some(3), "{plan:?}");
}

#[tokio::test]
async fn pushing_one_repo_commits_pushes_and_opens_its_pull_request() {
    let fixture = Fixture::new().await;
    let session = fixture.session("Retry writes").await;
    let id = session.id.clone().unwrap();
    let api = clone_named(&session, "api").clone();
    let ledger = clone_named(&session, "ledger").clone();
    std::fs::write(path_of(&api).join("retry.rs"), "fn retry() {}\n").unwrap();
    std::fs::write(path_of(&ledger).join("later.sql"), "-- later\n").unwrap();
    let plan = fixture.plan(&id).await;

    let operation = fixture
        .push(&id, vec![choice("api", "Retry ledger writes", true)], plan.token)
        .await
        .unwrap();

    assert_eq!(state(&operation), pb::OperationState::Succeeded, "{operation:?}");
    assert!(fixture.remote_has("api", &api.branch, "retry.rs"));
    assert_eq!(
        git(&["-C", &api.path, "status", "--porcelain"]),
        "",
        "the commit took the uncommitted file"
    );
    match &operation.result {
        Some(pb::operation::Result::PullRequest(pr)) => {
            assert_eq!(pr.number, 7);
            assert_eq!(pr.repo, Some(repo_ref("acme", "api")));
        }
        other => panic!("no pull request in the result: {other:?}"),
    }
    let creates: Vec<String> = fixture
        .gh_calls()
        .into_iter()
        .filter(|call| call.starts_with("pr create"))
        .collect();
    assert_eq!(creates.len(), 1, "{creates:?}");
    assert!(creates[0].contains(&api.branch), "{creates:?}");
    assert!(creates[0].contains("Retry ledger writes"), "{creates:?}");
    // The repo left for later is as it was.
    assert!(!fixture.remote_branch_exists("ledger", &ledger.branch));
    assert!(path_of(&ledger).join("later.sql").exists());
    assert_ne!(git(&["-C", &ledger.path, "status", "--porcelain"]), "");
}

#[tokio::test]
async fn pushing_without_a_pull_request_never_runs_gh_create() {
    let fixture = Fixture::new().await;
    let session = fixture.session("Retry writes").await;
    let id = session.id.clone().unwrap();
    let api = clone_named(&session, "api").clone();
    std::fs::write(path_of(&api).join("retry.rs"), "fn retry() {}\n").unwrap();
    let plan = fixture.plan(&id).await;

    let operation = fixture
        .push(&id, vec![choice("api", "Retry", false)], plan.token)
        .await
        .unwrap();

    assert_eq!(state(&operation), pb::OperationState::Succeeded, "{operation:?}");
    assert!(fixture.remote_has("api", &api.branch, "retry.rs"));
    assert!(!fixture.gh_calls().iter().any(|call| call.starts_with("pr create")));
}

#[tokio::test]
async fn a_push_needs_a_plan_for_that_session_used_once() {
    let fixture = Fixture::new().await;
    let session = fixture.session("Retry writes").await;
    let id = session.id.clone().unwrap();
    let other = fixture.session("Another").await;
    let other_plan = fixture.plan(other.id.as_ref().unwrap()).await;

    for token in [None, other_plan.token] {
        let status = fixture
            .push(&id, vec![choice("api", "", false)], token)
            .await
            .unwrap_err();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition, "{status:?}");
    }

    let plan = fixture.plan(&id).await;
    let first = fixture
        .push(&id, vec![choice("api", "", false)], plan.token.clone())
        .await
        .unwrap();
    assert_eq!(state(&first), pb::OperationState::Succeeded, "{first:?}");
    let again = fixture
        .push(&id, vec![choice("api", "", false)], plan.token)
        .await
        .unwrap_err();
    assert_eq!(again.code(), tonic::Code::FailedPrecondition, "{again:?}");
}

#[tokio::test]
async fn a_failed_pull_request_says_what_to_fix_and_keeps_the_push() {
    let fixture = Fixture::new().await;
    let session = fixture.session("Retry writes").await;
    let id = session.id.clone().unwrap();
    let api = clone_named(&session, "api").clone();
    std::fs::write(path_of(&api).join("retry.rs"), "fn retry() {}\n").unwrap();
    let plan = fixture.plan(&id).await;
    fixture.gh_flag("fail");

    let operation = fixture
        .push(&id, vec![choice("api", "Retry", true)], plan.token)
        .await
        .unwrap();

    assert_eq!(state(&operation), pb::OperationState::Failed, "{operation:?}");
    let error = operation.error.clone().expect("a failed push says why");
    assert!(!error.fix.is_empty(), "{error:?}");
    assert!(
        fixture.remote_has("api", &api.branch, "retry.rs"),
        "the push before the PR stays done"
    );
}

#[tokio::test]
async fn commit_push_pr_then_archive_archives_after_the_pull_request() {
    let fixture = Fixture::new().await;
    let session = fixture.session("Retry writes").await;
    let id = session.id.clone().unwrap();
    let api = clone_named(&session, "api").clone();
    std::fs::write(path_of(&api).join("retry.rs"), "fn retry() {}\n").unwrap();
    let plan = fixture
        .daemon
        .session_service()
        .await
        .plan_archive(pb::PlanArchiveRequest {
            id: Some(id.clone()),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(plan
        .choices
        .contains(&(pb::ArchiveChoice::CommitPushPrThenArchive as i32)));

    let started = fixture
        .daemon
        .session_service()
        .await
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            choice: pb::ArchiveChoice::CommitPushPrThenArchive as i32,
            plan: plan.token,
        })
        .await
        .unwrap()
        .into_inner();
    let operation = finished(&fixture.daemon, &started).await;

    assert_eq!(state(&operation), pb::OperationState::Succeeded, "{operation:?}");
    assert!(fixture.remote_has("api", &api.branch, "retry.rs"));
    assert!(fixture.gh_calls().iter().any(|call| call.starts_with("pr create")));
    let archived = get_session(&fixture.daemon, &id).await;
    assert_eq!(archived.status, Status::Archived as i32);
}

#[tokio::test]
async fn commit_push_pr_then_archive_archives_nothing_when_the_pull_request_fails() {
    let fixture = Fixture::new().await;
    let session = fixture.session("Retry writes").await;
    let id = session.id.clone().unwrap();
    let api = clone_named(&session, "api").clone();
    std::fs::write(path_of(&api).join("retry.rs"), "fn retry() {}\n").unwrap();
    let plan = fixture
        .daemon
        .session_service()
        .await
        .plan_archive(pb::PlanArchiveRequest {
            id: Some(id.clone()),
        })
        .await
        .unwrap()
        .into_inner();
    fixture.gh_flag("fail");

    let started = fixture
        .daemon
        .session_service()
        .await
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            choice: pb::ArchiveChoice::CommitPushPrThenArchive as i32,
            plan: plan.token,
        })
        .await
        .unwrap()
        .into_inner();
    let operation = finished(&fixture.daemon, &started).await;

    assert_eq!(state(&operation), pb::OperationState::Failed, "{operation:?}");
    assert!(operation.error.as_ref().is_some_and(|e| !e.fix.is_empty()));
    let after = get_session(&fixture.daemon, &id).await;
    // Nothing was archived and the agent runs on; its turn state may have
    // gone quiet meanwhile (SE-22), so only "not archived" is asserted.
    assert_ne!(after.status, Status::Archived as i32, "{after:?}");
    assert!(after.archive.is_none(), "{after:?}");
    let listed = fixture
        .daemon
        .session_service()
        .await
        .list_sessions(pb::ListSessionsRequest {
            channel: None,
            machine: None,
            include_archived: false,
            page: None,
        })
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert!(listed.iter().any(|s| s.id == after.id), "it is still listed");
    assert!(path_of(&api).exists());
}
