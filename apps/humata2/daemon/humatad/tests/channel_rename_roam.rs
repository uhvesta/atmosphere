//! MV-16, D314, D801 D: renaming a channel renames its folder on every
//! machine the channel lists, not only this Mac. Each machine does what this
//! Mac does: it renames the folder once and rewrites its records, and its
//! running agents keep running under the old name, which stays a link to
//! the new folder while they do. A machine that is offline gets the
//! rename when it next links, and until then it keeps the folder it has: the
//! host never shows it as renamed.
//!
//! Every wait is on the host's Sessions feed; `LONG` is only a safety net.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::path::Path;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{AgentState, Store};
use remote_rig::{LONG, Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");

/// Renames `channel` to `name` through the host.
async fn rename(rig: &Rig, channel: &pb::Channel, name: &str) -> pb::Channel {
    pb::channel_service_client::ChannelServiceClient::new(rig.channel().await)
        .rename_channel(pb::RenameChannelRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: format!("rename-{name}") }),
            id: channel.id.clone(),
            name: name.to_owned(),
            expected_revision: None,
            confirm_running: true,
        })
        .await
        .expect("the host renames the channel")
        .into_inner()
}

/// Whether `id`'s agent runs in `store` and its session works under `folder`.
/// The agent keeps the folder it started under until it is next resumed
/// (D801 D), so its session's record is what names the new folder.
fn runs_in(store: &Store, id: &pb::SessionId, folder: &Path) -> bool {
    let running = store
        .agent(&id.value)
        .ok()
        .flatten()
        .is_some_and(|agent| matches!(agent.state, AgentState::Running { .. }));
    running && store.session(&id.value).ok().flatten().is_some_and(|session| session.folder.starts_with(folder))
}

/// Whether `path` is gone, or only the link that keeps a renamed folder's old
/// name for the agents that run under it.
fn gone_or_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).map_or(true, |meta| meta.file_type().is_symlink())
}

/// Waits until the host shows `id` live and `store()` has its agent running
/// in `folder`.
async fn resumed_in(rig: &Rig, id: &pb::SessionId, store: impl Fn() -> Store, folder: &Path) {
    rig.wait_session(id, "running in the renamed channel's folder", |session| {
        is_live(session) && runs_in(&store(), id, folder)
    })
    .await;
}

fn host_store(rig: &Rig) -> Store {
    Store::open(rig.host.state.join("store")).unwrap()
}

/// MV-16: with box-1 linked, a rename moves the channel's folder on this Mac
/// and on box-1, rewrites box-1's records, and resumes the agent of each
/// machine's session in the new folder.
#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_channel_renames_its_folder_on_this_mac_and_on_box_1_and_the_agents_resume_there() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let here = rig.start_on(&channel, &host, "Ledger fix").await;
    let there = rig.start_on(&channel, &one, "Ledger audit").await;

    let renamed = rename(&rig, &channel, "Billing").await;
    assert_ne!(renamed.folder, channel.folder, "the folder follows the name");

    let boxed = rig.the("box-1");
    for root in [rig.world.channels.clone(), boxed.channels()] {
        assert!(gone_or_link(&root.join(&channel.folder)), "{} still has the old folder", root.display());
        assert!(root.join(&renamed.folder).is_dir(), "{} lacks the new folder", root.display());
    }
    let record = boxed.store().channel(&channel.id.as_ref().unwrap().value).unwrap().expect("box-1 keeps the channel");
    assert_eq!((record.name.as_str(), record.folder.as_str()), ("Billing", renamed.folder.as_str()));

    let new_here = rig.world.channels.join(&renamed.folder);
    let new_there = boxed.channels().join(&renamed.folder);
    resumed_in(&rig, &here, || host_store(&rig), &new_here).await;
    resumed_in(&rig, &there, || rig.the("box-1").store(), &new_there).await;
    let moved = boxed.store().session(&there.value).unwrap().expect("box-1 keeps its session");
    assert!(moved.folder.starts_with(&new_there), "box-1's session names the new folder: {}", moved.folder.display());
}

/// MV-16, D348, D801 D: a machine that is offline while the channel is
/// renamed keeps its folder as it was. When it next links the host catches
/// it up, and its agent, in a turn or not, keeps running: nothing is asked.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_offline_during_a_channel_rename_is_renamed_when_it_links() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let there = rig.start_on(&channel, &one, "Ledger audit").await;

    rig.link_lost("box-1").await;
    let renamed = rename(&rig, &channel, "Billing").await;
    assert!(rig.world.channels.join(&renamed.folder).is_dir(), "this Mac renamed its folder");
    let boxed = rig.the("box-1");
    assert!(boxed.channels().join(&channel.folder).is_dir(), "box-1 keeps the folder it has");
    assert!(!boxed.channels().join(&renamed.folder).exists(), "box-1 was not renamed while it was away");

    rig.link_back("box-1").await;
    let new_there = boxed.channels().join(&renamed.folder);
    resumed_in(&rig, &there, || rig.the("box-1").store(), &new_there).await;
    assert!(pending(&rig).await.is_empty(), "nothing is asked: an agent in a turn is not cut off");
    assert!(gone_or_link(&boxed.channels().join(&channel.folder)), "box-1's old folder is gone once it is renamed");
    let record = boxed.store().channel(&channel.id.as_ref().unwrap().value).unwrap().expect("box-1 keeps the channel");
    assert_eq!((record.name.as_str(), record.folder.as_str()), ("Billing", renamed.folder.as_str()));
}

