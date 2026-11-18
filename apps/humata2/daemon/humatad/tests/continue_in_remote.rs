//! Continue in another channel (CV-8, D307) of a session on another
//! machine: the continue runs on that machine, into a channel the machine
//! is in, as it continues any of its own sessions. The new session's
//! folder, clones and conversation copy are there; the session it came
//! from runs on untouched. A channel the machine isn't in is refused, as
//! is a machine at another release, before anything is made. A link cut
//! part way, a host restarted while the link is down, and a host cut
//! between keeping the continue and asking for it list nothing half-made,
//! and once the machine is back the new session is listed once (D554). A
//! replay of the caller's key while the continue runs names that session.
//!
//! Waits are on the host's feeds (the rig's `wait_session`, `ended`) and on
//! a FIFO hold point, never on a timer.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;
#[path = "common/upgrade.rs"]
mod upgrade;
#[path = "common/upgrade_rig.rs"]
mod upgrade_rig;

use std::path::{Path, PathBuf};

use common::error_of;
use humata2_grpc::humata::v1 as pb;
use remote_rig::{LONG, Rig, is_live};
use tonic::{Code, Status};

const REPO: (&str, &str) = ("acme", "api");
const BOX: &str = "box-2";

async fn say(rig: &Rig, id: &pb::SessionId, line: &str) {
    let tab = common::agent_tab(&rig.host, id).await;
    let open = pb::AttachOpen {
        session: Some(id.clone()),
        ..common::open(&tab, 0)
    };
    let mut attach = common::Attach::start(&rig.host, open).await;
    attach.wait_for("agent up").await;
    attach.type_text(&format!("{line}\n")).await;
    attach.wait_for(&format!("got: {line}")).await;
}

async fn continue_in(rig: &Rig, source: &pb::SessionId, channel: &pb::Channel) -> Result<pb::StartSessionResponse, Status> {
    continue_keyed(rig, source, channel, None).await
}

/// As [`continue_in`], under the idempotency key `key` when given.
async fn continue_keyed(
    rig: &Rig,
    source: &pb::SessionId,
    channel: &pb::Channel,
    key: Option<&str>,
) -> Result<pb::StartSessionResponse, Status> {
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .continue_in_channel(pb::ContinueInChannelRequest {
            idempotency_key: key.map(|value| pb::IdempotencyKey { value: value.to_owned() }),
            session: Some(source.clone()),
            channel: channel.id.clone(),
            before_turn: 0,
        })
        .await
        .map(tonic::Response::into_inner)
}

/// The sessions the host lists.
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

/// Waits until the host lists `id`: read, and read again each time its
/// Sessions feed tells of a change.
async fn wait_listed(rig: &Rig, id: &pb::SessionId) {
    let mut feed = pb::event_service_client::EventServiceClient::new(rig.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Sessions as i32],
            ..Default::default()
        })
        .await
        .expect("the host's Sessions feed opens")
        .into_inner();
    let deadline = tokio::time::Instant::now() + LONG;
    while !listed(rig).await.contains(id) {
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            Ok(ended) => panic!("the host's Sessions feed ended ({ended:?}) before it listed {}", id.value),
            Err(_) => panic!("the host never listed {} within {LONG:?}", id.value),
        }
    }
}

/// Waits until the host lists `count` sessions, read again each time its
/// Sessions feed tells of a change, and returns them.
async fn wait_count(rig: &Rig, count: usize) -> Vec<pb::SessionId> {
    let mut feed = pb::event_service_client::EventServiceClient::new(rig.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Sessions as i32],
            ..Default::default()
        })
        .await
        .expect("the host's Sessions feed opens")
        .into_inner();
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        let now = listed(rig).await;
        if now.len() >= count {
            return now;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            Ok(ended) => panic!("the host's Sessions feed ended ({ended:?}) with {} sessions listed", now.len()),
            Err(_) => panic!("the host never listed {count} sessions within {LONG:?}: {now:?}"),
        }
    }
}

/// `ids` sorted, to compare as sets.
fn sorted(mut ids: Vec<pb::SessionId>) -> Vec<pb::SessionId> {
    ids.sort_by(|a, b| a.value.cmp(&b.value));
    ids
}

/// The session the ended continue made.
fn made_by(ended: &pb::Operation) -> pb::SessionId {
    match &ended.result {
        Some(pb::operation::Result::Session(id)) => id.clone(),
        _ => panic!("the continue named no session: {ended:?}"),
    }
}

/// Restarts the host so a continue of a box session holds once the box
/// answered, until the returned FIFO is opened for writing.
async fn hold_continues(rig: &mut Rig) -> PathBuf {
    let hold = rig.host.dir.path().join("hold-continue.fifo");
    assert!(std::process::Command::new("mkfifo").arg(&hold).status().unwrap().success());
    let named = hold.display().to_string();
    rig.restart_host(&[("HUMATA2_HOLD_AT", "continue-there-following"), ("HUMATA2_HOLD_FIFO", named.as_str())])
        .await;
    hold
}

