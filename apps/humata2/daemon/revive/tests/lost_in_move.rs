//! A holder lost while it moved onto a new holder build: its move marker is
//! left beside its socket and no holder answers. The daemon's reconcile
//! reports the session as lost in its restart report and removes the
//! marker, and brings the session back as it does any lost holder. A holder
//! that died with no marker is brought back without that report.
//!
//! The agent is a small `sh` script standing in for one, as in the kill
//! matrix: `--resume` writes to its conversation and says it came up.

use std::ffi::OsString;
use std::fs;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use humata2_holder::{Event, holder_pid};
use humata2_revive::{RestartReports, ReviveConfig, Revived, launch_agent, reconcile};
use humata2_store::{AgentLaunch, SessionRecord, Store, TabRecord};
use rustix::process::{Pid, Signal};

const WAIT: Duration = Duration::from_secs(20);

const AGENT: &str = r#"mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
  --fresh) f="$CONV_DIR/other.jsonl"; : > "$f" ;;
  --die) echo "dying"; exit 3 ;;
esac
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#;

fn holder_binary() -> PathBuf {
    let path = std::env::var_os("HUMATA2_HOLDER")
        .map(PathBuf::from)
        .expect("set HUMATA2_HOLDER to the humata2_holder binary");
    fs::canonicalize(path).unwrap()
}

/// One machine's state folder, holders, conversations and work folder,
/// under a short path in /tmp so socket paths fit.
struct World {
    root: PathBuf,
    _dir: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        let dir = humata2_tmp_folder::folder("h2lim");
        let root = dir.path().to_owned();
        for folder in ["state", "conv", "h", "logs", "work"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        fs::write(root.join("agent.sh"), AGENT).unwrap();
        Self { root, _dir: dir }
    }

    fn store(&self) -> Arc<Store> {
        Arc::new(Store::open(self.root.join("state")).unwrap())
    }

    fn config(&self) -> ReviveConfig {
        ReviveConfig {
            contain: None,
            builds: None,
            versions: None,
            gate: None,
            holder: holder_binary(),
            logs: self.root.join("logs"),
            scrollback_bytes: 1 << 20,
            shell: PathBuf::from("/bin/sh"),
            shell_args: Vec::new(),
            home: self.root.clone(),
            resume_timeout: Duration::from_secs(10),
            settle: Duration::from_millis(200),
            reports: RestartReports {
                folder: self.root.join("restarts"),
                version: "0.0.0-test".to_owned(),
                machine: "ma_1".to_owned(),
                os: "test".to_owned(),
                daemon_log: Some(self.root.join("logs/humatad2.previous.log")),
            },
        }
    }

    fn env(&self) -> impl Fn(&TabRecord) -> Vec<(OsString, OsString)> + Sync + 'static {
        let conv = self.conv();
        move |_tab: &TabRecord| vec![("CONV_DIR".into(), conv.clone().into_os_string())]
    }

    fn conv(&self) -> PathBuf {
        self.root.join("conv")
    }

    fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    fn socket(&self) -> PathBuf {
        self.root.join("h/tb_a.sock")
    }

    fn session(&self) -> SessionRecord {
        SessionRecord {
            id: "se_a".to_owned(),
            channel: "ch_1".to_owned(),
            machine: "ma_1".to_owned(),
            title: "Fix the build".to_owned(),
            agent: "fake".to_owned(),
            folder: self.work(),
            created_unix_ms: 1,
        }
    }

    fn agent_args(&self, args: &[&str]) -> Vec<OsString> {
        let mut argv = vec![OsString::from("/bin/sh"), self.root.join("agent.sh").into()];
        argv.extend(args.iter().map(OsString::from));
        argv
    }

    fn launch(&self) -> AgentLaunch {
        AgentLaunch {
            agent: "fake".to_owned(),
            folder: self.work(),
            argv: self.agent_args(&["--session-id", "c1"]),
            resume_argv: self.agent_args(&["--resume", "c1"]),
            variable_names: vec!["CONV_DIR".to_owned()],
            conversation_id: "c1".to_owned(),
            conversation_file: self.conv().join("c1.jsonl"),
            holder_socket: self.socket(),
            cols: 80,
            rows: 24,
            cli_version: None,
        }
    }

    async fn launch_with(&self, store: &Arc<Store>, launch: AgentLaunch) {
        store.put_session(&self.session()).unwrap();
        launch_agent(store, &self.config(), &self.session(), "tb_a", launch, None, &self.env())
            .await
            .unwrap();
    }
}

