//! The Inbox and a channel's access rules through `LiveDaemon` against a
//! real `humatad2` (IN-9, IN-12 to IN-19). The agent is a script in the
//! session's own tab that runs the real `humata` there, so the daemon takes
//! its calls for that session's agent; the app's side goes only through
//! [`Daemon`]. Every case has its own daemon, folder and socket.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

use humata2_clients::model::{
    AccessRule, Agent, Answer, Capability, ChannelId, MachineId, Request, RuleSetting, SessionId,
    SessionOptions,
};
use humata2_clients::{Daemon, DaemonError, Event, Events, Workspace};
use humata2_clients_live_daemon::{LiveDaemon, LiveOptions};
use tempfile::TempDir;

const AGENT_NAME: &str = "fake";

/// How long anything may take before the case fails.
const DEADLINE: Duration = Duration::from_secs(60);

/// How often a file or a fresh load is looked at while waiting for it.
const POLL: Duration = Duration::from_millis(50);

/// The line that makes the agent list the channels: a read the
/// `global_read` rule governs.
const LIST_CHANNELS: &str = "list-channels";

/// The line that makes the agent ask ahead for one network connection.
const ASK_AHEAD: &str = "ask-ahead";

/// `humata`'s exit code for a denial (CLI.md §7).
const EXIT_DENIED: i32 = 4;

/// `humata`'s exit code for a call that stopped waiting with its request
/// still unanswered (CLI.md §7).
const EXIT_PENDING: i32 = 12;

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

/// Stands in for an agent. A line that names a call runs the real `humata`
/// from the agent's own terminal, where the daemon put the session's token,
/// and waits there for the user's answer as a tool call would. What the
/// call printed and its exit code are left in `<call>.out` and
/// `<call>.code`; `<call>.done` appears last, once both are written. The
/// daemon gives the tab no socket to find, so the script names this
/// daemon's.
fn agent_script(root: &Path, conv: &Path, socket: &Path) -> String {
    format!(
        r#"DIR='{root}'
CONV_DIR='{conv}'
HUMATA_SOCKET='{socket}'; export HUMATA_SOCKET
HOME="$DIR"; export HOME
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
  --fresh) f="$CONV_DIR/other.jsonl"; : > "$f" ;;
esac
call() {{
  name=$1; shift
  "$DIR/humata" --json --wait 120 "$@" > "$DIR/$name.out" 2> "$DIR/$name.err" < /dev/null
  echo $? > "$DIR/$name.code"
  : > "$DIR/$name.done"
}}
echo "agent up $mode"
: > "$DIR/agent.up"
while IFS= read -r line; do
  case "$line" in
    {LIST_CHANNELS}) call {LIST_CHANNELS} channels ;;
    {ASK_AHEAD}) call {ASK_AHEAD} access ask-ahead network_reach network:localhost:5432 --reason 'the migration needs the database' ;;
    *) echo "got: $line" ;;
  esac
  printf '%s\n' "$line" >> "$f"
done
"#,
        root = root.display(),
        conv = conv.display(),
        socket = socket.display(),
    )
}

async fn within<T>(what: &str, work: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(DEADLINE, work).await {
        Ok(value) => value,
        Err(_) => panic!("timed out waiting for {what}"),
    }
}

/// How one of the agent's `humata` calls ended.
#[derive(Debug)]
struct Ended {
    code: i32,
    /// The one JSON object it printed.
    printed: String,
}

/// A real `humatad2` in a folder of its own. Dropping it stops the daemon
/// and everything the case started under it.
struct Served {
    dir: TempDir,
    child: Child,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Holders outlive the daemon on purpose, and so do the agents they
        // run and the `humata` an agent is waiting in. Everything this case
        // started has its folder in its command line, so stop those by it.
        let _ = Command::new("pkill")
            .arg("-f")
            .arg(self.dir.path())
            .status();
    }
}

/// The daemon and the app's client to it.
struct World {
    daemon: Arc<dyn Daemon>,
    served: Served,
}

