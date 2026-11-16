//! The session's changes through `LiveDaemon` against a real `humatad2`
//! (admin build) and a real git clone: what the agent changed in its clone
//! is in what `Daemon::diffs` reads, line by line, as the Diff panel and
//! the document tab draw it (RV-1, flows 26.3, 26.14 and 26.20).
//! Every case has its own daemon, folder and socket.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use humata2_clients::model::{Agent, DiffBase, DiffLine, LineKind, RepoDiff, SessionId, SessionOptions};
use humata2_clients::Daemon;
use humata2_clients_live::{LiveDaemon, LiveOptions};
use tempfile::TempDir;

const AGENT_NAME: &str = "fake";

/// The folder and branch of the session every case starts, titled "Fix the
/// ledger".
const SESSION_FOLDER: &str = "fix-the-ledger";

/// How long anything may take before the case fails.
const DEADLINE: Duration = Duration::from_secs(60);

/// How often the socket is tried while waiting for it.
const POLL: Duration = Duration::from_millis(20);

fn binary(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable} to the binary's path"));
    std::fs::canonicalize(path).expect("the binary exists")
}

fn git(args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Stands in for an agent: starts an empty conversation and reads its
/// terminal.
fn agent_script(conv: &Path) -> String {
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) : > "$CONV_DIR/$id.jsonl" ;;
esac
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; done
"#,
        conv.display()
    )
}

async fn within<T>(what: &str, work: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(DEADLINE, work).await {
        Ok(value) => value,
        Err(_) => panic!("timed out waiting for {what}"),
    }
}

/// A real `humatad2` in a folder of its own, which can be killed and
/// started again on the same state. Dropping it stops the daemon and
/// everything the case started under it.
struct Host {
    dir: TempDir,
    child: Option<Child>,
}

impl Host {
    async fn start() -> Self {
        use std::os::unix::fs::PermissionsExt;
        // Short, so the socket's path fits.
        let dir = tempfile::Builder::new()
            .prefix("h2d")
            .tempdir_in("/tmp")
            .expect("make a temp folder");
        // The daemon refuses a socket folder other users can open.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("restrict the temp folder");
        let root = dir.path();
        let conv = root.join("conv");
        std::fs::create_dir_all(&conv).expect("make conversations folder");
        std::fs::create_dir_all(root.join("remotes")).expect("make remotes folder");
        let script = root.join("agent.sh");
        std::fs::write(&script, agent_script(&conv)).expect("write the agent");
        let spec = serde_json::json!([{
            "name": AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", conv.display()),
        }]);
        std::fs::write(root.join("agents.json"), spec.to_string()).expect("write agents.json");
        let mut host = Self { dir, child: None };
        host.launch().await;
        host
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn socket(&self) -> PathBuf {
        self.root().join("d.sock")
    }

    async fn launch(&mut self) {
        let root = self.root().to_owned();
        let child = Command::new(binary("HUMATAD2"))
            .arg("serve")
            .arg("--admin")
            .arg("--channels")
            .arg(root.join("ch"))
            .arg("--agents")
            .arg(root.join("agents.json"))
            .arg("--default-agent")
            .arg(AGENT_NAME)
            .arg("--remote-template")
            .arg(format!(
                "file://{}/remotes/{{owner}}/{{name}}.git",
                root.display()
            ))
            .arg("--socket")
            .arg(self.socket())
            .arg("--state")
            .arg(root.join("state"))
            .arg("--root")
            .arg(&root)
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER"))
            .arg("--shell")
            .arg("/bin/sh")
            .arg("--shell-arg=-i")
            .env("PS1", "$ ")
            .env("ENV", "/dev/null")
            .spawn()
            .expect("start humatad2");
        self.child = Some(child);
        let socket = self.socket();
        within("humatad2 to listen", async {
            while tokio::net::UnixStream::connect(&socket).await.is_err() {
                tokio::time::sleep(POLL).await;
            }
        })
        .await;
    }

    /// A connection of its own: another client of the same daemon.
    async fn connect(&self) -> LiveDaemon {
        within(
            "the connection to humatad2",
            LiveDaemon::connect(
                LiveOptions::new(self.socket()).with_agent(Agent::Claude, AGENT_NAME),
            ),
        )
        .await
        .expect("connect to humatad2")
    }

    /// A bare remote `owner/<name>` on `main` with one commit.
    fn repo(&self, name: &str) -> String {
        let root = self.root();
        let bare = root.join("remotes").join("owner").join(format!("{name}.git"));
        std::fs::create_dir_all(&bare).expect("make the remote folder");
        let bare_text = bare.display().to_string();
        git(&["init", "--bare", "--initial-branch=main", &bare_text]);
        let work = root.join(format!("scratch-{name}"));
        std::fs::create_dir_all(&work).expect("make the scratch folder");
        let work_text = work.display().to_string();
        git(&["init", "--initial-branch=main", &work_text]);
        std::fs::write(work.join("README.md"), format!("owner/{name}\n")).expect("write");
        git(&["-C", &work_text, "add", "."]);
        git(&["-C", &work_text, "commit", "-m", "first"]);
        git(&["-C", &work_text, "push", &bare_text, "main"]);
        format!("owner/{name}")
    }

    /// A session titled `title`, in a channel of its own, that the host's
    /// store knows.
    async fn session(&self, daemon: &LiveDaemon, title: &str) -> SessionId {
        let workspace = within("load", daemon.load()).await.expect("load the workspace");
        let mac = workspace
            .machines
            .first()
            .expect("the workspace has a machine")
            .id
            .clone();
        let repo = self.repo("ledger");
        let channel = within(
            "create_channel",
            daemon.create_channel("payments", vec![repo.clone()], vec![mac.clone()]),
        )
        .await
        .expect("create the channel");
        within(
            "create_session",
            daemon.create_session(
                &channel,
                Agent::Claude,
                &mac,
                title,
                SessionOptions {
                    repos: vec![repo],
                    ..SessionOptions::default()
                },
            ),
        )
        .await
        .expect("create the session")
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // Holders outlive the daemon on purpose, and so do the agents they
        // run. Everything this case started has its folder in its command
        // line, so stop those by it.
        let _ = Command::new("pkill")
            .arg("-f")
            .arg(self.dir.path())
            .status();
    }
}

/// The clone of `name` in the session's own folder
/// (`ch/<channel>/sessions/fix-the-ledger/<name>`), once the daemon has
/// made it and checked out the session's branch. The channel's `main`
/// clone of the same repository is never it.
async fn clone_of(host: &Host, name: &str) -> PathBuf {
    fn find(channels: &Path, name: &str) -> Option<PathBuf> {
        for channel in std::fs::read_dir(channels).ok()?.flatten() {
            let clone = channel.path().join("sessions").join(SESSION_FOLDER).join(name);
            if clone.join("README.md").exists() && on_session_branch(&clone) {
                return Some(clone);
            }
        }
        None
    }
    fn on_session_branch(clone: &Path) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(clone)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .is_ok_and(|out| out.status.success() && out.stdout.trim_ascii() == SESSION_FOLDER.as_bytes())
    }
    let channels = host.root().join("ch");
    within("the session's clone", async {
        loop {
            if let Some(found) = find(&channels, name) {
                return found;
            }
            tokio::time::sleep(POLL).await;
        }
    })
    .await
}

