//! Rebuild a forgotten machine's session on another machine (SL-4, LD-11,
//! flow 26.23 steps 4-5; D195): each repo is cloned again at its branch, at
//! the last pushed sha when it had unpushed commits; the conversation
//! resumes from the host's mirror; the agent is told what was lost. A host
//! that dies part way leaves the session archived and rebuildable.
//!
//! Waits are on the host's feeds (the rig's `wait_session`, `ended`).

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use common::git;
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::transport::Channel;

const REPO: (&str, &str) = ("acme", "api");
const CLONE: &str = "sessions/cache-warmup/api";

async fn sessions(rig: &Rig) -> pb::session_service_client::SessionServiceClient<Channel> {
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
}

async fn machines(rig: &Rig) -> pb::machine_service_client::MachineServiceClient<Channel> {
    pb::machine_service_client::MachineServiceClient::new(rig.channel().await)
}

async fn attach(rig: &Rig, id: &pb::SessionId) -> common::Attach {
    let tab = common::agent_tab(&rig.host, id).await;
    let open = pb::AttachOpen {
        session: Some(id.clone()),
        ..common::open(&tab, 0)
    };
    common::Attach::start(&rig.host, open).await
}

async fn say(rig: &Rig, id: &pb::SessionId, line: &str) {
    let mut agent = attach(rig, id).await;
    agent.wait_for("agent up").await;
    agent.type_text(&format!("{line}\n")).await;
    agent.wait_for(&format!("got: {line}")).await;
}

/// A session on box-1 whose branch was pushed at one sha, then got an
/// unpushed commit and a dirty file; box-1 then dies and is forgotten.
/// Returns the pushed sha.
async fn forgotten(rig: &mut Rig, channel: &pb::Channel) -> (pb::SessionId, String) {
    forgotten_after(rig, channel, 0).await
}

/// As [`forgotten`], with `padding` JSON lines of a KiB each written to
/// the agent's conversation before it is told "before the loss", and the
/// host's mirror holding all of them before box-1 dies.
async fn forgotten_after(rig: &mut Rig, channel: &pb::Channel, padding: usize) -> (pb::SessionId, String) {
    let one = Rig::box_id("box-1");
    let id = rig.start_on(channel, &one, "Cache warmup").await;
    let session = rig.wait_session(&id, "live", is_live).await;
    if padding > 0 {
        let file = rig.the("box-1").conversation(&rig.the("box-1").conversation_id(&id));
        let line = format!("{{\"text\":\"padding {}\"}}\n", "x".repeat(1000));
        let mut written = std::fs::OpenOptions::new().append(true).create(true).open(&file).unwrap();
        std::io::Write::write_all(&mut written, line.repeat(padding).as_bytes()).unwrap();
    }
    // A user message in Claude's JSONL, as real agents write: the raw lines keep
    // only JSON lines (D189.4), and the mirror's turns only messages.
    say(rig, &id, r#"{"type":"user","message":{"content":"before the loss"}}"#).await;
    if padding > 0 {
        mirrored(rig, &id, "before the loss").await;
    }
    let clone = rig.the("box-1").path(channel, CLONE).display().to_string();
    let branch = &session.repos[0].branch;
    std::fs::write(format!("{clone}/pushed.txt"), "pushed\n").unwrap();
    git(&["-C", &clone, "add", "pushed.txt"]);
    git(&["-C", &clone, "commit", "-m", "pushed work"]);
    git(&["-C", &clone, "push", "-u", "origin", branch]);
    let pushed = git(&["-C", &clone, "rev-parse", "HEAD"]);
    std::fs::write(format!("{clone}/lost.txt"), "lost\n").unwrap();
    git(&["-C", &clone, "add", "lost.txt"]);
    git(&["-C", &clone, "commit", "-m", "lost work"]);
    std::fs::write(format!("{clone}/README.md"), "dirty on the box\n").unwrap();
    rig.wait_session(&id, "showing the lost work", |s| {
        s.pointers.iter().any(|p| p.unpushed_commits == 1 && p.dirty_files == 1)
    })
    .await;

    rig.the_mut("box-1").kill();
    rig.link_lost("box-1").await;
    let plan = machines(rig).await.plan_forget_machine(pb::PlanForgetMachineRequest { id: Some(one.clone()) }).await.expect("the plan is made").into_inner();
    machines(rig)
        .await
        .forget_machine(pb::ForgetMachineRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "forget-1".to_owned() }),
            id: Some(one),
            plan: plan.token,
            presence: None,
        })
        .await
        .expect("box-1 is forgotten");
    rig.wait_session(&id, "archived as lost", |s| s.archive.as_ref().is_some_and(|a| a.machine_lost)).await;
    (id, pushed)
}

