//! U6, PER-5, PER-6, D373: holders across an upgrade. A daemon upgrade
//! never touches a running holder: the sessions started under release `a`
//! keep their holder processes when the daemon becomes release `b`, whose
//! `humata2_holder` is another file. One VIP "Migrate the holders" item
//! for this machine lists the agent sessions still on an old holder.
//! Migrate restarts each of them onto the current holder at its next idle
//! moment, with the same session and conversation, and the item ends
//! Approved once none waits. Shells are never restarted: they stay on their
//! holder until closed, and they never hold the item open.
//!
//! Release `b` ships `HUMATA2_HOLDER_FAULTS` as its holder: a build that
//! behaves as the plain holder with no fault switched on, and whose bytes,
//! and so whose sha256, differ. The daemon spawns holders from the
//! installed `bin/humata2_holder` (`--holder`), so after the swap new
//! holders are release `b`'s.
//!
//! The agent is the hooks agent of `tests/turns.rs`: a line
//! `run:<command>` runs a hook command, as Claude Code does, so a test
//! turns a session Idle (Stop) or Working (UserPromptSubmit) and waits on
//! the status it reports, never on a guessed time.
//!
//! What a holder was spawned from is kept in
//! `<state>/holder_builds/<session>.sha`. A test deletes one while the
//! daemon is down to stand for a session started before D373, which has
//! no record, and to force a second detection with something new to show.
//!
//! These tests are about holders too old to move onto a new build in place
//! (LIVEHOLDER): before the upgrade each record loses its `moves` line, so
//! every holder stands for one from before LIVEHOLDER, which Migrate
//! restarts. Moving holders is `tests/upgrade_holders_move.rs`.

mod common;
#[path = "common/upgrade.rs"]
mod upgrade;

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    AGENT_NAME, Attach, Build, Daemon, World, agent_tab, binary, create_channel, folder, get_session,
    holder_answers, is_running, machine_id, open, shell_token, start_session, stored_tabs, wait_running,
    wait_status,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::TabKind;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};

/// The most a test waits for the daemon to start itself again, or an
/// agent to come back on a new holder, on a loaded machine: a safety net,
/// never a time anything is expected to take.
const LONG: Duration = Duration::from_secs(90);

/// Release `a`'s files in `bin`, by the test binary each is a copy of.
const RELEASE_A: [(&str, &str); 4] = [
    ("humatad2", "HUMATAD2"),
    ("humata2_holder", "HUMATA2_HOLDER"),
    ("humata", "HUMATA"),
    ("stack", "STACK"),
];

/// Release `b`'s: the same daemon, CLI and stack, another holder file.
const RELEASE_B: [(&str, &str); 4] = [
    ("humatad2", "HUMATAD2"),
    ("humata2_holder", "HUMATA2_HOLDER_FAULTS"),
    ("humata", "HUMATA"),
    ("stack", "STACK"),
];

fn manifest_a() -> Vec<u8> {
    upgrade::manifest(&upgrade::admin_sha(), "a")
}

fn manifest_b() -> Vec<u8> {
    upgrade::manifest_with_holder(
        &upgrade::admin_sha(),
        &upgrade::sha256_of(&binary("HUMATA2_HOLDER_FAULTS")),
        "b",
    )
}

/// Copies the test binary `variable` names to `to`, as an install does.
fn put_file(variable: &str, to: &Path) {
    std::fs::copy(binary(variable), to)
        .unwrap_or_else(|error| panic!("couldn't copy {variable} to {}: {error}", to.display()));
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755)).expect("the file is made runnable");
}

/// The test agent: every line typed to it is answered and kept in its
/// conversation, and `run:<command>` runs the command, as Claude Code runs
/// a hook's.
fn agent_script(world: &World) -> String {
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
# Reads keys itself, as agent CLIs do, so typing never waits out the
# holder's keys limit (see common/mod.rs).
stty -icanon 2>/dev/null
echo "agent up $mode"
while IFS= read -r line; do
  case "$line" in
    run:*) eval "${{line#run:}}" ;;
    *) echo "got: $line" ;;
  esac
  printf '%s\n' "$line" >> "$f"
