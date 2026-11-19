//! Pasted pull requests through `LivePullRequests` against a real `humatad2`
//! (admin build, which runs the `gh` named by `HUMATA2_GH`). The answers are
//! the ones the fake client gives for the same pull request (PR-1, PR-3), and
//! each gh failure comes back as its own error with what to do about it.
//! Reviews themselves (start, heads, submit) need a clone of a pull request
//! head and so are covered by the daemon's own tests, not here.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use humata2_clients::model::Agent;
use humata2_clients::pull_requests::{
    FakePullRequests, PullRequestError, PullRequestInfo, PullRequestRef, PullRequestState,
    PullRequestsClient,
};
use humata2_clients_live_core::{LiveConnection, LiveOptions};
use humata2_clients_live_review::LivePullRequests;
use tempfile::TempDir;

/// How long anything may take before the case fails.
const DEADLINE: Duration = Duration::from_secs(60);

/// How often the socket is tried while the daemon starts.
const POLL: Duration = Duration::from_millis(50);

/// A signed-in gh. `acme/api#482` is a pull request with checks; `#401` says
/// gh is signed out, `#429` is rate limited (its reset is `RESET`, which the
/// test fills in), `#999` is not a pull request and `acme/private` is a
/// repository this account can't see. Anything else is unexpected.
const FAKE_GH: &str = r#"#!/bin/sh
case "$*" in
  "pr view 482 --repo acme/api --json "*)
    echo '{"number":482,"url":"https://github.com/acme/api/pull/482","title":"Round refunds half a cent","author":{"login":"mira"},"baseRefName":"main","headRefName":"fix/refund-rounding","headRefOid":"b2","changedFiles":3,"additions":42,"deletions":7,"state":"OPEN","mergedAt":null,"statusCheckRollup":[{"conclusion":"SUCCESS"},{"conclusion":"SUCCESS"},{"conclusion":"SUCCESS"},{"conclusion":"FAILURE"}]}' ;;
  "pr view 401 --repo acme/api --json "*)
    echo "To get started with GitHub CLI, please run:  gh auth login" >&2; exit 4 ;;
  "pr view 429 --repo acme/api --json "*)
    echo "GraphQL: API rate limit exceeded for user ID 1. (HTTP 403)" >&2; exit 1 ;;
  "pr view 999 --repo acme/api --json "*)
    echo "GraphQL: Could not resolve to a PullRequest with the number of 999. (repository.pullRequest)" >&2; exit 1 ;;
  "pr view 1 --repo acme/private --json "*)
    echo "GraphQL: Could not resolve to a Repository with the name 'acme/private'. (repository)" >&2; exit 1 ;;
  "api rate_limit")
    echo '{"resources":{"core":{"limit":5000,"remaining":0,"reset":RESET},"graphql":{"limit":5000,"remaining":0,"reset":RESET}},"rate":{"limit":5000,"remaining":0,"reset":RESET}}' ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

fn binary(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable} to the binary's path"));
    std::fs::canonicalize(path).expect("the binary exists")
}

async fn within<T>(what: &str, work: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(DEADLINE, work).await {
        Ok(value) => value,
        Err(_) => panic!("timed out waiting for {what}"),
    }
}

/// A real `humatad2` in a folder of its own; dropping it stops the daemon.
struct Served {
    dir: TempDir,
    child: Child,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = Command::new("pkill").arg("-f").arg(self.dir.path()).status();
    }
}

/// A daemon whose gh says the limit resets in ten minutes.
async fn start() -> (Served, LiveConnection) {
    use std::os::unix::fs::PermissionsExt;
    // Short, so the socket's path fits.
    let dir = tempfile::Builder::new()
        .prefix("h2q")
        .tempdir_in("/tmp")
        .expect("make a temp folder");
    // The daemon refuses a socket folder other users can open.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
        .expect("restrict the temp folder");
    let root = dir.path().to_path_buf();
    let reset = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
        + 600;
    let gh = root.join("gh");
    std::fs::write(&gh, FAKE_GH.replace("RESET", &reset.to_string())).expect("write the fake gh");
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).expect("let the fake gh run");
    std::fs::create_dir_all(root.join("remotes")).expect("make remotes folder");
    let socket = root.join("d.sock");
    let child = Command::new(binary("HUMATAD2"))
        .arg("serve")
        .arg("--admin")
        .arg("--channels")
        .arg(root.join("ch"))
        .arg("--remote-template")
        .arg(format!("file://{}/remotes/{{owner}}/{{name}}.git", root.display()))
        .arg("--socket")
        .arg(&socket)
        .arg("--state")
        .arg(root.join("state"))
        .arg("--root")
        .arg(&root)
        .arg("--holder")
        .arg(binary("HUMATA2_HOLDER"))
        .arg("--shell")
        .arg("/bin/sh")
        .env("HUMATA2_GH", &gh)
        .env_remove("HUMATA_SESSION_TOKEN")
        .env_remove("HUMATA_SOCKET")
        .spawn()
        .expect("start humatad2");
    // From here a failure stops the daemon.
    let served = Served { dir, child };
    let until = Instant::now() + DEADLINE;
    while tokio::net::UnixStream::connect(&socket).await.is_err() {
        assert!(Instant::now() < until, "humatad2 never started listening");
        tokio::time::sleep(POLL).await;
    }
    let live = within(
        "the connection to humatad2",
        LiveConnection::connect(LiveOptions::new(socket).with_agent(Agent::Claude, "claude")),
    )
    .await
    .expect("connect to humatad2");
    (served, live)
}

