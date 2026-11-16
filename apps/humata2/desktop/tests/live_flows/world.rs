//! A real `humatad2` in a temp folder, a test agent that answers every line
//! with `got: <line>`, and the means to stop everything as a reboot would
//! and start the daemon again on the same folder.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use humata2_clients::Clients;
use humata2_clients::model::{Agent, MachineId};
use humata2_clients_live::{LiveComments, LiveDaemon, LiveOptions};
use tca::{MemoryStorage, SystemClock};
use std::sync::Arc;

const AGENT_NAME: &str = "fake";

/// Why the world couldn't be set up. Tests stop on any of these.
#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    /// A needed file or folder couldn't be made or read.
    #[error("{what}: {source}")]
    Io {
        what: String,
        #[source]
        source: std::io::Error,
    },
    /// A tool path from the test's environment is missing.
    #[error("set {variable} to the binary's path (the bazel target passes it)")]
    NoBinary { variable: &'static str },
    /// git failed while making a remote.
    #[error("git {args} failed: {stderr}")]
    Git { args: String, stderr: String },
    /// The daemon never listened.
    #[error("humatad2 never started listening on {}", socket.display())]
    NeverListened { socket: PathBuf },
    /// Connecting to the daemon failed.
    #[error("couldn't connect to humatad2: {source}")]
    Connect {
        #[source]
        source: humata2_clients_live::ConnectError,
    },
    /// Processes started in the folder are still alive.
    #[error("processes started in {} did not stop", folder.display())]
    WontDie { folder: PathBuf },
}

fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> WorldError {
    let what = what.into();
    move |source| WorldError::Io { what, source }
}

fn binary(variable: &'static str) -> Result<PathBuf, WorldError> {
    let path = std::env::var_os(variable).ok_or(WorldError::NoBinary { variable })?;
    std::fs::canonicalize(&path).map_err(io(format!("find {}", Path::new(&path).display())))
}

fn git(args: &[&str]) -> Result<(), WorldError> {
    let output = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .map_err(io("run git"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(WorldError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Stands in for an agent: answers every line typed with `got: <line>`, and
/// keeps each line in its conversation file.
fn agent_script(conv: &Path) -> String {
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
  --fresh) f="$CONV_DIR/other.jsonl"; : > "$f" ;;
esac
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#,
        conv.display()
    )
}

/// GitHub as the tests see it: logs each call beside itself, answers no
/// open pull requests, and opens PR 7.
const FAKE_GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
echo "$*" >> "$here/gh.log"
case "$1 $2" in
  "pr list") echo '[]' ;;
  "pr create") echo "https://github.com/owner/api/pull/7" ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

pub struct World {
    dir: tempfile::TempDir,
    daemon: Option<Child>,
}

