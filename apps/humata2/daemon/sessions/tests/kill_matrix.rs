//! PER-23 and PER-27 for channel setup and session start: the daemon is
//! killed at each point of creating a channel, cloning its mains and
//! starting a session; then revive's reconcile and `recover` run, and the
//! channel is set up and the session either runs on its branch or is gone
//! entirely, folder and partial clones included. Also a killed holder and
//! a reboot.
//!
//! The daemon killed part way is this test binary run again with
//! `kill_child` selected and `HUMATA2_STOP_AT` set; that test does nothing
//! otherwise. Needs the crates built with `faults`.

use std::ffi::OsString;
use std::fs;
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use humata2_holder::{Event, holder_pid};
use humata2_revive::{RestartReports, ReviveConfig, reconcile};
use humata2_sessions::{
    Agents, Layout, Leftovers, NewChannel, StartRequest, Starter, clone_mains, create_channel,
    recover,
};
use humata2_store::{AgentState, MainClone, StartStep, Store, TabRecord};
use rustix::process::{Pid, Signal};

const CHILD: &str = "HUMATA2_SESSIONS_KILL_CHILD";
const ROOT: &str = "HUMATA2_SESSIONS_ROOT";
const WAIT: Duration = Duration::from_secs(30);

const AGENT: &str = r#"mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
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

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

struct World {
    root: PathBuf,
    owned: Option<tempfile::TempDir>,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("h2s")
            .tempdir_in("/tmp")
            .unwrap();
        let root = dir.path().to_owned();
        for folder in ["state", "conv", "h", "logs", "Humata"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        fs::write(root.join("agent.sh"), AGENT).unwrap();
        for repo in ["api", "web"] {
            let path = root.join("remote/acme").join(repo);
            fs::create_dir_all(&path).unwrap();
            git(&path, &["init", "-b", "main"]);
            fs::write(path.join("f"), "x").unwrap();
            git(&path, &["add", "f"]);
            git(&path, &["commit", "-m", "one"]);
        }
        Self {
            root,
            owned: Some(dir),
        }
    }

    fn at(root: PathBuf) -> Self {
        Self { root, owned: None }
    }

    fn store(&self) -> Arc<Store> {
        Arc::new(Store::open(self.root.join("state")).unwrap())
    }

    fn layout(&self) -> Layout {
        Layout::new(self.root.join("Humata"))
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
                daemon_log: None,
            },
        }
    }

    fn starter(&self, store: &Arc<Store>) -> Starter {
        let root = self.root.display();
        let agents = format!(
            r#"[{{"name":"fake","program":"/bin/sh",
            "start_args":["{root}/agent.sh","--session-id","{{conversation}}"],
            "resume_args":["{root}/agent.sh","--resume","{{conversation}}"],
            "conversation_file":"{root}/conv/{{conversation}}.jsonl"}}]"#
        );
        Starter {
            store: store.clone(),
            layout: self.layout(),
            remote_template: format!("file://{root}/remote/{{owner}}/{{name}}"),
            agents: Agents::from_json(agents.as_bytes()).unwrap(),
            home: self.root.clone(),
            revive: self.config(),
            holders: self.root.join("h"),
            first_message: Default::default(),
        }
    }

    fn env(&self) -> impl Fn(&TabRecord) -> Vec<(OsString, OsString)> + Sync + 'static {
        let conv = self.root.join("conv");
        move |_tab: &TabRecord| vec![("CONV_DIR".into(), conv.clone().into_os_string())]
    }
}

