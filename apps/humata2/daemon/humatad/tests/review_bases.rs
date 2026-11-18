//! ListBases on a real clone in a session's folder (B3): the branches one
//! repository's changes can be compared with, from its remote branches as
//! of the last fetch, default branch first; a repository the session doesn't
//! have is refused.

mod common;

use std::path::Path;
use std::process::Command;

use common::{Daemon, folder, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use tonic::{Code, Status};

const SESSION: &str = "se_bases";
const REPO: &str = "api";

/// A daemon that knows `SESSION`, whose folder holds a clone of `REPO`. The
/// clone is made from an upstream with `main`, `release/2.4` and `a-hotfix`,
/// which sorts before `main`, so its origin/HEAD points at `main` and the test
/// shows the default comes first.
async fn daemon_with_clone() -> Daemon {
    let dir = folder();
    // Outside the session's folder, so it isn't one of the session's repos.
    let upstream = folder();
    git(upstream.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(upstream.path().join("refund.rs"), "fn refund() {}\n").unwrap();
    git(upstream.path(), &["add", "."]);
    git(upstream.path(), &["commit", "-q", "-m", "first"]);
    git(upstream.path(), &["branch", "release/2.4"]);
    git(upstream.path(), &["branch", "a-hotfix"]);

    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_test".to_owned(),
            machine: String::new(),
            title: "bases".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    git(dir.path(), &["clone", "-q", upstream.path().to_str().unwrap(), REPO]);
    drop(store);
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

fn session() -> pb::SessionId {
    pb::SessionId {
        value: SESSION.to_owned(),
    }
}

fn repo(name: &str) -> pb::RepoRef {
    pb::RepoRef {
        owner: "acme".to_owned(),
        name: name.to_owned(),
    }
}

async fn list_bases(daemon: &Daemon, repo: pb::RepoRef) -> Result<pb::ListBasesResponse, Status> {
    pb::diff_service_client::DiffServiceClient::new(daemon.channel().await)
        .list_bases(pb::ListBasesRequest {
            session: Some(session()),
            repo: Some(repo),
        })
        .await
        .map(|response| response.into_inner())
}

#[tokio::test(flavor = "multi_thread")]
async fn one_clone_lists_its_remote_branches_with_the_default_first() {
    let daemon = daemon_with_clone().await;
    let listed = list_bases(&daemon, repo(REPO)).await.unwrap();
    assert_eq!(listed.branches, ["main", "a-hotfix", "release/2.4"]);
    assert_eq!(listed.default_branch, "main");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_the_session_does_not_have_is_refused() {
    let daemon = daemon_with_clone().await;
    let refused = list_bases(&daemon, repo("web")).await.unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
}
