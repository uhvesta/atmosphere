//! PushAndOpenPullRequests with a base branch per repo, and PlanPush with a
//! spec (B4): `--base` for a new pull request, an open one left as it is with
//! a note, a base missing on origin refused before any push, and each repo's
//! counts against its base in the spec. Through a real `humatad2`; GitHub is
//! a fake `gh` that logs every call, as in tests/push.rs.

use humatad_test_common as common;

use std::path::{Path, PathBuf};

use common::{
    Daemon, World, create_channel, finished, folder, git, repo_ref, start_session, wait_running,
};
use humata2_grpc::humata::v1 as pb;

/// A `gh` that logs each call as one line in `gh.log` beside it. `pr list`
/// answers no open PRs unless `open-pr` exists (then PR 3, which targets
/// main); `pr create` answers PR 7's URL; with `fail` present every call
/// fails as signed out.
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
      echo '[{"number":3,"url":"https://github.com/acme/api/pull/3","baseRefName":"main"}]'
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
        wait_running(&self.daemon, &id).await
    }

    fn gh_calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.gh.with_file_name("gh.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn creates(&self) -> Vec<String> {
        self.gh_calls()
            .into_iter()
            .filter(|call| call.starts_with("pr create"))
            .collect()
    }

    fn gh_flag(&self, name: &str) {
        std::fs::write(self.gh.with_file_name(name), "").unwrap();
    }

    async fn plan(&self, id: &pb::SessionId) -> pb::PushPlan {
        self.plan_with(id, None).await
    }

    async fn plan_with(&self, id: &pb::SessionId, spec: Option<pb::DiffSpec>) -> pb::PushPlan {
        self.daemon
            .session_service()
            .await
            .plan_push(pb::PlanPushRequest {
                id: Some(id.clone()),
                spec,
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

    /// Puts the clone's HEAD on origin as `branch` and fetches, so the clone
    /// lists the branch as origin's.
    fn publish(&self, repo: &pb::SessionRepo, branch: &str) {
        git(&["-C", &repo.path, "push", "origin", &format!("HEAD:refs/heads/{branch}")]);
        git(&["-C", &repo.path, "fetch", "--quiet", "origin"]);
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

/// A choice that opens a pull request against `base_branch` (empty: none).
fn choice(name: &str, commit_message: &str, base_branch: &str) -> pb::PushChoice {
    pb::PushChoice {
        repo: Some(repo_ref("acme", name)),
        commit_message: commit_message.to_owned(),
        open_pull_request: true,
        pr_title: "Retry ledger writes".to_owned(),
        pr_body: "Retries a write that timed out.".to_owned(),
        base_branch: base_branch.to_owned(),
    }
}

/// A spec that compares `name` with `base` (a merge base, as `origin/main`).
fn spec_against(name: &str, base: &str) -> pb::DiffSpec {
    pb::DiffSpec {
        repos: vec![pb::RepoDiffSpec {
            repo: Some(repo_ref("acme", name)),
            base: Some(pb::DiffSpecBase {
                base: Some(pb::diff_spec_base::Base::MergeBaseWith(base.to_owned())),
            }),
            ..pb::RepoDiffSpec::default()
        }],
    }
}

fn repo_plan<'a>(plan: &'a pb::PushPlan, name: &str) -> &'a pb::push_plan::Repo {
    plan.repos
        .iter()
        .find(|repo| repo.repo.as_ref().is_some_and(|r| r.name == name))
        .unwrap_or_else(|| panic!("{name} isn't in the plan: {plan:?}"))
}

fn state(operation: &pb::Operation) -> pb::OperationState {
    pb::OperationState::try_from(operation.state).unwrap()
}

#[tokio::test]
async fn a_new_pull_request_targets_the_picked_base() {
    let fixture = Fixture::new().await;
    let session = fixture.session("Retry writes").await;
    let id = session.id.clone().unwrap();
    let api = clone_named(&session, "api").clone();
    fixture.publish(&api, "release/2.4");
    std::fs::write(path_of(&api).join("retry.rs"), "fn retry() {}\n").unwrap();
    let plan = fixture.plan(&id).await;

    let operation = fixture
        .push(&id, vec![choice("api", "Retry", "release/2.4")], plan.token)
        .await
        .unwrap();

    assert_eq!(state(&operation), pb::OperationState::Succeeded, "{operation:?}");
    assert!(
        fixture
            .creates()
            .iter()
            .any(|call| call.contains("--base release/2.4")),
        "{:?}",
        fixture.gh_calls()
    );
}

#[tokio::test]
async fn an_origin_prefix_on_the_base_is_dropped() {
    let fixture = Fixture::new().await;
    let session = fixture.session("Retry writes").await;
    let id = session.id.clone().unwrap();
    let api = clone_named(&session, "api").clone();
    fixture.publish(&api, "release/2.4");
    std::fs::write(path_of(&api).join("retry.rs"), "fn retry() {}\n").unwrap();
    let plan = fixture.plan(&id).await;

    let operation = fixture
        .push(
            &id,
            vec![choice("api", "Retry", "origin/release/2.4")],
            plan.token,
        )
        .await
        .unwrap();

    assert_eq!(state(&operation), pb::OperationState::Succeeded, "{operation:?}");
    assert!(
        fixture
            .creates()
            .iter()
            .any(|call| call.contains("--base release/2.4")),
        "{:?}",
        fixture.gh_calls()
    );
}

#[tokio::test]
async fn a_base_missing_on_origin_is_refused_before_anything_is_pushed() {
    let fixture = Fixture::new().await;
    let session = fixture.session("Retry writes").await;
    let id = session.id.clone().unwrap();
    let api = clone_named(&session, "api").clone();
    std::fs::write(path_of(&api).join("retry.rs"), "fn retry() {}\n").unwrap();
    let plan = fixture.plan(&id).await;

    let status = fixture
        .push(&id, vec![choice("api", "Retry", "release/9.9")], plan.token)
        .await
        .unwrap_err();

    assert_eq!(status.code(), tonic::Code::FailedPrecondition, "{status:?}");
    assert!(!fixture.remote_branch_exists("api", &api.branch));
    assert!(fixture.creates().is_empty(), "{:?}", fixture.gh_calls());
}

#[tokio::test]
async fn an_open_pull_request_is_not_retargeted_and_the_result_says_where_it_targets() {
    let fixture = Fixture::new().await;
    fixture.gh_flag("open-pr");
    let session = fixture.session("Retry writes").await;
    let id = session.id.clone().unwrap();
    let api = clone_named(&session, "api").clone();
    fixture.publish(&api, "release/2.4");
    std::fs::write(path_of(&api).join("retry.rs"), "fn retry() {}\n").unwrap();
    let plan = fixture.plan(&id).await;

    let operation = fixture
        .push(&id, vec![choice("api", "Retry", "release/2.4")], plan.token)
        .await
        .unwrap();

    assert_eq!(state(&operation), pb::OperationState::Succeeded, "{operation:?}");
    assert!(fixture.remote_has("api", &api.branch, "retry.rs"));
    let calls = fixture.gh_calls();
    assert!(!calls.iter().any(|call| call.starts_with("pr edit")), "{calls:?}");
    assert!(fixture.creates().is_empty(), "{calls:?}");
    match operation.pull_requests.as_slice() {
        [pr] => {
            assert_eq!(pr.number, 3, "{pr:?}");
            assert!(!pr.note.is_empty(), "{pr:?}");
        }
        other => panic!("no pull request in the result: {other:?}"),
    }
}

#[tokio::test]
async fn the_plan_counts_each_repo_against_its_base_in_the_spec() {
    let fixture = Fixture::new().await;
    let session = fixture.session("Retry writes").await;
    let id = session.id.clone().unwrap();
    let api = clone_named(&session, "api").clone();
    // The session's commit becomes release/2.4's tip. Against release/2.4
    // only the work after it counts; against main the commit counts too.
    std::fs::write(path_of(&api).join("session.txt"), "one\ntwo\nthree\n").unwrap();
    git(&["-C", &api.path, "add", "session.txt"]);
    git(&["-C", &api.path, "commit", "-m", "Session work"]);
    fixture.publish(&api, "release/2.4");
    std::fs::write(path_of(&api).join("retry.rs"), "fn retry() {}\nfn again() {}\n").unwrap();

    let unset = fixture.plan(&id).await;
    let against_main = fixture
        .plan_with(&id, Some(spec_against("api", "origin/main")))
        .await;
    let against_release = fixture
        .plan_with(&id, Some(spec_against("api", "origin/release/2.4")))
        .await;

    let counts = |plan: &pb::PushPlan| {
        let api = repo_plan(plan, "api");
        (api.added, api.removed)
    };
    assert_eq!(counts(&against_release), (2, 0), "{against_release:?}");
    assert_eq!(counts(&against_main), (5, 0), "{against_main:?}");
    assert_eq!(counts(&unset), counts(&against_main), "{unset:?}");
}