/// The session's diff of `repo` against `base`, read now, if there is one.
async fn diff_against(
    daemon: &LiveDaemon,
    session: &SessionId,
    repo: &str,
    base: &DiffBase,
) -> Option<RepoDiff> {
    let diffs = within("read the diffs", daemon.diffs(session, base))
        .await
        .expect("read the session's diffs");
    let summary: Vec<(String, String, Vec<String>)> = diffs
        .iter()
        .map(|diff| {
            let files = diff.files.iter().map(|file| file.path.clone()).collect();
            (diff.session.to_string(), diff.repo.clone(), files)
        })
        .collect();
    eprintln!("diffs of {session} against {base:?}: {summary:?}");
    diffs
        .into_iter()
        .find(|diff| &diff.session == session && diff.repo == repo)
}

/// The session's diff of `repo` against origin/main, read now.
async fn diff_of(daemon: &LiveDaemon, session: &SessionId, repo: &str) -> Option<RepoDiff> {
    diff_against(daemon, session, repo, &DiffBase::OriginMain).await
}

/// The paths changed in the session's diff of `repo` against `base`.
async fn paths_against(
    daemon: &LiveDaemon,
    session: &SessionId,
    repo: &str,
    base: &DiffBase,
) -> Vec<String> {
    let mut paths: Vec<String> = diff_against(daemon, session, repo, base)
        .await
        .map(|diff| diff.files.into_iter().map(|file| file.path).collect())
        .unwrap_or_default();
    paths.sort_unstable();
    paths
}

fn line(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str) -> DiffLine {
    DiffLine {
        kind,
        old,
        new,
        text: text.to_owned(),
    }
}

