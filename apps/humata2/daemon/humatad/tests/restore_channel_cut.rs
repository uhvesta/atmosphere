//! A channel restored under another folder (MV-16, D194) that the daemon
//! is cut down on right after its records came back, and before its
//! conversations moved to the keys of the new folder: after a restart the
//! conversations are at the new place, they name it, nothing is left at
//! the old one, and the note the restore kept is gone.
//!
//! The agent keeps its conversation at `<conv>/<folder, dashed>/<id>.jsonl`
//! as Claude does, so a session whose folder moved resumes only if its
//! conversation moved with it.

mod common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Build, Daemon, WAIT, World, create_channel, finished, folder, start_session, wait_running};
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;

/// The test agent: it starts or resumes the conversation kept by folder and
/// writes the folder it was given as the conversation's first line.
const AGENT: &str = r#"CONV_DIR='@CONV@'
mode=$1; id=$2; key=$3; place=$4
f="$CONV_DIR/$key/$id.jsonl"
case "$mode" in
  --session-id) mkdir -p "$CONV_DIR/$key"; printf '{"cwd":"%s"}\n' "$place" > "$f" ;;
  --resume) [ -f "$f" ] || { echo "no conversation at $f"; exit 3; } ;;
esac
echo "agent up $mode"
while IFS= read -r line; do printf '%s\n' "$line" >> "$f"; done
"#;

/// How an agent that keys its conversations by folder names that folder.
fn dashed(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Makes the world's agent the one above.
fn keep_conversations_by_folder(world: &World) {
    let script = world.root.join("agent.sh");
    std::fs::write(&script, AGENT.replace("@CONV@", &world.conv.display().to_string())).unwrap();
    let agent = serde_json::json!({
        "name": common::AGENT_NAME,
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}", "{folder_dashed}", "{folder}"],
        "resume_args": [script, "--resume", "{conversation}", "{folder_dashed}", "{folder}"],
        "conversation_file": format!("{}/{{folder_dashed}}/{{conversation}}.jsonl", world.conv.display()),
    });
    std::fs::write(&world.agents, serde_json::json!([agent]).to_string()).unwrap();
}

