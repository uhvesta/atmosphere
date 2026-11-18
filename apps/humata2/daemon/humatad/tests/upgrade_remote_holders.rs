//! D378: holders on a machine. The machine's daemon finds which of its own
//! holders are not its current holder file, as this Mac's does (D373), and
//! reports them to the host over its link, again on each change and each
//! time the link comes back. The host shows them as one VIP "Migrate the
//! holders on gpu" item. Migrate asks the machine to restart each idle
//! agent session onto the current holder, with the same session and
//! conversation; shells stay on theirs until closed and never hold the
//! item open, and the item ends Approved once the machine reports none
//! waiting. A link that drops while it migrates leaves the item migrating,
//! and the report the machine makes when it links again settles it.
//!
//! `gpu` runs the holder the tests run, which its release names, so a
//! holder it records is current. A test stands for a holder spawned from
//! another file by deleting its record while `gpu`'s daemon is down
//! (`<state>/holder_builds/<session>.sha`, `<tab>.shell.sha`): a holder
//! with no record counts as old.
//!
//! `gpu`'s agent tells its turns by its output: quiet for a few seconds is
//! probably idle, the moment a holder may be restarted at. A line
//! `run:<command>` runs the command, so a test keeps an agent writing, and
//! so working, until it creates a file. Every wait ends on a pushed change
//! or on a status the daemons report; none is a guessed time.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;
#[path = "common/upgrade.rs"]
mod upgrade;
#[path = "common/upgrade_rig.rs"]
mod upgrade_rig;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Attach, Daemon, agent_tab, get_session, is_running, open, wait_for_path, wait_status};
use humata2_grpc::humata::v1 as pb;
use humata2_store::TabKind;
use remote_rig::Rig;
use upgrade_rig::BOX;

/// The most a test waits for an agent on `gpu` to come back on a new
/// holder: a safety net, never a time anything is expected to take.
const LONG: Duration = Duration::from_secs(120);

/// `gpu`'s agent: the rig's own, which also runs `run:<command>` lines.
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
while IFS= read -r line; do
  case "$line" in
    run:*) eval "${{line#run:}}" ;;
    *) echo "got: $line" ;;
  esac
  printf '%s\n' "$line" >> "$f"
done
"#,
        conv.display()
    )
}

/// The host and `gpu` at one release, with a channel on `gpu` and an agent
/// session there per title, each with its agent up.
async fn box_with_sessions(titles: &[&str]) -> (Rig, Vec<pb::SessionId>) {
    let (rig, _) = upgrade_rig::at_one_release().await;
    let home = rig.the(BOX).home().to_owned();
    // The rig's agents file names this script, which each start runs.
    std::fs::write(home.join("agent.sh"), agent_script(&home.join("conv"))).expect("gpu's agent script is written");
    let channel = rig
        .create_channel("Payments", &[("acme", "api")], &[upgrade_rig::box_id()])
        .await;
    let mut sessions = Vec::new();
    for title in titles {
        let session = rig.start_on(&channel, &upgrade_rig::box_id(), title).await;
        on_box(&rig, &session).await.wait_for("agent up").await;
        sessions.push(session);
    }
    (rig, sessions)
}

/// An attach to `session`'s agent tab on `gpu`, from its first byte.
async fn on_box(rig: &Rig, session: &pb::SessionId) -> Attach {
    let daemon = rig.the(BOX).daemon();
    let tab = agent_tab(daemon, session).await;
    Attach::start(daemon, open(&tab, 0)).await
}

fn agent_record(session: &pb::SessionId) -> String {
    format!("{}.sha", session.value)
}

fn shell_record(tab: &pb::TabId) -> String {
    format!("{}.shell.sha", tab.value)
}

fn record_path(rig: &Rig, name: &str) -> PathBuf {
    rig.the(BOX).daemon().state.join("holder_builds").join(name)
}

/// Stops `gpu`'s daemon as a crash would, forgets the holder records
/// `names`, and starts it again: its holders keep running, it detects at
/// ready, and the link is up again when this returns.
async fn forget_on_box(rig: &mut Rig, names: &[String]) {
    for name in names {
        wait_for_path(&record_path(rig, name)).await;
    }
    rig.the_mut(BOX).kill();
    rig.link_ended(BOX).await;
    for name in names {
        let path = record_path(rig, name);
        std::fs::remove_file(&path).unwrap_or_else(|error| panic!("couldn't remove {}: {error}", path.display()));
    }
    rig.the_mut(BOX).relaunch(&[]).await;
    rig.link_back(BOX).await;
}