done
"#,
        world.conv.display(),
    )
}

/// A world whose agent is the one above, its turns told by `turn_end`
/// (`hooks`, `output` or `none`).
fn world_with_agent(dir: &Path, turn_end: &str) -> World {
    let world = World::new(dir);
    let script = dir.join("turn-agent.sh");
    std::fs::write(&script, agent_script(&world)).expect("the agent script is written");
    let agents = serde_json::json!([{
        "name": AGENT_NAME,
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}"],
        "resume_args": [script, "--resume", "{conversation}"],
        "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
        "turn_end": turn_end,
    }]);
    std::fs::write(&world.agents, agents.to_string()).expect("the agents file is written");
    world
}

/// This Mac's daemon at release `a`, installed in `bin` beside its state,
/// spawning holders from `bin/humata2_holder`, with a channel to start
/// agent sessions in.
struct Mac {
    daemon: Daemon,
    world: World,
    bin: PathBuf,
    channel: pb::Channel,
    machine: pb::MachineId,
}

impl Mac {
    async fn start() -> Self {
        Self::start_with_turns("hooks").await
    }

    /// As [`Self::start`], with an agent whose turns `turn_end` tells.
    async fn start_with_turns(turn_end: &str) -> Self {
        let dir = folder();
        let world = world_with_agent(dir.path(), turn_end);
        world.remote("acme", "api");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).expect("the bin folder is made");
        for (name, variable) in RELEASE_A {
            put_file(variable, &bin.join(name));
        }
        let from = upgrade::install(&bin, &manifest_a());
        let mut args = world.args();
        // The hooks run the installed CLI, which names the release beside
        // it, as a CLI must to a daemon that has one.
        args.extend(["--humata".to_owned(), bin.join("humata").display().to_string()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_holder(Build::Admin, dir, &refs, &bin.join("humata2_holder")).await;
        assert_eq!(upgrade::build_of(&daemon).await.bundle, from, "the daemon runs release a");
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{setup:?}");
        let machine = machine_id(&daemon).await;
        Self {
            daemon,
            world,
            bin,
            channel,
            machine,
        }
    }

    /// Starts an agent session and waits until its agent's program runs.
    async fn agent_session(&self, title: &str) -> pb::SessionId {
        let (session, operation) = start_session(&self.daemon, &self.channel, title).await;
        assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
        wait_running(&self.daemon, &session).await;
        self.agent(&session).await.wait_for("agent up").await;
        session
    }

    /// An attach to `session`'s agent tab, from its first byte.
    async fn agent(&self, session: &pb::SessionId) -> Attach {
        let tab = agent_tab(&self.daemon, session).await;
        Attach::start(&self.daemon, open(&tab, 0)).await
    }

    /// Where the daemon keeps which holder file `session`'s holder was
    /// spawned from (D373).
    fn record(&self, session: &pb::SessionId) -> PathBuf {
        self.daemon
            .state
            .join("holder_builds")
            .join(format!("{}.sha", session.value))
    }

    /// Stages release `b` and upgrades the daemon to it in place, as
    /// `tests/upgrade_local.rs` does; returns once the daemon answers as
    /// `b`.
    async fn upgrade_to_b(&self) -> String {
        too_old_to_move(&self.daemon.state);
        for (name, variable) in RELEASE_B {
            put_file(variable, &self.bin.join(format!(".{name}.next")));
        }
        let bundle = manifest_b();
        let staged = self.bin.join(".bundle.json.next");
        std::fs::write(&staged, &bundle).expect("the staged bundle is written");
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o644)).expect("the bundle is readable");
        let to = upgrade::bundle_id(&bundle);
        pb::daemon_service_client::DaemonServiceClient::new(self.daemon.channel().await)
            .upgrade(pb::UpgradeRequest {
                to_bundle: to.clone(),
                upgrade_id: "up_holders".to_owned(),
                thing: pb::MigrateThing::Unspecified.into(),
            })
            .await
            .expect("the user's upgrade is taken");
        wait_at_release(&self.daemon, &to).await;
        to
    }

    /// The command Claude Code runs for `event`, as the daemon wrote it.
    fn hook_command(&self, event: &str) -> String {
        let file = self.daemon.state.join("claude-hooks.json");
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(file).expect("the hooks file reads"))
                .expect("the hooks file is JSON");
        json["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("no hook for {event}"))
            .to_owned()
    }

    /// `session`'s agent runs `event`'s hook, and the test waits until the
    /// session shows `status`. The attach is returned, to keep typing.
    async fn turn(&self, session: &pb::SessionId, event: &str, status: pb::SessionStatus) -> Attach {
        let mut attach = self.agent(session).await;
        attach.type_text(&format!("run:{}\r", self.hook_command(event))).await;
        wait_status(&self.daemon, session, status).await;
        attach
    }

    /// The process id of the holder behind `session`'s agent tab now, if
    /// it has one the store knows.
    async fn agent_holder(&self, session: &pb::SessionId) -> Option<u32> {
        let tabs = self
            .daemon
            .terminals()
            .await
            .list_tabs(pb::ListTabsRequest {
                session: Some(session.clone()),
            })
            .await
            .ok()?
            .into_inner()
            .tabs;
        let tab = tabs.into_iter().find(|tab| tab.kind == pb::tab::Kind::Agent as i32)?.id?;
        stored_tabs(&self.daemon.state)
            .into_iter()
            .find(|stored| stored.id == tab.value)
            .map(|stored| stored.holder_pid)
    }

    async fn agent_holder_now(&self, session: &pb::SessionId) -> u32 {
        self.agent_holder(session)
            .await
            .unwrap_or_else(|| panic!("session {} has an agent holder", session.value))
    }

    /// Waits until `session`'s agent runs on another holder than `old`, and
    /// returns the session as it is then.
    async fn restarted(&self, session: &pb::SessionId, old: u32) -> pb::Session {
        let deadline = Instant::now() + LONG;
        loop {
            let now = get_session(&self.daemon, session).await;
            let holder = self.agent_holder(session).await;
            if holder.is_some_and(|pid| pid != old) && is_running(&now) {
                return now;
            }
            assert!(
                Instant::now() < deadline,
                "session {} was never restarted onto a new holder; its holder is {holder:?} (was {old}), it is {:?}",
                session.value,
                now.status()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The pending Holders item for this machine once it lists at least
    /// `sessions`.
    async fn holders_item_listing(&self, sessions: &[&pb::SessionId]) -> pb::InboxRequest {
        let wanted = ids(sessions);
        upgrade::inbox_until(&self.daemon, "a Holders item listed the sessions", |items| {
            items
                .iter()
                .find(|item| {
                    upgrade::pending_migrate(item, pb::MigrateThing::Holders, &self.machine)
                        && wanted.is_subset(&waiting(item))
                })
                .cloned()
        })
        .await
    }

    /// Every Holders item for this machine the Inbox holds, answered ones
    /// too.
    async fn holders_items(&self) -> Vec<pb::InboxRequest> {
        upgrade::inbox(&self.daemon)
            .await
            .into_iter()
            .filter(|item| is_holders(item, &self.machine))
            .collect()
    }

    /// The user clicks Migrate on item `id`.
    async fn migrate(&self, id: &pb::RequestId) {
        self.daemon
            .inbox_service()
            .await
            .answer_request(pb::AnswerRequestRequest {
                idempotency_key: Some(pb::IdempotencyKey {
                    value: format!("migrate-{}", id.value),
                }),
                id: Some(id.clone()),
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
                }),
                ..Default::default()
            })
            .await
            .expect("the user migrates the holders");
    }

    /// Item `id` once it is no longer pending.
    async fn ended(&self, id: &pb::RequestId) -> pb::InboxRequest {
        upgrade::request_until(&self.daemon, id, "ended", |request| {
            request.state() != pb::RequestState::Pending
        })
        .await
    }

    /// Stops the daemon as a crash would, forgets `session`'s holder
    /// record, and starts the daemon again: its holders keep running, and
    /// it detects again at ready.
    async fn restart_forgetting(&mut self, session: &pb::SessionId) {
        let record = self.record(session);
        wait_for(&record).await;
        self.daemon.kill();
        std::fs::remove_file(&record).unwrap_or_else(|error| panic!("couldn't remove {}: {error}", record.display()));
        self.daemon.restart().await;
    }
}

