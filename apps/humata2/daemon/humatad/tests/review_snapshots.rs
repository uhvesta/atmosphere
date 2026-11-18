//! Reviewed snapshots (RV-28) when a session's repos are compared against
//! different bases. A snapshot is per repo: the new side the user was shown in
//! that repo, kept in that repo's clone, with no base in it. So "Since I last
//! reviewed" must show each repo's own change since its own snapshot, whatever
//! base each repo was shown against when it was marked.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Daemon, folder, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_grpc::humata::v1::diff_spec_base::Base as SpecBase;
use humata2_store::{SessionRecord, Store};
use tonic::transport::Channel;

const SESSION: &str = "se_snapshots";
const LEDGER: &str = "ledger";
const API: &str = "api";

/// A session folder holding two clones with no owner: `api` on main, and
/// `ledger`, whose `release/2.4` branch main is one commit behind.
async fn daemon_with_two_repos() -> (Daemon, PathBuf, PathBuf) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_test".to_owned(),
            machine: String::new(),
            title: "snapshots".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    let api = dir.path().join(API);
    std::fs::create_dir(&api).unwrap();
    git(&api, &["init", "-q", "-b", "main"]);
    std::fs::write(api.join("refund.rs"), lines(1..=10)).unwrap();
    git(&api, &["add", "."]);
    git(&api, &["commit", "-q", "-m", "api first"]);

    let ledger = dir.path().join(LEDGER);
    std::fs::create_dir(&ledger).unwrap();
    git(&ledger, &["init", "-q", "-b", "main"]);
    std::fs::write(ledger.join("ledger.rs"), lines(1..=10)).unwrap();
    git(&ledger, &["add", "."]);
    git(&ledger, &["commit", "-q", "-m", "ledger first"]);
    git(&ledger, &["branch", "release/2.4"]);
    std::fs::write(ledger.join("fees.rs"), "fn fees() {}\n").unwrap();
    git(&ledger, &["add", "."]);
    git(&ledger, &["commit", "-q", "-m", "fees on main"]);

    drop(store);
    (Daemon::start_in(dir).await, api, ledger)
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

fn repo(name: &str) -> pb::RepoRef {
    pb::RepoRef {
        owner: String::new(),
        name: name.to_owned(),
    }
}

fn spec_repo(name: &str, base: SpecBase) -> pb::RepoDiffSpec {
    pb::RepoDiffSpec {
        repo: Some(repo(name)),
        base: Some(pb::DiffSpecBase { base: Some(base) }),
        ..pb::RepoDiffSpec::default()
    }
}

/// A spec with one base per repo: `ledger` against `ledger_base`, `api`
/// against `api_base`.
fn spec(ledger_base: SpecBase, api_base: SpecBase) -> Option<pb::DiffBase> {
    Some(pb::DiffBase {
        base: Some(pb::diff_base::Base::Spec(pb::DiffSpec {
            repos: vec![spec_repo(LEDGER, ledger_base), spec_repo(API, api_base)],
        })),
    })
}

fn since_last_reviewed() -> Option<pb::DiffBase> {
    spec(SpecBase::ReviewedSnapshot(0), SpecBase::ReviewedSnapshot(0))
}

async fn diffs(daemon: &Daemon) -> pb::diff_service_client::DiffServiceClient<Channel> {
    pb::diff_service_client::DiffServiceClient::new(daemon.channel().await)
}

