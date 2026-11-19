//! Searching and reading sessions' conversations through `LiveHistory`
//! against a real `humatad2` (admin build, so every call is the user's)
//! (LD-23 to LD-25). Each case starts one session whose stand-in agent
//! writes a Claude conversation file the case made; the host mirrors it
//! and indexes it. Every case has its own daemon, folder and socket.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use humata2_clients::history::{History, SearchHit, SearchPage, TurnRole};
use humata2_clients::model::{Agent, ChannelId, SessionId, SessionOptions};
use humata2_clients::{Daemon, DaemonError};
use humata2_clients_live_daemon::history::LiveHistory;
use humata2_clients_live_daemon::{LiveDaemon, LiveOptions};
use serde_json::{Value, json};
use tempfile::TempDir;

const AGENT_NAME: &str = "fake";

/// How long anything may take before the case fails.
const DEADLINE: Duration = Duration::from_secs(60);

/// How often the socket, a file or a search is tried again while waiting.
const POLL: Duration = Duration::from_millis(50);

/// The conversation the next session's agent starts with, under the case's
/// folder.
const SEED: &str = "seed.jsonl";

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

/// Stands in for an agent: a new conversation starts as a copy of the
/// case's seed, then `agent.up` appears. What is typed is answered, never
/// written to the conversation.
fn agent_script(root: &Path, conv: &Path) -> String {
    format!(
        r#"DIR='{root}'
CONV_DIR='{conv}'
mode=$1; id=$2
case "$mode" in
  --session-id) cp "$DIR/{SEED}" "$CONV_DIR/$id.jsonl" ;;
esac
echo "agent up $mode"
: > "$DIR/agent.up"
while IFS= read -r line; do echo "got: $line"; done
"#,
        root = root.display(),
        conv = conv.display(),
    )
}

async fn within<T>(what: &str, work: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(DEADLINE, work).await {
        Ok(value) => value,
        Err(_) => panic!("timed out waiting for {what}"),
    }
}

// Lines as Claude writes them to its conversation file.

fn at(n: usize) -> String {
    format!("2026-11-10T{:02}:{:02}:00.000Z", 9 + n / 60, n % 60)
}

fn line(kind: &str, content: Value, n: usize) -> String {
    json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "external",
        "cwd": "/work",
        "sessionId": "0b6c1a4e-1111-4222-8333-444455556666",
        "version": "9.9.9",
        "type": kind,
        "message": {"role": kind, "content": content},
        "uuid": format!("7a1d2c3b-aaaa-4bbb-8ccc-{n:012}"),
        "timestamp": at(n),
    })
    .to_string()
}

/// A prompt the user typed.
fn user(text: &str, n: usize) -> String {
    line("user", json!(text), n)
}

/// An agent line that only says `words`.
fn says(words: &str, n: usize) -> String {
    line("assistant", json!([{"type": "text", "text": words}]), n)
}

fn tool_use(id: &str, name: &str, input: Value, n: usize) -> String {
    line(
        "assistant",
        json!([{"type": "tool_use", "id": id, "name": name, "input": input}]),
        n,
    )
}

fn tool_result(id: &str, answer: &str, n: usize) -> String {
    line(
        "user",
        json!([{"type": "tool_result", "tool_use_id": id, "content": answer}]),
        n,
    )
}

