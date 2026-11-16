//! Archive and restore a session that runs on another machine (MV-7..MV-9
//! on a box; D195): the box archives it and frees its clones there after
//! the undo window, the host lists it archived, and restoring can bring it
//! back on the box (Undo, in place) or on this Mac (a fresh clone of its
//! branch with the mirrored conversation).
//!
//! Boxes run the daemon as shipped, so their undo window is the real 10
//! seconds. Waits are on the host's feeds; a freed folder is waited for on
//! disk.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::path::Path;
use std::time::Duration;

use common::git;
use humata2_grpc::humata::v1 as pb;
use remote_rig::{LONG, Rig, is_live};
use tonic::transport::Channel;

const REPO: (&str, &str) = ("acme", "api");
const CLONE: &str = "sessions/cache-warmup/api";

async fn sessions(rig: &Rig) -> pb::session_service_client::SessionServiceClient<Channel> {
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
}

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

async fn archive(rig: &Rig, id: &pb::SessionId) -> pb::Operation {
    let started = start_archive(rig, id, pb::ArchiveChoice::Archive).await;
    rig.ended(&started).await
}

/// Plans the archive of `id`, checks the plan offers `choice`, and starts
/// it; returns the operation as it started.
async fn start_archive(rig: &Rig, id: &pb::SessionId, choice: pb::ArchiveChoice) -> pb::Operation {
    let plan = sessions(rig).await.plan_archive(pb::PlanArchiveRequest { id: Some(id.clone()) }).await.expect("the archive is planned").into_inner();
    assert!(plan.choices.contains(&(choice as i32)), "the plan offers {choice:?}: {plan:?}");
    sessions(rig)
        .await
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            choice: choice as i32,
            plan: plan.token,
        })
        .await
        .expect("the archive starts")
        .into_inner()
}

async fn restore(rig: &Rig, id: &pb::SessionId, on: Option<pb::MachineId>) -> pb::Operation {
    let started = sessions(rig)
        .await
        .restore_session(pb::RestoreSessionRequest { idempotency_key: None, id: Some(id.clone()), machine: on })
        .await
        .expect("the restore starts")
        .into_inner();
    rig.ended(&started).await
}

async fn listed(rig: &Rig) -> Vec<pb::SessionId> {
    sessions(rig)
        .await
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions
        .into_iter()
        .filter_map(|session| session.id)
        .collect()
}

/// The box frees a folder by itself once the undo window ends; nothing on
/// the host's feeds tells of it.
async fn wait_gone(path: &Path) {
    let deadline = tokio::time::Instant::now() + LONG;
    while path.exists() {
        assert!(tokio::time::Instant::now() < deadline, "{} was never freed", path.display());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A live session on box-1 that has said something.
async fn on_box() -> (Rig, pb::Channel, pb::SessionId) {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host, one.clone()]).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    say(&rig, &id, "before the archive").await;
    (rig, channel, id)
}

/// MV-7 on a box: archived there, listed archived on the host, its agent
/// stopped and its clone freed on the box after the window.
#[tokio::test(flavor = "multi_thread")]
async fn archiving_on_box_1_frees_its_clones_there_and_lists_it_archived_on_the_host() {
    let (rig, channel, id) = on_box().await;

    let ended = archive(&rig, &id).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    let archived = rig.wait_session(&id, "archived", |s| s.archive.is_some()).await;
    assert_eq!(archived.machine, Some(Rig::box_id("box-1")));
    assert!(!listed(&rig).await.contains(&id), "an archived session leaves the plain list");
    wait_gone(&rig.the("box-1").path(&channel, CLONE)).await;
    assert!(rig.the("box-1").tabs().is_empty(), "its agent stopped on box-1");
}

/// MV-8 elsewhere: box-1's archive restored on this Mac, after its folder
/// was freed, is a fresh clone of its branch with the conversation.
#[tokio::test(flavor = "multi_thread")]
async fn restoring_box_1s_archive_on_this_mac_clones_its_branch_and_resumes() {
    let (rig, channel, id) = on_box().await;
    let branch = rig.session(&id).await.repos[0].branch.clone();
    let clone = rig.the("box-1").path(&channel, CLONE);
    git(&["-C", &clone.display().to_string(), "push", "-u", "origin", &branch]);
    let ended = archive(&rig, &id).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    wait_gone(&clone).await;

    let host = rig.host_id().await;
    let ended = restore(&rig, &id, Some(host.clone())).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    let back = rig.wait_session(&id, "live on this Mac", |s| s.machine.as_ref() == Some(&host) && is_live(s)).await;
    assert!(back.archive.is_none());
    assert_eq!(back.repos[0].branch, branch);
    let here = rig.host_path(&channel, CLONE);
    assert_eq!(git(&["-C", &here.display().to_string(), "rev-parse", "--abbrev-ref", "HEAD"]), branch);
    say(&rig, &id, "back on this mac").await;
    let talk = std::fs::read_to_string(rig.host_conversation(&back.conversation_id)).unwrap();
    assert!(talk.contains("before the archive"), "the conversation came back: {talk}");
}

