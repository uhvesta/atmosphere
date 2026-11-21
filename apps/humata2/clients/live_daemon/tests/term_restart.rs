//! A session's shell tab across an app restart, on a real `humatad2` (TERM):
//! the app quits (its client and links drop), the program prints while no
//! client is attached, and a new client's link reattaches the same holder,
//! catches the output up from the screen, and keeps typing off until that
//! screen has arrived.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use ghostty_vt::Terminal;
use humata2_clients::model::{Agent, SessionId, SessionOptions, Status};
use humata2_clients::{Daemon, SendError, TerminalLink, TerminalNotice, Terminals};
use humata2_clients_live_daemon::{LiveDaemon, LiveOptions};
use tempfile::TempDir;
use terminal_wire::{ClientFrame, ServerFrame};

const AGENT_NAME: &str = "fake";

/// How long anything may take before the case fails.
const DEADLINE: Duration = Duration::from_secs(60);

/// How often the socket or a file is tried while waiting for it.
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
            .prefix("h2m")
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
        let mut events = within("events", daemon.events())
            .await
            .expect("subscribe to daemon events");
        let channel = within(
            "create_channel",
            daemon.create_channel("payments", vec![repo.clone()], vec![mac.clone()]),
        )
        .await
        .expect("create the channel");
        within("channel setup done", async {
            while let Some(item) = events.recv().await {
                if let Ok(humata2_clients::Event::ChannelSetup(setup)) = item {
                    if !setup.steps.is_empty()
                        && setup
                            .steps
                            .iter()
                            .all(|step| step.state == humata2_clients::model::SetupState::Done)
                    {
                        return;
                    }
                    if let Some(reason) = setup.steps.iter().find_map(|step| match &step.state {
                        humata2_clients::model::SetupState::Failed { reason } => {
                            Some(reason.as_str())
                        }
                        _ => None,
                    }) {
                        panic!("channel setup failed: {reason}");
                    }
                }
            }
            panic!("event stream closed before channel setup finished");
        })
        .await;
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


/// What a tab's frames add up to: the screen the link installed, with the
/// output after it, and the input the daemon has confirmed.
struct Screen {
    terminal: Option<Terminal>,
    written: u64,
}

impl Screen {
    fn new() -> Self {
        Screen {
            terminal: None,
            written: 0,
        }
    }

    fn take(&mut self, frame: ServerFrame) {
        match frame {
            ServerFrame::Snapshot { bytes, .. } => {
                self.terminal = Some(Terminal::from_checkpoint(&bytes).expect("a screen installs"));
            }
            ServerFrame::Output { bytes, .. } => {
                if let Some(terminal) = self.terminal.as_mut() {
                    terminal.feed(&bytes);
                }
            }
            ServerFrame::InputWritten { seq, .. } => self.written = self.written.max(seq),
            ServerFrame::Error { message, .. } => panic!("the link failed: {message}"),
            _ => {}
        }
    }

    fn shows(&mut self, text: &str) -> bool {
        self.terminal
            .as_mut()
            .is_some_and(|terminal| terminal.search(text).is_ok_and(|found| !found.is_empty()))
    }
}

