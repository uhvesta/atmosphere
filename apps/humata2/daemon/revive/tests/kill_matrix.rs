//! PER-23's kill matrix: kills an agent's holder, and the daemon at each
//! point of a launch and of a resume, then starts the daemon's reconcile
//! and checks the session comes back with its conversation and output, or
//! says exactly why it couldn't and leaves nothing running in its place.
//!
//! The agent is a small `sh` script standing in for one: `--session-id`
//! starts a conversation file, `--resume` writes to it, `--fresh` starts
//! another, and `--die` exits 3. The daemon killed part way is this test
//! binary run again with `kill_child` selected and `HUMATA2_STOP_AT` set;
//! that test does nothing otherwise. Needs the crates built with `faults`.

use std::ffi::OsString;
use std::fs;
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use humata2_holder::{Event, HolderSpec, holder_pid, read_saved, saved_scrollback_path};
use humata2_revive::{
    RestartReports, ReviveConfig, Revived, ShellRevived, launch_agent, reconcile,
};
use humata2_store::{
    AgentLaunch, AgentRecord, AgentState, Change, Restored, ResumeFailure, RowChange,
    SessionRecord, Store, TabKind, TabRecord,
};
use rustix::process::{Pid, Signal};

const CHILD: &str = "HUMATA2_REVIVE_KILL_CHILD";
const ROOT: &str = "HUMATA2_REVIVE_ROOT";
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
    owned: Option<tempfile::TempDir>,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("h2r")
            .tempdir_in("/tmp")
            .unwrap();
        let root = dir.path().to_owned();
        for folder in ["state", "conv", "h", "logs", "work"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        fs::write(root.join("agent.sh"), AGENT).unwrap();
        Self {
            root,
            owned: Some(dir),
        }
    }

    /// The world a parent test made, seen from the child it started.
    fn at(root: PathBuf) -> Self {
        Self { root, owned: None }
    }

    fn store(&self) -> Arc<Store> {
        Arc::new(Store::open(self.root.join("state")).unwrap())
    }

    fn config(&self) -> ReviveConfig {
        ReviveConfig {
            contain: None,
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
        }
    }

    async fn launch_with(&self, store: &Arc<Store>, launch: AgentLaunch) -> AgentRecord {
        store.put_session(&self.session()).unwrap();
        launch_agent(store, &self.config(), &self.session(), "tb_a", launch, None, &self.env())
            .await
            .unwrap()
    }
}

impl Drop for World {
    /// Stops every holder still answering, however the test ended.
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
    }
}

fn pid_of(pid: u32) -> Pid {
    Pid::from_raw(i32::try_from(pid).unwrap()).unwrap()
}

/// The daemon killed part way. Runs only when a test below starts it.
#[test]
fn kill_child() {
    let Ok(mode) = std::env::var(CHILD) else {
        return;
    };
    let world = World::at(PathBuf::from(std::env::var_os(ROOT).unwrap()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let store = world.store();
        let (config, env) = (world.config(), world.env());
        match mode.as_str() {
            "launch" => {
                launch_agent(&store, &config, &world.session(), "tb_a", world.launch(), None, &env)
                    .await
                    .unwrap();
            }
            "reconcile" => {
                reconcile(&store, &config, &env, None).await.unwrap();
            }
            other => panic!("no kill mode {other}"),
        }
    });
}

/// Runs the daemon in `mode` until `stop_at`, and checks it was killed
/// there rather than finishing.
fn kill_at(world: &World, mode: &str, stop_at: &str) {
    let status = child(world, mode, stop_at).status().unwrap();
    assert_eq!(status.signal(), Some(9), "{mode} never reached {stop_at}");
}

/// As [`kill_at`], with the power cut there too: the state folder and the
/// holders' folder lose what the daemon hadn't synced, and the holders it
/// started inherit the cut, played with SIGUSR2.
fn power_cut_at(world: &World, mode: &str, stop_at: &str) {
    let roots = std::env::join_paths([world.root.join("state"), world.root.join("h")]).unwrap();
    let status = child(world, mode, stop_at)
        .env("HUMATA2_POWER_CUT_ROOTS", roots)
        .env("HUMATA2_POWER_CUT_SPARE", world.root.join("spare"))
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(9), "{mode} never reached {stop_at}");
}

