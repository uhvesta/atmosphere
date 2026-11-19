//! The repos `gh` sees, through `LiveDaemon` against a real `humatad2`
//! (admin build, which runs the `gh` named by `HUMATA2_GH`): every repo
//! comes back as `owner/name` in gh's order, across more than one page, and
//! a query narrows them (CH-3, D272).

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use humata2_clients::Daemon;
use humata2_clients::model::Agent;
use humata2_clients_live_daemon::{LiveDaemon, LiveOptions};
use tempfile::TempDir;

/// How many repos the fake `gh` sees: more than two of the client's pages.
const REPOS: usize = 250;

/// How long anything may take before the case fails.
const DEADLINE: Duration = Duration::from_secs(60);

/// How often the socket is tried while the daemon starts.
const POLL: Duration = Duration::from_millis(50);

/// A signed-in `gh` that sees `acme/repo-001` to `acme/repo-250`, most
/// recently pushed first, then `octo/notes` (the loop's 250 is [`REPOS`]);
/// it prints what `gh api --paginate user/repos --jq` would, and signs in
/// as `octo` for the login check.
const FAKE_GH: &str = r#"#!/bin/sh
case "$*" in
  *user/repos*)
    i=1
    while [ "$i" -le 250 ]; do
      printf '{"full_name":"acme/repo-%03d","description":null,"private":false,"default_branch":"main","pushed_at":"2026-11-15T10:00:00Z"}\n' "$i"
      i=$((i + 1))
    done
    echo '{"full_name":"octo/notes","description":"Notes","private":true,"default_branch":"trunk","pushed_at":"2026-11-01T10:00:00Z"}'
    ;;
  *"api user"*) echo octo ;;
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
        let _ = Command::new("pkill")
            .arg("-f")
            .arg(self.dir.path())
            .status();
    }
}

async fn start() -> (Served, LiveDaemon) {
    start_with(FAKE_GH).await
}

/// [`start`], with `gh_script` as the fake `gh`.
async fn start_with(gh_script: &str) -> (Served, LiveDaemon) {
    use std::os::unix::fs::PermissionsExt;
    // Short, so the socket's path fits.
    let dir = tempfile::Builder::new()
        .prefix("h2g")
        .tempdir_in("/tmp")
        .expect("make a temp folder");
    // The daemon refuses a socket folder other users can open.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
        .expect("restrict the temp folder");
    let root = dir.path().to_path_buf();
    let gh = root.join("gh");
    std::fs::write(&gh, gh_script).expect("write the fake gh");
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755))
        .expect("let the fake gh run");
    std::fs::create_dir_all(root.join("remotes")).expect("make remotes folder");
    let socket = root.join("d.sock");
    let child = Command::new(binary("HUMATAD2"))
        .arg("serve")
        .arg("--admin")
        .arg("--channels")
        .arg(root.join("ch"))
        .arg("--remote-template")
        .arg(format!(
            "file://{}/remotes/{{owner}}/{{name}}.git",
            root.display()
        ))
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
        LiveDaemon::connect(LiveOptions::new(socket).with_agent(Agent::Claude, "claude")),
    )
    .await
    .expect("connect to humatad2");
    (served, live)
}

#[tokio::test(flavor = "multi_thread")]
async fn every_repo_gh_sees_comes_back_in_order_across_pages() {
    let (_served, live) = start().await;
    let listed = within("the repos", live.github_repos(""))
        .await
        .expect("the repos are listed");
    let mut expected: Vec<String> = (1..=REPOS).map(|i| format!("acme/repo-{i:03}")).collect();
    expected.push("octo/notes".to_owned());
    assert_eq!(listed, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_narrows_the_repos() {
    let (_served, live) = start().await;
    let listed = within("the repos", live.github_repos("NOTES"))
        .await
        .expect("the repos are listed");
    assert_eq!(listed, vec!["octo/notes".to_owned()]);
}

/// A signed-in `gh` that GitHub has rate-limited: listing repos fails as
/// gh prints it.
const RATE_LIMITED_GH: &str = r#"#!/bin/sh
case "$*" in
  *user/repos*) echo "gh: API rate limit exceeded for user ID 42. (HTTP 403)" >&2; exit 1 ;;
  *"api user"*) echo octo ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

/// Macro-02: a rate limit is told as a wait. Signing in again would not fix it.
#[tokio::test(flavor = "multi_thread")]
async fn a_rate_limited_gh_is_refused_as_a_wait_and_not_a_sign_in() {
    let (_served, live) = start_with(RATE_LIMITED_GH).await;
    let error = within("the refusal", live.github_repos(""))
        .await
        .expect_err("a rate-limited gh lists no repos");
    let told = error.to_string();
    assert!(!told.contains("auth login"), "a rate limit is not fixed by signing in: {told}");
}
