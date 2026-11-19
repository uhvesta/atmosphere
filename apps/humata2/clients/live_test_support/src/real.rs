//! A real `humatad2` in a temp folder, reached through `LiveDaemon`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use humata2_clients::model::Agent;
use humata2_clients::{Daemon, Terminals};
use humata2_clients_live_core::LiveOptions;
use humata2_clients_live_daemon::LiveDaemon;
use tempfile::TempDir;

use super::cases::World;

const AGENT_NAME: &str = "fake";

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

/// Stands in for an agent: answers every line typed with `got: <line>`.
fn agent_script(conv: &Path) -> String {
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
  --fresh) f="$CONV_DIR/other.jsonl"; : > "$f" ;;
esac
# Reads keys itself, as an agent's CLI does: a holder types into a program
# still editing lines only after waiting out its keys limit (5 s).
stty -icanon 2>/dev/null
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#,
        conv.display()
    )
}

pub struct RealWorld {
    dir: TempDir,
    child: Child,
}

/// A cached bare git repository on `main` with one initial commit, copied into
/// each test's `remotes/<owner>/<name>.git` instead of spawning 5 `git`
/// subprocesses per repo.
static TEMPLATE_BARE: OnceLock<TempDir> = OnceLock::new();

fn template_bare() -> &'static Path {
    TEMPLATE_BARE
        .get_or_init(|| {
            let dir = tempfile::Builder::new()
                .prefix("h2t")
                .tempdir_in("/tmp")
                .expect("make template repo folder");
            let bare = dir.path().join("template.git");
            std::fs::create_dir_all(&bare).expect("make template bare folder");
            let bare_text = bare.display().to_string();
            git(&["init", "--bare", "--initial-branch=main", &bare_text]);
            let work = dir.path().join("scratch");
            std::fs::create_dir_all(&work).expect("make template scratch folder");
            let work_text = work.display().to_string();
            git(&["init", "--initial-branch=main", &work_text]);
            std::fs::write(work.join("README.md"), "owner/ledger\n").expect("write");
            git(&["-C", &work_text, "add", "."]);
            git(&["-C", &work_text, "commit", "-m", "first"]);
            git(&["-C", &work_text, "push", &bare_text, "main"]);
            let _ = std::fs::remove_dir_all(&work);
            dir
        })
        .path()
}

fn copy_dir_all(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create destination folder");
    for entry in std::fs::read_dir(src).expect("read source folder") {
        let entry = entry.expect("read dir entry");
        let ty = entry.file_type().expect("read file type");
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), to).expect("copy template file");
        }
    }
}

impl World for RealWorld {
    /// A bare remote `owner/name` on `main` with one commit.
    fn repo(&self, owner: &str, name: &str) -> String {
        let root = self.dir.path();
        let bare = root.join("remotes").join(owner).join(format!("{name}.git"));
        copy_dir_all(&template_bare().join("template.git"), &bare);
        format!("{owner}/{name}")
    }

    fn agent(&self) -> Agent {
        Agent::Claude
    }

    fn answer_to_ping(&self) -> &'static str {
        "got: ping"
    }

    fn paths_exist(&self) -> bool {
        true
    }
}

impl Drop for RealWorld {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Holders outlive the daemon on purpose, and so do the agents they
        // run. Everything this test started has its folder in its command
        // line, so stop those by it — skip the expensive process-table scan
        // when no holder was ever spawned.
        let holders = self.dir.path().join("state").join("holders");
        if std::fs::read_dir(&holders).is_ok_and(|mut entries| entries.next().is_some()) {
            let _ = Command::new("pkill")
                .arg("-f")
                .arg(self.dir.path())
                .status();
        }
    }
}

async fn start_live() -> (RealWorld, LiveDaemon) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new()
        .prefix("h2p")
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
    let agents = root.join("agents.json");
    let spec = serde_json::json!([{
        "name": AGENT_NAME,
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}"],
        "resume_args": [script, "--resume", "{conversation}"],
        "conversation_file": format!("{}/{{conversation}}.jsonl", conv.display()),
    }]);
    std::fs::write(&agents, spec.to_string()).expect("write agents.json");
    let socket = root.join("d.sock");
    let child = Command::new(binary("HUMATAD2"))
        .arg("serve")
        .arg("--admin")
        .arg("--channels")
        .arg(root.join("ch"))
        .arg("--agents")
        .arg(&agents)
        .arg("--default-agent")
        .arg(AGENT_NAME)
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
        .arg(root)
        .arg("--holder")
        .arg(binary("HUMATA2_HOLDER"))
        .arg("--shell")
        .arg("/bin/sh")
        .arg("--shell-arg=-i")
        .env("PS1", "$ ")
        .env("ENV", "/dev/null")
        .spawn()
        .expect("start humatad2");
    let world = RealWorld { dir, child };
    let deadline = Instant::now() + Duration::from_secs(15);
    while tokio::net::UnixStream::connect(&socket).await.is_err() {
        assert!(Instant::now() < deadline, "humatad2 never started listening");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let live = LiveDaemon::connect(LiveOptions::new(socket).with_agent(Agent::Claude, AGENT_NAME))
        .await
        .expect("connect to humatad2");
    (world, live)
}

pub async fn start() -> (RealWorld, Arc<dyn Daemon>, Arc<dyn Terminals>) {
    let (world, live) = start_live().await;
    let live = Arc::new(live);
    let daemon: Arc<dyn Daemon> = live.clone();
    let terminals: Arc<dyn Terminals> = live;
    (world, daemon, terminals)
}

/// The daemon client itself, for a case that builds its own client over it
/// (the review chat, pull requests).
#[allow(dead_code)]
pub async fn start_daemon() -> (RealWorld, LiveDaemon) {
    start_live().await
}