impl Drop for World {
    /// Stops every holder still answering, however the test ended.
    fn drop(&mut self) {
        let Ok(entries) = fs::read_dir(self.root.join("h")) else {
            return;
        };
        for entry in entries.flatten() {
            let socket = entry.path();
            if socket.extension().is_some_and(|ext| ext == "sock")
                && UnixStream::connect(&socket).is_ok()
                && let Some(pid) = holder_pid(&socket)
            {
                let _ = rustix::process::kill_process(pid_of(pid), Signal::KILL);
            }
        }
        // A killed holder can't end its program: end what runs from this
        // world's folder, which no other test uses.
        let _ = std::process::Command::new("pkill")
            .arg("-KILL")
            .arg("-f")
            .arg(format!("{}/", self.root.display()))
            .status();
    }
}

fn pid_of(pid: u32) -> Pid {
    Pid::from_raw(i32::try_from(pid).unwrap()).unwrap()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

async fn answers(socket: &Path) -> bool {
    humata2_holder::attach(socket, u64::MAX).await.is_ok()
}

async fn until(what: &str, check: impl AsyncFn() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !check().await {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// What the terminal on `socket` shows from `from`, read until `needle`.
async fn read_until(socket: &Path, from: u64, needle: &str) -> Vec<u8> {
    let mut attached = humata2_holder::attach(socket, from).await.unwrap();
    let mut bytes = Vec::new();
    let read = tokio::time::timeout(WAIT, async {
        while !contains(&bytes, needle) {
            match attached.events.next().await.unwrap() {
                Some(Event::Output { data, .. }) => bytes.extend(data),
                Some(_) => {}
                None => panic!("the holder closed the connection"),
            }
        }
    })
    .await;
    assert!(
        read.is_ok(),
        "never saw {needle:?} in {:?}",
        String::from_utf8_lossy(&bytes)
    );
    bytes
}

/// `kill -9` the holder on `socket`, and waits until nothing answers.
async fn kill_holder(socket: &Path) {
    let pid = holder_pid(socket).unwrap();
    rustix::process::kill_process(pid_of(pid), Signal::KILL).unwrap();
    until("the killed holder stops answering", async || !answers(socket).await).await;
}

/// Launches the agent through revive, as the kill matrix does, then kills
/// its holder, and runs the daemon's reconcile with `marker` written beside
/// the socket first when `marker` is set.
async fn lose_holder_and_reconcile(world: &World, marker: bool) -> humata2_revive::Reconciled {
    let store = world.store();
    world.launch_with(&store, world.launch()).await;
    read_until(&world.socket(), 0, "agent up --session-id").await;
    if marker {
        fs::write(humata2_holder::handoff_path(&world.socket()), b"H2HANDOFF").unwrap();
    }
    kill_holder(&world.socket()).await;
    reconcile(&store, &world.config(), &world.env(), None).await.unwrap()
}

fn restart_json(folder: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(folder.join("restart.json")).unwrap()).unwrap()
}

fn session_entry<'a>(json: &'a serde_json::Value, session: &str) -> &'a serde_json::Value {
    json["sessions"]
        .as_array()
        .and_then(|entries| entries.iter().find(|entry| entry["session"] == session))
        .unwrap_or_else(|| panic!("no entry for {session} in {json}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_whose_holder_died_mid_move_is_reported_lost_and_its_marker_removed() {
    let world = World::new();
    let marker = humata2_holder::handoff_path(&world.socket());

    let reconciled = lose_holder_and_reconcile(&world, true).await;

    let folder = reconciled.report.as_ref().expect("no restart report was written");
    let json = restart_json(folder);
    let entry = session_entry(&json, "se_a");
    assert!(entry["lost"].as_str().is_some_and(|text| !text.is_empty()), "{entry}");
    assert!(!marker.exists(), "the move marker was left beside the holder's socket");
    let Some((session, Ok(Revived::Resumed { .. }))) = reconciled.agents.first() else {
        panic!("not resumed: {reconciled:?}");
    };
    assert_eq!(session, "se_a");
    read_until(&world.socket(), 0, "agent up --resume").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_holder_that_died_without_a_marker_is_not_reported_lost() {
    let world = World::new();

    let reconciled = lose_holder_and_reconcile(&world, false).await;

    let folder = reconciled.report.as_ref().expect("no restart report was written");
    let json = restart_json(folder);
    let entry = session_entry(&json, "se_a");
    assert!(entry["lost"].is_null(), "{entry}");
    let Some((session, Ok(Revived::Resumed { .. }))) = reconciled.agents.first() else {
        panic!("not resumed: {reconciled:?}");
    };
    assert_eq!(session, "se_a");
    read_until(&world.socket(), 0, "agent up --resume").await;
}