/// Lets the held continue go on; it opens the FIFO once it gets there.
async fn release(hold: &Path) {
    let hold = hold.to_owned();
    let opened = tokio::task::spawn_blocking(move || std::fs::OpenOptions::new().write(true).open(hold).map(drop));
    tokio::time::timeout(LONG, opened)
        .await
        .expect("the continue never got to where it is held")
        .unwrap()
        .expect("the held continue's FIFO opens");
}

/// Box-2 runs the daemon as shipped, in the machine role, with no test
/// access: the continue the host relays passes box-2's own relay allowlist
/// and access gate, or box-2 makes nothing and the operation fails.
#[tokio::test(flavor = "multi_thread")]
async fn a_box_session_continues_in_a_channel_on_that_box_and_runs_there() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let two = Rig::box_id(BOX);
    let payments = rig.create_channel("Payments", &[REPO], &[two.clone()]).await;
    let billing = rig.create_channel("Billing", &[REPO], &[two.clone()]).await;
    let source = rig.start_on(&payments, &two, "Cache warmup").await;
    rig.wait_session(&source, "live", is_live).await;
    say(&rig, &source, "remember the retry").await;
    let machine = rig.the(BOX);
    let holder = machine.tabs()[0].holder_pid;
    let conversation = machine.conversation_id(&source);

    let started = continue_in(&rig, &source, &billing).await.expect("the continue starts");
    let ended = rig.ended(started.operation.as_ref().expect("a continue has an operation")).await;
    assert_eq!(ended.state(), pb::OperationState::Succeeded, "{:?}", ended.error);
    let id = made_by(&ended);
    let child = rig
        .wait_session(&id, "live on box-2", |session| session.machine.as_ref() == Some(&two) && is_live(session))
        .await;
    assert_eq!(child.channel, billing.id, "{child:?}");

    let detail = machine
        .store()
        .session_detail(&id.value)
        .unwrap()
        .expect("box-2 has the new session's start record");
    let in_billing = machine.path(&billing, "sessions");
    assert!(!detail.repos.is_empty(), "{detail:?}");
    assert!(
        detail.repos.iter().all(|repo| repo.path.starts_with(&in_billing)),
        "its clones are in Billing on box-2: {detail:?}"
    );

    let theirs = machine.conversation_id(&id);
    assert_ne!(theirs, conversation, "the new session's conversation has its own id");
    let talk = std::fs::read_to_string(machine.conversation(&theirs)).unwrap_or_default();
    assert!(talk.contains("remember the retry"), "the conversation came along: {talk}");

    let tabs = machine.tabs();
    assert_eq!(tabs.len(), 2, "the source's agent and the new session's: {tabs:?}");
    assert!(tabs.iter().any(|tab| tab.holder_pid == holder), "the source's agent runs on");
    assert_eq!(machine.conversation_id(&source), conversation, "the source keeps its conversation");
    let after = rig.session(&source).await;
    assert_eq!(after.channel, payments.id, "the source stays in Payments");
    assert!(is_live(&after), "{after:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_session_continued_in_a_channel_its_box_isnt_on_is_refused_and_nothing_changes() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let (host, two) = (rig.host_id().await, Rig::box_id(BOX));
    let payments = rig.create_channel("Payments", &[REPO], &[two.clone()]).await;
    let billing = rig.create_channel("Billing", &[REPO], &[host]).await;
    let source = rig.start_on(&payments, &two, "Cache warmup").await;
    rig.wait_session(&source, "live", is_live).await;
    let tabs = rig.the(BOX).tabs().len();

    let refused = continue_in(&rig, &source, &billing).await.expect_err("a channel box-2 isn't on is refused");

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(
        matches!(error_of(&refused).kind, Some(pb::error::Kind::InvalidField(_))),
        "the refusal names the channel: {refused:?}"
    );
    assert_eq!(listed(&rig).await, vec![source.clone()], "no new session");
    assert_eq!(rig.the(BOX).tabs().len(), tabs, "nothing started on box-2");
    assert!(is_live(&rig.session(&source).await));
}