impl World {
    async fn start() -> Self {
        use std::os::unix::fs::PermissionsExt;
        // Short, so the socket's path fits.
        let dir = tempfile::Builder::new()
            .prefix("h2i")
            .tempdir_in("/tmp")
            .expect("make a temp folder");
        // The daemon refuses a socket folder other users can open.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("restrict the temp folder");
        let root = dir.path();
        let conv = root.join("conv");
        std::fs::create_dir_all(&conv).expect("make conversations folder");
        std::fs::create_dir_all(root.join("remotes")).expect("make remotes folder");
        // `humata` under the case's folder, so its command line names it.
        std::os::unix::fs::symlink(binary("HUMATA"), root.join("humata"))
            .expect("link humata into the temp folder");
        let socket = root.join("d.sock");
        let script = root.join("agent.sh");
        std::fs::write(&script, agent_script(root, &conv, &socket)).expect("write the agent");
        let agents = root.join("agents.json");
        let spec = serde_json::json!([{
            "name": AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", conv.display()),
        }]);
        std::fs::write(&agents, spec.to_string()).expect("write agents.json");
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
        // From here a failure stops the daemon.
        let served = Served { dir, child };
        let until = Instant::now() + DEADLINE;
        while tokio::net::UnixStream::connect(&socket).await.is_err() {
            assert!(Instant::now() < until, "humatad2 never started listening");
            tokio::time::sleep(POLL).await;
        }
        let live = within(
            "the connection to humatad2",
            LiveDaemon::connect(LiveOptions::new(socket).with_agent(Agent::Claude, AGENT_NAME)),
        )
        .await
        .expect("connect to humatad2");
        Self {
            daemon: Arc::new(live),
            served,
        }
    }

    fn root(&self) -> &Path {
        self.served.dir.path()
    }