/// The texts of every entry the host's mirror holds for `id`.
async fn mirror_texts(rig: &Rig, id: &pb::SessionId) -> Vec<String> {
    let mut history = pb::history_service_client::HistoryServiceClient::new(rig.channel().await);
    let mut request = pb::ReadTurnsRequest { session: Some(id.clone()), ..Default::default() };
    let mut texts = Vec::new();
    loop {
        let response = history.read_turns(request.clone()).await.expect("the mirror reads").into_inner();
        texts.extend(response.entries.into_iter().map(|entry| entry.text));
        let next = response.page.map(|page| page.next_page_token).unwrap_or_default();
        if next.is_empty() {
            return texts;
        }
        request.page = Some(pb::PageRequest { page_size: 0, page_token: next });
    }
}

/// Waits until the host's mirror of `id` holds an entry with `text`. The
/// mirror follows the machine on its own and tells no feed, so this asks
/// again until it does, within the rig's long wait.
async fn mirrored(rig: &Rig, id: &pb::SessionId, text: &str) {
    let deadline = std::time::Instant::now() + remote_rig::LONG;
    while !mirror_texts(rig, id).await.iter().any(|entry| entry.contains(text)) {
        assert!(std::time::Instant::now() < deadline, "the host's mirror never held {text:?}");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn rebuild(rig: &Rig, id: &pb::SessionId, on: &pb::MachineId, key: &str) -> pb::Operation {
    let started = sessions(rig)
        .await
        .rebuild_session(pb::RebuildSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
            id: Some(id.clone()),
            machine: Some(on.clone()),
        })
        .await
        .expect("the rebuild starts")
        .into_inner();
    rig.ended(&started).await
}

async fn rig_and_channel() -> (Rig, pb::Channel) {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let machines = [rig.host_id().await, Rig::box_id("box-1"), Rig::box_id("box-2")];
    let channel = rig.create_channel("Payments", &[REPO], &machines).await;
    (rig, channel)
}

/// 26.23 step 4: rebuilt on box-2 at the last pushed sha, the conversation
/// resumed from the mirror, and the agent told of the lost commit and file.
#[tokio::test(flavor = "multi_thread")]
async fn rebuild_on_box_2_clones_last_pushed_sha_resumes_mirrored_conversation_and_names_lost_work() {
    let (mut rig, channel) = rig_and_channel().await;
    let (id, pushed) = forgotten(&mut rig, &channel).await;
    let two = Rig::box_id("box-2");

    let ended = rebuild(&rig, &id, &two, "rebuild-1").await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    let back = rig.wait_session(&id, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s)).await;
    assert!(back.archive.is_none());
    let clone = rig.the("box-2").path(&channel, CLONE).display().to_string();
    assert_eq!(git(&["-C", &clone, "rev-parse", "HEAD"]), pushed, "rebuilt at the last pushed sha");
    assert_eq!(git(&["-C", &clone, "rev-parse", "--abbrev-ref", "HEAD"]), back.repos[0].branch);
    assert!(!std::path::Path::new(&clone).join("lost.txt").exists());

    let box_two = rig.the("box-2");
    let talk = std::fs::read_to_string(box_two.conversation(&box_two.conversation_id(&id))).unwrap();
    assert!(talk.contains("before the loss"), "resumed from the mirror: {talk}");
    let mut agent = attach(&rig, &id).await;
    agent.wait_for("README.md").await;
}