#[tokio::test(flavor = "multi_thread")]
async fn continuing_a_session_on_a_machine_at_another_release_is_refused_with_its_item() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    let gpu = upgrade_rig::box_id();
    let payments = rig.create_channel("Payments", &[REPO], &[gpu.clone()]).await;
    let billing = rig.create_channel("Billing", &[REPO], &[gpu.clone()]).await;
    let source = rig.start_on(&payments, &gpu, "On gpu").await;
    rig.wait_session(&source, "live", is_live).await;
    upgrade_rig::box_at_release(&mut rig, "b").await;
    let item = upgrade_rig::daemon_item(&rig).await;
    let tabs = rig.the(upgrade_rig::BOX).tabs().len();

    let refused = continue_in(&rig, &source, &billing).await.expect_err("a continue across releases is refused");

    let needed = upgrade::migration_needed(&error_of(&refused));
    assert_eq!(needed.thing(), pb::MigrateThing::Daemon, "{needed:?}");
    assert_eq!(needed.machine, Some(gpu), "{needed:?}");
    assert_eq!(needed.request, item.id, "{needed:?}");
    assert_eq!(rig.the(upgrade_rig::BOX).tabs().len(), tabs, "nothing started on gpu");
    assert_eq!(listed(&rig).await, vec![source], "no new session");
}

/// The link to box-2 cut while it continues: the host's operation fails and
/// lists nothing half-made; box-2 finishes on its own, and once its link is
/// back the host lists the new session, once, on box-2.
#[tokio::test(flavor = "multi_thread")]
async fn a_continue_whose_link_is_cut_lists_nothing_half_made_and_the_session_once_box_2_is_back() {
    let mut rig = Rig::start(&[BOX], &[REPO]).await;
    let two = Rig::box_id(BOX);
    let payments = rig.create_channel("Payments", &[REPO], &[two.clone()]).await;
    let billing = rig.create_channel("Billing", &[REPO], &[two.clone()]).await;
    let source = rig.start_on(&payments, &two, "Cache warmup").await;
    let hold = hold_continues(&mut rig).await;
    rig.wait_session(&source, "live again", is_live).await;
    say(&rig, &source, "remember the retry").await;

    let started = continue_in(&rig, &source, &billing).await.expect("the continue starts");
    let made = started.session.clone().expect("box-2 named the session it makes");
    rig.link_lost(BOX).await;
    release(&hold).await;
    let ended = rig.ended(started.operation.as_ref().expect("a continue has an operation")).await;
    assert_eq!(ended.state(), pb::OperationState::Failed, "the host can't tell how it went: {ended:?}");
    assert!(!listed(&rig).await.contains(&made), "nothing half-made is listed");

    rig.link_back(BOX).await;
    wait_listed(&rig, &made).await;
    let child = rig
        .wait_session(&made, "live on box-2", |session| session.machine.as_ref() == Some(&two) && is_live(session))
        .await;
    assert_eq!(child.channel, billing.id, "{child:?}");
    let mut sessions = listed(&rig).await;
    sessions.sort_by(|a, b| a.value.cmp(&b.value));
    let mut want = vec![source.clone(), made.clone()];
    want.sort_by(|a, b| a.value.cmp(&b.value));
    assert_eq!(sessions, want, "the source and the new session, once each");
    assert_eq!(rig.the(BOX).store().sessions().unwrap().len(), 2, "box-2 made one new session");
    assert!(is_live(&rig.session(&source).await));
}

/// A replay of the caller's key while the continue is held on box-2 answers
/// with the same operation and the session box-2 named, and makes nothing
/// more; once it ends a replay names the same session.
#[tokio::test(flavor = "multi_thread")]
async fn replaying_the_key_while_the_continue_runs_answers_with_the_session_it_makes() {
    let mut rig = Rig::start(&[BOX], &[REPO]).await;
    let two = Rig::box_id(BOX);
    let payments = rig.create_channel("Payments", &[REPO], &[two.clone()]).await;
    let billing = rig.create_channel("Billing", &[REPO], &[two.clone()]).await;
    let source = rig.start_on(&payments, &two, "Cache warmup").await;
    let hold = hold_continues(&mut rig).await;
    rig.wait_session(&source, "live again", is_live).await;
    say(&rig, &source, "remember the retry").await;

    let started = continue_keyed(&rig, &source, &billing, Some("continue-once"))
        .await
        .expect("the continue starts");
    let operation = started.operation.clone().expect("a continue has an operation");
    let made = started.session.clone().expect("box-2 named the session it makes");
    let again = continue_keyed(&rig, &source, &billing, Some("continue-once"))
        .await
        .expect("a replay of the key answers");
    let replayed = again.operation.clone().expect("the replay has the operation");
    assert_eq!(replayed.id, operation.id, "the replay is the same continue");
    assert_eq!(replayed.state(), pb::OperationState::Running, "it is still held: {replayed:?}");
    assert_eq!(again.session, Some(made.clone()), "the replay names the session the continue makes");

    release(&hold).await;
    let ended = rig.ended(&operation).await;
    assert_eq!(ended.state(), pb::OperationState::Succeeded, "{:?}", ended.error);
    assert_eq!(made_by(&ended), made);
    wait_listed(&rig, &made).await;
    let after = continue_keyed(&rig, &source, &billing, Some("continue-once"))
        .await
        .expect("a replay after the end answers");
    assert_eq!(after.session, Some(made.clone()), "{after:?}");
    assert_eq!(sorted(listed(&rig).await), sorted(vec![source, made]), "the source and the new session, once each");
    assert_eq!(rig.the(BOX).store().sessions().unwrap().len(), 2, "box-2 made one new session");
}