/// Two turns: the first names a quokka, the second runs a tool.
fn two_turns() -> Vec<String> {
    vec![
        user("Rename the quokka ledger column", 0),
        says("Renamed the column.", 1),
        user("Now run the tests", 2),
        tool_use("toolu_1", "Bash", json!({"command": "make test"}), 3),
        tool_result("toolu_1", "12 passed", 4),
        says("All twelve pass.", 5),
    ]
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
            .prefix("h2h")
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
        std::fs::write(&script, agent_script(root, &conv)).expect("write the agent");
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

    /// Kills the daemon, as a crash would, and starts it again on the same
    /// state. Connections made before are to the old daemon.
    async fn restart(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // On macOS a killed daemon's socket still takes connections for a
        // moment after it is reaped; start the next one once it refuses.
        let socket = self.socket();
        within("the killed humatad2's socket to close", async {
            while tokio::net::UnixStream::connect(&socket).await.is_ok() {
                tokio::time::sleep(POLL).await;
            }
        })
        .await;
        self.launch().await;
    }

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

    /// Waits for `name` to appear in the case's folder.
    async fn file(&self, name: &str, what: &str) {
        let path = self.root().join(name);
        within(what, async {
            while !path.exists() {
                tokio::time::sleep(POLL).await;
            }
        })
        .await;
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

/// A host with one session, "Fix the ledger" in channel "payments", whose
/// agent wrote `lines` as its conversation; and the history client of a
/// connection to it.
///
/// The host mirrors every session it knows when it starts, and after that
/// on the session's events. The agent's file is written after the session
/// is made, so which event comes after it isn't fixed; the host is
/// restarted once the file is there, so the start's pass is sure to see
/// it. Searches still wait for the host to catch up.
struct Seeded {
    _host: Host,
    _daemon: LiveDaemon,
    history: LiveHistory,
    channel: ChannelId,
    session: SessionId,
}

async fn seeded(lines: &[String]) -> Seeded {
    let mut host = Host::start().await;
    let daemon = host.connect().await;
    let seed: String = lines.iter().map(|line| format!("{line}\n")).collect();
    std::fs::write(host.root().join(SEED), seed).expect("write the seed conversation");

    let workspace = within("load", daemon.load()).await.expect("load the workspace");
    let mac = workspace
        .machines
        .first()
        .expect("the workspace has a machine")
        .id
        .clone();
    let repo = host.repo("ledger");
    let channel = within(
        "create_channel",
        daemon.create_channel("payments", vec![repo.clone()], vec![mac.clone()]),
    )
    .await
    .expect("create the channel");
    let session = within(
        "create_session",
        daemon.create_session(
            &channel,
            Agent::Claude,
            &mac,
            "Fix the ledger",
            SessionOptions {
                repos: vec![repo],
                ..SessionOptions::default()
            },
        ),
    )
    .await
    .expect("create the session");
    host.file("agent.up", "the session's agent to write its conversation")
        .await;
    drop(daemon);

    host.restart().await;
    let daemon = host.connect().await;
    let history = LiveHistory::new(&daemon);
    Seeded {
        _host: host,
        _daemon: daemon,
        history,
        channel,
        session,
    }
}

async fn search(history: &LiveHistory, query: &str, token: &str) -> SearchPage {
    within("search", history.search(query, token))
        .await
        .expect("search the conversations")
}

/// The first page for `query` once it has a hit of `session`: the host
/// mirrors in the background, so until it has caught up a search finds
/// nothing.
async fn caught_up(history: &LiveHistory, query: &str, session: &SessionId) -> SearchPage {
    let until = Instant::now() + DEADLINE;
    loop {
        let page = search(history, query, "").await;
        if page.0.iter().any(|hit| &hit.session == session) {
            return page;
        }
        assert!(
            Instant::now() < until,
            "the host never found {query:?} in the session"
        );
        tokio::time::sleep(POLL).await;
    }
}

fn the_hit<'a>(page: &'a SearchPage, session: &SessionId) -> &'a SearchHit {
    let hits: Vec<&SearchHit> = page.0.iter().filter(|hit| &hit.session == session).collect();
    assert_eq!(hits.len(), 1, "one hit for one matching turn: {hits:?}");
    hits[0]
}