    /// Everything the app would read on opening, read again from the daemon.
    async fn load(&self) -> Workspace {
        within("load", self.daemon.load())
            .await
            .expect("load the workspace")
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

    async fn this_mac(&self) -> MachineId {
        self.load()
            .await
            .machines
            .first()
            .expect("the workspace has a machine")
            .id
            .clone()
    }

    /// A channel `name` around a repository of its own.
    async fn channel(&self, name: &str) -> (ChannelId, String, MachineId) {
        let mac = self.this_mac().await;
        let repo = self.repo(name);
        let id = within(
            "create_channel",
            self.daemon
                .create_channel(name, vec![repo.clone()], vec![mac.clone()]),
        )
        .await
        .expect("create the channel");
        (id, repo, mac)
    }

    /// A channel with one session whose agent is up and reading its
    /// terminal.
    async fn session(&self) -> (ChannelId, SessionId) {
        let (channel, repo, mac) = self.channel("payments").await;
        let session = within(
            "create_session",
            self.daemon.create_session(
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
        self.file("agent.up", "the session's agent to start").await;
        (channel, session)
    }

    /// Waits for `name` to appear in the case's folder.
    async fn file(&self, name: &str, what: &str) -> PathBuf {
        let path = self.root().join(name);
        let until = Instant::now() + DEADLINE;
        while !path.exists() {
            assert!(Instant::now() < until, "timed out waiting for {what}");
            tokio::time::sleep(POLL).await;
        }
        path
    }

    /// Has the session's agent make `call` (one of the lines its script
    /// knows) from its own terminal.
    async fn agent_calls(&self, session: &SessionId, call: &str) {
        within("send", self.daemon.send(session, call))
            .await
            .expect("type the line into the agent");
    }

    /// How the agent's `call` ended, if it has.
    fn ended_by_now(&self, call: &str) -> Option<Ended> {
        let root = self.root();
        if !root.join(format!("{call}.done")).exists() {
            return None;
        }
        let code = std::fs::read_to_string(root.join(format!("{call}.code")))
            .expect("read the call's exit code")
            .trim()
            .parse()
            .expect("the exit code is a number");
        let printed = std::fs::read_to_string(root.join(format!("{call}.out")))
            .expect("read what the call printed");
        Some(Ended { code, printed })
    }

    /// Waits for the agent's `call` to end.
    async fn ended(&self, call: &str) -> Ended {
        self.file(&format!("{call}.done"), "the agent's call to end")
            .await;
        self.ended_by_now(call).expect("the call ended")
    }

    /// The session's request that waits for an answer, as fresh loads list
    /// it, once the agent's `call` has made it.
    async fn waiting_request(&self, session: &SessionId, call: &str) -> Request {
        let until = Instant::now() + DEADLINE;
        loop {
            let waiting = self
                .load()
                .await
                .requests
                .into_iter()
                .find(|request| &request.session == session && request.answer.is_none());
            if let Some(request) = waiting {
                return request;
            }
            // A call that ended as "pending" left its request behind, which
            // a later load lists; any other end means nothing was asked.
            let ended = self
                .ended_by_now(call)
                .filter(|ended| ended.code != EXIT_PENDING);
            if let Some(ended) = ended {
                panic!("the agent's call ended without asking the user: {ended:?}");
            }
            assert!(
                Instant::now() < until,
                "the agent's waiting call never showed as a request in a fresh load"
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// Sets a rule of the channel from the app.
    async fn set_rule(&self, channel: &ChannelId, capability: &str, setting: RuleSetting) {
        within(
            "set_rule",
            self.daemon
                .set_rule(channel, &Capability::new(capability), setting),
        )
        .await
        .unwrap_or_else(|error| panic!("set the {capability} rule to {setting:?}: {error}"));
    }

    /// Answers a request from the app.
    async fn answer(&self, request: &Request, answer: Answer) {
        within("answer", self.daemon.answer(&request.id, answer.clone()))
            .await
            .unwrap_or_else(|error| panic!("answer the request with {answer:?}: {error}"));
    }
}

/// The channel's rule row for `capability` in what was loaded.
fn rule(workspace: &Workspace, channel: &ChannelId, capability: &str) -> AccessRule {
    workspace
        .rules
        .get(channel)
        .unwrap_or_else(|| panic!("the load lists no access rules for channel {channel}"))
        .iter()
        .find(|rule| rule.capability.as_str() == capability)
        .unwrap_or_else(|| panic!("the channel's rules have no {capability} row"))
        .clone()
}

/// Whether the request still waits for an answer in what was loaded.
fn waits(workspace: &Workspace, request: &Request) -> bool {
    workspace
        .requests
        .iter()
        .any(|listed| listed.id == request.id && listed.answer.is_none())
}

/// The next request pushed for `session`.
async fn pushed_request(events: &mut Events, session: &SessionId) -> Request {
    let until = Instant::now() + DEADLINE;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, events.recv()).await {
            Ok(Some(Ok(Event::Request(request)))) if &request.session == session => {
                return request;
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("the daemon's changes ended before the request was pushed"),
            Err(_) => panic!("no request was pushed for the agent's waiting call"),
        }
    }
}

/// Under the default rule the agent's read isn't held, so nothing here
/// goes through the Inbox: it shows that the agent in these cases reaches
/// the daemon as its session and that its calls' results are read back.
#[tokio::test(flavor = "multi_thread")]
async fn an_agents_call_under_an_allow_rule_runs_without_asking() {
    let world = World::start().await;
    let (channel, session) = world.session().await;

    world.agent_calls(&session, LIST_CHANNELS).await;
    let ended = world.ended(LIST_CHANNELS).await;

    assert_eq!(ended.code, 0, "{ended:?}");
    assert!(ended.printed.contains(channel.as_str()), "{ended:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rule_set_from_the_app_is_listed_for_its_channel_only() {
    let world = World::start().await;
    let (channel, _, _) = world.channel("payments").await;
    let (other, _, _) = world.channel("billing").await;

    world.set_rule(&channel, "global_read", RuleSetting::Ask).await;

    let workspace = world.load().await;
    assert_eq!(
        rule(&workspace, &channel, "global_read").setting,
        Some(RuleSetting::Ask)
    );
    assert_eq!(
        rule(&workspace, &other, "global_read").setting,
        Some(RuleSetting::Allow)
    );
}

/// `own_clones` is the IN-16 row, which has no setting; changing access is
/// the user's alone (IN-23), so no rule governs it.
#[tokio::test(flavor = "multi_thread")]
async fn a_rule_with_no_setting_is_refused_and_nothing_changes() {
    let world = World::start().await;
    let (channel, _, _) = world.channel("payments").await;
    // A change that is taken, so the refusals below are the daemon's and
    // the rules to compare with are ones the app has read.
    world.set_rule(&channel, "global_read", RuleSetting::Ask).await;
    let before = world.load().await.rules.get(&channel).cloned();

    for capability in ["own_clones", "access_change"] {
        let refused = within(
            "set_rule",
            world
                .daemon
                .set_rule(&channel, &Capability::new(capability), RuleSetting::Allow),
        )
        .await;
        assert!(
            matches!(
                refused,
                Err(DaemonError::Refused { .. } | DaemonError::NotFound { .. })
            ),
            "setting {capability} should be refused, got {refused:?}"
        );
    }

    let after = world.load().await.rules.get(&channel).cloned();
    assert!(before.is_some(), "the load lists no access rules for the channel");
    assert_eq!(after, before);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_call_under_an_ask_rule_shows_as_a_waiting_request() {
    let world = World::start().await;
    let (channel, session) = world.session().await;
    world.set_rule(&channel, "global_read", RuleSetting::Ask).await;
    let mut events = within("events", world.daemon.events())
        .await
        .expect("follow the daemon's changes");

    world.agent_calls(&session, LIST_CHANNELS).await;
    let pushed = pushed_request(&mut events, &session).await;
    let listed = world.waiting_request(&session, LIST_CHANNELS).await;

    assert_eq!(pushed.answer, None);
    for request in [&pushed, &listed] {
        assert_eq!(request.id, pushed.id);
        assert_eq!(request.session, session);
        assert_eq!(request.channel, channel);
        assert!(!request.title.is_empty(), "the card has no action: {request:?}");
        let asked_by = request
            .asked_by
            .as_ref()
            .unwrap_or_else(|| panic!("the card doesn't name the rule that asked: {request:?}"));
        assert_eq!(asked_by.capability.as_str(), "global_read");
        assert_eq!(asked_by.setting, RuleSetting::Ask);
    }
    assert!(
        world.ended_by_now(LIST_CHANNELS).is_none(),
        "the agent's call ended before it was answered"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn allow_ends_the_agents_wait_with_the_result() {
    let world = World::start().await;
    let (channel, session) = world.session().await;
    world.set_rule(&channel, "global_read", RuleSetting::Ask).await;
    world.agent_calls(&session, LIST_CHANNELS).await;
    let request = world.waiting_request(&session, LIST_CHANNELS).await;

    world.answer(&request, Answer::Approved).await;
    let ended = world.ended(LIST_CHANNELS).await;

    assert_eq!(ended.code, 0, "{ended:?}");
    assert!(ended.printed.contains(channel.as_str()), "{ended:?}");
    assert!(!waits(&world.load().await, &request));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_card_names_the_operation_the_agent_called() {
    let world = World::start().await;
    let (channel, session) = world.session().await;
    world.set_rule(&channel, "global_read", RuleSetting::Ask).await;
    world.agent_calls(&session, LIST_CHANNELS).await;

    let request = world.waiting_request(&session, LIST_CHANNELS).await;

    // TP-16: `humata channels` is ListChannels, `channel.list` on the card.
    assert_eq!(request.operation, "channel.list", "{request:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn deny_with_a_reply_ends_the_agents_wait_as_denied() {
    let world = World::start().await;
    let (channel, session) = world.session().await;
    world.set_rule(&channel, "global_read", RuleSetting::Ask).await;
    world.agent_calls(&session, LIST_CHANNELS).await;
    let request = world.waiting_request(&session, LIST_CHANNELS).await;

    world
        .answer(
            &request,
            Answer::Denied {
                reply: Some("stay-in-payments".to_owned()),
            },
        )
        .await;
    let ended = world.ended(LIST_CHANNELS).await;

    assert_eq!(ended.code, EXIT_DENIED, "{ended:?}");
    // The agent gets the user's note with the denial (IN-13).
    assert!(ended.printed.contains("stay-in-payments"), "{ended:?}");
    assert!(!waits(&world.load().await, &request));
}

#[tokio::test(flavor = "multi_thread")]
async fn always_allow_makes_a_grant_the_app_lists_and_can_remove() {
    let world = World::start().await;
    let (channel, session) = world.session().await;
    // Ask is this rule's default (IN-17); set here so the case doesn't
    // lean on it.
    world.set_rule(&channel, "network_reach", RuleSetting::Ask).await;
    world.agent_calls(&session, ASK_AHEAD).await;
    let request = world.waiting_request(&session, ASK_AHEAD).await;
    assert!(
        !request.rule.is_empty(),
        "the card offers no “Always allow”: {request:?}"
    );

    world
        .answer(
            &request,
            Answer::Always {
                rule: request.rule.clone(),
            },
        )
        .await;
    // The agent's ask-ahead waited (`--wait`) and ends approved.
    let ended = world.ended(ASK_AHEAD).await;
    assert_eq!(ended.code, 0, "{ended:?}");
    let granted = rule(&world.load().await, &channel, "network_reach").grants;
    assert_eq!(granted.len(), 1, "{granted:?}");
    assert_eq!(granted[0].session, session);

    within("remove_grant", world.daemon.remove_grant(&granted[0].id))
        .await
        .expect("remove the grant");

    let left = rule(&world.load().await, &channel, "network_reach").grants;
    assert!(left.is_empty(), "{left:?}");
}