/// Makes every holder record of the daemon at `state` stand for a holder
/// from before LIVEHOLDER: the sha256 stays and the `moves` line goes, so
/// Migrate restarts its agent rather than move it, and a shell stays.
fn too_old_to_move(state: &Path) {
    let Ok(records) = std::fs::read_dir(state.join("holder_builds")) else {
        return;
    };
    for record in records.flatten() {
        let path = record.path();
        if path.extension().is_none_or(|extension| extension != "sha") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("couldn't read {}: {error}", path.display()));
        let sha = text.lines().next().unwrap_or_default().trim().to_owned();
        std::fs::write(&path, sha).unwrap_or_else(|error| panic!("couldn't write {}: {error}", path.display()));
    }
}

fn is_holders(item: &pb::InboxRequest, machine: &pb::MachineId) -> bool {
    item.kind() == pb::RequestKind::Migrate
        && upgrade::migrate_of(item)
            .is_some_and(|card| card.thing() == pb::MigrateThing::Holders && card.machine.as_ref() == Some(machine))
}

/// The sessions a Holders item says wait for a new holder.
fn waiting(item: &pb::InboxRequest) -> BTreeSet<String> {
    upgrade::migrate_of(item)
        .map(|card| card.waiting_sessions.iter().map(|session| session.value.clone()).collect())
        .unwrap_or_default()
}

