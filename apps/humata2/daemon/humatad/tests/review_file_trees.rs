//! GetFileDiff with the trees a client was shown (DV-4): the hunks of a
//! file come from those trees, listed again, even after the file's working
//! copy has changed back; without them the file is the spec's as it is now.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Daemon, folder, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use tonic::transport::Channel;
use tonic::{Code, Request};

const SESSION: &str = "se_review_trees";
const REPO: &str = "api";
const PATH: &str = "refund.rs";

/// A daemon that knows `SESSION`, whose folder holds a clone of `REPO`
/// with one commit.
async fn daemon_with_clone() -> (Daemon, PathBuf) {
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
    let clone = dir.path().join(REPO);
    std::fs::create_dir(&clone).unwrap();
    git(&clone, &["init", "-q", "-b", "main"]);
    std::fs::write(clone.join(PATH), lines(1..=10)).unwrap();
    git(&clone, &["add", "."]);
    git(&clone, &["commit", "-q", "-m", "first"]);
    drop(store);
    (Daemon::start_in(dir).await, clone)
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

fn repo() -> pb::RepoRef {
    pb::RepoRef {
        owner: "acme".to_owned(),
        name: REPO.to_owned(),
    }
}

fn file() -> Option<pb::RepoPath> {
    Some(pb::RepoPath {
        repo: Some(repo()),
        path: PATH.to_owned(),
    })
}

fn uncommitted() -> Option<pb::DiffBase> {
    Some(pb::DiffBase {
        base: Some(pb::diff_base::Base::UncommittedOnly(pb::UncommittedOnly {})),
    })
}

async fn diffs(daemon: &Daemon) -> pb::diff_service_client::DiffServiceClient<Channel> {
    pb::diff_service_client::DiffServiceClient::new(daemon.channel().await)
}

async fn get_diff(daemon: &Daemon) -> pb::GetDiffResponse {
    diffs(daemon)
        .await
        .get_diff(pb::GetDiffRequest {
            session: Some(session()),
            base: uncommitted(),
            repos: vec![repo()],
        })
        .await
        .unwrap()
        .into_inner()
}

async fn get_file_diff(
    daemon: &Daemon,
    trees: Option<pb::DiffTrees>,
) -> Result<pb::FileDiff, tonic::Status> {
    diffs(daemon)
        .await
        .get_file_diff(Request::new(pb::GetFileDiffRequest {
            session: Some(session()),
            base: uncommitted(),
            file: file(),
            trees,
            ..pb::GetFileDiffRequest::default()
        }))
        .await
        .map(|response| response.into_inner())
}

fn added_and_removed(diff: &pb::FileDiff) -> Vec<(i32, String)> {
    diff.hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter(|line| line.kind != pb::diff_line::Kind::Context as i32)
        .map(|line| (line.kind, line.text.clone()))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn hunks_follow_the_shown_trees_after_the_file_changes_back() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join(PATH), lines(1..=10).replace("line 5\n", "line five\n")).unwrap();
    let shown = get_diff(&daemon).await;
    let trees = shown.repos[0].trees.clone();
    assert!(trees.is_some(), "the diff names the trees it showed");

    // The edit is undone before the user asks for the file's hunks.
    git(&clone, &["checkout", "--", PATH]);

    let hunks = get_file_diff(&daemon, trees)
        .await
        .expect("the shown trees still list the edited file");
    assert_eq!(
        added_and_removed(&hunks),
        [
            (pb::diff_line::Kind::Removed as i32, "line 5".to_owned()),
            (pb::diff_line::Kind::Added as i32, "line five".to_owned()),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn without_trees_the_file_is_the_specs_as_it_is_now() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join(PATH), lines(1..=10).replace("line 5\n", "line five\n")).unwrap();
    let _shown = get_diff(&daemon).await;
    git(&clone, &["checkout", "--", PATH]);

    let status = get_file_diff(&daemon, None).await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound, "a file with no changes now has no hunks");
}
