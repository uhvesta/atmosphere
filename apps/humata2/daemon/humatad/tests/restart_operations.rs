//! PER-29, D241: the restart report names each operation the start found
//! cut short, with how it ended. A host killed mid-move or mid-fork writes
//! `restarts/<time>/operations.jsonl` at its next start: the operation, the
//! step it reached, and a terminal state once its recovery has settled.
//!
//! Every wait is on the host's feeds; the report is read only after the
//! session shows what the recovery left (it is written before that).

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");

/// What the newest report of the host's state folder says of each
/// operation: the last line per id, as `(id, kind, step, state)`.
fn operations(rig: &Rig) -> Vec<(String, String, String, String)> {
    let mut folders: Vec<_> = std::fs::read_dir(rig.host.state.join("restarts"))
        .expect("the host wrote restart reports")
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
    let mut latest: Vec<(String, String, String, String)> = Vec::new();
    for line in std::fs::read_to_string(file).unwrap().lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let field = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
        let entry = (field("id"), field("kind"), field("step"), field("state"));
        match latest.iter_mut().find(|known| known.0 == entry.0) {
            Some(known) => *known = entry,
            None => latest.push(entry),
        }
    }
    latest
}

fn move_request(id: &pb::SessionId, to: &pb::MachineId, key: &str) -> pb::MoveSessionRequest {
    pb::MoveSessionRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
        id: Some(id.clone()),
        to: Some(to.clone()),
    }
}

/// Moves `id` to `to` under a host killed at `at`, then starts the host again.
async fn move_killed_at(rig: &mut Rig, id: &pb::SessionId, to: &pb::MachineId, at: &str) {
    rig.restart_host(&[("HUMATA2_STOP_AT", at)]).await;
    let _ = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .move_session(move_request(id, to, &format!("move-killed-{at}")))
        .await;
    rig.host_stopped().await;
    rig.restart_host(&[]).await;
}

fn not_moving(session: &pb::Session) -> bool {
    session.status != pb::SessionStatus::Moving as i32
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_cut_short_is_named_in_the_restart_report_with_how_it_ended() {
    for (at, step, state, back_on_one) in [
        ("move-copied", "copied", "undone", true),
        ("move-switched", "switched", "finished", false),
    ] {
        let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
        let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
        let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
        let id = rig.start_on(&channel, &one, "Cache warmup").await;
        rig.wait_session(&id, "live", is_live).await;

        move_killed_at(&mut rig, &id, &two, at).await;

        let settled = if back_on_one { &one } else { &two };
        rig.wait_session(&id, "settled", |s| s.machine.as_ref() == Some(settled) && is_live(s) && not_moving(s)).await;
        let named = operations(&rig);
        assert_eq!(named.len(), 1, "{at}: {named:?}");
        let (_, kind, reached, ended) = &named[0];
        assert_eq!((kind.as_str(), reached.as_str(), ended.as_str()), ("move", step, state), "{at}");
    }
}
