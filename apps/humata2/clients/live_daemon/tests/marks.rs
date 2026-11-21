//! Pins and VIP marks through `LiveMarks` against a real `humatad2` (admin
//! build, so every call is the user's) (NV-20, NV-21, NV-24, NV-25; D170).
//! The host keeps the marks and their order: two clients on two
//! connections to one daemon follow each other, and a restart keeps them.
//! Every case has its own daemon, folder and socket.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use humata2_clients::marks::{MarksClient, PinnedList, PinnedLists, Thing};
use humata2_clients::model::{Agent, ChannelId, SessionId, SessionOptions};
use humata2_clients::{Daemon, DaemonError};
use humata2_clients_live_daemon::marks::LiveMarks;
use humata2_clients_live_daemon::{LiveDaemon, LiveOptions};
use tempfile::TempDir;

const AGENT_NAME: &str = "fake";

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

/// The marks client of a connection.
fn marks(daemon: &LiveDaemon) -> LiveMarks {
    LiveMarks::new(daemon)
}

fn item(id: &str) -> Thing {
    Thing::BoardItem(id.to_owned())
}

fn channel(id: &str) -> Thing {
    Thing::Channel(ChannelId::new(id))
}

async fn pinned(marks: &LiveMarks) -> PinnedList {
    within("pinned", marks.pinned()).await.expect("read the pins")
}

async fn set(marks: &LiveMarks, thing: &Thing, pinned: Option<bool>, vip: Option<bool>) -> PinnedList {
    within("set", marks.set(thing, pinned, vip))
        .await
        .expect("set the marks")
}

async fn pin(marks: &LiveMarks, thing: &Thing) -> PinnedList {
    set(marks, thing, Some(true), None).await
}

async fn watch(marks: &LiveMarks) -> PinnedLists {
    within("watch", marks.watch()).await.expect("watch the pins")
}

fn things(list: &PinnedList) -> Vec<Thing> {
    list.items.iter().map(|item| item.thing.clone()).collect()
}

/// The pinned things in order, each with its VIP flag.
fn flags(list: &PinnedList) -> Vec<(Thing, bool)> {
    list.items
        .iter()
        .map(|item| (item.thing.clone(), item.vip))
        .collect()
}

