//! A remote continue's journal that can't be read (D554). The host's
//! journal of a continue it asked another machine for is one file in the
//! carry's `remote_archive/continue` folder. A file that is garbage, cut
//! short or not a continue ends in the restart report as a stuck
//! `remote_continue` for the operation its name says, is moved aside as a
//! `.bad` file with its contents, is never asked of the machine, and is not
//! met again at the next start. A readable journal beside it still goes on.
//!
//! Waits are on the host's feeds, never on a timer.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;
#[path = "common/upgrade.rs"]
mod upgrade;
#[path = "common/upgrade_rig.rs"]
mod upgrade_rig;

use std::path::PathBuf;

use humata2_grpc::humata::v1 as pb;
use remote_rig::{LONG, Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");
const BOX: &str = "box-2";

/// Journals that can't be read, as `(operation, contents)`.
const BAD: [(&str, &str); 3] = [
    ("op_garbage", "this is not a journal"),
    ("op_truncated", "{\"operation\": \"op_truncated\", \"machine\": \"box-2\", \"chan"),
    ("op_wrong_shape", "{\"operation\": \"op_wrong_shape\"}"),
];

fn live_folder(rig: &Rig) -> PathBuf {
    rig.host.state.join("carry").join("remote_archive").join("continue")
}

/// Puts `BAD` in the host's journal folder while it is stopped.
fn plant_bad(rig: &Rig) {
    let folder = live_folder(rig);
    std::fs::create_dir_all(&folder).unwrap();
    for (operation, contents) in BAD {
        std::fs::write(folder.join(format!("{operation}.json")), contents).unwrap();
    }
}

/// Each line the report of `daemon`'s last start has, as `(id, kind,
/// state, reason)`, once that start has written it. A start that found no
/// operations writes no `operations.jsonl`: its report has no lines.
async fn newest_report(daemon: &common::Daemon) -> Vec<(String, String, String, String)> {
    // The report is whole once its restart.json is there, which is before
    // the start says it brought everything back.
    common::last_start(daemon).await;
    let mut folders: Vec<PathBuf> = std::fs::read_dir(daemon.state.join("restarts"))
        .expect("the host wrote restart reports")
        .flatten()
        .map(|entry| entry.path())
        .filter(|folder| folder.join("restart.json").exists())
        .collect();
    folders.sort();
    let newest = folders.last().expect("the host wrote a whole restart report");
    let text = std::fs::read_to_string(newest.join("operations.jsonl")).unwrap_or_default();
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .map(|value| {
            let field = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
            (field("id"), field("kind"), field("state"), field("reason"))
        })
        .collect()
}

/// The last state the report gives `operation` as a `remote_continue`.
fn last_state(report: &[(String, String, String, String)], operation: &str) -> Option<(String, String)> {
    report
        .iter()
        .rev()
        .find(|(id, kind, _, _)| id == operation && kind == "remote_continue")
        .map(|(_, _, state, reason)| (state.clone(), reason.clone()))
}

fn assert_set_aside(rig: &Rig) {
    let folder = live_folder(rig);
    for (operation, contents) in BAD {
        assert!(!folder.join(format!("{operation}.json")).exists(), "{operation} is out of the live journal folder");
        assert_eq!(
            std::fs::read_to_string(folder.join(format!("{operation}.json.bad"))).unwrap(),
            contents,
            "{operation} is kept as it was"
        );
    }
}

async fn listed(rig: &Rig) -> Vec<pb::SessionId> {
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .expect("the host lists its sessions")
        .into_inner()
        .sessions
        .into_iter()
        .filter_map(|session| session.id)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn journals_that_cant_be_read_end_stuck_in_the_report_and_are_set_aside_not_retried() {
    let mut rig = Rig::start(&[BOX], &[REPO]).await;
    plant_bad(&rig);

    rig.restart_host(&[]).await;
    let report = newest_report(&rig.host).await;
    for (operation, _) in BAD {
        let (state, reason) = last_state(&report, operation)
            .unwrap_or_else(|| panic!("the report names {operation} as a remote_continue: {report:?}"));
        assert_eq!(state, "stuck", "{operation} ends visibly: {report:?}");
        assert!(!reason.is_empty(), "{operation} says what to do");
    }
    assert_set_aside(&rig);
    assert!(listed(&rig).await.is_empty(), "nothing was made or listed from an unreadable journal");

    // The next start doesn't meet them again.
    rig.restart_host(&[]).await;
    let report = newest_report(&rig.host).await;
    for (operation, _) in BAD {
        assert_eq!(last_state(&report, operation), None, "{operation} isn't retried: {report:?}");
    }
    assert_set_aside(&rig);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_readable_journal_beside_a_bad_one_is_still_finished_once() {
    let mut rig = Rig::start(&[BOX], &[REPO]).await;
    let two = Rig::box_id(BOX);
    let payments = rig.create_channel("Payments", &[REPO], &[two.clone()]).await;
    let billing = rig.create_channel("Billing", &[REPO], &[two.clone()]).await;
    let source = rig.start_on(&payments, &two, "Cache warmup").await;
    rig.restart_host(&[("HUMATA2_STOP_AT", "continue-there-journaled")]).await;
    rig.wait_session(&source, "live again", is_live).await;

    let asked = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .continue_in_channel(pb::ContinueInChannelRequest {
            idempotency_key: None,
            session: Some(source.clone()),
            channel: billing.id.clone(),
            before_turn: 0,
        })
        .await;
    asked.expect_err("the host stops before it answers");
    rig.host_stopped().await;
    plant_bad(&rig);

    rig.restart_host(&[]).await;
    let report = newest_report(&rig.host).await;
    for (operation, _) in BAD {
        assert_eq!(last_state(&report, operation).map(|(state, _)| state).as_deref(), Some("stuck"), "{report:?}");
    }
    assert_set_aside(&rig);

    // The good journal's continue is asked and listed once.
    let deadline = tokio::time::Instant::now() + LONG;
    let mut feed = pb::event_service_client::EventServiceClient::new(rig.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Sessions as i32],
            ..Default::default()
        })
        .await
        .expect("the host's Sessions feed opens")
        .into_inner();
    loop {
        let now = listed(&rig).await;
        if now.len() >= 2 {
            assert_eq!(now.len(), 2, "the source and the new session, once each: {now:?}");
            break;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            other => panic!("the readable journal's continue was never listed ({other:?}): {now:?}"),
        }
    }
    assert_eq!(rig.the(BOX).store().sessions().unwrap().len(), 2, "box-2 made one new session");
}