fn child(world: &World, mode: &str, stop_at: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["kill_child", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, mode)
        .env(ROOT, &world.root)
        .env("HUMATA2_STOP_AT", stop_at)
        .env_remove("TESTBRIDGE_TEST_ONLY");
    command
}

/// Cuts the power of every holder still answering: each drops what it
/// hadn't synced and dies.
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

/// Types `text` and a return, and waits for `needle`.
async fn type_and_see(socket: &Path, text: &str, needle: &str) {
    let mut attached = humata2_holder::attach(socket, u64::MAX).await.unwrap();
    attached
        .control
        .input(1, format!("{text}\r").as_bytes())
        .await
        .unwrap();
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

/// `kill -9` the holder on `socket`, and waits until nothing answers.
async fn kill_holder(socket: &Path) -> u32 {
    let pid = holder_pid(socket).unwrap();
    rustix::process::kill_process(pid_of(pid), Signal::KILL).unwrap();
    until("the killed holder stops answering", async || !answers(socket).await).await;
    pid
}

/// Checks the agent is saved as running in the one holder answering on
/// its socket, with its tab, and returns that holder's pid.
async fn one_running(world: &World, store: &Store) -> u32 {
    let record = store.agent("se_a").unwrap().unwrap();
    let AgentState::Running { holder_pid: pid } = record.state else {
        panic!("the agent isn't running: {:?}", record.state);
    };
    assert!(answers(&world.socket()).await, "no holder answers");
    assert_eq!(holder_pid(&world.socket()), Some(pid));
    let tab = store.tab("tb_a").unwrap().unwrap();
    assert_eq!((tab.holder_pid, tab.kind), (pid, TabKind::Agent));
    pid
}

/// Checks the conversation is the one launched, never a fresh one.
fn same_conversation(world: &World, said: &str) {
    let conversation = fs::read_to_string(world.conv().join("c1.jsonl")).unwrap();
    assert!(conversation.contains(said), "the conversation lost {said:?}");
    assert!(!world.conv().join("other.jsonl").exists(), "a fresh conversation was started");
}

/// How many times the agent was told it was interrupted, as its
/// conversation shows.
fn times_told(world: &World) -> usize {
    let conversation = fs::read_to_string(world.conv().join("c1.jsonl")).unwrap();
    conversation.matches("Run `git status`").count()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_killed_holder_is_resumed_with_its_conversation_and_its_output() {
    let world = World::new();
    let store = world.store();
    world.launch_with(&store, world.launch()).await;
    type_and_see(&world.socket(), "hello", "got: hello").await;
    let killed = kill_holder(&world.socket()).await;

    let reconciled = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();

    let Some((session, Ok(Revived::Resumed { interruption }))) = reconciled.agents.first() else {
        panic!("not resumed: {reconciled:?}");
    };
    assert_eq!(session, "se_a");
    assert!(interruption.output_saved_through > 0);
    assert!(interruption.output_resumes_at >= interruption.output_saved_through);
    assert!(!interruption.saved_tail_damaged);
    let pid = one_running(&world, &store).await;
    assert_ne!(pid, killed);
    let record = store.agent("se_a").unwrap().unwrap();
    assert_eq!(record.generation, 2);
    assert_eq!(record.interruptions, vec![interruption.clone()]);
    // Told it was interrupted, in the conversation it had.
    same_conversation(&world, "hello");
    same_conversation(&world, "git status");
    assert_eq!(times_told(&world), 1);
    // The old output is shown above the resumed agent's.
    let shown = read_until(&world.socket(), 0, "agent up --resume").await;
    assert!(contains(&shown, "got: hello"));
    type_and_see(&world.socket(), "again", "got: again").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_killed_while_launching_comes_back_with_one_agent_or_none() {
    for point in [
        "store.agent.put.committing",
        "revive.launch.recorded",
        "revive.launch.spawned",
        "store.agent_and_tab.put.committing",
        "store.agent_and_tab.put.committed",
    ] {
        let world = World::new();
        world.store().put_session(&world.session()).unwrap();
        kill_at(&world, "launch", point);
        let left = match answers(&world.socket()).await {
            true => holder_pid(&world.socket()),
            false => None,
        };

        let store = world.store();
        let reconciled = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();

        if point == "store.agent.put.committing" {
            // Nothing was saved, so nothing was started.
            assert!(reconciled.agents.is_empty(), "{point}: {reconciled:?}");
            assert!(left.is_none() && !answers(&world.socket()).await, "{point}");
            continue;
        }
        let outcome = &reconciled.agents[0].1;
        match point {
            "store.agent_and_tab.put.committed" => {
                assert!(matches!(outcome, Ok(Revived::Reattached)), "{point}: {outcome:?}");
            }
            "revive.launch.recorded" => {
                assert!(matches!(outcome, Ok(Revived::Relaunched)), "{point}: {outcome:?}");
            }
            // It may have started its conversation before it was stopped.
            _ => assert!(
                matches!(outcome, Ok(Revived::Relaunched | Revived::Resumed { .. })),
                "{point}: {outcome:?}"
            ),
        }
        let pid = one_running(&world, &store).await;
        match left {
            Some(left) if point == "store.agent_and_tab.put.committed" => assert_eq!(pid, left),
            Some(left) => {
                assert_ne!(pid, left, "{point}");
                until("the unsaved holder is gone", async || {
                    rustix::process::test_kill_process(pid_of(left)).is_err()
                })
                .await;
            }
            None => {}
        }
        type_and_see(&world.socket(), "ping", "got: ping").await;
        assert!(!world.conv().join("other.jsonl").exists(), "{point}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_killed_while_resuming_finishes_the_resume_when_it_starts_again() {
    for point in [
        "store.agent.put.committing",
        "revive.resume.recorded",
        "revive.resume.spawned",
        "revive.resume.telling",
        "revive.resume.told",
        "revive.resume.verified",
        "store.agent_and_tab.put.committed",
    ] {
        let world = World::new();
        let store = world.store();
        world.launch_with(&store, world.launch()).await;
        type_and_see(&world.socket(), "hello", "got: hello").await;
        kill_holder(&world.socket()).await;
        kill_at(&world, "reconcile", point);

        let reconciled = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();

        let outcome = &reconciled.agents[0].1;
        if point == "store.agent_and_tab.put.committed" {
            assert!(matches!(outcome, Ok(Revived::Reattached)), "{point}: {outcome:?}");
        } else {
            assert!(matches!(outcome, Ok(Revived::Resumed { .. })), "{point}: {outcome:?}");
        }
        one_running(&world, &store).await;
        let record = store.agent("se_a").unwrap().unwrap();
        assert!(!record.interruptions.is_empty(), "{point}");
        same_conversation(&world, "hello");
        // Told it was interrupted once, however far the first try got.
        assert_eq!(times_told(&world), 1, "{point}");
        read_until(&world.socket(), 0, "got: hello").await;
        type_and_see(&world.socket(), "ping", "got: ping").await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_that_cannot_be_checked_says_why_and_runs_nothing() {
    #[derive(Clone, Copy)]
    enum Break {
        DeleteConversation,
        DeleteFolder,
        ResumeWith(&'static [&'static str]),
    }
    for (broken, reason, exit_code) in [
        (Break::DeleteConversation, ResumeFailure::ConversationFileMissing, None),
        (Break::DeleteFolder, ResumeFailure::FolderMissing, None),
        (Break::ResumeWith(&["--fresh", "c1"]), ResumeFailure::AgentStartedAnotherConversation, None),
        (Break::ResumeWith(&["--die"]), ResumeFailure::AgentExited, Some(3)),
        (Break::ResumeWith(&[]), ResumeFailure::AgentNotInstalled, None),
    ] {
        let world = World::new();
        let store = world.store();
        let mut launch = world.launch();
        match broken {
            Break::ResumeWith([]) => launch.resume_argv = vec!["/nonexistent/humata-agent".into()],
            Break::ResumeWith(args) => launch.resume_argv = world.agent_args(args),
            _ => {}
        }
        world.launch_with(&store, launch).await;
        type_and_see(&world.socket(), "hello", "got: hello").await;
        kill_holder(&world.socket()).await;
        match broken {
            Break::DeleteConversation => fs::remove_file(world.conv().join("c1.jsonl")).unwrap(),
            Break::DeleteFolder => fs::remove_dir_all(world.work()).unwrap(),
            Break::ResumeWith(_) => {}
        }

        let reconciled = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();

        let outcome = &reconciled.agents[0].1;
        assert!(
            matches!(outcome, Ok(Revived::ResumeFailed { reason: r, exit_code: c }) if *r == reason && *c == exit_code),
            "{reason:?}: {outcome:?}"
        );
        let record = store.agent("se_a").unwrap().unwrap();
        assert_eq!(record.state, AgentState::ResumeFailed { reason, exit_code });
        assert!(!answers(&world.socket()).await, "{reason:?}: something runs in its place");
        // What it printed before is kept for the "Couldn't resume" view.
        let saved = read_saved(&saved_scrollback_path(&world.socket())).unwrap().unwrap();
        assert!(saved.saved_through > 0, "{reason:?}");

        let again = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();
        assert!(matches!(again.agents[0].1, Ok(Revived::Left)), "{reason:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_agent_is_reattached_and_one_that_ended_is_saved_as_exited() {
    let world = World::new();
    let store = world.store();
    let launched = world.launch_with(&store, world.launch()).await;

    let reconciled = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();
    assert!(matches!(reconciled.agents[0].1, Ok(Revived::Reattached)));
    assert_eq!(store.agent("se_a").unwrap().unwrap(), launched);

    let mut attached = humata2_holder::attach(&world.socket(), u64::MAX).await.unwrap();
    attached.control.input(1, b"\x04").await.unwrap();
    until("the agent exits", async || {
        humata2_holder::attach(&world.socket(), u64::MAX)
            .await
            .is_ok_and(|attached| attached.welcome.exited.is_some())
    })
    .await;

    let reconciled = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();
    assert!(matches!(reconciled.agents[0].1, Ok(Revived::Exited { code: 0 })));
    assert_eq!(
        store.agent("se_a").unwrap().unwrap().state,
        AgentState::Exited { code: 0 }
    );
    let again = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();
    assert!(matches!(again.agents[0].1, Ok(Revived::Left)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_whose_holder_was_killed_comes_back_fresh_in_its_folder() {
    let world = World::new();
    let store = world.store();
    let (socket, spawned) = shell_tab(&world, &store);
    type_and_see(&socket, "echo mark-$((1+1))", "mark-2").await;
    kill_holder(&socket).await;

    let reconciled = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();

    assert!(reconciled.agents.is_empty());
    let Some((tab, Ok(ShellRevived::Restarted { saved: Some(saved) }))) = reconciled.shells.first()
    else {
        panic!("not restarted: {reconciled:?}");
    };
    assert_eq!(tab, "tb_s");
    assert!(saved.saved_through > 0);
    let restarted = store.tab("tb_s").unwrap().unwrap();
    assert_ne!(restarted.holder_pid, spawned);
    assert_eq!(holder_pid(&socket), Some(restarted.holder_pid));
    let shown = read_until(&socket, 0, "[Humata]").await;
    assert!(contains(&shown, "mark-2"));
    let folder = format!(
        "{}/work",
        world.root.file_name().unwrap().to_string_lossy()
    );
    type_and_see(&socket, "pwd", &folder).await;

    let again = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();
    assert!(matches!(again.shells[0].1, Ok(ShellRevived::Reattached)));
}

/// A shell tab of session se_a in a holder of its own, saved; returns its
/// socket and holder pid.
fn shell_tab(world: &World, store: &Store) -> (PathBuf, u32) {
    store.put_session(&world.session()).unwrap();
    let socket = world.root.join("h/tb_s.sock");
    let spawned = humata2_holder::spawn(&HolderSpec {
        holder: holder_binary(),
        socket: socket.clone(),
        program: PathBuf::from("/bin/sh"),
        args: Vec::new(),
        cwd: world.work(),
        env: Vec::new(),
        cols: 80,
        rows: 24,
        scrollback_bytes: 1 << 20,
        log: world.root.join("logs/tb_s.log"),
    })
    .unwrap();
    store
        .put_tab(&TabRecord {
            id: "tb_s".to_owned(),
            session: Some("se_a".to_owned()),
            kind: TabKind::Shell,
            title: "sh".to_owned(),
            holder_socket: socket.clone(),
            holder_pid: spawned.pid,
            created_unix_ms: 1,
        })
        .unwrap();
    (socket, spawned.pid)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_killed_as_a_resume_fails_fails_it_again_and_runs_nothing() {
    let world = World::new();
    let store = world.store();
    let mut launch = world.launch();
    launch.resume_argv = world.agent_args(&["--die"]);
    world.launch_with(&store, launch).await;
    type_and_see(&world.socket(), "hello", "got: hello").await;
    kill_holder(&world.socket()).await;
    kill_at(&world, "reconcile", "revive.resume.failing");
    assert!(!answers(&world.socket()).await, "the failed resume's holder was left running");

    let reconciled = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();

    let outcome = &reconciled.agents[0].1;
    assert!(
        matches!(outcome, Ok(Revived::ResumeFailed { reason: ResumeFailure::AgentExited, exit_code: Some(3) })),
        "{outcome:?}"
    );
    assert!(!answers(&world.socket()).await, "something runs in its place");
    let again = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();
    assert!(matches!(again.agents[0].1, Ok(Revived::Left)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_killed_before_it_saved_a_restarted_shell_finds_it_and_saves_it() {
    let world = World::new();
    let store = world.store();
    let (socket, killed) = shell_tab(&world, &store);
    kill_holder(&socket).await;
    kill_at(&world, "reconcile", "revive.shell.spawned");
    assert!(answers(&socket).await, "the restarted shell's holder didn't outlive the daemon");
    assert_eq!(store.tab("tb_s").unwrap().unwrap().holder_pid, killed);

    let reconciled = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();

    assert!(matches!(reconciled.shells[0].1, Ok(ShellRevived::Reattached)), "{reconciled:?}");
    let tab = store.tab("tb_s").unwrap().unwrap();
    assert_eq!(holder_pid(&socket), Some(tab.holder_pid));
    assert_ne!(tab.holder_pid, killed);
    type_and_see(&socket, "echo mark-$((2+2))", "mark-4").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_power_cut_while_launching_or_resuming_comes_back_with_its_conversation() {
    let launching = [
        "store.agent.put.committing",
        "revive.launch.recorded",
        "revive.launch.spawned",
        "store.agent_and_tab.put.committing",
        "store.agent_and_tab.put.committed",
    ];
    let resuming = [
        "store.agent.put.committing",
        "revive.resume.recorded",
        "revive.resume.spawned",
        "revive.resume.telling",
        "revive.resume.told",
        "revive.resume.verified",
        "store.agent_and_tab.put.committed",
    ];
    let cases = launching
        .iter()
        .map(|point| ("launch", *point))
        .chain(resuming.iter().map(|point| ("reconcile", *point)));
    for (mode, point) in cases {
        let world = World::new();
        {
            let store = world.store();
            store.put_session(&world.session()).unwrap();
            if mode == "reconcile" {
                world.launch_with(&store, world.launch()).await;
                type_and_see(&world.socket(), "hello", "got: hello").await;
                kill_holder(&world.socket()).await;
            }
            // Closed, so the cut daemon is the only one writing.
        }
        power_cut_at(&world, mode, point);
        cut_holders(&world).await;

        let store = world.store();
        let reconciled = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();

        let Some((_, outcome)) = reconciled.agents.first() else {
            // Lost before it was synced: nothing was started either.
            assert_eq!((mode, point), ("launch", "store.agent.put.committing"));
            assert!(!answers(&world.socket()).await, "{mode} {point}");
            continue;
        };
        let missing = !world.conv().join("c1.jsonl").exists();
        match outcome {
            Ok(Revived::Resumed { .. } | Revived::Relaunched) => {}
            Ok(Revived::ResumeFailed { reason: ResumeFailure::ConversationFileMissing, .. })
                if mode == "launch" && missing => continue,
            other => panic!("{mode} {point}: {other:?}"),
        }
        one_running(&world, &store).await;
        if mode == "reconcile" {
            same_conversation(&world, "hello");
            // Once per interruption: a cut after the resume was saved
            // interrupts the resumed agent too.
            let interruptions = if point == "store.agent_and_tab.put.committed" { 2 } else { 1 };
            assert_eq!(times_told(&world), interruptions, "{point}");
        }
        type_and_see(&world.socket(), "ping", "got: ping").await;
        assert!(!world.conv().join("other.jsonl").exists(), "{mode} {point}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_resume_is_reported_with_what_it_takes_to_diagnose_it() {
    let world = World::new();
    let store = world.store();
    let mut launch = world.launch();
    launch.resume_argv = world.agent_args(&["--die"]);
    world.launch_with(&store, launch).await;
    type_and_see(&world.socket(), "hello", "got: hello").await;
    kill_holder(&world.socket()).await;
    let restored = Restored {
        copy_taken_unix_ms: 1,
        damaged_kept_at: world.root.join("state/damaged-1/state.sqlite"),
        damage: "database disk image is malformed".to_owned(),
        changes: vec![RowChange {
            table: "sessions".to_owned(),
            key: "'se_b'".to_owned(),
            change: Change::AddedSinceCopy,
        }],
        unreadable_tables: Vec::new(),
    };

    fs::create_dir_all(world.root.join("logs")).unwrap();
    fs::write(
        world.root.join("logs/humatad2.previous.log"),
        "humatad2: serving\nhumatad2: the last thing before the crash\n",
    )
    .unwrap();

    let reconciled = reconcile(&store, &world.config(), &world.env(), Some(&restored)).await.unwrap();

    let folder = reconciled.report.as_ref().expect("no restart report was written");
    assert!(folder.starts_with(world.root.join("restarts")));
    for file in ["report.md", "events.jsonl", "state.txt", "daemon.log", "restart.json"] {
        assert!(folder.join(file).is_file(), "the report has no {file}");
    }
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(folder.join("restart.json")).unwrap()).unwrap();
    assert_eq!(json["version"], "0.0.0-test");
    assert!(json["store_schema_version"].as_i64().unwrap() > 0);
    assert_eq!(json["restored"]["lost"][0]["key"], "'se_b'");
    let session = &json["sessions"][0];
    assert_eq!(session["session"], "se_a");
    assert_eq!(session["was"], "running");
    assert_eq!(session["status"], "recoverable");
    assert_eq!(session["reason"], "agent_exited");
    assert_eq!(session["exit_code"], 3);
    assert_eq!(session["conversation_id"], "c1");
    for key in ["step", "needs"] {
        assert!(session[key].as_str().is_some_and(|text| !text.is_empty()), "no {key}");
    }
    assert!(session["holder_pid_saved"].as_u64().is_some());
    let file = |role: &str| {
        session["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["role"] == role)
            .unwrap_or_else(|| panic!("no {role} file listed"))
            .clone()
    };
    for role in ["conversation", "scrollback"] {
        let found = file(role);
        assert!(found["bytes"].as_u64().is_some_and(|bytes| bytes > 0), "{role}: {found}");
        assert!(found["modified_unix_ms"].as_i64().is_some(), "{role}: {found}");
    }
    // The terminals' variable values are masked, even where a path holds one.
    let conversation = file("conversation")["path"].as_str().unwrap().to_owned();
    assert!(conversation.ends_with("/c1.jsonl"), "{conversation}");
    let conv = world.conv().display().to_string();
    assert!(!fs::read_to_string(folder.join("restart.json")).unwrap().contains(&conv));
    // The failure is in the log a person reads, with the session.
    let log = fs::read_to_string(folder.join("daemon.log")).unwrap();
    assert!(contains(log.as_bytes(), "agent_exited"), "{log}");
    let state = fs::read_to_string(folder.join("state.txt")).unwrap();
    assert!(contains(state.as_bytes(), "se_a"), "{state}");
    // So are the daemon's own last lines from before the restart.
    assert!(contains(log.as_bytes(), "the last thing before the crash"), "{log}");
    assert_eq!(json["daemon_log"][1], "humatad2: the last thing before the crash");

    // A restart that brings everything back is reported too.
    let again = reconcile(&store, &world.config(), &world.env(), None).await.unwrap();
    let second = again.report.as_ref().expect("no second report");
    assert_ne!(second, folder);
}