/// Opens a shell tab in the session and returns its index.
async fn shell_tab(daemon: &LiveDaemon, session: &SessionId) -> usize {
    let until = Instant::now() + DEADLINE;
    loop {
        match daemon.open(session, Agent::Shell).await {
            Ok((tab, _)) => return tab,
            Err(error) => {
                assert!(Instant::now() < until, "could not open the shell tab: {error:?}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

/// Reads `link` until `done` holds of the screen.
async fn read_until(
    link: &mut TerminalLink,
    screen: &mut Screen,
    what: &str,
    mut done: impl FnMut(&mut Screen) -> bool,
) {
    let until = Instant::now() + DEADLINE;
    while !done(screen) {
        let left = until.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, link.frames.recv()).await {
            Ok(Some(frame)) => screen.take(frame),
            Ok(None) => panic!("the link ended while waiting for {what}"),
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

fn typed(seq: u64, line: &str) -> ClientFrame {
    ClientFrame::Input {
        id: 0,
        seq,
        bytes: format!("{line}\r").into_bytes(),
    }
}

/// Waits for `path` to exist.
async fn exists(path: &Path) {
    within("the shell's job to finish", async {
        while !path.exists() {
            tokio::time::sleep(POLL).await;
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_tab_catches_up_after_the_app_restarts_and_types_only_once_synced() {
    let host = Host::start().await;
    let gate = host.root().join("gate");
    let done = host.root().join("done");

    // The app before it quits: a background job in the tab's shell waits
    // for the gate file, then prints a line only the shell's own variable
    // can complete (the typed line holds `$HELD`, never `MARKok`).
    let first = host.connect().await;
    let session = host.session(&first, "Fix the ledger").await;
    let tab = shell_tab(&first, &session).await;
    let mut link = first.link(&session, tab).await.expect("the tab's link opens");
    let mut screen = Screen::new();
    read_until(&mut link, &mut screen, "the first screen", |s| s.terminal.is_some()).await;
    let job = format!(
        "HELD=ok; (while [ ! -e {} ]; do sleep 0.05; done; echo MARK$HELD; touch {}) &",
        gate.display(),
        done.display()
    );
    // Typing is admitted once the screen is in; retry until it is.
    let until = Instant::now() + DEADLINE;
    while link.sink.send(typed(1, &job)) == Err(SendError::Closed) {
        assert!(Instant::now() < until, "typing was never admitted");
        tokio::time::sleep(POLL).await;
    }
    read_until(&mut link, &mut screen, "the daemon to take the line", |s| s.written >= 1).await;

    // The app quits: its client and link are gone, the shell is not.
    drop(link);
    drop(first);
    std::fs::write(&gate, b"").expect("open the gate");
    exists(&done).await;

    // The app starts again.
    let second = host.connect().await;
    let mut link = second.link(&session, tab).await.expect("the tab's link opens");
    let mut screen = Screen::new();
    let mut accepted = false;
    let until = Instant::now() + DEADLINE;
    loop {
        if !accepted {
            match link.sink.send(typed(1, "echo SENT$((1+1))")) {
                Ok(()) => {
                    // Typing is admitted only once the screen is on its
                    // way: if it is not here yet, it is the next frame.
                    while let Ok(frame) = link.frames.try_recv() {
                        screen.take(frame);
                    }
                    if screen.terminal.is_none() {
                        let next = tokio::time::timeout(DEADLINE, link.frames.recv())
                            .await
                            .expect("the screen arrives")
                            .expect("the link is open");
                        assert!(
                            matches!(next, ServerFrame::Snapshot { .. }),
                            "typing was admitted before the screen was sent"
                        );
                        screen.take(next);
                    }
                    accepted = true;
                }
                Err(SendError::Closed) => {}
                Err(other) => panic!("typing failed: {other:?}"),
            }
        }
        if accepted && screen.shows("MARKok") && screen.shows("SENT2") {
            break;
        }
        assert!(Instant::now() < until, "the screen never showed the catch-up and the answer");
        match tokio::time::timeout(Duration::from_millis(500), link.frames.recv()).await {
            Ok(Some(frame)) => screen.take(frame),
            Ok(None) => panic!("the link ended"),
            Err(_) => {}
        }
    }
}

/// A Terminal session (SPEC §1, flow 9 step 3): the app asks a real daemon
/// for its plain shell, and the session's first tab is a shell that answers,
/// with no agent and no turns.
#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_session_starts_a_shell_with_no_turns() {
    let host = Host::start().await;
    let daemon = host.connect().await;
    let workspace = within("load", daemon.load()).await.expect("load the workspace");
    let mac = workspace
        .machines
        .first()
        .expect("the workspace has a machine")
        .id
        .clone();
    let repo = host.repo("ledger");
    let mut events = within("events", daemon.events())
        .await
        .expect("subscribe to daemon events");
    let channel = within(
        "create_channel",
        daemon.create_channel("ledger", vec![repo.clone()], vec![mac.clone()]),
    )
    .await
    .expect("create the channel");
    within("channel setup done", async {
        while let Some(item) = events.recv().await {
            if let Ok(humata2_clients::Event::ChannelSetup(setup)) = item {
                if !setup.steps.is_empty()
                    && setup
                        .steps
                        .iter()
                        .all(|step| step.state == humata2_clients::model::SetupState::Done)
                {
                    return;
                }
                if let Some(reason) = setup.steps.iter().find_map(|step| match &step.state {
                    humata2_clients::model::SetupState::Failed { reason } => Some(reason.as_str()),
                    _ => None,
                }) {
                    panic!("channel setup failed: {reason}");
                }
            }
        }
        panic!("event stream closed before channel setup finished");
    })
    .await;
    let session = within(
        "create_session",
        daemon.create_session(
            &channel,
            Agent::Shell,
            &mac,
            "Tail the logs",
            SessionOptions {
                repos: vec![repo],
                ..SessionOptions::default()
            },
        ),
    )
    .await
    .expect("the daemon starts a Terminal session");

    // Its first tab is the shell, and it answers what is typed.
    let mut link = daemon.link(&session, 0).await.expect("the shell's link opens");
    let mut screen = Screen::new();
    read_until(&mut link, &mut screen, "the first screen", |s| s.terminal.is_some()).await;
    let until = Instant::now() + DEADLINE;
    while link.sink.send(typed(1, "echo SHELL$((2+3))")) == Err(SendError::Closed) {
        assert!(Instant::now() < until, "typing was never admitted");
        tokio::time::sleep(POLL).await;
    }
    read_until(&mut link, &mut screen, "the shell's answer", |s| s.shows("SHELL5")).await;

    // A running shell has no turns, so it settles at Idle, not Working.
    let until = Instant::now() + DEADLINE;
    loop {
        let workspace = within("load", daemon.load()).await.expect("load the workspace");
        let found = workspace
            .sessions
            .iter()
            .find(|candidate| candidate.id == session)
            .expect("the workspace lists the session");
        assert_eq!(found.agent, Agent::Shell);
        if found.status == Status::Idle {
            break;
        }
        assert!(
            Instant::now() < until,
            "the Terminal session reads {:?}, not Idle",
            found.status
        );
        tokio::time::sleep(POLL).await;
    }
}

/// The session's clone folder of `repo`, found under the daemon's channels.
fn clone_folder(host: &Host, repo: &str) -> PathBuf {
    let sessions = std::fs::read_dir(host.root().join("ch"))
        .expect("read the channels")
        .filter_map(Result::ok)
        .map(|channel| channel.path().join("sessions"))
        .filter(|sessions| sessions.is_dir());
    for sessions in sessions {
        for session in std::fs::read_dir(sessions).expect("read the sessions").filter_map(Result::ok) {
            let clone = session.path().join(repo);
            if clone.is_dir() {
                return clone;
            }
        }
    }
    panic!("no clone of {repo} under the channels");
}

/// "Open terminal here" (PL-10): the live client asks the real daemon for a
/// new shell in a folder of the session's clone, and the shell is there; a
/// folder elsewhere on the machine is refused and opens nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_shell_opened_in_a_clone_subfolder_starts_there_and_a_folder_elsewhere_is_refused() {
    let host = Host::start().await;
    let daemon = host.connect().await;
    let session = host.session(&daemon, "Open here").await;
    // The daemon answers once the session is recorded and clones after;
    // the agent's terminal opens only once the start is done.
    let until = Instant::now() + DEADLINE;
    while let Err(error) = daemon.open(&session, Agent::Claude).await {
        assert!(Instant::now() < until, "the session never started: {error:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let sub = clone_folder(&host, "ledger").join("open_here_sub");
    std::fs::create_dir_all(&sub).expect("make the subfolder");
    let elsewhere = host.root().join("not_in_the_session");
    std::fs::create_dir_all(&elsewhere).expect("make the other folder");

    let until = Instant::now() + DEADLINE;
    let tab = loop {
        match daemon.open_in(&session, Agent::Shell, &sub.display().to_string()).await {
            Ok((tab, _)) => break tab,
            Err(error) => {
                assert!(Instant::now() < until, "could not open the shell in the folder: {error:?}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    };
    let mut link = daemon.link(&session, tab).await.expect("the tab's link opens");
    let mut screen = Screen::new();
    read_until(&mut link, &mut screen, "the first screen", |s| s.terminal.is_some()).await;
    let until = Instant::now() + DEADLINE;
    while link.sink.send(typed(1, "pwd")) == Err(SendError::Closed) {
        assert!(Instant::now() < until, "typing was never admitted");
        tokio::time::sleep(POLL).await;
    }
    read_until(&mut link, &mut screen, "the shell to print its folder", |s| {
        s.shows("open_here_sub")
    })
    .await;

    let refused = daemon
        .open_in(&session, Agent::Shell, &elsewhere.display().to_string())
        .await;
    assert!(refused.is_err(), "a folder outside the session's places must be refused");
}

/// Reads `link`'s frames and notices until a notice says `others` others
/// can type into the tab.
async fn writers_become(link: &mut TerminalLink, screen: &mut Screen, others: u32) {
    let until = Instant::now() + DEADLINE;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        let notice = tokio::time::timeout(left, async {
            tokio::select! {
                frame = link.frames.recv() => match frame {
                    Some(frame) => {
                        screen.take(frame);
                        None
                    }
                    None => panic!("the link ended while waiting for {others} other writers"),
                },
                Some(notice) = link.notices.recv() => Some(notice),
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the link never said {others} others can type"));
        if notice == Some(TerminalNotice::Writers { others }) {
            return;
        }
    }
}

/// Two viewers on one shell tab (SE-24, micro case 20), each on its own
/// connection to a real daemon: the first one's link says one other can
/// type once the second attaches, and none once it has gone.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_viewer_on_a_shell_tab_is_told_to_the_first_and_so_is_its_leaving() {
    let host = Host::start().await;
    let a = host.connect().await;
    let session = host.session(&a, "Fix the ledger").await;
    let tab = shell_tab(&a, &session).await;
    let mut first = a.link(&session, tab).await.expect("viewer A's link opens");
    let mut screen = Screen::new();
    writers_become(&mut first, &mut screen, 0).await;

    let b = host.connect().await;
    let mut second = b.link(&session, tab).await.expect("viewer B's link opens");
    let mut other = Screen::new();
    writers_become(&mut second, &mut other, 1).await;
    writers_become(&mut first, &mut screen, 1).await;

    drop(second);
    drop(b);
    writers_become(&mut first, &mut screen, 0).await;
}