async fn get_diff(daemon: &Daemon, base: Option<pb::DiffBase>) -> pb::GetDiffResponse {
    diffs(daemon)
        .await
        .get_diff(pb::GetDiffRequest {
            session: Some(session()),
            base,
            repos: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner()
}

async fn kept(daemon: &Daemon, name: &str) -> Vec<pb::ReviewedSnapshot> {
    diffs(daemon)
        .await
        .list_reviewed_snapshots(pb::ListReviewedSnapshotsRequest {
            session: Some(session()),
            repos: vec![repo(name)],
        })
        .await
        .unwrap()
        .into_inner()
        .snapshots
}

fn repo_name(repo: &Option<pb::RepoRef>) -> &str {
    repo.as_ref().map_or("", |repo| repo.name.as_str())
}

fn files(response: &pb::GetDiffResponse, name: &str) -> Vec<String> {
    response
        .repos
        .iter()
        .filter(|diff| repo_name(&diff.repo) == name)
        .flat_map(|diff| diff.files.iter().map(|file| file.path.clone()))
        .collect()
}

fn new_tree(response: &pb::GetDiffResponse, name: &str) -> String {
    response
        .repos
        .iter()
        .find(|diff| repo_name(&diff.repo) == name)
        .and_then(|diff| diff.trees.as_ref())
        .map(|trees| trees.new_tree.clone())
        .unwrap_or_else(|| panic!("the diff shows no {name}"))
}

fn snapshot_tree(snapshots: &[pb::ReviewedSnapshot], name: &str) -> String {
    snapshots
        .iter()
        .find(|snapshot| repo_name(&snapshot.repo) == name)
        .map(|snapshot| snapshot.tree.clone())
        .unwrap_or_else(|| panic!("no snapshot of {name}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn since_last_reviewed_shows_each_repos_own_change_when_the_bases_differ() {
    let (daemon, api, ledger) = daemon_with_two_repos().await;
    let mixed = spec(
        SpecBase::MergeBaseWith("release/2.4".to_owned()),
        SpecBase::MergeBaseWith("main".to_owned()),
    );
    let shown = get_diff(&daemon, mixed).await;
    assert_eq!(files(&shown, LEDGER), ["fees.rs"], "ledger against release/2.4 has main's commit");
    assert!(files(&shown, API).is_empty(), "api against main has no change yet");

    // Marked while ledger is compared with release/2.4 and api with main.
    let marked = diffs(&daemon)
        .await
        .mark_reviewed(pb::MarkReviewedRequest {
            session: Some(session()),
            ..pb::MarkReviewedRequest::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(marked.snapshots.len(), 2, "one snapshot per repo of the session");
    assert_eq!(
        snapshot_tree(&marked.snapshots, LEDGER),
        new_tree(&shown, LEDGER),
        "ledger's snapshot is the tree it was shown"
    );
    assert_eq!(
        snapshot_tree(&marked.snapshots, API),
        new_tree(&shown, API),
        "api's snapshot is the tree it was shown"
    );

    // A new change in each repo after the review.
    std::fs::write(ledger.join("late_ledger.rs"), "fn late_ledger() {}\n").unwrap();
    std::fs::write(api.join("late_api.rs"), "fn late_api() {}\n").unwrap();
    let since = get_diff(&daemon, since_last_reviewed()).await;
    assert_eq!(files(&since, LEDGER), ["late_ledger.rs"], "ledger shows only its own change");
    assert_eq!(files(&since, API), ["late_api.rs"], "api shows only its own change");

    // Marked again: the new snapshots are what was shown last, and the first
    // ones stay as they were.
    diffs(&daemon)
        .await
        .mark_reviewed(pb::MarkReviewedRequest {
            session: Some(session()),
            ..pb::MarkReviewedRequest::default()
        })
        .await
        .unwrap();
    let caught_up = get_diff(&daemon, since_last_reviewed()).await;
    assert!(!caught_up.any_changes, "after marking again, neither repo has anything new");

    let ledger_kept = kept(&daemon, LEDGER).await;
    assert_eq!(ledger_kept.iter().map(|snapshot| snapshot.n).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(ledger_kept[0].tree, new_tree(&shown, LEDGER), "ledger's first snapshot is untouched");
    let api_kept = kept(&daemon, API).await;
    assert_eq!(api_kept.iter().map(|snapshot| snapshot.n).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(api_kept[0].tree, new_tree(&shown, API), "api's first snapshot is untouched");
}