/// Waits up to `WAIT` for `done` to hold.
async fn until<F, Fut>(what: &str, done: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + WAIT;
    while !done().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A restore cut down right after its records came back: the daemon is
/// stopped, with the note of the files still to move on disk.
struct Cut {
    daemon: Daemon,
    world: World,
    channel: pb::Channel,
    id: pb::SessionId,
    session: pb::Session,
    before: PathBuf,
    old: PathBuf,
}

/// Sets up a channel with a running session, deletes it, takes its old folder
/// so it comes back at `-2`, and cuts the restore at `restore_channel.imported`.
async fn cut_a_restore() -> Cut {
    let dir = folder();
    cut_a_restore_at(dir, "restore_channel.imported").await
}

/// As [`cut_a_restore`], but the daemon is cut at `stop_at` instead, for a
/// test that needs the restore stopped at another point.
async fn cut_a_restore_at(dir: tempfile::TempDir, stop_at: &str) -> Cut {
    let world = World::new(dir.path());
    keep_conversations_by_folder(&world);
    world.remote("acme", "api");
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
    let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
    let (id, operation) = start_session(&daemon, &channel, "Ledger fix").await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
    let session = wait_running(&daemon, &id).await;
    let state = daemon.state.clone();
    let store = move || Store::open(state.join("store")).unwrap();
    let before = store().session(&id.value).unwrap().unwrap().folder;
    let old = world.channels.join(&channel.folder);
    let old_conversation = world.conv.join(dashed(&before)).join(format!("{}.jsonl", session.conversation_id));
    assert!(old_conversation.is_file(), "the agent keeps its conversation at {}", old_conversation.display());

    let mut channels = daemon.channel_service().await;
    let plan = channels
        .plan_delete_channel(pb::PlanDeleteChannelRequest { channel: channel.id.clone() })
        .await
        .unwrap()
        .into_inner();
    let deleted = channels
        .delete_channel(pb::DeleteChannelRequest {
            idempotency_key: None,
            channel: plan.channel.clone(),
            plan: plan.token.clone(),
            presence: None,
            delete_unpushed_work: true,
        })
        .await
        .unwrap()
        .into_inner();
    let done = finished(&daemon, deleted.operation.as_ref().unwrap()).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");
    let item = deleted.trash_item.unwrap();
    // Something takes the old folder, so the channel comes back at `-2`.
    std::fs::create_dir_all(&old).unwrap();

    daemon.restart_env(&[("HUMATA2_STOP_AT", stop_at)]).await;
    common::last_start(&daemon).await;
    // The daemon kills itself at the point, so the reply may not come.
    let _ = pb::trash_service_client::TrashServiceClient::new(daemon.channel().await)
        .restore_from_trash(pb::RestoreFromTrashRequest { idempotency_key: None, id: Some(item) })
        .await;
    let deadline = Instant::now() + WAIT;
    while std::os::unix::net::UnixStream::connect(&daemon.socket).is_ok() {
        assert!(Instant::now() < deadline, "the daemon never died at the restore's cut");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Cut { daemon, world, channel, id, session, before, old }
}

/// What the newest restart report says of the restore: `stuck`, `finished`...
fn restore_state(state: &Path) -> Option<String> {
    let mut folders: Vec<_> = std::fs::read_dir(state.join("restarts")).ok()?.flatten().map(|entry| entry.path()).collect();
    folders.sort();
    let text = std::fs::read_to_string(folders.last()?.join("operations.jsonl")).ok()?;
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value["kind"] == "restore_channel")
        .filter_map(|value| value["state"].as_str().map(str::to_owned))
        .next_back()
}

/// A restore whose files can't be moved is named stuck, with its note kept,
/// and the next restart, with nothing failing, finishes it and names that.
#[tokio::test(flavor = "multi_thread")]
async fn a_restore_whose_files_fail_to_move_is_stuck_and_the_next_restart_finishes_it() {
    let Cut { mut daemon, world, .. } = cut_a_restore().await;
    let notes = world.channels.join(".restoring");

    daemon.restart_env(&[("HUMATA2_FAIL_AT", "restore_channel.finish_files")]).await;
    common::last_start(&daemon).await;
    let state = daemon.state.clone();
    until("the restore named stuck", || async { restore_state(&state).as_deref() == Some("stuck") }).await;
    assert!(std::fs::read_dir(&notes).is_ok_and(|mut left| left.next().is_some()), "the note is kept for the next restart");

    daemon.restart().await;
    common::last_start(&daemon).await;
    let state = daemon.state.clone();
    until("the restore named finished", || async { restore_state(&state).as_deref() == Some("finished") }).await;
    until("the note going", || async { std::fs::read_dir(&notes).map_or(true, |mut left| left.next().is_none()) }).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restore_cut_after_its_records_came_back_finishes_the_conversations_at_restart() {
    let Cut { mut daemon, world, channel, id, session, before, old } = cut_a_restore().await;
    let old_conversation = world.conv.join(dashed(&before)).join(format!("{}.jsonl", session.conversation_id));
    let state = daemon.state.clone();
    let store = move || Store::open(state.join("store")).unwrap();

    daemon.restart().await;
    common::last_start(&daemon).await;

    let new = world.channels.join(format!("{}-2", channel.folder));
    let after: PathBuf = new.join(before.strip_prefix(&old).unwrap());
    let new_conversation = world.conv.join(dashed(&after)).join(format!("{}.jsonl", session.conversation_id));
    let notes = world.channels.join(".restoring");
    until("the restore finishing", || async {
        new_conversation.is_file() && std::fs::read_dir(&notes).map_or(true, |mut left| left.next().is_none())
    })
    .await;
    assert!(!old_conversation.exists(), "nothing is left at the old key");
    let kept = std::fs::read_to_string(&new_conversation).unwrap();
    assert!(kept.contains(&after.display().to_string()), "it names the new folder: {kept:?}");
    assert!(!kept.contains(&before.display().to_string()), "it no longer names the old one: {kept:?}");
    assert_eq!(store().session(&id.value).unwrap().unwrap().folder, after);
    assert_eq!(store().agent(&id.value).unwrap().unwrap().launch.folder, after);
}

/// A restore whose records can't be imported is named stuck with its side file
/// kept in its folder, and the next restart, with nothing failing, imports them
/// and names the restore finished.
#[tokio::test(flavor = "multi_thread")]
async fn a_restore_whose_records_fail_to_import_is_stuck_and_the_next_restart_brings_it_back() {
    let Cut { mut daemon, world, channel, .. } = cut_a_restore_at(folder(), "store.channel.import.committing").await;
    let id = channel.id.clone().unwrap().value;
    let back = world.channels.join(format!("{}-2", channel.folder));
    let state = daemon.state.clone();
    let store = move || Store::open(state.join("store")).unwrap();
    assert!(store().channel(&id).unwrap().is_none(), "the cut came before the records were imported");

    daemon.restart_env(&[("HUMATA2_FAIL_AT", "restore_channel.import_record")]).await;
    common::last_start(&daemon).await;
    let state = daemon.state.clone();
    until("the restore named stuck", || async { restore_state(&state).as_deref() == Some("stuck") }).await;
    assert!(back.join(humata2_store::SIDE_FILE).is_file(), "the folder keeps its side file for the next restart");
    assert!(store().channel(&id).unwrap().is_none(), "nothing was imported");

    daemon.restart().await;
    common::last_start(&daemon).await;
    let state = daemon.state.clone();
    until("the restore named finished", || async { restore_state(&state).as_deref() == Some("finished") }).await;
    assert!(store().channel(&id).unwrap().is_some(), "the channel is back once the restore finishes");
}