/// A rename that doesn't set `confirm_running`.
async fn rename_asking(rig: &Rig, channel: &pb::Channel, name: &str) -> pb::Channel {
    pb::channel_service_client::ChannelServiceClient::new(rig.channel().await)
        .rename_channel(pb::RenameChannelRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: format!("rename-asking-{name}") }),
            id: channel.id.clone(),
            name: name.to_owned(),
            expected_revision: None,
            confirm_running: false,
        })
        .await
        .expect("the host renames the channel and leaves a machine that can't to the Inbox")
        .into_inner()
}

async fn inbox(rig: &Rig) -> pb::inbox_service_client::InboxServiceClient<tonic::transport::Channel> {
    pb::inbox_service_client::InboxServiceClient::new(rig.channel().await)
}

/// The requests the host's Inbox holds open.
async fn pending(rig: &Rig) -> Vec<pb::InboxRequest> {
    inbox(rig)
        .await
        .list_requests(pb::ListRequestsRequest::default())
        .await
        .expect("the host lists its Inbox")
        .into_inner()
        .requests
}

/// The first request the host's Inbox holds open. Nothing sleeps: it is read
/// again each time the Inbox feed tells of a change.
async fn the_request(rig: &Rig) -> pb::InboxRequest {
    let mut feed = pb::event_service_client::EventServiceClient::new(rig.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Inbox as i32],
            ..Default::default()
        })
        .await
        .expect("the host's Inbox feed opens")
        .into_inner();
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        if let Some(first) = pending(rig).await.into_iter().next() {
            return first;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            other => panic!("no request reached the host's Inbox within {LONG:?}: {other:?}"),
        }
    }
}

async fn approve(rig: &Rig, request: &pb::InboxRequest) -> pb::InboxRequest {
    inbox(rig)
        .await
        .answer_request(pb::AnswerRequestRequest {
            id: request.id.clone(),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            ..Default::default()
        })
        .await
        .expect("the host takes the approval")
        .into_inner()
}

/// D801 D: a machine whose agents may be in a turn renames on its own and
/// asks nothing: its agent keeps running, and the host's Inbox stays empty.
#[tokio::test(flavor = "multi_thread")]
async fn a_busy_machine_renames_its_folder_without_asking_and_its_agent_keeps_running() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let there = rig.start_on(&channel, &one, "Ledger audit").await;
    let boxed = rig.the("box-1");

    let renamed = rename_asking(&rig, &channel, "Billing").await;

    let new_there = boxed.channels().join(&renamed.folder);
    assert!(new_there.is_dir(), "box-1 renamed its folder");
    assert!(pending(&rig).await.is_empty(), "nothing is asked");
    resumed_in(&rig, &there, || rig.the("box-1").store(), &new_there).await;
    assert!(gone_or_link(&boxed.channels().join(&channel.folder)));
}

/// D348: a machine that already has something at the new folder picks no
/// other name. The host's Inbox holds one request that explains the clash;
/// once the folder is cleared, approving it renames the machine.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_whose_new_folder_is_taken_asks_in_the_inbox_and_renames_once_it_is_cleared() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let there = rig.start_on(&channel, &one, "Ledger audit").await;
    let boxed = rig.the("box-1");
    let taken = boxed.channels().join("billing");
    std::fs::create_dir_all(&taken).unwrap();
    std::fs::write(taken.join("keep.txt"), "mine").unwrap();

    let renamed = rename_asking(&rig, &channel, "Billing").await;
    let asked = the_request(&rig).await;

    assert_eq!(renamed.folder, "billing");
    assert_eq!(pending(&rig).await.len(), 1, "one request");
    assert_eq!(asked.channel, channel.id);
    assert!(asked.reason.contains(&renamed.folder), "it names the folder in the way: {}", asked.reason);
    assert!(boxed.channels().join(&channel.folder).is_dir(), "box-1 keeps its own folder");
    assert_eq!(std::fs::read_to_string(taken.join("keep.txt")).unwrap(), "mine", "what was there is untouched");
    let record = boxed.store().channel(&channel.id.as_ref().unwrap().value).unwrap().expect("box-1 keeps the channel");
    assert_eq!(record.folder, channel.folder, "box-1 picked no other name");

    std::fs::remove_dir_all(&taken).unwrap();
    approve(&rig, &asked).await;

    resumed_in(&rig, &there, || rig.the("box-1").store(), &taken).await;
    assert!(gone_or_link(&boxed.channels().join(&channel.folder)), "box-1's old folder is gone once it is renamed");
}