/// A host killed while the rebuild clones leaves the session archived; a
/// second rebuild after the restart finishes it.
#[tokio::test(flavor = "multi_thread")]
async fn rebuild_killed_at_rebuild_clone_api_stays_archived_and_rebuildable() {
    let (mut rig, channel) = rig_and_channel().await;
    let (id, pushed) = forgotten(&mut rig, &channel).await;
    let two = Rig::box_id("box-2");
    rig.restart_host(&[("HUMATA2_STOP_AT", "rebuild-clone-api")]).await;

    let _ = sessions(&rig)
        .await
        .rebuild_session(pb::RebuildSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "rebuild-killed".to_owned() }),
            id: Some(id.clone()),
            machine: Some(two.clone()),
        })
        .await;
    rig.restart_host(&[]).await;

    let after = rig.session(&id).await;
    assert!(after.archive.as_ref().is_some_and(|a| a.machine_lost), "still archived as lost: {after:?}");

    let ended = rebuild(&rig, &id, &two, "rebuild-2").await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    rig.wait_session(&id, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s)).await;
    let clone = rig.the("box-2").path(&channel, CLONE).display().to_string();
    assert_eq!(git(&["-C", &clone, "rev-parse", "HEAD"]), pushed);
}

/// A conversation larger than one gRPC message (6 MiB here) still comes
/// back whole: the host sends it as a file, not inside StartSession.
#[tokio::test(flavor = "multi_thread")]
async fn rebuild_carries_a_conversation_larger_than_one_message() {
    let (mut rig, channel) = rig_and_channel().await;
    let (id, _) = forgotten_after(&mut rig, &channel, 6 * 1024).await;
    let two = Rig::box_id("box-2");

    let ended = rebuild(&rig, &id, &two, "rebuild-big").await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    rig.wait_session(&id, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s)).await;
    let box_two = rig.the("box-2");
    let talk = std::fs::read_to_string(box_two.conversation(&box_two.conversation_id(&id))).unwrap();
    assert_eq!(talk.matches("padding").count(), 6 * 1024, "every line came back");
    assert!(talk.contains("before the loss"));
    assert!(staged_none(&rig.the("box-2").path(&channel, ".carry")), "nothing left staged on box-2");
    assert!(staged_none(&rig.host_path(&channel, ".carry")), "nothing left staged on the host");
}

/// Whether `carry` holds nothing.
fn staged_none(carry: &std::path::Path) -> bool {
    std::fs::read_dir(carry).map_or(true, |mut entries| entries.next().is_none())
}

/// The rebuilt conversation repeats what the host's mirror held: the mirror
/// reads the new machine's file without adding that history twice (D209).
#[tokio::test(flavor = "multi_thread")]
async fn a_rebuilt_conversation_is_mirrored_once() {
    let (mut rig, channel) = rig_and_channel().await;
    let (id, _) = forgotten_after(&mut rig, &channel, 1).await;
    let two = Rig::box_id("box-2");

    let ended = rebuild(&rig, &id, &two, "rebuild-once").await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    rig.wait_session(&id, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s)).await;
    say(&rig, &id, r#"{"type":"user","message":{"content":"after the rebuild"}}"#).await;
    mirrored(&rig, &id, "after the rebuild").await;

    let texts = mirror_texts(&rig, &id).await;
    let held = |text: &str| texts.iter().filter(|entry| entry.contains(text)).count();
    // The padding line is JSON but no message, so the mirror holds no entry
    // for it; it only puts a non-entry line before the history.
    assert_eq!(held("before the loss"), 1, "{texts:?}");
    assert_eq!(held("after the rebuild"), 1, "{texts:?}");
}