/// The lines of a file's diff, without its hunk headers.
fn body(lines: &[DiffLine]) -> Vec<DiffLine> {
    lines
        .iter()
        .filter(|line| line.kind != LineKind::Hunk)
        .cloned()
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn what_the_agent_changed_in_its_clone_is_in_its_diff() {
    let host = Host::start().await;
    let daemon = host.connect().await;
    let session = host.session(&daemon, "Fix the ledger").await;
    let clone = clone_of(&host, "ledger").await;
    std::fs::write(clone.join("README.md"), "owner/ledger, renamed\nsecond line\n").expect("edit");
    std::fs::create_dir_all(clone.join("src")).expect("make src");
    std::fs::write(clone.join("src/new.rs"), "fn new() {}\n").expect("add a file");

    let diff = diff_of(&daemon, &session, "owner/ledger")
        .await
        .expect("the session's diff of owner/ledger is loaded");
    let mut paths: Vec<&str> = diff.files.iter().map(|file| file.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["README.md", "src/new.rs"]);

    let readme = diff
        .files
        .iter()
        .find(|file| file.path == "README.md")
        .expect("README.md changed");
    assert_eq!((readme.added, readme.removed), (2, 1));
    assert_eq!(readme.lines.first().map(|line| line.kind), Some(LineKind::Hunk));
    assert_eq!(
        body(&readme.lines),
        [
            line(LineKind::Removed, Some(1), None, "owner/ledger"),
            line(LineKind::Added, None, Some(1), "owner/ledger, renamed"),
            line(LineKind::Added, None, Some(2), "second line"),
        ]
    );

    let new = diff
        .files
        .iter()
        .find(|file| file.path == "src/new.rs")
        .expect("src/new.rs was added");
    assert_eq!((new.added, new.removed), (1, 0));
    assert_eq!(body(&new.lines), [line(LineKind::Added, None, Some(1), "fn new() {}")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_commit_the_agent_made_on_its_branch_stays_in_the_diff() {
    let host = Host::start().await;
    let daemon = host.connect().await;
    let session = host.session(&daemon, "Fix the ledger").await;
    let clone = clone_of(&host, "ledger").await;
    std::fs::write(clone.join("notes.md"), "kept\n").expect("add a file");
    let clone_text = clone.display().to_string();
    git(&["-C", &clone_text, "add", "."]);
    git(&["-C", &clone_text, "commit", "-m", "notes"]);

    let diff = diff_of(&daemon, &session, "owner/ledger")
        .await
        .expect("the session's diff of owner/ledger is loaded");
    let notes = diff
        .files
        .iter()
        .find(|file| file.path == "notes.md")
        .expect("the committed file is in the diff");
    assert_eq!(body(&notes.lines), [line(LineKind::Added, None, Some(1), "kept")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clone_with_no_changes_has_no_changed_files() {
    let host = Host::start().await;
    let daemon = host.connect().await;
    let session = host.session(&daemon, "Fix the ledger").await;
    clone_of(&host, "ledger").await;

    let files = diff_of(&daemon, &session, "owner/ledger")
        .await
        .map(|diff| diff.files)
        .unwrap_or_default();
    assert!(files.is_empty(), "a clean clone shows no changes: {files:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_file_comes_whole_across_pages() {
    let host = Host::start().await;
    let daemon = host.connect().await;
    let session = host.session(&daemon, "Fix the ledger").await;
    let clone = clone_of(&host, "ledger").await;
    let text: String = (1..=5000).map(|n| format!("line {n}\n")).collect();
    std::fs::write(clone.join("long.txt"), text).expect("add a long file");

    let diff = diff_of(&daemon, &session, "owner/ledger")
        .await
        .expect("the session's diff of owner/ledger is loaded");
    let long = diff
        .files
        .iter()
        .find(|file| file.path == "long.txt")
        .expect("long.txt was added");
    let added: Vec<u32> = body(&long.lines).iter().filter_map(|line| line.new).collect();
    assert_eq!(added, (1..=5000).collect::<Vec<u32>>());
}

#[tokio::test(flavor = "multi_thread")]
async fn since_i_last_reviewed_holds_only_what_changed_after_mark_reviewed() {
    let host = Host::start().await;
    let daemon = host.connect().await;
    let session = host.session(&daemon, "Fix the ledger").await;
    let clone = clone_of(&host, "ledger").await;
    std::fs::write(clone.join("first.md"), "seen\n").expect("add a file");
    within("mark reviewed", daemon.mark_reviewed(&session))
        .await
        .expect("mark the session reviewed");
    assert_eq!(
        paths_against(&daemon, &session, "owner/ledger", &DiffBase::SinceLastReviewed).await,
        Vec::<String>::new()
    );

    std::fs::write(clone.join("second.md"), "new\n").expect("add another file");
    assert_eq!(
        paths_against(&daemon, &session, "owner/ledger", &DiffBase::SinceLastReviewed).await,
        ["second.md"]
    );
    assert_eq!(
        paths_against(&daemon, &session, "owner/ledger", &DiffBase::OriginMain).await,
        ["first.md", "second.md"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn uncommitted_only_leaves_out_what_the_agent_committed() {
    let host = Host::start().await;
    let daemon = host.connect().await;
    let session = host.session(&daemon, "Fix the ledger").await;
    let clone = clone_of(&host, "ledger").await;
    let clone_text = clone.display().to_string();
    std::fs::write(clone.join("committed.md"), "kept\n").expect("add a file");
    git(&["-C", &clone_text, "add", "."]);
    git(&["-C", &clone_text, "commit", "-m", "kept"]);
    std::fs::write(clone.join("loose.md"), "loose\n").expect("add another file");
    assert_eq!(
        paths_against(&daemon, &session, "owner/ledger", &DiffBase::UncommittedOnly).await,
        ["loose.md"]
    );
}