/// The process id of the holder behind `session`'s agent tab on `gpu`.
fn box_agent_holder(rig: &Rig, session: &pb::SessionId) -> Option<u32> {
    rig.the(BOX)
        .tabs()
        .into_iter()
        .find(|tab| tab.kind == TabKind::Agent && tab.session.as_deref() == Some(session.value.as_str()))
        .map(|tab| tab.holder_pid)
}

fn box_agent_holder_now(rig: &Rig, session: &pb::SessionId) -> u32 {
    box_agent_holder(rig, session).unwrap_or_else(|| panic!("session {} has an agent holder on gpu", session.value))
}

/// Waits until `session`'s agent on `gpu` runs on another holder than
/// `old`.
async fn restarted_on_box(rig: &Rig, session: &pb::SessionId, old: u32) {
    let deadline = Instant::now() + LONG;
    loop {
        let holder = box_agent_holder(rig, session);
        if holder.is_some_and(|pid| pid != old) && is_running(&get_session(rig.the(BOX).daemon(), session).await) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "session {} was never restarted onto a new holder on gpu; its holder is {holder:?} (was {old})",
            session.value
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Keeps `session`'s agent on `gpu` writing, and so working, until `flag`
/// exists; returns once `gpu` shows it working. The attach is returned to
/// keep the tab in use.
async fn keep_busy(rig: &Rig, session: &pb::SessionId, flag: &Path) -> Attach {
    let mut attach = on_box(rig, session).await;
    // The typed line doesn't hold the text it prints.
    attach
        .type_text(&format!(
            "run:while [ ! -e '{}' ]; do echo \"busy\"\"-tick\"; sleep 0.2; done\r",
            flag.display()
        ))
        .await;
    attach.wait_for("busy-tick").await;
    wait_status(rig.the(BOX).daemon(), session, pb::SessionStatus::Working).await;
    attach
}

/// Opens a machine shell on `gpu` and waits until its shell answers.
async fn box_shell(rig: &Rig) -> pb::TabId {
    let daemon = rig.the(BOX).daemon();
    let tab = daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .expect("a machine shell opens on gpu")
        .into_inner()
        .id
        .expect("the machine shell has a tab");
    shell_answers(daemon, &tab, "opened").await;
    tab
}

/// The shell in `tab` still runs: a line typed to it is answered with
/// `still-<mark>`, which nothing before it printed.
async fn shell_answers(daemon: &Daemon, tab: &pb::TabId, mark: &str) {
    let mut attach = Attach::start(daemon, open(tab, 0)).await;
    attach.type_text(&format!("printf 'still-%s\\n' {mark}\r")).await;
    attach.wait_for(&format!("still-{mark}")).await;
}

fn shell_holder(rig: &Rig, tab: &pb::TabId) -> (u32, u64) {
    rig.the(BOX)
        .tabs()
        .into_iter()
        .find(|stored| stored.id == tab.value)
        .map(|stored| (stored.holder_pid, stored.holder_generation))
        .unwrap_or_else(|| panic!("gpu's store has shell tab {}", tab.value))
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

fn the_id(item: &pb::InboxRequest) -> pb::RequestId {
    item.id.clone().expect("an Inbox item has an id")
}

fn phase(item: &pb::InboxRequest) -> pb::MigratePhase {
    upgrade::migrate_of(item)
        .unwrap_or_else(|| panic!("not a Migrate item: {item:?}"))
        .phase()
}

/// Every Holders item for `machine` in `items`, answered ones too.
fn holders_items<'a>(items: &'a [pb::InboxRequest], machine: &pb::MachineId) -> Vec<&'a pb::InboxRequest> {
    items
        .iter()
        .filter(|item| {
            item.kind() == pb::RequestKind::Migrate
                && upgrade::migrate_of(item).is_some_and(|card| {
                    card.thing() == pb::MigrateThing::Holders && card.machine.as_ref() == Some(machine)
                })
        })
        .collect()
}

/// The host's pending Holders item for `gpu` once it lists at least
/// `sessions` as waiting.
async fn holders_item_listing(rig: &Rig, sessions: &[&pb::SessionId]) -> pb::InboxRequest {
    let wanted = ids(sessions);
    upgrade::inbox_until(&rig.host, "a Holders item for gpu listed the sessions", |items| {
        items
            .iter()
            .find(|item| {
                upgrade::pending_migrate(item, pb::MigrateThing::Holders, &upgrade_rig::box_id())
                    && wanted.is_subset(&waiting(item))
            })
            .cloned()
    })
    .await
}

/// The user clicks Migrate on item `id` on the host.
async fn migrate(rig: &Rig, id: &pb::RequestId) {
    rig.host
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
        .expect("the user migrates gpu's holders");
}

/// Item `id` on the host once it is no longer pending.
async fn ended(rig: &Rig, id: &pb::RequestId) -> pb::InboxRequest {
    upgrade::request_until(&rig.host, id, "ended", |request| request.state() != pb::RequestState::Pending).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machines_agent_sessions_on_old_holders_are_one_holders_item_for_that_machine_on_the_host() {
    let (mut rig, sessions) = box_with_sessions(&["Fix login", "Fix billing"]).await;
    let (login, billing) = (&sessions[0], &sessions[1]);
    let before = (box_agent_holder_now(&rig, login), box_agent_holder_now(&rig, billing));

    forget_on_box(&mut rig, &[agent_record(login), agent_record(billing)]).await;

    let listed = holders_item_listing(&rig, &[login, billing]).await;
    assert_eq!(waiting(&listed), ids(&[login, billing]), "{listed:?}");
    assert!(listed.vip, "a Migrate item is VIP: {listed:?}");
    assert_ne!(phase(&listed), pb::MigratePhase::Migrating, "nothing migrates before the user clicks Migrate");
    let item = upgrade::request_until(&rig.host, &the_id(&listed), "had a row for each session", |request| {
        upgrade::agent_rows(request).len() == 2
    })
    .await;
    assert_eq!(
        upgrade::agent_rows(&item),
        [
            (login.value.clone(), pb::HolderNote::RestartsWhenIdle),
            (billing.value.clone(), pb::HolderNote::RestartsWhenIdle),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>(),
        "{item:?}"
    );
    let items = upgrade::inbox(&rig.host).await;
    let for_box = holders_items(&items, &upgrade_rig::box_id());
    assert_eq!(for_box.len(), 1, "one Holders item for gpu: {for_box:?}");
    let host = rig.host_id().await;
    let for_host: Vec<_> = holders_items(&items, &host)
        .into_iter()
        .filter(|item| item.state() == pb::RequestState::Pending)
        .collect();
    assert!(for_host.is_empty(), "gpu's holders are not the host's: {for_host:?}");
    assert_eq!(
        (box_agent_holder_now(&rig, login), box_agent_holder_now(&rig, billing)),
        before,
        "detecting them restarts nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machines_shells_are_listed_as_staying_until_closed_and_never_hold_its_item_open() {
    let (mut rig, sessions) = box_with_sessions(&["Fix login"]).await;
    let login = &sessions[0];
    let shell = box_shell(&rig).await;
    let shell_before = shell_holder(&rig, &shell);
    forget_on_box(&mut rig, &[agent_record(login), shell_record(&shell)]).await;
    let listed = holders_item_listing(&rig, &[login]).await;
    assert_eq!(waiting(&listed), ids(&[login]), "a shell isn't listed as waiting: {listed:?}");
    let id = the_id(&listed);
    let item = upgrade::request_until(&rig.host, &id, "had rows for the session and the shell", |request| {
        upgrade::agent_rows(request).contains_key(&login.value) && !upgrade::shell_rows(request).is_empty()
    })
    .await;
    assert_eq!(upgrade::shell_rows(&item), vec![pb::HolderNote::StaysUntilClosed], "{item:?}");
    let old = box_agent_holder_now(&rig, login);

    migrate(&rig, &id).await;

    // The shell is still on its old holder, and the item ends anyway.
    let done = ended(&rig, &id).await;
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    restarted_on_box(&rig, login, old).await;
    assert_eq!(shell_holder(&rig, &shell), shell_before, "no shell was restarted");
    shell_answers(rig.the(BOX).daemon(), &shell, "after-migrate").await;
    let items = upgrade::inbox(&rig.host).await;
    let pending: Vec<_> = holders_items(&items, &upgrade_rig::box_id())
        .into_iter()
        .filter(|item| item.state() == pb::RequestState::Pending)
        .collect();
    assert!(pending.is_empty(), "a shell alone holds no item: {pending:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn migrate_restarts_an_idle_agent_session_on_the_machine_with_its_conversation() {
    let (mut rig, sessions) = box_with_sessions(&["Fix login"]).await;
    let login = &sessions[0];
    let mut typing = on_box(&rig, login).await;
    typing.type_text("before-migrate\r").await;
    typing.wait_for("got: before-migrate").await;
    drop(typing);
    let conversation = rig.the(BOX).conversation_id(login);
    forget_on_box(&mut rig, &[agent_record(login)]).await;
    let item = holders_item_listing(&rig, &[login]).await;
    let old = box_agent_holder_now(&rig, login);

    migrate(&rig, &the_id(&item)).await;

    restarted_on_box(&rig, login, old).await;
    assert_eq!(rig.the(BOX).conversation_id(login), conversation, "the same conversation");
    let mut agent = on_box(&rig, login).await;
    agent.wait_for("agent up --resume").await;
    // Input after the restart reaches the resumed agent; what was typed
    // before it is kept once, not lost and not typed to it again.
    agent.type_text("after-restart\r").await;
    agent.wait_for("got: after-restart").await;
    let said = std::fs::read_to_string(rig.the(BOX).conversation(&conversation)).expect("the conversation reads");
    let lines: Vec<&str> = said.lines().collect();
    let before_at: Vec<usize> = (0..lines.len()).filter(|at| lines[*at] == "before-migrate").collect();
    let after_at: Vec<usize> = (0..lines.len()).filter(|at| lines[*at] == "after-restart").collect();
    assert_eq!(before_at.len(), 1, "typed once before, kept once: {lines:?}");
    assert_eq!(after_at.len(), 1, "typed once after, kept once: {lines:?}");
    assert!(before_at[0] < after_at[0], "{lines:?}");
    let shown = rig.wait_session(login, "live", remote_rig::is_live).await;
    assert_eq!(shown.id.as_ref(), Some(login), "the same session, as the host shows it");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_machines_holders_item_ends_approved_once_the_machine_reports_none_waiting() {
    let (mut rig, sessions) = box_with_sessions(&["Fix login", "Fix billing"]).await;
    let (login, billing) = (&sessions[0], &sessions[1]);
    forget_on_box(&mut rig, &[agent_record(login), agent_record(billing)]).await;
    let item = holders_item_listing(&rig, &[login, billing]).await;
    let id = the_id(&item);
    let flag = rig.the(BOX).home().join("billing-done");
    let _busy = keep_busy(&rig, billing, &flag).await;
    let (login_old, billing_old) = (box_agent_holder_now(&rig, login), box_agent_holder_now(&rig, billing));

    migrate(&rig, &id).await;

    // The idle one goes first; the machine reports the working one still
    // waits, and the item waits with it.
    let midway = upgrade::request_until(&rig.host, &id, "listed only the working session", |request| {
        request.state() != pb::RequestState::Pending || waiting(request) == ids(&[billing])
    })
    .await;
    assert_eq!(midway.state(), pb::RequestState::Pending, "{midway:?}");
    assert_eq!(phase(&midway), pb::MigratePhase::Migrating, "{midway:?}");
    restarted_on_box(&rig, login, login_old).await;
    assert_eq!(box_agent_holder(&rig, billing), Some(billing_old), "a working agent isn't restarted");

    std::fs::write(&flag, b"").expect("the flag is written");

    let done = ended(&rig, &id).await;
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    restarted_on_box(&rig, billing, billing_old).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_that_drops_while_it_migrates_leaves_it_migrating_and_the_relink_report_settles_it() {
    let (mut rig, sessions) = box_with_sessions(&["Fix login"]).await;
    let login = &sessions[0];
    let conversation = rig.the(BOX).conversation_id(login);
    forget_on_box(&mut rig, &[agent_record(login)]).await;
    let item = holders_item_listing(&rig, &[login]).await;
    let id = the_id(&item);
    // Working, so it can't be restarted before the link drops.
    let flag = rig.the(BOX).home().join("login-done");
    let _busy = keep_busy(&rig, login, &flag).await;
    let old = box_agent_holder_now(&rig, login);
    migrate(&rig, &id).await;
    upgrade::request_until(&rig.host, &id, "showed it migrating", |request| {
        phase(request) == pb::MigratePhase::Migrating
    })
    .await;

    rig.link_lost(BOX).await;

    let away = upgrade::request(&rig.host, &id).await;
    assert_eq!(away.state(), pb::RequestState::Pending, "a dropped link ends nothing: {away:?}");
    assert_eq!(phase(&away), pb::MigratePhase::Migrating, "a dropped link leaves it migrating: {away:?}");
    assert_eq!(waiting(&away), ids(&[login]), "{away:?}");
    // The agent goes quiet while gpu is away; gpu restarts it at its own
    // pace, and what it says when it links again settles the item.
    std::fs::write(&flag, b"").expect("the flag is written");
    rig.link_back(BOX).await;

    let done = ended(&rig, &id).await;
    assert_eq!(done.state(), pb::RequestState::Approved, "{done:?}");
    restarted_on_box(&rig, login, old).await;
    assert_eq!(rig.the(BOX).conversation_id(login), conversation, "the same conversation");
}