impl World {
    /// Makes the folder and starts the daemon in it.
    pub async fn start() -> Result<Self, WorldError> {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new()
            .prefix("h2f")
            .tempdir_in("/tmp")
            .map_err(io("make a temp folder"))?;
        // The daemon refuses a socket folder other users can open.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(io("restrict the temp folder"))?;
        let root = dir.path();
        let conv = root.join("conv");
        std::fs::create_dir_all(&conv).map_err(io("make conversations folder"))?;
        std::fs::create_dir_all(root.join("remotes")).map_err(io("make remotes folder"))?;
        let script = root.join("agent.sh");
        std::fs::write(&script, agent_script(&conv)).map_err(io("write the agent"))?;
        let spec = serde_json::json!([{
            "name": AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", conv.display()),
        }]);
        std::fs::write(root.join("agents.json"), spec.to_string())
            .map_err(io("write agents.json"))?;
        // GitHub is a fake `gh` (the admin daemon reads HUMATA2_GH): it logs
        // each call to gh.log, finds no open pull request and opens PR 7.
        let gh = root.join("gh");
        std::fs::write(&gh, FAKE_GH).map_err(io("write the fake gh"))?;
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755))
            .map_err(io("make the fake gh runnable"))?;
        let mut world = Self { dir, daemon: None };
        world.spawn_daemon().await?;
        Ok(world)
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("d.sock")
    }

    /// The folder the agent keeps its conversations in.
    pub fn conversations(&self) -> PathBuf {
        self.dir.path().join("conv")
    }

    /// Every conversation's text, joined.
    pub fn conversation_text(&self) -> String {
        let Ok(entries) = std::fs::read_dir(self.conversations()) else {
            return String::new();
        };
        entries
            .flatten()
            .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
            .collect()
    }

    /// Deletes every conversation, as if they had been lost.
    pub fn lose_conversations(&self) -> Result<(), WorldError> {
        for entry in std::fs::read_dir(self.conversations())
            .map_err(io("read conversations"))?
            .flatten()
        {
            std::fs::remove_file(entry.path()).map_err(io("delete a conversation"))?;
        }
        Ok(())
    }

    async fn spawn_daemon(&mut self) -> Result<(), WorldError> {
        let root = self.dir.path().to_path_buf();
        let socket = self.socket();
        let _ = std::fs::remove_file(&socket);
        let child = Command::new(binary("HUMATAD2")?)
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
            .arg(&socket)
            .arg("--state")
            .arg(root.join("state"))
            .arg("--root")
            .arg(&root)
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER")?)
            .arg("--shell")
            .arg("/bin/sh")
            .arg("--shell-arg=-i")
            .env("HUMATA2_GH", root.join("gh"))
            .env("PS1", "$ ")
            .env("ENV", "/dev/null")
            .spawn()
            .map_err(io("start humatad2"))?;
        self.daemon = Some(child);
        let deadline = Instant::now() + Duration::from_secs(30);
        while tokio::net::UnixStream::connect(&socket).await.is_err() {
            if Instant::now() >= deadline {
                return Err(WorldError::NeverListened { socket });
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }

    /// Clients on the live daemon, with fakes for what has no real client
    /// yet, as the app starts them.
    pub async fn clients(&self) -> Result<Clients, WorldError> {
        let daemon = LiveDaemon::connect(
            LiveOptions::new(self.socket()).with_agent(Agent::Claude, AGENT_NAME),
        )
        .await
        .map_err(|source| WorldError::Connect { source })?;
        // Built before `daemon` moves into the terminals client, as the app does.
        let file_manager = humata2_clients_live::LiveFileManager::new(&daemon);
        Ok(Clients {
            daemon: Arc::new(daemon.clone()),
            desk: Arc::new(daemon.clone()),
            comments: Arc::new(LiveComments::new(&daemon)),
            history: Arc::new(humata2_clients_live::history::LiveHistory::new(&daemon)),
            marks: Arc::new(humata2_clients_live::marks::LiveMarks::new(&daemon)),
            terminals: Arc::new(daemon),
            clock: Arc::new(SystemClock::new()),
            clipboard: Arc::new(humata2_clients::FakeClipboard::default()),
            files: Arc::new(humata2_clients::FakeFileSystem::default()),
            file_manager: Arc::new(file_manager),
            keychain: Arc::new(humata2_clients::FakeKeychain::default()),
            // The live machines client replaces this fake when it is wired in.
            machines: Arc::new(humata2_clients::machines::FakeMachines::new()),
            storage: Arc::new(MemoryStorage::new()),
        })
    }

    /// A bare remote `owner/name` on `main` with one commit; returns the
    /// name the daemon clones it by.
    pub fn repo(&self, owner: &str, name: &str) -> Result<String, WorldError> {
        let root = self.dir.path();
        let bare = root.join("remotes").join(owner).join(format!("{name}.git"));
        std::fs::create_dir_all(&bare).map_err(io("make the remote folder"))?;
        let bare_text = bare.display().to_string();
        git(&["init", "--bare", "--initial-branch=main", &bare_text])?;
        let work = root.join(format!("scratch-{owner}-{name}"));
        std::fs::create_dir_all(&work).map_err(io("make the scratch folder"))?;
        let work_text = work.display().to_string();
        git(&["init", "--initial-branch=main", &work_text])?;
        std::fs::write(work.join("README.md"), format!("{owner}/{name}\n"))
            .map_err(io("write the readme"))?;
        git(&["-C", &work_text, "add", "."])?;
        git(&["-C", &work_text, "commit", "-m", "first"])?;
        git(&["-C", &work_text, "push", &bare_text, "main"])?;
        Ok(format!("{owner}/{name}"))
    }

    /// The branches the bare remote `owner/name` has.
    pub fn remote_branches(&self, owner: &str, name: &str) -> Vec<String> {
        let bare = self
            .dir
            .path()
            .join("remotes")
            .join(owner)
            .join(format!("{name}.git"));
        std::process::Command::new("git")
            .arg("-C")
            .arg(&bare)
            .args(["for-each-ref", "--format=%(refname:short)", "refs/heads"])
            .output()
            .map(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Each call the app's daemon made to `gh`, one line each.
    pub fn gh_calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("gh.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// What a reboot does: the daemon and every holder and agent started in
    /// this folder die at once. Nothing is cleaned up first.
    pub async fn reboot(&mut self) -> Result<(), WorldError> {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let folder = self.dir.path().to_path_buf();
        // Holders keep running when the daemon dies, and each has the
        // folder in its command line.
        let _ = Command::new("pkill").args(["-9", "-f"]).arg(&folder).status();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let alive = Command::new("pgrep")
                .arg("-f")
                .arg(&folder)
                .output()
                .map(|out| out.status.success())
                .unwrap_or(false);
            if !alive {
                break;
            }
            if Instant::now() >= deadline {
                return Err(WorldError::WontDie { folder });
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.spawn_daemon().await
    }

    /// The first machine of the workspace: This Mac.
    pub async fn this_mac(clients: &Clients) -> Option<MachineId> {
        let workspace = clients.daemon.load().await.ok()?;
        workspace.machines.first().map(|machine| machine.id.clone())
    }
}

impl Drop for World {
    fn drop(&mut self) {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = Command::new("pkill").arg("-f").arg(self.dir.path()).status();
    }
}
