//! Boards through `LiveDaemon` against a real `humatad2` (admin build, so
//! calls with no credential are the person's): items, their columns,
//! dependencies, sessions, history and pushed changes (BD-1, BD-3, BD-14,
//! SE-3). Every case has its own daemon, folder and socket.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

use humata2_clients::model::{
    Agent, ChannelId, Item, ItemId, ItemStatus, MachineId, Scope, SessionId, SessionOptions,
};
use humata2_clients::{Daemon, DaemonError, Event, Workspace};
use humata2_clients_live_daemon::{LiveDaemon, LiveOptions};
use tempfile::TempDir;

const AGENT_NAME: &str = "fake";

/// The channel every case makes; its items' keys start with its first
/// three letters (D155).
const CHANNEL: &str = "payments";
const CHANNEL_PREFIX: &str = "PAY";
const GLOBAL_PREFIX: &str = "GLB";

/// How long anything may take before the case fails.
const DEADLINE: Duration = Duration::from_secs(60);

/// How often a file is looked at while waiting for it.
const POLL: Duration = Duration::from_millis(2);

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

/// Stands in for an agent: it keeps each typed line in its conversation
/// file and marks that it is up.
fn agent_script(root: &Path, conv: &Path) -> String {
    format!(
        r#"DIR='{root}'
CONV_DIR='{conv}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
  --fresh) f="$CONV_DIR/other.jsonl"; : > "$f" ;;
esac
echo "agent up $mode"
: > "$DIR/agent.up"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$f"
done
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
        // Holders and the agents they run outlive the daemon; each has the
        // case's folder in its command line.
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
            .prefix("h2b")
            .tempdir_in("/tmp")
            .expect("make a temp folder");
        // The daemon refuses a socket folder other users can open.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("restrict the temp folder");
        let root = dir.path();
        let conv = root.join("conv");
        std::fs::create_dir_all(&conv).expect("make conversations folder");
        std::fs::create_dir_all(root.join("remotes")).expect("make remotes folder");
        let socket = root.join("d.sock");
        let script = root.join("agent.sh");
        std::fs::write(&script, agent_script(root, &conv)).expect("write the agent");
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

    /// The channel [`CHANNEL`] around a repository of its own.
    async fn channel(&self) -> (ChannelId, String, MachineId) {
        let mac = self
            .load()
            .await
            .machines
            .first()
            .expect("the workspace has a machine")
            .id
            .clone();
        let repo = self.repo(CHANNEL);
        let mut events = within("events", self.daemon.events())
            .await
            .expect("subscribe to daemon events");
        let id = within(
            "create_channel",
            self.daemon
                .create_channel(CHANNEL, vec![repo.clone()], vec![mac.clone()]),
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
        (id, repo, mac)
    }

    /// A session in `channel`, made with `options` (the repo is added).
    async fn session(
        &self,
        channel: &ChannelId,
        repo: &str,
        mac: &MachineId,
        options: SessionOptions,
    ) -> SessionId {
        within(
            "create_session",
            self.daemon.create_session(
                channel,
                Agent::Claude,
                mac,
                "Fix the ledger",
                SessionOptions {
                    repos: vec![repo.to_owned()],
                    ..options
                },
            ),
        )
        .await
        .expect("create the session")
    }

    async fn create_item(&self, scope: Scope, title: &str) -> ItemId {
        within("create_item", self.daemon.create_item(scope, title))
            .await
            .unwrap_or_else(|error| panic!("make the item {title:?}: {error}"))
    }

    /// A move that checks nothing, as someone who hasn't read the item first
    /// (an agent, another window) makes it.
    async fn move_item(&self, item: &ItemId, status: ItemStatus) {
        within("move_item", self.daemon.move_item(item, status, None))
            .await
            .unwrap_or_else(|error| panic!("move {item} to {status:?}: {error}"));
    }
}

/// `id` as a fresh load lists it.
fn listed<'w>(workspace: &'w Workspace, id: &ItemId) -> &'w Item {
    workspace
        .items
        .iter()
        .find(|item| &item.id == id)
        .unwrap_or_else(|| {
            panic!(
                "the load lists no item {id}; it lists {:?}",
                workspace.items.iter().map(|item| &item.id).collect::<Vec<_>>()
            )
        })
}

