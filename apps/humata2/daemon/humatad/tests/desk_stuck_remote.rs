//! A Desk start cut short on a box (D150, PER-27, PER-29; SAFE): the box is
//! killed while it starts a Desk session, and the restart can't take that
//! session back, so it names the start stuck. The host's start ends with an
//! error rather than hanging and keeps no record of it; the box's restart
//! report names the start pending and then stuck, and its restart names the
//! session it couldn't finish or remove; a fresh start on the box then
//! works. Waits are bounded by `LONG`, and the box's own answer is read once
//! its start is done.
//!
//! The undo is made to fail by taking the write permission off the session's
//! folder, which only a user other than root is held to, so the test says it
//! is skipped when it runs as root. The restart's recovery of a Desk session
//! finds no channel for it (a Desk session's channel, "desk", has no channel
//! record), so finishing the start fails at once and it is undone; the undo
//! then can't remove the folder, which leaves the session stuck.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use humata2_grpc::humata::v1 as pb;
use remote_rig::{LONG, Rig};
use tonic::transport::Channel;

const BOX: &str = "box-1";
const REPO: (&str, &str) = ("acme", "api");
/// Where the box is killed: its Desk session recorded, its agent not launched.
const KILLED_AT: &str = "sessions.start.launching";

async fn desks(rig: &Rig) -> pb::desk_service_client::DeskServiceClient<Channel> {
    pb::desk_service_client::DeskServiceClient::new(rig.channel().await)
}

/// A plain Desk session on box-1, asked of the host under `key`.
fn start_on_box(key: &str) -> pb::StartDeskSessionRequest {
    pb::StartDeskSessionRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
        title: "Stuck notes".to_owned(),
        machine: Some(Rig::box_id(BOX)),
        access: Some(pb::DeskAccess { boards: true, channels_and_machines: false }),
        ..Default::default()
    }
}

/// Gives a folder its write permission back when dropped, so the test's
/// temporary folder can be removed even when an assertion fails.
struct Writable(PathBuf);

impl Drop for Writable {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

/// Every line the newest restart report of the daemon in `state` has for an
/// operation, in the order they were written, as `(id, kind, step, state)`.
fn operations_in(state: &Path) -> Vec<(String, String, String, String)> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(state.join("restarts"))
        .expect("the box wrote restart reports")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    folders.sort();
    let file = folders
        .iter()
        .rev()
        .map(|folder| folder.join("operations.jsonl"))
        .find(|file| file.exists())
        .expect("a restart report names the operations");
    let mut lines = Vec::new();
    for line in std::fs::read_to_string(file).unwrap().lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let field = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
        lines.push((field("id"), field("kind"), field("step"), field("state")));
    }
    lines
}

#[tokio::test(flavor = "multi_thread")]
async fn a_desk_start_cut_on_a_box_is_named_stuck_by_its_restart_and_a_fresh_start_works() {
    if rustix::process::geteuid().is_root() {
        eprintln!("skipped: root can write to a folder that has no write permission, so the undo this test needs to fail would succeed");
        return;
    }
    // The box is the `--admin` test daemon, so its restart's account is
    // read from its server info as the user.
    let mut rig = Rig::start_seen_with_env(&[BOX], &[REPO], &[("HUMATA2_STOP_AT", KILLED_AT)]).await;

    // The box dies in the middle of the start. The host's call must end with
    // the link, and with an error: it must not wait on a box that will not
    // answer, nor answer that the start is running.
    let mut client = desks(&rig).await;
    let answered = tokio::time::timeout(LONG, client.start_desk_session(start_on_box("desk-stuck")))
        .await
        .expect("the host's start didn't end after box-1 was killed");
    assert!(answered.is_err(), "the host answered a start box-1 was killed in: {answered:?}");
    rig.box_stopped(BOX).await;

    let listed = client
        .list_desk_sessions(pb::ListDeskSessionsRequest::default())
        .await
        .expect("the host lists its Desk sessions")
        .into_inner();
    assert!(listed.sessions.is_empty(), "the host kept no record of the start it was answered for: {:?}", listed.sessions);

    // The session box-1 kept: its folder holds the repository the start made.
    let kept: Vec<_> = rig
        .the(BOX)
        .store()
        .sessions()
        .expect("box-1's store reads")
        .into_iter()
        .filter(|session| session.channel == "desk")
        .collect();
    assert_eq!(kept.len(), 1, "box-1 kept the Desk session it was killed starting: {kept:?}");
    let (id, folder) = (kept[0].id.clone(), kept[0].folder.clone());
    assert!(folder.join(".git").is_dir(), "the folder holds the versions repository the start made");

    // The restart can't take the session back: finishing it fails (a Desk
    // session has no channel record), and a folder that isn't writable can't
    // be removed, so the undo fails too.
    let _writable = Writable(folder.clone());
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o500)).unwrap();
    rig.restart_box_env(BOX, &[]).await;
    let start = common::last_start(rig.the(BOX).daemon()).await;

    assert!(
        start.not_brought_back.iter().any(|line| line.session == id),
        "the restart names {id} as one it couldn't finish or remove: {:?}",
        start.not_brought_back
    );
    let operation = format!("start:{id}");
    let states: Vec<String> = operations_in(&rig.the(BOX).daemon().state)
        .into_iter()
        .filter(|(op, kind, _, _)| op == &operation && kind == "start")
        .map(|(_, _, _, state)| state)
        .collect();
    assert_eq!(states, ["pending", "stuck"], "the restart report names the start pending, then stuck");

    // A fresh start on box-1 works, in a folder of its own.
    let fresh = client
        .start_desk_session(start_on_box("desk-fresh"))
        .await
        .expect("a fresh start is asked")
        .into_inner();
    let operation = fresh.operation.clone().expect("a start has an operation");
    let ended = rig.ended(&operation).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "a fresh start on box-1 works: {:?}", ended.error);
    let fresh_folder = fresh.desk.and_then(|desk| desk.folder).expect("a start says where it works").path;
    assert_ne!(PathBuf::from(fresh_folder), folder, "the fresh start works in a folder of its own");
}