/// MV-8 Undo: restored inside the window, it comes back on box-1 in its
/// own folder, with what the folder held.
#[tokio::test(flavor = "multi_thread")]
async fn undo_inside_the_window_restores_it_on_box_1_in_place() {
    let (rig, channel, id) = on_box().await;
    let clone = rig.the("box-1").path(&channel, CLONE);
    // Not work (MV-9 would refuse a plain archive over that), and a fresh
    // clone wouldn't have it: it shows the clone itself came back.
    std::fs::write(clone.join(".git").join("humata-test-marker"), "kept through the archive\n").unwrap();
    let ended = archive(&rig, &id).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    let ended = restore(&rig, &id, None).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    let back = rig.wait_session(&id, "live on box-1", |s| s.machine.as_ref() == Some(&Rig::box_id("box-1")) && is_live(s)).await;
    assert!(back.archive.is_none());
    assert_eq!(std::fs::read_to_string(clone.join(".git").join("humata-test-marker")).unwrap(), "kept through the archive\n");
}

/// Every entry the host's mirror of `id` holds, page by page.
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

/// Waits until the host's mirror of `id` holds an entry with `text`: the
/// mirror follows the machine and tells no feed, so this asks again.
async fn mirrored(rig: &Rig, id: &pb::SessionId, text: &str) {
    let deadline = tokio::time::Instant::now() + LONG;
    while !mirror_texts(rig, id).await.iter().any(|entry| entry.contains(text)) {
        assert!(tokio::time::Instant::now() < deadline, "the host's mirror never held {text:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// D211: box-1's archive restored on this Mac brings its conversation here
/// as a file that repeats what the host's mirror held; the mirror adds only
/// what follows.
#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_restored_on_this_mac_is_mirrored_once() {
    let (rig, channel, id) = on_box().await;
    // JSON lines, as real agents write: the mirror keeps only those (D189.4).
    say(&rig, &id, r#"{"type":"user","message":{"content":"before the restore"}}"#).await;
    mirrored(&rig, &id, "before the restore").await;
    let branch = rig.session(&id).await.repos[0].branch.clone();
    let clone = rig.the("box-1").path(&channel, CLONE);
    git(&["-C", &clone.display().to_string(), "push", "-u", "origin", &branch]);
    let ended = archive(&rig, &id).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    wait_gone(&clone).await;

    let host = rig.host_id().await;
    let ended = restore(&rig, &id, Some(host.clone())).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    rig.wait_session(&id, "live on this Mac", |s| s.machine.as_ref() == Some(&host) && is_live(s)).await;
    say(&rig, &id, r#"{"type":"user","message":{"content":"after the restore"}}"#).await;
    mirrored(&rig, &id, "after the restore").await;

    let texts = mirror_texts(&rig, &id).await;
    let held = |text: &str| texts.iter().filter(|entry| entry.contains(text)).count();
    assert_eq!(held("before the restore"), 1, "{texts:?}");
    assert_eq!(held("after the restore"), 1, "{texts:?}");
}

/// MV-7 with work: the host's archive of a box-1 session shows what box-1's
/// archive kept, the files the clone changed and the subjects of the
/// commits never pushed, as an archive on this Mac does.
#[tokio::test(flavor = "multi_thread")]
async fn a_box_1_archive_with_work_shows_its_files_and_unpushed_commits_on_the_host() {
    let (rig, channel, id) = on_box().await;
    let clone = rig.the("box-1").path(&channel, CLONE);
    let at = clone.display().to_string();
    std::fs::write(clone.join("warm.txt"), "warmed\n").unwrap();
    git(&["-C", &at, "add", "warm.txt"]);
    git(&["-C", &at, "commit", "-q", "-m", "Warm the cache"]);
    std::fs::write(clone.join("wip.txt"), "not committed yet\n").unwrap();
    // The host learns of the work from box-1's report.
    rig.wait_session(&id, "reporting its work", |s| {
        s.pointers.iter().any(|pointer| {
            pointer.dirty_files > 0 && pointer.unpushed_subjects.iter().any(|subject| subject == "Warm the cache")
        })
    })
    .await;

    let started = start_archive(&rig, &id, pb::ArchiveChoice::ArchiveAndDeleteWork).await;
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    let archived = rig.wait_session(&id, "archived", |s| s.archive.is_some()).await;
    let summary = archived.archive.expect("an archived session has a summary");
    let repo = summary.repos.first().expect("the summary names the session's repository");
    let files: Vec<&str> = repo.files.iter().map(|file| file.path.as_str()).collect();
    assert!(files.contains(&"wip.txt"), "the uncommitted file is listed: {files:?}");
    assert!(files.contains(&"warm.txt"), "the committed file is listed: {files:?}");
    assert!(
        repo.unpushed_subjects.iter().any(|subject| subject == "Warm the cache"),
        "the unpushed commit is named: {:?}",
        repo.unpushed_subjects
    );
}

/// Waits until box-1's own store holds an archive row for `id`: box-1 took
/// the archive in. Nothing on the host's feeds tells of it.
async fn recorded_on_box_1(rig: &Rig, id: &pb::SessionId) {
    let deadline = tokio::time::Instant::now() + LONG;
    while rig.the("box-1").store().archive(&id.value).unwrap().is_none() {
        assert!(tokio::time::Instant::now() < deadline, "box-1 never recorded the archive");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Restarts the host so a remote archive holds, once box-1 has answered,
/// until the returned FIFO is opened for writing.
async fn hold_remote_archives(rig: &mut Rig) -> std::path::PathBuf {
    let hold = rig.host.dir.path().join("hold-archive.fifo");
    assert!(std::process::Command::new("mkfifo").arg(&hold).status().unwrap().success());
    let named = hold.display().to_string();
    rig.restart_host(&[("HUMATA2_HOLD_AT", "archive-remote-asked"), ("HUMATA2_HOLD_FIFO", named.as_str())]).await;
    hold
}

/// Lets the held archive go on; it opens the FIFO once it gets there.
async fn release(hold: &Path) {
    let hold = hold.to_owned();
    let opened = tokio::task::spawn_blocking(move || std::fs::OpenOptions::new().write(true).open(hold).map(drop));
    tokio::time::timeout(LONG, opened)
        .await
        .expect("the archive never got to where it is held")
        .unwrap()
        .expect("the held archive's FIFO opens");
}

/// A link lost while box-1 archives: box-1 finishes on its own, and once
/// its link is back the host lists the session archived, with no new ask.
#[tokio::test(flavor = "multi_thread")]
async fn an_archive_whose_link_was_lost_shows_archived_once_box_1_is_back() {
    let (mut rig, _channel, id) = on_box().await;
    let hold = hold_remote_archives(&mut rig).await;
    rig.wait_session(&id, "live again", is_live).await;

    let started = start_archive(&rig, &id, pb::ArchiveChoice::Archive).await;
    recorded_on_box_1(&rig, &id).await;
    rig.link_lost("box-1").await;
    release(&hold).await;
    // Whether the host saw the end before the link went, the operation ends.
    rig.ended(&started).await;

    rig.link_back("box-1").await;
    let archived = rig.wait_session(&id, "archived", |s| s.archive.is_some()).await;
    assert_eq!(archived.machine, Some(Rig::box_id("box-1")));
    assert!(!listed(&rig).await.contains(&id), "an archived session leaves the plain list");
}

/// MV-8 elsewhere with box-1 away: its archive restored on this Mac takes
/// the conversation from the host's mirror, runs here, mirrors each entry
/// once, and box-1 lets its copy go once it is back.
#[tokio::test(flavor = "multi_thread")]
async fn restoring_box_1s_archive_on_this_mac_while_box_1_is_away_uses_the_mirror() {
    let (rig, channel, id) = on_box().await;
    say(&rig, &id, r#"{"type":"user","message":{"content":"said on box-1"}}"#).await;
    mirrored(&rig, &id, "said on box-1").await;
    let branch = rig.session(&id).await.repos[0].branch.clone();
    let clone = rig.the("box-1").path(&channel, CLONE);
    git(&["-C", &clone.display().to_string(), "push", "-u", "origin", &branch]);
    let ended = archive(&rig, &id).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    wait_gone(&clone).await;
    rig.link_lost("box-1").await;

    let host = rig.host_id().await;
    let ended = restore(&rig, &id, Some(host.clone())).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    let back = rig.wait_session(&id, "live on this Mac", |s| s.machine.as_ref() == Some(&host) && is_live(s)).await;
    assert!(back.archive.is_none());
    let here = rig.host_path(&channel, CLONE);
    assert_eq!(git(&["-C", &here.display().to_string(), "rev-parse", "--abbrev-ref", "HEAD"]), branch);
    let talk = std::fs::read_to_string(rig.host_conversation(&back.conversation_id)).unwrap();
    assert!(talk.contains("said on box-1"), "the conversation came from the mirror: {talk}");
    say(&rig, &id, r#"{"type":"user","message":{"content":"said on this mac"}}"#).await;
    mirrored(&rig, &id, "said on this mac").await;
    let texts = mirror_texts(&rig, &id).await;
    let held = |text: &str| texts.iter().filter(|entry| entry.contains(text)).count();
    assert_eq!(held("said on box-1"), 1, "{texts:?}");
    assert_eq!(held("said on this mac"), 1, "{texts:?}");

    // Back, box-1 drops its copy: the session runs here alone.
    rig.link_back("box-1").await;
    released_on_box_1(&rig, &id).await;
    stays_here(&rig, &id).await;
}

/// Waits until box-1's store no longer holds `id`: box-1 let its copy go.
/// Nothing on the host's feeds tells of it.
async fn released_on_box_1(rig: &Rig, id: &pb::SessionId) {
    let deadline = tokio::time::Instant::now() + LONG;
    while rig.the("box-1").store().session(&id.value).unwrap().is_some() {
        assert!(tokio::time::Instant::now() < deadline, "box-1 kept its copy of the session");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Box-1, back and done with its copy, didn't take `id` back: the host's
/// record names this Mac, and the session is live and unarchived here.
async fn stays_here(rig: &Rig, id: &pb::SessionId) {
    let host = rig.host_id().await;
    let shown = rig.session(id).await;
    assert_eq!(shown.machine, Some(host), "the session still runs on this Mac");
    assert!(is_live(&shown), "the session is still live here: {shown:?}");
    assert!(shown.archive.is_none(), "the session isn't archived again");
    assert!(listed(rig).await.contains(id), "the session is in the plain list");
}

/// D222: a link lost while box-1 archives, then a restart of the host while
/// box-1 is away. Once box-1 is back the host still learns it archived the
/// session and lists it archived.
#[tokio::test(flavor = "multi_thread")]
async fn an_archive_whose_link_was_lost_shows_archived_after_a_host_restart_once_box_1_is_back() {
    let (mut rig, _channel, id) = on_box().await;
    let hold = hold_remote_archives(&mut rig).await;
    rig.wait_session(&id, "live again", is_live).await;

    let started = start_archive(&rig, &id, pb::ArchiveChoice::Archive).await;
    recorded_on_box_1(&rig, &id).await;
    rig.link_lost("box-1").await;
    release(&hold).await;
    rig.ended(&started).await;

    // Box-1 stays away through the host's restart: its daemon is down, so
    // the restart doesn't wait for its link.
    rig.the_mut("box-1").kill();
    rig.restart_host(&[]).await;
    assert!(rig.session(&id).await.archive.is_none(), "nothing is known of box-1's archive yet");
    rig.the_mut("box-1").relaunch(&[]).await;
    rig.link_back("box-1").await;

    let archived = rig.wait_session(&id, "archived", |s| s.archive.is_some()).await;
    assert_eq!(archived.machine, Some(Rig::box_id("box-1")));
    assert!(!listed(&rig).await.contains(&id), "an archived session leaves the plain list");
}

/// D222 and the flip back: box-1's archive restored on this Mac from the
/// mirror while box-1 is away, then a restart of the host. Once box-1 is
/// back it lets its copy go, and its reports don't take the session back.
#[tokio::test(flavor = "multi_thread")]
async fn a_mirror_restore_releases_box_1s_copy_after_a_host_restart_and_stays_here() {
    let (mut rig, channel, id) = on_box().await;
    say(&rig, &id, r#"{"type":"user","message":{"content":"said on box-1"}}"#).await;
    mirrored(&rig, &id, "said on box-1").await;
    let branch = rig.session(&id).await.repos[0].branch.clone();
    let clone = rig.the("box-1").path(&channel, CLONE);
    git(&["-C", &clone.display().to_string(), "push", "-u", "origin", &branch]);
    let ended = archive(&rig, &id).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    wait_gone(&clone).await;
    rig.link_lost("box-1").await;

    let host = rig.host_id().await;
    let ended = restore(&rig, &id, Some(host.clone())).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    rig.wait_session(&id, "live on this Mac", |s| s.machine.as_ref() == Some(&host) && is_live(s)).await;

    rig.the_mut("box-1").kill();
    rig.restart_host(&[]).await;
    rig.wait_session(&id, "live on this Mac after the restart", |s| s.machine.as_ref() == Some(&host) && is_live(s)).await;
    assert!(rig.the("box-1").store().session(&id.value).unwrap().is_some(), "box-1 was away, so its copy is still there");

    rig.the_mut("box-1").relaunch(&[]).await;
    rig.link_back("box-1").await;
    released_on_box_1(&rig, &id).await;
    stays_here(&rig, &id).await;
}