/// The sessions working on `id` in what was loaded: the card's one and
/// the drawer's list.
fn working(workspace: &Workspace, id: &ItemId) -> (Option<SessionId>, Vec<SessionId>) {
    let card = listed(workspace, id).session.clone();
    let drawer = workspace
        .item_details
        .get(id)
        .map(|details| details.sessions.clone())
        .unwrap_or_default();
    (card, drawer)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_channel_item_gets_the_channels_key_and_is_listed_in_to_do() {
    let world = World::start().await;
    let (channel, _, _) = world.channel().await;

    let id = world
        .create_item(Scope::Channel(channel.clone()), "Retry failed charges")
        .await;

    assert!(
        id.as_str().starts_with(CHANNEL_PREFIX),
        "a {CHANNEL} item's key starts with {CHANNEL_PREFIX}: {id}"
    );
    let workspace = world.load().await;
    let item = listed(&workspace, &id);
    assert_eq!(item.scope, Scope::Channel(channel));
    assert_eq!(item.status, ItemStatus::Todo);
    assert_eq!(item.title, "Retry failed charges");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_global_item_gets_the_global_key_and_scope() {
    let world = World::start().await;

    let id = world.create_item(Scope::Global, "Plan the quarter").await;

    assert!(
        id.as_str().starts_with(GLOBAL_PREFIX),
        "a Global item's key starts with {GLOBAL_PREFIX}: {id}"
    );
    let workspace = world.load().await;
    assert_eq!(listed(&workspace, &id).scope, Scope::Global);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_moved_item_is_in_its_new_column_on_the_next_load() {
    let world = World::start().await;
    let (channel, _, _) = world.channel().await;
    let id = world
        .create_item(Scope::Channel(channel), "Retry failed charges")
        .await;

    world.move_item(&id, ItemStatus::Done).await;

    assert_eq!(listed(&world.load().await, &id).status, ItemStatus::Done);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_from_an_old_copy_is_refused_as_a_conflict_until_the_item_is_read_again() {
    let world = World::start().await;
    let (channel, _, _) = world.channel().await;
    let id = world
        .create_item(Scope::Channel(channel), "Retry failed charges")
        .await;
    let old = listed(&world.load().await, &id).clone();

    // Someone else moves it after this copy was read.
    world.move_item(&id, ItemStatus::InProgress).await;

    let refused = within(
        "move_item",
        world.daemon.move_item(&id, ItemStatus::Done, Some(old.revision)),
    )
    .await;
    assert!(
        matches!(refused, Err(DaemonError::Conflict { .. })),
        "a move from the old copy is a conflict, got {refused:?}"
    );
    let current = listed(&world.load().await, &id).clone();
    assert_eq!(current.status, ItemStatus::InProgress, "the other move stands");
    assert!(current.revision > old.revision, "the item's revision went up");

    within(
        "move_item",
        world.daemon.move_item(&id, ItemStatus::Done, Some(current.revision)),
    )
    .await
    .expect("a move from the current copy lands");
    assert_eq!(listed(&world.load().await, &id).status, ItemStatus::Done);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dependency_is_listed_refuses_a_loop_and_can_be_removed() {
    let world = World::start().await;
    let (channel, _, _) = world.channel().await;
    let a = world
        .create_item(Scope::Channel(channel.clone()), "Ship the ledger")
        .await;
    let b = world
        .create_item(Scope::Channel(channel), "Write the migration")
        .await;

    within("add_dependency", world.daemon.add_dependency(&a, &b))
        .await
        .expect("make a wait for b");
    assert_eq!(listed(&world.load().await, &a).depends_on, vec![b.clone()]);

    let looped = within("add_dependency", world.daemon.add_dependency(&b, &a)).await;
    assert!(
        matches!(looped, Err(DaemonError::Refused { .. })),
        "b waiting for a would make a loop, so it is refused: {looped:?}"
    );
    let workspace = world.load().await;
    assert!(listed(&workspace, &b).depends_on.is_empty());
    assert_eq!(listed(&workspace, &a).depends_on, vec![b.clone()]);

    within("remove_dependency", world.daemon.remove_dependency(&a, &b))
        .await
        .expect("stop a waiting for b");
    assert!(listed(&world.load().await, &a).depends_on.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_set_on_an_item_is_listed_until_it_is_taken_off() {
    let world = World::start().await;
    let (channel, repo, mac) = world.channel().await;
    let session = world
        .session(&channel, &repo, &mac, SessionOptions::default())
        .await;
    let id = world
        .create_item(Scope::Channel(channel), "Retry failed charges")
        .await;

    within(
        "set_item_session",
        world.daemon.set_item_session(&id, &session, true),
    )
    .await
    .expect("put the session on the item");
    let (card, drawer) = working(&world.load().await, &id);
    assert_eq!(card, Some(session.clone()));
    assert_eq!(drawer, vec![session.clone()]);

    within(
        "set_item_session",
        world.daemon.set_item_session(&id, &session, false),
    )
    .await
    .expect("take the session off the item");
    let (card, drawer) = working(&world.load().await, &id);
    assert_eq!(card, None);
    assert!(drawer.is_empty(), "{drawer:?}");
}

/// SE-3: a session started for a To do item works on it and moves it to
/// In progress.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_started_for_a_to_do_item_works_on_it_and_moves_it_in_progress() {
    let world = World::start().await;
    let (channel, repo, mac) = world.channel().await;
    let id = world
        .create_item(Scope::Channel(channel.clone()), "Retry failed charges")
        .await;

    let session = world
        .session(
            &channel,
            &repo,
            &mac,
            SessionOptions {
                item: Some(id.clone()),
                ..SessionOptions::default()
            },
        )
        .await;

    let workspace = world.load().await;
    let (card, drawer) = working(&workspace, &id);
    assert_eq!(card, Some(session.clone()));
    assert!(drawer.contains(&session), "{drawer:?}");
    assert_eq!(listed(&workspace, &id).status, ItemStatus::InProgress);
}

/// SE-3: only a To do item moves; one In review keeps its column.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_started_for_an_in_review_item_leaves_its_column() {
    let world = World::start().await;
    let (channel, repo, mac) = world.channel().await;
    let id = world
        .create_item(Scope::Channel(channel.clone()), "Retry failed charges")
        .await;
    world.move_item(&id, ItemStatus::InReview).await;

    let session = world
        .session(
            &channel,
            &repo,
            &mac,
            SessionOptions {
                item: Some(id.clone()),
                ..SessionOptions::default()
            },
        )
        .await;

    let workspace = world.load().await;
    let (_, drawer) = working(&workspace, &id);
    assert!(drawer.contains(&session), "{drawer:?}");
    assert_eq!(listed(&workspace, &id).status, ItemStatus::InReview);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_items_history_has_its_move_and_an_unknown_item_is_not_found() {
    let world = World::start().await;
    let (channel, _, _) = world.channel().await;
    let id = world
        .create_item(Scope::Channel(channel), "Retry failed charges")
        .await;
    world.move_item(&id, ItemStatus::InProgress).await;

    let history = within("item_history", world.daemon.item_history(&id))
        .await
        .expect("read the item's history");
    assert!(!history.is_empty(), "a moved item has history");

    let unknown = within(
        "item_history",
        world.daemon.item_history(&ItemId::new("NOPE-404")),
    )
    .await;
    assert!(
        matches!(unknown, Err(DaemonError::NotFound { .. })),
        "an unknown item's history is not found: {unknown:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_after_subscribing_is_pushed_as_an_item_change() {
    let world = World::start().await;
    let (channel, _, _) = world.channel().await;
    let id = world
        .create_item(Scope::Channel(channel), "Retry failed charges")
        .await;
    let mut events = within("events", world.daemon.events())
        .await
        .expect("follow the daemon's changes");

    world.move_item(&id, ItemStatus::Done).await;

    let until = Instant::now() + DEADLINE;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, events.recv()).await {
            Ok(Some(Ok(Event::Item { item, .. })))
                if item.id == id && item.status == ItemStatus::Done =>
            {
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("the daemon's changes ended before the move was pushed"),
            Err(_) => panic!("the move to Done was never pushed"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_subscription_starts_with_a_resync() {
    let world = World::start().await;
    let mut events = within("events", world.daemon.events())
        .await
        .expect("follow the daemon's changes");
    let first = within("first event", events.recv()).await;
    assert!(matches!(first, Some(Ok(Event::Resync))), "{first:?}");
}

// D216: an item made on a session's board comes back on that session's board,
// with its channel, so the app can place it under BD-5–BD-8.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_board_item_is_listed_on_that_sessions_board() {
    let world = World::start().await;
    let (channel, repo, mac) = world.channel().await;
    let session = world.session(&channel, &repo, &mac, SessionOptions::default()).await;
    let scope = Scope::Session {
        channel: channel.clone(),
        session: session.clone(),
    };

    let id = world.create_item(scope.clone(), "Check the ledger totals").await;

    let workspace = world.load().await;
    let item = listed(&workspace, &id);
    assert_eq!(item.scope, scope);
    assert_eq!(item.scope.channel(), Some(&channel));
}