/// The pull request the fake gh answers for `acme/api#482`; the fake client is
/// seeded with the same one.
fn api_482() -> PullRequestInfo {
    PullRequestInfo {
        pr: PullRequestRef::new("acme/api", 482),
        title: "Round refunds half a cent".to_owned(),
        author: "mira".to_owned(),
        base_branch: "main".to_owned(),
        head_branch: "fix/refund-rounding".to_owned(),
        head_sha: "b2".to_owned(),
        files: 3,
        added: 42,
        removed: 7,
        state: PullRequestState::Open,
        checks: "3 passed, 1 failing".to_owned(),
    }
}

/// The same answer from either client.
async fn a_pasted_pull_request_reads_the_same(client: &dyn PullRequestsClient) {
    let info = within("resolve", client.resolve(&PullRequestRef::new("acme/api", 482)))
        .await
        .expect("the pull request resolves");
    assert_eq!(info, api_482());
}

/// The same refusal from either client.
async fn a_pull_request_github_cannot_find_is_not_found(client: &dyn PullRequestsClient) {
    let missing = PullRequestRef::new("acme/api", 999);
    let error = within("resolve", client.resolve(&missing))
        .await
        .expect_err("there is no such pull request");
    assert_eq!(error, PullRequestError::NotFound { pr: missing });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pasted_pull_request_reads_the_same_on_the_fake() {
    let fake = FakePullRequests::new();
    fake.add_pull_request(api_482());
    a_pasted_pull_request_reads_the_same(&fake).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pasted_pull_request_reads_the_same_on_the_live_daemon() {
    let (_served, live) = start().await;
    a_pasted_pull_request_reads_the_same(&LivePullRequests::new(&live)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_pull_request_is_not_found_on_the_fake() {
    a_pull_request_github_cannot_find_is_not_found(&FakePullRequests::new()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_pull_request_is_not_found_on_the_live_daemon() {
    let (_served, live) = start().await;
    a_pull_request_github_cannot_find_is_not_found(&LivePullRequests::new(&live)).await;
}

// The gh failures below only the real gh can give (its stderr), so they run on
// the live daemon alone.

#[tokio::test(flavor = "multi_thread")]
async fn a_repository_the_account_cannot_see_says_so() {
    let (_served, live) = start().await;
    let client = LivePullRequests::new(&live);
    let error = within("resolve", client.resolve(&PullRequestRef::new("acme/private", 1)))
        .await
        .expect_err("the account can't see the repository");
    assert_eq!(
        error,
        PullRequestError::NotVisible {
            repo: "acme/private".to_owned()
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_out_gh_says_to_sign_in_on_this_machine() {
    let (_served, live) = start().await;
    let client = LivePullRequests::new(&live);
    let error = within("resolve", client.resolve(&PullRequestRef::new("acme/api", 401)))
        .await
        .expect_err("gh is signed out");
    assert_eq!(
        error,
        PullRequestError::SignedOut {
            machine: live.facts().machine.clone()
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rate_limit_keeps_how_long_to_wait() {
    let (_served, live) = start().await;
    let client = LivePullRequests::new(&live);
    let error = within("resolve", client.resolve(&PullRequestRef::new("acme/api", 429)))
        .await
        .expect_err("GitHub is limiting requests");
    let PullRequestError::RateLimited { retry_after, .. } = error else {
        panic!("a rate limit maps to RateLimited, got {error:?}");
    };
    let wait = retry_after.expect("GitHub said when the limit resets");
    assert!(
        wait > Duration::from_secs(500) && wait <= Duration::from_secs(600),
        "about ten minutes: {wait:?}"
    );
}