/// The first list `changes` yields that `wanted` accepts. Lists before it
/// (another change landing first) are passed over.
async fn told(
    changes: &mut PinnedLists,
    what: &str,
    wanted: impl Fn(&PinnedList) -> bool,
) -> PinnedList {
    within(what, async {
        loop {
            match changes.recv().await {
                Some(list) if wanted(&list) => return list,
                Some(_) => {}
                None => panic!("the watch ended while waiting for {what}"),
            }
        }
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_session_shows_with_its_title() {
    let host = Host::start().await;
    let daemon = host.connect().await;
    let session = Thing::Session(host.session(&daemon, "Fix the ledger").await);
    let marks = marks(&daemon);
    assert!(pinned(&marks).await.items.is_empty());

    let list = pin(&marks, &session).await;
    assert_eq!(things(&list), [session.clone()]);
    assert_eq!(list.items[0].title, "Fix the ledger");
    assert!(!list.items[0].vip);

    let read = pinned(&marks).await;
    assert_eq!(things(&read), [session]);
    assert_eq!(read.items[0].title, "Fix the ledger");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_pin_goes_last() {
    let host = Host::start().await;
    let marks = marks(&host.connect().await);
    let [a, b, c, d] = [item("item-1"), channel("ch_billing"), item("item-2"), item("item-3")];
    for thing in [&a, &b, &c] {
        pin(&marks, thing).await;
    }
    assert_eq!(things(&pinned(&marks).await), [a.clone(), b.clone(), c.clone()]);

    assert_eq!(
        things(&pin(&marks, &d).await),
        [a.clone(), b.clone(), c.clone(), d.clone()]
    );

    // Unpinned, it leaves the rest in order; pinned again, it is new.
    assert_eq!(
        things(&set(&marks, &b, Some(false), None).await),
        [a.clone(), c.clone(), d.clone()]
    );
    assert_eq!(things(&pin(&marks, &b).await), [a, c, d, b]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reorder_from_the_current_revision_takes_that_order() {
    let host = Host::start().await;
    let marks = marks(&host.connect().await);
    let [a, b, c] = [item("item-1"), item("item-2"), channel("ch_billing")];
    for thing in [&a, &b, &c] {
        pin(&marks, thing).await;
    }
    let before = pinned(&marks).await;

    let order = [c, a, b];
    let list = within("reorder", marks.reorder(&order, before.revision))
        .await
        .expect("reorder the pins");
    assert_eq!(things(&list), order);
    assert!(list.revision > before.revision, "{list:?} after {before:?}");
    assert_eq!(things(&pinned(&marks).await), order);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reorder_from_a_stale_revision_is_refused_and_changes_nothing() {
    let host = Host::start().await;
    let app = marks(&host.connect().await);
    let cli = marks(&host.connect().await);
    let [a, b, c] = [item("item-1"), item("item-2"), item("item-3")];
    for thing in [&a, &b, &c] {
        pin(&app, thing).await;
    }
    let seen = pinned(&app).await;
    // Another client changes a mark after this one read the list.
    let host_list = set(&cli, &a, None, Some(true)).await;

    let result = within("reorder", app.reorder(&[c, b, a], seen.revision)).await;
    // A conflict the app can act on by reading the pins again, not the host
    // being offline or the thing being gone.
    assert!(
        matches!(result, Err(DaemonError::Conflict { .. })),
        "expected a refusal, got {result:?}"
    );
    let now = pinned(&app).await;
    assert_eq!(flags(&now), flags(&host_list));
    assert_eq!(now.revision, host_list.revision);
}

#[tokio::test(flavor = "multi_thread")]
async fn vip_is_set_apart_from_the_pin_and_reads_back() {
    let host = Host::start().await;
    let marks = marks(&host.connect().await);
    let [a, b] = [item("item-1"), item("item-2")];
    pin(&marks, &a).await;
    pin(&marks, &b).await;

    let list = set(&marks, &b, None, Some(true)).await;
    assert_eq!(flags(&list), [(a.clone(), false), (b.clone(), true)]);
    assert_eq!(flags(&pinned(&marks).await), flags(&list));

    let list = set(&marks, &b, None, Some(false)).await;
    assert_eq!(flags(&list), [(a.clone(), false), (b.clone(), false)]);

    // VIP before it is pinned: not listed, but kept for when it is.
    let c = Thing::PrReview(SessionId::new("se_review"));
    assert_eq!(things(&set(&marks, &c, None, Some(true)).await), [a.clone(), b.clone()]);
    assert_eq!(
        flags(&pin(&marks, &c).await),
        [(a, false), (b, false), (c, true)]
    );
}

/// The exit criterion: pins and VIP marks live on the host and show in two
/// clients at once.
#[tokio::test(flavor = "multi_thread")]
async fn pins_and_vip_marks_show_in_two_clients_at_once() {
    let host = Host::start().await;
    let app = marks(&host.connect().await);
    let cli = marks(&host.connect().await);
    let mut app_changes = watch(&app).await;
    let mut cli_changes = watch(&cli).await;
    let [a, b] = [item("item-1"), channel("ch_billing")];

    pin(&cli, &a).await;
    pin(&cli, &b).await;
    let shown = told(&mut app_changes, "the other client's pins", |list| {
        things(list) == [a.clone(), b.clone()]
    })
    .await;
    assert!(shown.items.iter().all(|item| !item.vip));

    set(&cli, &a, None, Some(true)).await;
    let shown = told(&mut app_changes, "the other client's VIP mark", |list| {
        flags(list) == [(a.clone(), true), (b.clone(), false)]
    })
    .await;

    // And the other way: the app's reorder reaches the other client.
    let order = [b.clone(), a.clone()];
    within("reorder", app.reorder(&order, shown.revision))
        .await
        .expect("reorder the pins");
    told(&mut cli_changes, "the app's new order", |list| things(list) == order).await;

    let from_app = pinned(&app).await;
    let from_cli = pinned(&cli).await;
    assert_eq!(flags(&from_app), [(b, false), (a, true)]);
    assert_eq!(flags(&from_cli), flags(&from_app));
    assert_eq!(from_cli.revision, from_app.revision);
}

#[tokio::test(flavor = "multi_thread")]
async fn pins_and_vip_marks_survive_a_restart() {
    let mut host = Host::start().await;
    let daemon = host.connect().await;
    let session = Thing::Session(host.session(&daemon, "Fix the ledger").await);
    let marks_before = marks(&daemon);
    let [a, vip_only] = [item("item-1"), item("item-2")];
    pin(&marks_before, &session).await;
    set(&marks_before, &a, Some(true), Some(true)).await;
    set(&marks_before, &vip_only, None, Some(true)).await;
    let seen = pinned(&marks_before).await;
    let order = [a.clone(), session.clone()];
    within("reorder", marks_before.reorder(&order, seen.revision))
        .await
        .expect("reorder the pins");
    let before = pinned(&marks_before).await;
    drop(marks_before);
    drop(daemon);

    host.restart().await;

    let after_marks = marks(&host.connect().await);
    let after = pinned(&after_marks).await;
    assert_eq!(flags(&after), [(a, true), (session.clone(), false)]);
    assert_eq!(after.items[1].title, "Fix the ledger");
    // The revision goes on from where it was, so a list read before the
    // restart is never taken for a newer one.
    assert!(after.revision >= before.revision, "{after:?} after {before:?}");

    // The VIP mark of the thing that wasn't pinned was kept too.
    let list = pin(&after_marks, &vip_only).await;
    assert_eq!(things(&list).last(), Some(&vip_only));
    assert!(list.items.last().is_some_and(|item| item.vip));
    assert!(list.revision > before.revision);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_pinned_and_marked_vip_through_the_daemon_client_shows_in_pinned() {
    let host = Host::start().await;
    let daemon = host.connect().await;
    let id = host.session(&daemon, "Fix the ledger").await;
    let session = Thing::Session(id.clone());
    let marks = marks(&daemon);

    within("set_pinned", daemon.set_pinned(&id, true))
        .await
        .expect("pin through the daemon client");
    within("set_vip", daemon.set_vip(&id, true))
        .await
        .expect("mark VIP through the daemon client");
    assert_eq!(flags(&pinned(&marks).await), [(session.clone(), true)]);

    within("unpin", daemon.set_pinned(&id, false))
        .await
        .expect("unpin through the daemon client");
    assert!(things(&pinned(&marks).await).is_empty());
}

/// D387: the workspace the client loads carries each session's pin and VIP
/// mark, and a restart of the host keeps them.
#[tokio::test(flavor = "multi_thread")]
async fn a_loaded_session_carries_its_pin_and_vip_mark() {
    let mut host = Host::start().await;
    let daemon = host.connect().await;
    let id = host.session(&daemon, "Fix the ledger").await;

    let workspace = within("load", daemon.load()).await.expect("load the workspace");
    let session = workspace.session(&id).expect("the session loads");
    assert!(!session.pinned && !session.vip);

    within("set_pinned", daemon.set_pinned(&id, true))
        .await
        .expect("pin");
    within("set_vip", daemon.set_vip(&id, true))
        .await
        .expect("mark VIP");
    let workspace = within("load", daemon.load()).await.expect("load the workspace");
    let session = workspace.session(&id).expect("the session loads");
    assert!(session.pinned && session.vip);

    host.restart().await;
    let daemon = host.connect().await;
    let workspace = within("load", daemon.load()).await.expect("load the workspace");
    let session = workspace.session(&id).expect("the session loads");
    assert!(session.pinned && session.vip, "the restart kept the marks");

    within("unpin", daemon.set_pinned(&id, false))
        .await
        .expect("unpin");
    within("clear VIP", daemon.set_vip(&id, false))
        .await
        .expect("clear VIP");
    let workspace = within("load", daemon.load()).await.expect("load the workspace");
    let session = workspace.session(&id).expect("the session loads");
    assert!(!session.pinned && !session.vip);
}