#[tokio::test(flavor = "multi_thread")]
async fn a_search_finds_the_turn_that_has_the_words() {
    let world = seeded(&two_turns()).await;

    let page = caught_up(&world.history, "quokka", &world.session).await;
    let hit = the_hit(&page, &world.session);
    assert_eq!(hit.channel, world.channel);
    assert_eq!(hit.channel_name, "payments");
    assert_eq!(hit.title, "Fix the ledger");
    assert_eq!(hit.turn, 1);
    assert!(hit.snippet.to_lowercase().contains("quokka"), "{hit:?}");
    assert_eq!(page.1, "", "one hit is one page");

    // A word of the agent's answer in the second turn finds that turn.
    let page = caught_up(&world.history, "twelve", &world.session).await;
    let hit = the_hit(&page, &world.session);
    assert_eq!(hit.turn, 2);
    assert!(hit.snippet.to_lowercase().contains("twelve"), "{hit:?}");

    // Every word must match.
    let (none, _) = search(&world.history, "quokka twelve", "").await;
    assert!(none.is_empty(), "{none:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_search_pages_through_every_hit() {
    // More matching turns than a page holds.
    const TURNS: usize = 120;
    let lines: Vec<String> = (0..TURNS)
        .flat_map(|n| {
            [
                user(&format!("prompt {n} about the wombat"), 2 * n),
                says("ok", 2 * n + 1),
            ]
        })
        .collect();
    let world = seeded(&lines).await;

    // Mirrored at once, so once one turn is found all are.
    let (mut hits, mut token) = caught_up(&world.history, "wombat", &world.session).await;
    assert!(!token.is_empty(), "{} hits fit on the first page", hits.len());
    let mut pages = 1;
    while !token.is_empty() {
        let (more, next) = search(&world.history, "wombat", &token).await;
        assert!(!more.is_empty(), "a page before the last is never empty");
        hits.extend(more);
        token = next;
        pages += 1;
        assert!(pages <= TURNS, "the pages never end");
    }

    assert!(hits.iter().all(|hit| hit.session == world.session));
    let turns: BTreeSet<u32> = hits.iter().map(|hit| hit.turn).collect();
    assert_eq!(turns.len(), hits.len(), "no turn is on two pages");
    let every: BTreeSet<u32> = (1..=u32::try_from(TURNS).unwrap()).collect();
    assert_eq!(turns, every);
}

#[tokio::test(flavor = "multi_thread")]
async fn read_returns_the_turns_in_order_with_who_wrote_them() {
    let world = seeded(&two_turns()).await;
    caught_up(&world.history, "quokka", &world.session).await;

    let turns = within("read", world.history.read(&world.session, 0, u32::MAX))
        .await
        .expect("read the conversation");
    let shape: Vec<(u32, TurnRole)> = turns.iter().map(|turn| (turn.turn, turn.role)).collect();
    assert_eq!(
        shape,
        [
            (1, TurnRole::User),
            (1, TurnRole::Agent),
            (2, TurnRole::User),
            (2, TurnRole::Tool),
            (2, TurnRole::Tool),
            (2, TurnRole::Agent),
        ]
    );
    assert!(
        turns.windows(2).all(|pair| pair[0].seq < pair[1].seq),
        "{turns:?}"
    );
    assert_eq!(turns[0].text, "Rename the quokka ledger column");
    assert_eq!(turns[1].text, "Renamed the column.");
    assert_eq!(turns[4].text, "12 passed");
    assert_eq!(turns[5].text, "All twelve pass.");

    // A range reads just those turns.
    let second = within("read", world.history.read(&world.session, 2, 2))
        .await
        .expect("read the second turn");
    assert_eq!(second, turns[2..].to_vec());
    let first = within("read", world.history.read(&world.session, 1, 1))
        .await
        .expect("read the first turn");
    assert_eq!(first, turns[..2].to_vec());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_query_is_refused() {
    let host = Host::start().await;
    let daemon = host.connect().await;
    let history = LiveHistory::new(&daemon);

    let result = within("search", history.search("", "")).await;
    assert!(
        matches!(result, Err(DaemonError::Refused { .. })),
        "expected a refusal, got {result:?}"
    );
}
