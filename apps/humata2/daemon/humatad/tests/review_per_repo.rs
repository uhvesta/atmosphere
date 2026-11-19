//! Per-repo bases in one review (VIEWS B1, B2, B5), against a real daemon and
//! real clones of real remotes in the session's folder: a multi-repo spec
//! shows each file against its own repo's base, a repo the spec leaves out
//! shows against its default branch (origin/HEAD), and a spec that names a
//! repo outside the session, or one twice, is refused.

use humatad_test_common as common;

use std::path::Path;
use std::process::Command;

use common::{Daemon, folder, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use tonic::transport::Channel;
use tonic::{Code, Status};

const SESSION: &str = "se_per_repo";
const API: &str = "api";
const LEDGER: &str = "ledger";

/// A daemon whose session folder holds one clone per `(repo, default branch)`.
/// Each remote has `<repo>.rs` with lines 1 to 10 on its default branch and on
/// `release/2.4`, where line 1 reads `release one`.
async fn daemon_with_repos(repos: &[(&str, &str)]) -> Daemon {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_test".to_owned(),
            machine: String::new(),
            title: "review".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let remotes = dir.path().join("remotes");
    let seeds = dir.path().join("seeds");
    std::fs::create_dir_all(&remotes).unwrap();
    std::fs::create_dir_all(&seeds).unwrap();
    for &(repo, default) in repos {
        let file = format!("{repo}.rs");
        let seed = seeds.join(repo);
        std::fs::create_dir(&seed).unwrap();
        git(&seed, &["init", "-q", "-b", default]);
        std::fs::write(seed.join(&file), lines(1..=10)).unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-q", "-m", "first"]);
        git(&seed, &["checkout", "-q", "-b", "release/2.4"]);
        std::fs::write(seed.join(&file), lines(1..=10).replacen("line 1\n", "release one\n", 1)).unwrap();
        git(&seed, &["commit", "-q", "-am", "release"]);
        git(&seed, &["checkout", "-q", default]);
        let bare_name = format!("{repo}.git");
        git(&remotes, &["init", "-q", "--bare", bare_name.as_str()]);
        let bare = remotes.join(&bare_name);
        let head = format!("refs/heads/{default}");
        git(&bare, &["symbolic-ref", "HEAD", head.as_str()]);
        git(&seed, &["push", "-q", bare.to_str().unwrap(), "--all"]);
        git(dir.path(), &["clone", "-q", bare.to_str().unwrap(), repo]);
    }
    Daemon::start_in(dir).await
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("run git");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn lines(range: std::ops::RangeInclusive<u32>) -> String {
    range.map(|n| format!("line {n}\n")).collect()
}

fn session() -> pb::SessionId {
    pb::SessionId {
        value: SESSION.to_owned(),
    }
}

fn repo_ref(name: &str) -> pb::RepoRef {
    pb::RepoRef {
        owner: String::new(),
        name: name.to_owned(),
    }
}

fn repo_file(name: &str) -> pb::RepoPath {
    pb::RepoPath {
        repo: Some(repo_ref(name)),
        path: format!("{name}.rs"),
    }
}

fn diff_base(base: pb::diff_base::Base) -> Option<pb::DiffBase> {
    Some(pb::DiffBase { base: Some(base) })
}

/// A spec with one entry per `(repo, rev)`, each compared with that rev.
fn spec(repos: &[(&str, &str)]) -> Option<pb::DiffBase> {
    diff_base(pb::diff_base::Base::Spec(pb::DiffSpec {
        repos: repos
            .iter()
            .map(|&(repo, rev)| pb::RepoDiffSpec {
                repo: Some(repo_ref(repo)),
                base: Some(pb::DiffSpecBase {
                    base: Some(pb::diff_spec_base::Base::Rev(rev.to_owned())),
                }),
                ..pb::RepoDiffSpec::default()
            })
            .collect(),
    }))
}

async fn diffs(daemon: &Daemon) -> pb::diff_service_client::DiffServiceClient<Channel> {
    pb::diff_service_client::DiffServiceClient::new(daemon.channel().await)
}

/// GetDiff for every repo of the session.
async fn get_diff(daemon: &Daemon, base: Option<pb::DiffBase>) -> Result<pb::GetDiffResponse, Status> {
    diffs(daemon)
        .await
        .get_diff(pb::GetDiffRequest {
            session: Some(session()),
            base,
            repos: Vec::new(),
        })
        .await
        .map(|response| response.into_inner())
}

async fn file_hunks(daemon: &Daemon, base: Option<pb::DiffBase>, repo: &str) -> Result<Vec<pb::Hunk>, Status> {
    diffs(daemon)
        .await
        .get_file_diff(pb::GetFileDiffRequest {
            session: Some(session()),
            base,
            file: Some(repo_file(repo)),
            ..pb::GetFileDiffRequest::default()
        })
        .await
        .map(|response| response.into_inner().hunks)
}

/// Lines added and removed in `repo`'s diff.
fn totals(response: &pb::GetDiffResponse, repo: &str) -> (u32, u32) {
    let diff = response
        .repos
        .iter()
        .find(|diff| diff.repo.as_ref().is_some_and(|name| name.name == repo))
        .expect("the response has the repo");
    (diff.added, diff.removed)
}

/// The changed lines of `hunks`, as their kind and text.
fn changed(hunks: &[pb::Hunk]) -> Vec<(i32, String)> {
    hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.kind != pb::diff_line::Kind::Context as i32)
        .map(|line| (line.kind, line.text.clone()))
        .collect()
}

fn removed(text: &str) -> (i32, String) {
    (pb::diff_line::Kind::Removed as i32, text.to_owned())
}

fn added(text: &str) -> (i32, String) {
    (pb::diff_line::Kind::Added as i32, text.to_owned())
}

/// Changes line 10 of `repo`'s file in its clone, uncommitted.
fn edit_line_ten(daemon: &Daemon, repo: &str) {
    let path = daemon.dir.path().join(repo).join(format!("{repo}.rs"));
    std::fs::write(path, lines(1..=10).replace("line 10\n", "line ten\n")).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn each_file_shows_against_its_own_repos_base() {
    let daemon = daemon_with_repos(&[(API, "main"), (LEDGER, "master")]).await;
    edit_line_ten(&daemon, API);
    edit_line_ten(&daemon, LEDGER);
    let per_repo = spec(&[(LEDGER, "origin/release/2.4"), (API, "origin/main")]);

    let shown = get_diff(&daemon, per_repo.clone()).await.unwrap();
    assert_eq!(totals(&shown, API), (1, 1), "api is compared with origin/main");
    assert_eq!(totals(&shown, LEDGER), (2, 2), "ledger is compared with origin/release/2.4");

    let api = file_hunks(&daemon, per_repo.clone(), API).await.unwrap();
    assert_eq!(
        changed(&api),
        [removed("line 10"), added("line ten")],
        "api's hunks are against its own base, not ledger's"
    );
    let ledger = file_hunks(&daemon, per_repo, LEDGER).await.unwrap();
    assert_eq!(
        changed(&ledger),
        [removed("release one"), added("line 1"), removed("line 10"), added("line ten")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_the_spec_leaves_out_shows_against_its_default_branch() {
    let daemon = daemon_with_repos(&[(API, "main"), (LEDGER, "master")]).await;
    edit_line_ten(&daemon, API);
    edit_line_ten(&daemon, LEDGER);

    let shown = get_diff(&daemon, spec(&[(LEDGER, "origin/release/2.4")])).await.unwrap();
    assert_eq!(shown.repos.len(), 2, "both repos of the session show");
    assert_eq!(totals(&shown, API), (1, 1), "api isn't in the spec, so it is compared with main");
    assert_eq!(totals(&shown, LEDGER), (2, 2));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spec_naming_a_repo_outside_the_session_is_refused() {
    let daemon = daemon_with_repos(&[(API, "main")]).await;
    let error = get_diff(&daemon, spec(&[(API, "origin/main"), ("nowhere", "origin/main")]))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spec_naming_a_repo_twice_is_refused() {
    let daemon = daemon_with_repos(&[(API, "main"), (LEDGER, "master")]).await;
    let error = get_diff(&daemon, spec(&[(LEDGER, "origin/release/2.4"), (LEDGER, "origin/master")]))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_whose_origin_head_is_master_diffs_against_master_by_default() {
    let daemon = daemon_with_repos(&[(API, "main"), (LEDGER, "master")]).await;
    edit_line_ten(&daemon, LEDGER);

    let default = diff_base(pb::diff_base::Base::OriginMain(pb::OriginMain {}));
    let shown = get_diff(&daemon, default).await.unwrap();
    assert_eq!(totals(&shown, LEDGER), (1, 1), "ledger has no origin/main, so origin/HEAD, master, is its base");

    let since_review = diff_base(pb::diff_base::Base::SinceLastReviewed(pb::SinceLastReviewed {}));
    let shown = get_diff(&daemon, since_review).await.unwrap();
    assert_eq!(totals(&shown, LEDGER), (1, 1), "with no review yet, since last reviewed starts at master too");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_with_no_default_branch_is_refused_only_where_one_is_needed() {
    let daemon = daemon_with_repos(&[(LEDGER, "master")]).await;
    git(&daemon.dir.path().join(LEDGER), &["remote", "set-head", "origin", "-d"]);

    let uncommitted = diff_base(pb::diff_base::Base::UncommittedOnly(pb::UncommittedOnly {}));
    assert!(get_diff(&daemon, uncommitted).await.is_ok(), "a base that needs no default still works");

    let default = diff_base(pb::diff_base::Base::OriginMain(pb::OriginMain {}));
    let error = get_diff(&daemon, default).await.unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread")]
async fn finding_the_default_branch_lets_the_default_base_read_again() {
    let daemon = daemon_with_repos(&[(LEDGER, "trunk")]).await;
    git(&daemon.dir.path().join(LEDGER), &["remote", "set-head", "origin", "-d"]);
    edit_line_ten(&daemon, LEDGER);
    let default = || diff_base(pb::diff_base::Base::OriginMain(pb::OriginMain {}));

    let error = get_diff(&daemon, default()).await.unwrap_err();
    let kind = common::error_of(&error).kind;
    assert!(
        matches!(&kind, Some(pb::error::Kind::NoDefaultBranch(missing)) if missing.repo.as_ref().is_some_and(|repo| repo.name == LEDGER)),
        "the refusal names the repository with no default branch: {kind:?}"
    );

    let found = diffs(&daemon)
        .await
        .find_default_branch(pb::FindDefaultBranchRequest {
            idempotency_key: None,
            session: Some(session()),
            repo: Some(repo_ref(LEDGER)),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(found.default_branch, "trunk");

    let shown = get_diff(&daemon, default()).await.unwrap();
    assert_eq!(totals(&shown, LEDGER), (1, 1), "the default base is trunk now");
}

#[tokio::test(flavor = "multi_thread")]
async fn finding_the_default_branch_of_a_repo_outside_the_session_is_refused() {
    let daemon = daemon_with_repos(&[(LEDGER, "trunk")]).await;
    let error = diffs(&daemon)
        .await
        .find_default_branch(pb::FindDefaultBranchRequest {
            idempotency_key: None,
            session: Some(session()),
            repo: Some(repo_ref("elsewhere")),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
}
