//! A Desk start on a box whose host dies between the ask and the answer
//! (D731): the box has started the session, and the host is killed before it
//! keeps the answer. On its restart the host asks the box again under the
//! same id rather than undoing the start: the box's session and its folder
//! survive, the box starts nothing twice, and the host's record names the
//! folder the box works in. Waits are bounded by `LONG`.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use remote_rig::{LONG, Rig};
use tonic::transport::Channel;

const BOX: &str = "box-1";
const REPO: (&str, &str) = ("acme", "api");
/// Where the host is killed: the box has answered the start, and the host
/// hasn't kept the answer.
const KILLED_AT: &str = "desk.remote.answered";
/// What the host's restart report calls a Desk start it asked a box for.
const KIND: &str = "remote_desk_start";

async fn desks(rig: &Rig) -> pb::desk_service_client::DeskServiceClient<Channel> {
    pb::desk_service_client::DeskServiceClient::new(rig.channel().await)
}

/// The Desk sessions box-1's store holds, as `(id, folder)`.
fn on_box(rig: &Rig) -> Vec<(String, PathBuf)> {
    rig.the(BOX)
        .store()
        .sessions()
        .expect("box-1's store reads")
        .into_iter()
        .filter(|session| session.channel == "desk")
        .map(|session| (session.id, session.folder))
        .collect()
}

/// The states the newest restart report of the daemon in `state` gives the
/// operation `id` of `kind`, in the order they were written.
fn states_in(state: &Path, id: &str, kind: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(state.join("restarts")) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    folders.sort();
    let Some(file) = folders
        .iter()
        .rev()
        .map(|folder| folder.join("operations.jsonl"))
        .find(|file| file.exists())
    else {
        return Vec::new();
    };
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value["id"] == id && value["kind"] == kind)
        .map(|value| value["state"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The states the host's restart gives the start of `id`, once it says more
/// than that the start waits.
async fn settled(rig: &Rig, id: &str) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        let states = states_in(&rig.host.state, id, KIND);
        if states.iter().any(|state| state != "pending") {
            return states;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the host's restart never settled the start of {id}: {states:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_killed_before_a_desk_starts_answer_asks_the_box_again_and_keeps_the_session() {
    let mut rig = Rig::start_with_env(&[BOX], &[REPO], &[("HUMATA2_STOP_AT", KILLED_AT)], &[]).await;

    // The host dies once box-1 has answered: the caller hears no answer.
    let answered = tokio::time::timeout(
        LONG,
        desks(&rig).await.start_desk_session(pb::StartDeskSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "desk-host-cut".to_owned() }),
            title: "Cut notes".to_owned(),
            machine: Some(Rig::box_id(BOX)),
            access: Some(pb::DeskAccess { boards: true, channels_and_machines: false }),
            ..Default::default()
        }),
    )
    .await
    .expect("the host's start didn't end after the host was killed");
    assert!(answered.is_err(), "a host killed before it kept the answer answered: {answered:?}");
    rig.host_stopped().await;
    let kept = on_box(&rig);
    assert_eq!(kept.len(), 1, "box-1 started the Desk session it was asked for: {kept:?}");
    let (id, folder) = kept[0].clone();

    rig.restart_host(&[]).await;
    let states = settled(&rig, &id).await;

    assert_eq!(states, ["pending", "finished"], "the host asked box-1 again and finished the start");
    assert_eq!(on_box(&rig), kept, "box-1's session survives, and nothing was started twice");
    assert!(folder.is_dir(), "box-1's session keeps its folder");
    let listed = desks(&rig)
        .await
        .list_desk_sessions(pb::ListDeskSessionsRequest::default())
        .await
        .expect("the host lists its Desk sessions")
        .into_inner()
        .sessions;
    let host_view = listed
        .iter()
        .find(|desk| desk.session.as_ref().is_some_and(|session| session.value == id))
        .unwrap_or_else(|| panic!("the host kept Desk session {id}: {listed:?}"));
    let shown = host_view.folder.as_ref().map(|folder| PathBuf::from(&folder.path));
    assert_eq!(shown, Some(folder), "the host's record names the folder box-1 works in");
}