impl Drop for World {
    fn drop(&mut self) {
        if self.owned.is_none() {
            return;
        }
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
        // A killed holder can't end its program, and a holder the test
        // killed as a reboot left one too: each runs this world's
        // `agent.sh`, a fresh folder's, so end them by it.
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

fn new_channel() -> NewChannel {
    NewChannel {
        id: None,
        name: "Payments Rewrite".to_owned(),
        repos: vec![
            ("acme".to_owned(), "api".to_owned()),
            ("acme".to_owned(), "web".to_owned()),
        ],
        machines: vec!["ma_1".to_owned()],
        branch_prefix: "me/".to_owned(),
        default_agent: "fake".to_owned(),
    }
}

fn request(channel: &str, first_message: Option<&str>) -> StartRequest {
    StartRequest {
        id: None,
        channel: channel.to_owned(),
        machine: "ma_1".to_owned(),
        title: "Fix the build".to_owned(),
        agent: None,
        model: None,
        repos: None,
        first_message: first_message.map(str::to_owned),
        branch: None,
        operation: None,
        cols: 80,
        rows: 24,
    }
}

/// Creates the channel, clones its mains and starts a session.
async fn setup_and_start(world: &World, first_message: Option<&str>) {
    let store = world.store();
    let starter = world.starter(&store);
    let channel = create_channel(&store, &starter.layout, new_channel()).unwrap();
    clone_mains(
        &store,
        &starter.layout,
        &starter.remote_template,
        &channel.id,
        |_, _| {},
    )
    .unwrap();
    let env = world.env();
    starter
        .start(request(&channel.id, first_message), &env, &mut |_| {})
        .await
        .unwrap();
}

/// The daemon killed part way. Runs only when a test below starts it.
#[test]
fn kill_child() {
    let Ok(_) = std::env::var(CHILD) else { return };
    let world = World::at(PathBuf::from(std::env::var_os(ROOT).unwrap()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(setup_and_start(&world, Some("hello")));
}

fn kill_at(world: &World, stop_at: &str) {
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["kill_child", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env(ROOT, &world.root)
        .env("HUMATA2_STOP_AT", stop_at)
        .env_remove("TESTBRIDGE_TEST_ONLY")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(9), "never reached {stop_at}");
}

async fn answers(socket: &Path) -> bool {
    humata2_holder::attach(socket, u64::MAX).await.is_ok()
}

async fn until(what: &str, check: impl AsyncFn() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !check().await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting until {what}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

/// What the terminal on `socket` shows from the start, read until `needle`.
async fn read_until(socket: &Path, needle: &str) {
    let mut attached = humata2_holder::attach(socket, 0).await.unwrap();
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
}

fn partials(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "partial") {
            found.push(path.clone());
        }
        if path.is_dir() {
            partials(&path, found);
        }
    }
}

fn branch_of(repo: &Path) -> String {
    let out = Command::new("git")
        .current_dir(repo)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Runs revive's reconcile and then recover, as the daemon does at start.
async fn restart(world: &World) -> Arc<Store> {
    let store = world.store();
    let env = world.env();
    let left = Leftovers::read(&store).unwrap();
    reconcile(&store, &world.config(), &env, None)
        .await
        .unwrap();
    recover(&world.starter(&store), &env, &left).await;
    store
}

/// Checks the world is either fully set up with one running session on its
/// branch, or has no trace of a session.
async fn check_settled(world: &World, point: &str) {
    let store = restart(world).await;
    let mut found = Vec::new();
    partials(&world.root.join("Humata"), &mut found);
    assert!(found.is_empty(), "{point}: partial clones left: {found:?}");

    let channels = store.channels().unwrap();
    let Some(channel) = channels.first() else {
        // Killed before the channel was saved.
        assert!(
            store.sessions().unwrap().is_empty(),
            "{point}: a session without a channel"
        );
        return;
    };
    for repo in &channel.repos {
        assert_eq!(
            repo.main,
            MainClone::Ready,
            "{point}: main clone of {} not ready",
            repo.name
        );
        let main = world.layout().main_clone(&channel.folder, &repo.name);
        assert!(
            main.join(".git").exists(),
            "{point}: no git repo at {main:?}"
        );
    }
    let sessions = store.sessions().unwrap();
    let folders = world.layout().sessions(&channel.folder);
    match sessions.as_slice() {
        [] => {
            let left = fs::read_dir(&folders).map(|d| d.count()).unwrap_or(0);
            assert_eq!(
                left, 0,
                "{point}: a session folder is left without a session"
            );
            assert!(
                store.session_details().unwrap().is_empty(),
                "{point}: details left"
            );
            assert!(store.agents().unwrap().is_empty(), "{point}: agent left");
        }
        [session] => {
            let detail = store.session_detail(&session.id).unwrap().unwrap();
            assert_eq!(detail.start, StartStep::Done, "{point}");
            assert_eq!(detail.repos.len(), 2, "{point}");
            for repo in &detail.repos {
                assert_eq!(
                    branch_of(&repo.path),
                    repo.branch,
                    "{point}: {:?}",
                    repo.path
                );
            }
            let agent = store.agent(&session.id).unwrap().unwrap();
            let AgentState::Running { holder_pid: pid } = agent.state else {
                panic!("{point}: the agent isn't running: {:?}", agent.state);
            };
            assert!(
                answers(&agent.launch.holder_socket).await,
                "{point}: no holder answers"
            );
            assert_eq!(
                holder_pid(&agent.launch.holder_socket),
                Some(pid),
                "{point}"
            );
        }
        more => panic!("{point}: {} sessions", more.len()),
    }
}

/// The points the daemon is killed at, and cut at by a power cut.
const POINTS: &[&str] = &[
    "store.channel.put.committing",
    "store.channel.put.committed",
    "sessions.channel.recorded",
    "sessions.channel.cloned",
    "sessions.channel.cloned@2",
    "store.channel.main_clone.committing",
    "store.channel.main_clone.committed",
    "store.channel.main_clone.committing@2",
    "sessions.start.recorded",
    "store.session.put.committing",
    "store.session.put.committed",
    "store.session_detail.put.committing",
    "store.session_detail.put.committed",
    "sessions.start.cloned",
    "sessions.start.cloned@2",
    "store.session_detail.put.committing@2",
    "store.session_detail.put.committed@2",
    "store.session_detail.put.committing@3",
    "sessions.start.launching",
    "revive.launch.recorded",
    "revive.launch.spawned",
    "store.session_detail.put.committing@4",
    "store.session_detail.put.committed@4",
];

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_killed_at_any_point_leaves_a_running_session_or_none() {
    for &point in POINTS {
        let world = World::new();
        kill_at(&world, point);
        check_settled(&world, point).await;
    }
}

/// As [`kill_at`], with the power cut there too: the state folder and the
/// holders' folder lose what the daemon hadn't synced, and the holders it
/// started are cut as well (see [`cut_holders`]).
fn power_cut_at(world: &World, stop_at: &str) {
    let roots = std::env::join_paths([world.root.join("state"), world.root.join("h")]).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["kill_child", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env(ROOT, &world.root)
        .env("HUMATA2_STOP_AT", stop_at)
        .env("HUMATA2_POWER_CUT_ROOTS", roots)
        .env("HUMATA2_POWER_CUT_SPARE", world.root.join("spare"))
        .env_remove("TESTBRIDGE_TEST_ONLY")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(9), "never reached {stop_at}");
}

/// Cuts the power of every holder still answering: each drops what it
/// hadn't synced and dies, as a test-build holder does on SIGUSR2.
async fn cut_holders(world: &World) {
    let Ok(entries) = fs::read_dir(world.root.join("h")) else {
        return;
    };
    for entry in entries.flatten() {
        let socket = entry.path();
        if socket.extension().is_some_and(|ext| ext == "sock")
            && answers(&socket).await
            && let Some(pid) = holder_pid(&socket)
        {
            let _ = rustix::process::kill_process(pid_of(pid), Signal::USR2);
            until("the cut holder stops answering", async || !answers(&socket).await).await;
        }
    }
}

fn entries_of(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|read| read.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// After [`check_settled`]: every folder, clone, session start, agent and
/// holder socket is one a saved row names, and every saved row names one
/// that is saved.
async fn nothing_orphaned(world: &World, point: &str) {
    let store = world.store();
    let channels = store.channels().unwrap();
    let sessions = store.sessions().unwrap();
    let details = store.session_details().unwrap();
    let agents = store.agents().unwrap();

    for session in &sessions {
        assert!(
            channels.iter().any(|c| c.id == session.channel),
            "{point}: session {} of no saved channel",
            session.id
        );
    }
    for detail in &details {
        assert!(
            sessions.iter().any(|s| s.id == detail.session),
            "{point}: a session start of no saved session: {}",
            detail.session
        );
    }
    for agent in &agents {
        assert!(
            sessions.iter().any(|s| s.id == agent.session),
            "{point}: an agent of no saved session: {}",
            agent.session
        );
    }

    let humata = world.layout().root();
    for folder in entries_of(&humata) {
        let Some(channel) = channels.iter().find(|c| c.folder == name_of(&folder)) else {
            panic!("{point}: a folder of no saved channel: {folder:?}");
        };
        for entry in entries_of(&world.layout().main(&channel.folder)) {
            let name = name_of(&entry);
            assert!(
                channel.repos.iter().any(|r| r.name == name),
                "{point}: a main clone of no saved repository: {entry:?}"
            );
        }
        for entry in entries_of(&world.layout().sessions(&channel.folder)) {
            let named = details.iter().any(|d| {
                d.repos.iter().any(|r| {
                    r.path.parent().map(name_of).as_deref() == Some(name_of(&entry).as_str())
                })
            });
            assert!(named, "{point}: a session folder no saved start names: {entry:?}");
        }
    }

    let running: Vec<String> = agents
        .iter()
        .filter(|a| matches!(a.state, AgentState::Running { .. }))
        .map(|a| name_of(&a.launch.holder_socket))
        .collect();
    for entry in entries_of(&world.root.join("h")) {
        if entry.extension().is_some_and(|ext| ext == "sock") && answers(&entry).await {
            assert!(
                running.contains(&name_of(&entry)),
                "{point}: a holder answers on {entry:?} that no running agent has"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_power_cut_at_any_point_leaves_a_running_session_or_none() {
    for &point in POINTS {
        let world = World::new();
        power_cut_at(&world, point);
        cut_holders(&world).await;
        check_settled(&world, point).await;
        nothing_orphaned(&world, point).await;
    }
}

/// Starts a session to the end, with a first message.
async fn started_session(world: &World) -> Arc<Store> {
    setup_and_start(world, Some("hello")).await;
    let store = world.store();
    let session = store.sessions().unwrap().remove(0);
    let agent = store.agent(&session.id).unwrap().unwrap();
    until("the first message reaches the agent", async || {
        fs::read_to_string(&agent.launch.conversation_file).is_ok_and(|c| c.contains("hello"))
    })
    .await;
    store
}

async fn resumed_with_its_conversation(world: &World, store: &Store) {
    let session = store.sessions().unwrap().remove(0);
    let agent = store.agent(&session.id).unwrap().unwrap();
    assert!(matches!(agent.state, AgentState::Running { .. }));
    assert_eq!(agent.generation, 2);
    let conversation = fs::read_to_string(&agent.launch.conversation_file).unwrap();
    assert!(conversation.contains("hello"));
    assert!(conversation.contains("git status"));
    let seen = fs::read_dir(world.root.join("conv")).unwrap().count();
    assert_eq!(seen, 1, "a fresh conversation was started");
    read_until(&agent.launch.holder_socket, "agent up --resume").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_killed_holder_resumes_the_session_with_its_conversation() {
    let world = World::new();
    let store = started_session(&world).await;
    let session = store.sessions().unwrap().remove(0);
    let socket = store
        .agent(&session.id)
        .unwrap()
        .unwrap()
        .launch
        .holder_socket;
    let pid = holder_pid(&socket).unwrap();
    rustix::process::kill_process(pid_of(pid), Signal::KILL).unwrap();
    until("the killed holder stops answering", async || {
        !answers(&socket).await
    })
    .await;
    drop(store);

    let store = restart(&world).await;
    resumed_with_its_conversation(&world, &store).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reboot_resumes_the_session_with_its_conversation() {
    let world = World::new();
    let store = started_session(&world).await;
    drop(store);
    // A reboot: every holder gone, and their sockets with them.
    for entry in fs::read_dir(world.root.join("h")).unwrap().flatten() {
        let socket = entry.path();
        if socket.extension().is_some_and(|e| e == "sock") {
            if let Some(pid) = holder_pid(&socket) {
                let _ = rustix::process::kill_process(pid_of(pid), Signal::KILL);
            }
            until("the holder stops answering", async || {
                !answers(&socket).await
            })
            .await;
        }
    }

    let store = restart(&world).await;
    resumed_with_its_conversation(&world, &store).await;
    drop(store);
    // Another restart changes nothing about the session.
    let again = restart(&world).await;
    assert_eq!(again.sessions().unwrap().len(), 1);
}