/// The link to box-2 cut while it continues, then the host restarted while
/// the link is still down: nothing half-made is listed, and once the link
/// is back the restarted host asks box-2 again under the same key and
/// lists the new session once, with its clones on box-2.
#[tokio::test(flavor = "multi_thread")]
async fn a_continue_cut_by_a_lost_link_and_a_host_restart_is_listed_once_when_box_2_is_back() {
    let mut rig = Rig::start(&[BOX], &[REPO]).await;
    let two = Rig::box_id(BOX);
    let payments = rig.create_channel("Payments", &[REPO], &[two.clone()]).await;
    let billing = rig.create_channel("Billing", &[REPO], &[two.clone()]).await;
    let source = rig.start_on(&payments, &two, "Cache warmup").await;
    let hold = hold_continues(&mut rig).await;
    rig.wait_session(&source, "live again", is_live).await;
    say(&rig, &source, "remember the retry").await;

    let started = continue_in(&rig, &source, &billing).await.expect("the continue starts");
    let made = started.session.clone().expect("box-2 named the session it makes");
    rig.link_lost(BOX).await;
    release(&hold).await;
    let ended = rig.ended(started.operation.as_ref().expect("a continue has an operation")).await;
    assert_eq!(ended.state(), pb::OperationState::Failed, "the host can't tell how it went: {ended:?}");

    rig.restart_host_unlinked(&[]).await;
    assert!(!listed(&rig).await.contains(&made), "nothing half-made is listed after the restart");

    rig.link_back(BOX).await;
    wait_listed(&rig, &made).await;
    let child = rig
        .wait_session(&made, "live on box-2", |session| session.machine.as_ref() == Some(&two) && is_live(session))
        .await;
    assert_eq!(child.channel, billing.id, "{child:?}");
    let in_billing = rig.the(BOX).path(&billing, "sessions");
    assert!(!child.repos.is_empty(), "the host's record has the new session's clones: {child:?}");
    assert!(
        child.repos.iter().all(|repo| Path::new(&repo.path).starts_with(&in_billing)),
        "its clones are in Billing on box-2: {child:?}"
    );
    assert_eq!(sorted(listed(&rig).await), sorted(vec![source.clone(), made]), "the source and the new session, once each");
    assert_eq!(rig.the(BOX).store().sessions().unwrap().len(), 2, "box-2 made one new session");
    assert!(is_live(&rig.session(&source).await));
}

/// The host cut after it kept the continue and before it asked box-2: the
/// next start asks box-2 under the kept key once it links, and the host
/// lists the new session once, whole, with box-2 having made one.
#[tokio::test(flavor = "multi_thread")]
async fn a_host_cut_between_keeping_and_asking_lists_the_continue_once_after_its_restart() {
    let mut rig = Rig::start(&[BOX], &[REPO]).await;
    let two = Rig::box_id(BOX);
    let payments = rig.create_channel("Payments", &[REPO], &[two.clone()]).await;
    let billing = rig.create_channel("Billing", &[REPO], &[two.clone()]).await;
    let source = rig.start_on(&payments, &two, "Cache warmup").await;
    rig.restart_host(&[("HUMATA2_STOP_AT", "continue-there-journaled")]).await;
    rig.wait_session(&source, "live again", is_live).await;
    say(&rig, &source, "remember the retry").await;

    continue_in(&rig, &source, &billing).await.expect_err("the host stops before it answers");
    rig.host_stopped().await;
    assert_eq!(rig.the(BOX).store().sessions().unwrap().len(), 1, "box-2 wasn't asked");

    rig.restart_host(&[]).await;
    let sessions = wait_count(&rig, 2).await;
    let made = sessions
        .iter()
        .find(|id| **id != source)
        .cloned()
        .expect("the new session is listed");
    let child = rig
        .wait_session(&made, "live on box-2", |session| session.machine.as_ref() == Some(&two) && is_live(session))
        .await;
    assert_eq!(child.channel, billing.id, "{child:?}");
    assert!(!child.repos.is_empty(), "the host's record has the new session's clones: {child:?}");
    assert_eq!(sorted(listed(&rig).await), sorted(vec![source.clone(), made]), "the source and the new session, once each");
    assert_eq!(rig.the(BOX).store().sessions().unwrap().len(), 2, "box-2 made one new session");
}