fn ids(sessions: &[&pb::SessionId]) -> BTreeSet<String> {
    sessions.iter().map(|session| session.value.clone()).collect()
}

/// GetServerInfo, or `None` while the daemon is between two runs of itself.
async fn try_info(daemon: &Daemon) -> Option<pb::ServerInfo> {
    let socket = daemon.socket.clone();
    let channel: Channel = Endpoint::try_from("http://[::]:50051")
        .ok()?
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let socket = socket.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(socket).await?)) }
        }))
        .await
        .ok()?;
    pb::daemon_service_client::DaemonServiceClient::new(channel)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .ok()
        .map(tonic::Response::into_inner)
}

/// Waits until the daemon answers as release `bundle`.
async fn wait_at_release(daemon: &Daemon, bundle: &str) {
    let deadline = Instant::now() + LONG;
    loop {
        if try_info(daemon)
            .await
            .and_then(|info| info.build)
            .is_some_and(|build| build.bundle == bundle)
        {
            return;
        }
        assert!(Instant::now() < deadline, "the daemon never answered as release {bundle}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits until `path` exists.
async fn wait_for(path: &Path) {
    let deadline = Instant::now() + LONG;
    while !path.exists() {
        assert!(Instant::now() < deadline, "{} never appeared", path.display());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Every stored tab's holder as (tab, process id, generation), in a stable
/// order: a holder that was replaced or revived shows as another.
fn holders(daemon: &Daemon) -> Vec<(String, u32, u64)> {
    let mut all: Vec<_> = stored_tabs(&daemon.state)
        .iter()
        .map(|tab| (tab.id.clone(), tab.holder_pid, tab.holder_generation))
        .collect();
    all.sort();
    all
}

/// The stored shell tabs, machine shells and sessions' shell tabs alike.
fn shells(daemon: &Daemon) -> Vec<humata2_store::TabRecord> {
    let mut all: Vec<_> = stored_tabs(&daemon.state)
        .into_iter()
        .filter(|tab| tab.kind == TabKind::Shell)
        .collect();
    all.sort_by(|a, b| a.id.cmp(&b.id));
    all
}

/// The shells' holders as (tab, process id, generation).
fn shell_holders(daemon: &Daemon) -> Vec<(String, u32, u64)> {
    shells(daemon)
        .into_iter()
        .map(|tab| (tab.id, tab.holder_pid, tab.holder_generation))
        .collect()
}

/// The shell in tab `tab` still runs: a line typed to it is answered with
/// `still-<mark>`, which nothing before it printed.
async fn shell_answers(daemon: &Daemon, tab: &str, mark: &str) {
    let mut attach = Attach::start(
        daemon,
        open(
            &pb::TabId {
                value: tab.to_owned(),
            },
            0,
        ),
    )
    .await;
    // The typed line doesn't hold the text it prints.
    attach.type_text(&format!("printf 'still-%s\\n' {mark}\r")).await;
    attach.wait_for(&format!("still-{mark}")).await;
}

/// Opens a machine shell (no session) and waits until its shell answers.
async fn machine_shell(daemon: &Daemon) -> pb::TabId {
    let tab = daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .expect("a machine shell opens")
        .into_inner()
        .id
        .expect("the machine shell has a tab");
    shell_answers(daemon, &tab.value, "opened").await;
    tab
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_keeps_every_holder_and_posts_one_holders_item_listing_the_agent_sessions_on_old_ones() {
    let mac = Mac::start().await;
    let login = mac.agent_session("Fix login").await;
    let billing = mac.agent_session("Fix billing").await;
    let before = holders(&mac.daemon);

    mac.upgrade_to_b().await;

    let item = mac.holders_item_listing(&[&login, &billing]).await;
    assert_eq!(waiting(&item), ids(&[&login, &billing]), "{item:?}");
    assert!(item.vip, "a Migrate item is VIP: {item:?}");
    let card = upgrade::migrate_of(&item).expect("the item is a Migrate card");
    assert_ne!(card.phase(), pb::MigratePhase::Migrating, "nothing migrates before the user clicks Migrate");
    // PER-5: the upgrade left every holder as it was.
    assert_eq!(holders(&mac.daemon), before, "the same holders, the same processes");
    for tab in stored_tabs(&mac.daemon.state) {
        assert!(holder_answers(&tab), "holder {} still answers", tab.holder_pid);
    }
    for session in [&login, &billing] {
        assert!(is_running(&get_session(&mac.daemon, session).await), "session {} kept running", session.value);
    }
    let all = mac.holders_items().await;
    assert_eq!(all.len(), 1, "one Holders item for this machine: {all:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_detection_updates_the_one_holders_item_rather_than_posting_another() {
    let mut mac = Mac::start().await;
    let login = mac.agent_session("Fix login").await;
    mac.upgrade_to_b().await;
    let first = mac.holders_item_listing(&[&login]).await;
    // Started at release b: its spawn is a detection, and its holder is
    // current, so it isn't listed.
    let billing = mac.agent_session("Fix billing").await;

    // A daemon restart is another detection, and with billing's record
    // gone there is something new for it to show.
    mac.restart_forgetting(&billing).await;

    let second = mac.holders_item_listing(&[&billing]).await;
    assert_eq!(second.id, first.id, "the same item, updated: {second:?}");
    assert_eq!(waiting(&second), ids(&[&login, &billing]), "{second:?}");
    let all = mac.holders_items().await;
    assert_eq!(all.len(), 1, "no second Holders item for this machine: {all:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_whose_holder_has_no_recorded_build_counts_as_old() {
    let mut mac = Mac::start().await;
    let login = mac.agent_session("Fix login").await;
    let billing = mac.agent_session("Fix billing").await;
    // Billing's record is kept: only a session with none counts as old.
    wait_for(&mac.record(&billing)).await;
    let holder = mac.agent_holder_now(&login).await;

    mac.restart_forgetting(&login).await;

    let item = mac.holders_item_listing(&[&login]).await;
    assert_eq!(waiting(&item), ids(&[&login]), "{item:?}");
    assert!(item.vip, "{item:?}");
    assert_eq!(mac.agent_holder_now(&login).await, holder, "detecting it restarts nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn migrate_restarts_an_idle_agent_session_onto_the_current_holder_with_its_conversation_and_then_ends() {
    let mac = Mac::start().await;
    let login = mac.agent_session("Fix login").await;
    let mut typing = mac.agent(&login).await;
    typing.type_text("before-migrate\r").await;
    typing.wait_for("got: before-migrate").await;
    let conversation = get_session(&mac.daemon, &login).await.conversation_id;
    mac.upgrade_to_b().await;
    let item = mac.holders_item_listing(&[&login]).await;
    let id = item.id.clone().expect("an Inbox item has an id");
    let old = mac.agent_holder_now(&login).await;
    // Idle before Migrate: nothing happens until the user clicks it.
    let _idle = mac.turn(&login, "Stop", pb::SessionStatus::Idle).await;
    assert_eq!(mac.agent_holder_now(&login).await, old, "no restart before Migrate");

    mac.migrate(&id).await;

    let done = mac.ended(&id).await;
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    let back = mac.restarted(&login, old).await;
    assert_eq!(back.id.as_ref(), Some(&login), "the same session");
    assert_eq!(back.conversation_id, conversation, "the same conversation");
    let mut agent = mac.agent(&login).await;
    agent.wait_for("agent up --resume").await;
    // Input after the restart reaches the resumed agent; what was typed
    // before it is kept once, not lost and not typed to it again.
    agent.type_text("after-restart\r").await;
    agent.wait_for("got: after-restart").await;
    let said = std::fs::read_to_string(mac.world.conversation(&conversation)).expect("the conversation reads");
    let lines: Vec<&str> = said.lines().collect();
    let before_at: Vec<usize> = (0..lines.len()).filter(|at| lines[*at] == "before-migrate").collect();
    let after_at: Vec<usize> = (0..lines.len()).filter(|at| lines[*at] == "after-restart").collect();
    assert_eq!(before_at.len(), 1, "typed once before, kept once: {lines:?}");
    assert_eq!(after_at.len(), 1, "typed once after, kept once: {lines:?}");
    assert!(before_at[0] < after_at[0], "{lines:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_working_agent_session_is_not_restarted_until_it_turns_idle() {
    let mac = Mac::start().await;
    let login = mac.agent_session("Fix login").await;
    let billing = mac.agent_session("Fix billing").await;
    mac.upgrade_to_b().await;
    let item = mac.holders_item_listing(&[&login, &billing]).await;
    let id = item.id.clone().expect("an Inbox item has an id");
    let mut working = mac.turn(&billing, "UserPromptSubmit", pb::SessionStatus::Working).await;
    let _idle = mac.turn(&login, "Stop", pb::SessionStatus::Idle).await;
    let (login_old, billing_old) = (mac.agent_holder_now(&login).await, mac.agent_holder_now(&billing).await);

    mac.migrate(&id).await;

    // The idle one goes first; the item still waits for the working one.
    let midway = upgrade::request_until(&mac.daemon, &id, "listed only the working session", |request| {
        request.state() != pb::RequestState::Pending || waiting(request) == ids(&[&billing])
    })
    .await;
    assert_eq!(midway.state(), pb::RequestState::Pending, "{midway:?}");
    assert_eq!(waiting(&midway), ids(&[&billing]), "{midway:?}");
    let card = upgrade::migrate_of(&midway).expect("the item is a Migrate card");
    assert_eq!(card.phase(), pb::MigratePhase::Migrating, "{midway:?}");
    mac.restarted(&login, login_old).await;
    assert_eq!(mac.agent_holder_now(&billing).await, billing_old, "a working agent isn't restarted");
    assert_eq!(get_session(&mac.daemon, &billing).await.status(), pb::SessionStatus::Working);

    working
        .type_text(&format!("run:{}\r", mac.hook_command("Stop")))
        .await;

    let done = mac.ended(&id).await;
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    let back = mac.restarted(&billing, billing_old).await;
    assert_eq!(back.id.as_ref(), Some(&billing), "the same session");
}

#[tokio::test(flavor = "multi_thread")]
async fn shells_stay_on_their_holders_and_never_hold_the_holders_item_open() {
    let mut mac = Mac::start().await;
    let login = mac.agent_session("Fix login").await;
    let _machine_shell = machine_shell(&mac.daemon).await;
    // A shell tab in the agent's own session.
    let (_token, _session_shell) = shell_token(&mac.daemon, &login).await;
    let shells_before = shells(&mac.daemon);
    let shell_holders_before = shell_holders(&mac.daemon);
    assert!(shells_before.len() >= 2, "a machine shell and the session's shell tab: {shells_before:?}");
    mac.upgrade_to_b().await;

    let item = mac.holders_item_listing(&[&login]).await;
    assert_eq!(waiting(&item), ids(&[&login]), "shells aren't listed as waiting: {item:?}");
    let card = upgrade::migrate_of(&item).expect("the item is a Migrate card");
    assert!(!card.keeps_running.is_empty(), "the card says the shells keep running: {item:?}");
    let id = item.id.clone().expect("an Inbox item has an id");
    let old = mac.agent_holder_now(&login).await;
    let _idle = mac.turn(&login, "Stop", pb::SessionStatus::Idle).await;

    mac.migrate(&id).await;

    // The shells are still on their old holders, and the item ends anyway.
    let done = mac.ended(&id).await;
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    mac.restarted(&login, old).await;
    assert_eq!(shell_holders(&mac.daemon), shell_holders_before, "PER-6: no shell was restarted");
    for shell in &shells_before {
        assert!(holder_answers(shell), "shell holder {} still answers", shell.holder_pid);
        shell_answers(&mac.daemon, &shell.id, "after-migrate").await;
    }
    let pending: Vec<_> = mac
        .holders_items()
        .await
        .into_iter()
        .filter(|item| item.state() == pb::RequestState::Pending)
        .collect();
    assert!(pending.is_empty(), "shells alone hold no item: {pending:?}");

    // Only shells are on old holders now: a daemon restart and a new
    // session's spawn are detections, and neither posts an item. There is
    // nothing to wait for that would say they ran, so this can pass for a
    // daemon that doesn't detect then; it never fails one that is right.
    mac.daemon.restart().await;
    mac.agent_session("Fix billing").await;
    let pending: Vec<_> = mac
        .holders_items()
        .await
        .into_iter()
        .filter(|item| item.state() == pb::RequestState::Pending)
        .collect();
    assert!(pending.is_empty(), "shells alone post no item: {pending:?}");
    assert_eq!(shell_holders(&mac.daemon), shell_holders_before, "a daemon restart restarts no shell either");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_opened_in_a_waiting_session_leaves_the_session_waiting() {
    let mut mac = Mac::start().await;
    let login = mac.agent_session("Fix login").await;
    mac.upgrade_to_b().await;
    mac.holders_item_listing(&[&login]).await;
    let billing = mac.agent_session("Fix billing").await;
    // Spawned from release b's holder, in the session whose agent is
    // still on release a's.
    let (_token, _shell) = shell_token(&mac.daemon, &login).await;

    // A detection that has billing to show, so the item says what it found
    // for login once it shows billing.
    mac.restart_forgetting(&billing).await;

    let item = mac.holders_item_listing(&[&billing]).await;
    assert_eq!(waiting(&item), ids(&[&login, &billing]), "login's agent is still on an old holder: {item:?}");
}

// --- D379: what each row of the Holders item says --------------------------

/// Item `id` once it has a row for `session`.
async fn with_row_for(daemon: &Daemon, id: &pb::RequestId, session: &pb::SessionId) -> pb::InboxRequest {
    upgrade::request_until(daemon, id, "had a row for the session", |request| {
        upgrade::agent_rows(request).contains_key(&session.value)
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_holders_item_has_a_row_per_old_holder_saying_what_migrate_does_with_it() {
    let mac = Mac::start().await;
    let login = mac.agent_session("Fix login").await;
    let _shell = machine_shell(&mac.daemon).await;
    let old_shells = shells(&mac.daemon).len();
    assert!(old_shells >= 1, "a machine shell is open");
    mac.upgrade_to_b().await;
    let listed = mac.holders_item_listing(&[&login]).await;
    let id = listed.id.clone().expect("an Inbox item has an id");

    let item = with_row_for(&mac.daemon, &id, &login).await;

    assert_eq!(
        upgrade::agent_rows(&item),
        [(login.value.clone(), pb::HolderNote::RestartsWhenIdle)].into_iter().collect::<std::collections::BTreeMap<_, _>>(),
        "the agent session restarts at its next idle moment: {item:?}"
    );
    assert_eq!(
        upgrade::shell_rows(&item),
        vec![pb::HolderNote::StaysUntilClosed; old_shells],
        "each shell stays until it is closed: {item:?}"
    );
    assert_eq!(waiting(&item), ids(&[&login]), "waiting_sessions is still filled: {item:?}");
    assert!(upgrade::step_states(&item).is_empty(), "a Holders item shows rows, not steps: {item:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn while_it_migrates_an_agent_row_says_it_restarts_when_idle_or_is_restarting() {
    let mac = Mac::start().await;
    let login = mac.agent_session("Fix login").await;
    mac.upgrade_to_b().await;
    let listed = mac.holders_item_listing(&[&login]).await;
    let id = listed.id.clone().expect("an Inbox item has an id");
    let item = with_row_for(&mac.daemon, &id, &login).await;
    let old = mac.agent_holder_now(&login).await;
    let _idle = mac.turn(&login, "Stop", pb::SessionStatus::Idle).await;

    mac.migrate(&id).await;

    let seen = upgrade::request_history(&mac.daemon, &id, "ended", |request| {
        request.state() != pb::RequestState::Pending
    })
    .await;
    let done = seen.last().expect("the history ends with the item ended");
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    for shown in std::iter::once(&item).chain(&seen) {
        if let Some(note) = upgrade::agent_rows(shown).get(&login.value) {
            assert!(
                [pb::HolderNote::RestartsWhenIdle, pb::HolderNote::Restarting].contains(note),
                "an agent with turns restarts when idle: {shown:?}"
            );
        }
    }
    mac.restarted(&login, old).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_with_no_turns_is_noted_as_having_no_idle_moment() {
    let mac = Mac::start_with_turns("none").await;
    let (server, operation) = start_session(&mac.daemon, &mac.channel, "Run the server").await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
    wait_status(&mac.daemon, &server, pb::SessionStatus::RunningUnknownTurn).await;
    mac.agent(&server).await.wait_for("agent up").await;
    mac.upgrade_to_b().await;
    let listed = mac.holders_item_listing(&[&server]).await;
    let id = listed.id.clone().expect("an Inbox item has an id");

    let item = with_row_for(&mac.daemon, &id, &server).await;

    assert_eq!(
        upgrade::agent_rows(&item).get(&server.value),
        Some(&pb::HolderNote::NoIdleMoment),
        "an agent with no turns has no idle moment to restart at: {item:?}"
    );
}
