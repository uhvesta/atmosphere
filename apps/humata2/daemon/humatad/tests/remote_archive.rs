//! Archive and restore a session that runs on another machine (MV-7..MV-9
//! on a box; D195): the box archives it and frees its clones there after
//! the undo window, the host lists it archived, and restoring can bring it
//! back on the box (Undo, in place) or on this Mac (a fresh clone of its
//! branch with the mirrored conversation).
//!
//! Boxes run the daemon as shipped, so their undo window is the real 10
//! seconds. Waits are on the host's feeds; a freed folder is waited for on
//! disk.
//!
//! The cut tests at the end stop the host or a box at each point of the
//! archive, the mirror restore and the carry (`archive_remote.*`,
//! `mirror_restore.*`, `carry.*`), start it again, and have the box link:
//! each ends whole, or is still where it was. Both run the faults daemon
//! for such a start (D240); a box without a fault variable in its start's
//! environment runs the daemon as shipped.

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

/// The side folder the host brought box-1's copy of `id` into, under the
/// session's folder here, once it is complete: its final name appears only
/// after the copy was checked and its notice written. Nothing on the host's
/// feeds tells of it.
async fn brought_from_box_1(rig: &Rig, channel: &pb::Channel) -> std::path::PathBuf {
    let sides = rig.host_path(channel, "sessions/cache-warmup/.humata");
    let complete = || {
        std::fs::read_dir(&sides).ok().and_then(|entries| {
            entries.flatten().map(|entry| entry.path()).find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("from-box-1-") && !name.ends_with(".partial"))
            })
        })
    };
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        if let Some(side) = complete() {
            return side;
        }
        assert!(tokio::time::Instant::now() < deadline, "the host never brought box-1's copy into {}", sides.display());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Every file under `root` with its contents read as text, lossily.
fn texts_under(root: &Path) -> Vec<(std::path::PathBuf, String)> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(folder) = stack.pop() {
        for entry in std::fs::read_dir(&folder).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                found.push((path, String::from_utf8_lossy(&bytes).into_owned()));
            }
        }
    }
    found
}

/// D224's carry: box-1's archive restored on this Mac from the mirror while
/// box-1 is away, when box-1's clone had a file never committed. Back,
/// box-1 has more than the restore brought, so what it holds is carried
/// into a side folder of the session here, with a notice that names the
/// work that is gone, and only then does box-1 let its copy go. The
/// session's own clone here is untouched, and it stays live here.
#[tokio::test(flavor = "multi_thread")]
async fn a_mirror_restore_brings_what_box_1_had_more_of_into_a_side_folder_then_releases_its_copy() {
    let (rig, channel, id) = on_box().await;
    say(&rig, &id, r#"{"type":"user","message":{"content":"said on box-1"}}"#).await;
    mirrored(&rig, &id, "said on box-1").await;
    let branch = rig.session(&id).await.repos[0].branch.clone();
    let clone = rig.the("box-1").path(&channel, CLONE);
    git(&["-C", &clone.display().to_string(), "push", "-u", "origin", &branch]);
    std::fs::write(clone.join("wip.txt"), "not committed yet\n").unwrap();
    rig.wait_session(&id, "reporting its work", |s| s.pointers.iter().any(|pointer| pointer.dirty_files > 0))
        .await;
    // A plain archive refuses work (MV-9); box-1's summary still lists it.
    let started = start_archive(&rig, &id, pb::ArchiveChoice::ArchiveAndDeleteWork).await;
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    wait_gone(&clone).await;
    rig.link_lost("box-1").await;

    let host = rig.host_id().await;
    let ended = restore(&rig, &id, Some(host.clone())).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    rig.wait_session(&id, "live on this Mac", |s| s.machine.as_ref() == Some(&host) && is_live(s)).await;

    rig.link_back("box-1").await;
    let side = brought_from_box_1(&rig, &channel).await;
    released_on_box_1(&rig, &id).await;

    let files = texts_under(&side);
    assert!(
        files.iter().any(|(path, text)| path.extension().is_some_and(|ext| ext == "jsonl") && text.contains("said on box-1")),
        "box-1's conversation was brought: {:?}",
        files.iter().map(|(path, _)| path).collect::<Vec<_>>()
    );
    let notice = std::fs::read_to_string(side.join("NOTICE.md")).expect("the side folder has a notice");
    assert!(notice.contains("wip.txt"), "the notice names the work that is gone: {notice}");
    assert!(!rig.host_path(&channel, CLONE).join("wip.txt").exists(), "the live clone here is untouched");
    stays_here(&rig, &id).await;
}

// Cuts (SAFE T2c). The host or a box is stopped at one point, started again,
// and the box links; what is left must be whole.

/// Plans and starts the archive of `id`, whether or not the host lives to
/// answer: a host cut at a point may die before it says the archive began.
async fn archive_until_cut(rig: &Rig, id: &pb::SessionId) {
    let plan = sessions(rig)
        .await
        .plan_archive(pb::PlanArchiveRequest { id: Some(id.clone()) })
        .await
        .expect("the archive is planned")
        .into_inner();
    let _ = sessions(rig)
        .await
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
        })
        .await;
}

/// The session as archived on box-1, listed once among the archived and not
/// among the plain list, with a summary that names its repository.
async fn archived_once(rig: &Rig, id: &pb::SessionId) -> pb::Session {
    let archived = rig.wait_session(id, "archived", |s| s.archive.is_some()).await;
    assert_eq!(archived.machine, Some(Rig::box_id("box-1")));
    assert!(!listed(rig).await.contains(id), "an archived session leaves the plain list");
    let all = sessions(rig)
        .await
        .list_sessions(pb::ListSessionsRequest { include_archived: true, ..Default::default() })
        .await
        .expect("the host lists sessions with the archived")
        .into_inner()
        .sessions;
    let shown = all.iter().filter(|session| session.id.as_ref() == Some(id)).count();
    assert_eq!(shown, 1, "the archived session is listed once: {all:?}");
    let summary = archived.archive.clone().expect("an archived session has a summary");
    assert!(!summary.repos.is_empty(), "the summary names the session's repository: {summary:?}");
    archived
}

/// A host cut at `point` while box-1 archives a session: once the host is
/// back and box-1 linked, the session is archived, and listed once.
async fn host_cut_while_archiving(point: &str) {
    let (mut rig, channel, id) = on_box().await;
    rig.restart_host(&[("HUMATA2_STOP_AT", point)]).await;
    rig.wait_session(&id, "live again", is_live).await;

    archive_until_cut(&rig, &id).await;
    rig.host_stopped().await;
    rig.restart_host(&[]).await;

    archived_once(&rig, &id).await;
    wait_gone(&rig.the("box-1").path(&channel, CLONE)).await;
    assert!(rig.the("box-1").tabs().is_empty(), "its agent stopped on box-1");
}

/// A host cut before box-1 was asked: nothing changed. The session is live,
/// unarchived and in the plain list, and archiving it again goes through.
async fn host_cut_before_box_1_was_asked(point: &str) {
    let (mut rig, _channel, id) = on_box().await;
    rig.restart_host(&[("HUMATA2_STOP_AT", point)]).await;
    rig.wait_session(&id, "live again", is_live).await;

    archive_until_cut(&rig, &id).await;
    rig.host_stopped().await;
    rig.restart_host(&[]).await;

    let shown = rig.wait_session(&id, "live", is_live).await;
    assert!(shown.archive.is_none(), "box-1 wasn't asked, so nothing is archived: {shown:?}");
    assert!(listed(&rig).await.contains(&id), "the session is in the plain list");
    assert!(!rig.the("box-1").tabs().is_empty(), "its agent still runs on box-1");
    assert!(rig.the("box-1").store().archive(&id.value).unwrap().is_none(), "box-1 holds no archive");
    say(&rig, &id, "after the cut").await;

    let ended = archive(&rig, &id).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    archived_once(&rig, &id).await;
}

macro_rules! host_archive_cuts {
    ($($name:ident: $body:ident($point:literal),)*) => {
        $(
            #[tokio::test(flavor = "multi_thread")]
            async fn $name() {
                $body($point).await
            }
        )*
    };
}

host_archive_cuts! {
    a_host_cut_before_box_1_is_asked_to_archive_leaves_the_session_live: host_cut_before_box_1_was_asked("archive_remote.journaled"),
    a_host_cut_after_box_1_was_asked_to_archive_lists_it_archived_once_box_1_links: host_cut_while_archiving("archive_remote.asked"),
    a_host_cut_after_box_1_archived_lists_it_archived_once_box_1_links: host_cut_while_archiving("archive_remote.ended"),
    a_host_cut_after_the_archive_row_was_written_keeps_it_archived_once: host_cut_while_archiving("archive_remote.row_written"),
    a_host_cut_after_the_archive_follow_up_was_settled_keeps_it_archived_once: host_cut_while_archiving("archive_remote.settled"),
}

/// A box cut right after it recorded its archive of a session, with the agent
/// still running: the box finishes the archive when it starts, and once it
/// links the host lists the session archived, once, with no new ask.
#[tokio::test(flavor = "multi_thread")]
async fn a_box_1_cut_after_it_recorded_its_archive_finishes_it_once_it_links() {
    let (mut rig, channel, id) = on_box().await;
    rig.restart_box_env("box-1", &[("HUMATA2_STOP_AT", "archive.recorded")]).await;
    rig.wait_session(&id, "live again", is_live).await;

    archive_until_cut(&rig, &id).await;
    rig.box_stopped("box-1").await;
    rig.restart_box_env("box-1", &[]).await;

    archived_once(&rig, &id).await;
    assert!(rig.the("box-1").store().archive(&id.value).unwrap().is_some(), "box-1 holds its archive");
    wait_gone(&rig.the("box-1").path(&channel, CLONE)).await;
    assert!(rig.the("box-1").tabs().is_empty(), "its agent stopped on box-1");
}

/// What a cut mirror restore leaves, for the test to check.
struct Cut {
    rig: Rig,
    channel: pb::Channel,
    id: pb::SessionId,
    host: pb::MachineId,
}

/// A mirror restore of box-1's archive of a session with work in its clone,
/// with the host cut at `point`: either inside the restore, or (`at_link`)
/// once box-1's link is back, when the host compares and carries box-1's
/// copy. Returns with the host started again, box-1 started again and
/// linked.
async fn cut_a_mirror_restore(point: &str, at_link: bool) -> Cut {
    let (mut rig, channel, id) = on_box().await;
    say(&rig, &id, r#"{"type":"user","message":{"content":"said on box-1"}}"#).await;
    mirrored(&rig, &id, "said on box-1").await;
    let branch = rig.session(&id).await.repos[0].branch.clone();
    let clone = rig.the("box-1").path(&channel, CLONE);
    git(&["-C", &clone.display().to_string(), "push", "-u", "origin", &branch]);
    std::fs::write(clone.join("wip.txt"), "not committed yet\n").unwrap();
    rig.wait_session(&id, "reporting its work", |s| s.pointers.iter().any(|pointer| pointer.dirty_files > 0))
        .await;
    let started = start_archive(&rig, &id, pb::ArchiveChoice::ArchiveAndDeleteWork).await;
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    wait_gone(&clone).await;
    rig.restart_host(&[("HUMATA2_STOP_AT", point)]).await;
    rig.link_lost("box-1").await;

    let host = rig.host_id().await;
    let asked = sessions(&rig)
        .await
        .restore_session(pb::RestoreSessionRequest { idempotency_key: None, id: Some(id.clone()), machine: Some(host.clone()) })
        .await;
    if at_link {
        let started = asked.expect("the restore starts").into_inner();
        let ended = rig.ended(&started).await;
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
        rig.wait_session(&id, "live on this Mac", |s| s.machine.as_ref() == Some(&host) && is_live(s)).await;
        rig.dials_through("box-1");
    }
    rig.host_stopped().await;
    // Box-1 is away through the host's restart, then links.
    rig.the_mut("box-1").kill();
    rig.restart_host(&[]).await;
    rig.the_mut("box-1").relaunch(&[]).await;
    rig.link_back("box-1").await;
    Cut { rig, channel, id, host }
}

/// The side folders brought from box-1 under the session's folder here:
/// (whole ones, unfinished ones).
fn side_folders(rig: &Rig, channel: &pb::Channel) -> (Vec<std::path::PathBuf>, Vec<std::path::PathBuf>) {
    let sides = rig.host_path(channel, "sessions/cache-warmup/.humata");
    let (mut whole, mut partial) = (Vec::new(), Vec::new());
    for entry in std::fs::read_dir(&sides).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_owned();
        if name.starts_with("from-box-1-") {
            if name.ends_with(".partial") { partial.push(path) } else { whole.push(path) }
        }
    }
    (whole, partial)
}

/// The session runs on this Mac and isn't archived: live, or live once the
/// user resumes it, which has to work.
async fn on_this_mac(rig: &Rig, id: &pb::SessionId, host: &pb::MachineId) {
    let shown = rig
        .wait_session(id, "on this Mac and not archived", |s| s.machine.as_ref() == Some(host) && s.archive.is_none())
        .await;
    if !is_live(&shown) {
        let started = sessions(rig)
            .await
            .resume_session(pb::ResumeSessionRequest {
                idempotency_key: None,
                id: Some(id.clone()),
                mode: pb::resume_session_request::Mode::Resume as i32,
            })
            .await
            .expect("the session can be resumed on this Mac")
            .into_inner();
        let ended = rig.ended(&started).await;
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    }
    rig.wait_session(id, "live on this Mac", |s| s.machine.as_ref() == Some(host) && is_live(s) && s.archive.is_none())
        .await;
}

/// A host cut in a mirror restore at or after its switch: the session runs
/// here, its conversation is the mirror's once, box-1 let its copy go, and
/// what box-1 had more of sits beside the session exactly once.
async fn host_cut_after_the_switch(point: &str, at_link: bool) {
    let Cut { rig, channel, id, host } = cut_a_mirror_restore(point, at_link).await;
    // Live on its own, with the conversation the mirror kept: a cut after
    // the switch finishes the resume at the host's start. The record names
    // this Mac before the agent exists, so the wait is for the agent's
    // conversation, not only for a live-looking status.
    let back = rig
        .wait_session(&id, "live on this Mac with its agent", |s| {
            s.machine.as_ref() == Some(&host) && is_live(s) && s.archive.is_none() && !s.conversation_id.is_empty()
        })
        .await;
    let talk = std::fs::read_to_string(rig.host_conversation(&back.conversation_id)).unwrap();
    assert!(talk.contains("said on box-1"), "the conversation came from the mirror: {talk}");
    released_on_box_1(&rig, &id).await;
    stays_here(&rig, &id).await;

    let (whole, partial) = side_folders(&rig, &channel);
    assert_eq!(whole.len(), 1, "what box-1 had more of is beside the session once: {whole:?}");
    assert!(partial.is_empty(), "no unfinished side folder is left: {partial:?}");
    let notice = std::fs::read_to_string(whole[0].join("NOTICE.md")).expect("the side folder has a notice");
    assert!(notice.contains("wip.txt"), "the notice names the work that is gone: {notice}");
    let files = texts_under(&whole[0]);
    assert!(
        files.iter().any(|(path, text)| path.extension().is_some_and(|ext| ext == "jsonl") && text.contains("said on box-1")),
        "box-1's conversation was brought: {:?}",
        files.iter().map(|(path, _)| path).collect::<Vec<_>>()
    );
    say(&rig, &id, r#"{"type":"user","message":{"content":"said on this mac"}}"#).await;
    mirrored(&rig, &id, "said on this mac").await;
    let texts = mirror_texts(&rig, &id).await;
    let held = |text: &str| texts.iter().filter(|entry| entry.contains(text)).count();
    assert_eq!(held("said on box-1"), 1, "{texts:?}");
    assert_eq!(held("said on this mac"), 1, "{texts:?}");
}

/// A host cut in a mirror restore before its switch: the session is still
/// archived on box-1, whole, with box-1's copy where it was; restoring it
/// again, now that box-1 is linked, goes through.
async fn host_cut_before_the_switch(point: &str) {
    let Cut { rig, id, host, .. } = cut_a_mirror_restore(point, false).await;
    let shown = rig.session(&id).await;
    assert!(shown.archive.is_some(), "the session is still archived: {shown:?}");
    assert_eq!(shown.machine, Some(Rig::box_id("box-1")), "its record still names box-1");
    assert!(!listed(&rig).await.contains(&id), "an archived session leaves the plain list");
    assert!(rig.the("box-1").store().session(&id.value).unwrap().is_some(), "box-1 kept its copy");

    let ended = restore(&rig, &id, Some(host.clone())).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    on_this_mac(&rig, &id, &host).await;
    released_on_box_1(&rig, &id).await;
    stays_here(&rig, &id).await;
}

macro_rules! mirror_restore_cuts {
    ($($name:ident: $body:ident($point:literal, $link:literal),)*) => {
        $(
            #[tokio::test(flavor = "multi_thread")]
            async fn $name() {
                $body($point, $link).await
            }
        )*
    };
}

macro_rules! mirror_restore_cuts_before_the_switch {
    ($($name:ident: $point:literal,)*) => {
        $(
            #[tokio::test(flavor = "multi_thread")]
            async fn $name() {
                host_cut_before_the_switch($point).await
            }
        )*
    };
}

mirror_restore_cuts_before_the_switch! {
    a_host_cut_after_a_mirror_restore_cloned_leaves_the_session_archived_on_box_1: "mirror_restore.cloned",
    a_host_cut_after_a_mirror_restore_kept_its_release_leaves_the_session_archived_on_box_1: "mirror_restore.journaled",
}

mirror_restore_cuts! {
    a_host_cut_after_a_mirror_restore_switched_still_runs_it_here_and_releases_box_1s_copy: host_cut_after_the_switch("mirror_restore.switched", false),
    a_host_cut_after_a_mirror_restore_resumed_still_runs_it_here_and_releases_box_1s_copy: host_cut_after_the_switch("mirror_restore.resumed", false),
    a_host_cut_after_box_1s_copy_was_compared_still_releases_it_and_carries_once: host_cut_after_the_switch("mirror_restore.compared", true),
    a_host_cut_after_box_1_let_go_of_its_copy_keeps_one_carried_folder: host_cut_after_the_switch("mirror_restore.released", true),
    a_host_cut_after_the_carry_staged_box_1s_copy_carries_it_once: host_cut_after_the_switch("carry.staged", true),
    a_host_cut_after_the_carry_copied_box_1s_files_carries_them_once: host_cut_after_the_switch("carry.copied", true),
    a_host_cut_after_the_carry_wrote_its_notice_carries_once_with_no_partial_left: host_cut_after_the_switch("carry.noticed", true),
    a_host_cut_after_the_carry_was_placed_carries_once: host_cut_after_the_switch("carry.placed", true),
}

/// A box cut while it lets go of its copy of a session restored on this
/// Mac: once it links, the host asks again, the copy goes, and the session
/// runs here alone.
async fn box_cut_while_letting_go(point: &str) {
    let (mut rig, channel, id) = on_box().await;
    rig.restart_box_env("box-1", &[("HUMATA2_STOP_AT", point)]).await;
    rig.wait_session(&id, "live again", is_live).await;
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
    rig.dials_through("box-1");

    rig.box_stopped("box-1").await;
    rig.restart_box_env("box-1", &[]).await;
    released_on_box_1(&rig, &id).await;
    stays_here(&rig, &id).await;
    let (whole, partial) = side_folders(&rig, &channel);
    assert!(whole.is_empty() && partial.is_empty(), "nothing was more on box-1, so nothing is carried: {whole:?} {partial:?}");
    let texts = mirror_texts(&rig, &id).await;
    assert_eq!(texts.iter().filter(|entry| entry.contains("said on box-1")).count(), 1, "{texts:?}");
}

macro_rules! box_release_cuts {
    ($($name:ident: $point:literal,)*) => {
        $(
            #[tokio::test(flavor = "multi_thread")]
            async fn $name() {
                box_cut_while_letting_go($point).await
            }
        )*
    };
}

box_release_cuts! {
    a_box_1_cut_after_trashing_its_copy_lets_go_of_it_once_it_links: "carry.trashed",
    a_box_1_cut_after_forgetting_its_copy_lets_go_of_it_once_it_links: "carry.forgotten",
}

/// What the host's newest restart report says of the operations it found,
/// and the end of the log of the run before, for a failure's message.
fn host_restart_story(rig: &Rig) -> String {
    let restarts = rig.host.state.join("restarts");
    let mut folders: Vec<std::path::PathBuf> =
        std::fs::read_dir(&restarts).into_iter().flatten().flatten().map(|entry| entry.path()).collect();
    folders.sort();
    let operations = folders
        .last()
        .and_then(|folder| std::fs::read_to_string(folder.join("operations.jsonl")).ok())
        .unwrap_or_default();
    let log = std::fs::read_to_string(rig.host.state.join("logs").join("humatad2.log")).unwrap_or_default();
    let tail: Vec<&str> = log.lines().rev().take(40).collect();
    format!("operations: {operations}\nlog: {}", tail.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

/// PER-27 for a restore on this Mac while box-1 is linked: the host cut at
/// `point` of it, once it was asked, and started again. The restore is
/// either undone (archived on box-1 as it was, with nothing of it taken in
/// here) or finished (live on this Mac, not archived).
async fn host_cut_while_restoring_here(point: &str) {
    let (mut rig, channel, id) = on_box().await;
    let branch = rig.session(&id).await.repos[0].branch.clone();
    let clone = rig.the("box-1").path(&channel, CLONE);
    git(&["-C", &clone.display().to_string(), "push", "-u", "origin", &branch]);
    let ended = archive(&rig, &id).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    wait_gone(&clone).await;
    let was = rig.wait_session(&id, "archived on box-1", |s| s.archive.is_some()).await;
    assert!(was.conversation_id.is_empty(), "nothing of box-1's agent is here yet: {was:?}");

    let arm = rig.host.dir.path().join("arm-restore");
    let armed = arm.display().to_string();
    rig.restart_host(&[("HUMATA2_STOP_AT", point), ("HUMATA2_FAULT_ARM", armed.as_str())]).await;
    std::fs::write(&arm, b"").unwrap();
    let host = rig.host_id().await;
    let _ = sessions(&rig)
        .await
        .restore_session(pb::RestoreSessionRequest { idempotency_key: None, id: Some(id.clone()), machine: Some(host.clone()) })
        .await;
    rig.host_stopped().await;
    rig.restart_host(&[]).await;

    let undone = |s: &pb::Session| {
        s.archive.is_some() && s.machine == Some(Rig::box_id("box-1")) && s.conversation_id.is_empty()
    };
    let finished = |s: &pb::Session| s.archive.is_none() && s.machine.as_ref() == Some(&host) && is_live(s);
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        let shown = rig.session(&id).await;
        if undone(&shown) || finished(&shown) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the restore cut at {point} is neither undone nor finished: {shown:?}\n{}",
            host_restart_story(&rig)
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

macro_rules! restore_here_cuts {
    ($($name:ident: $point:literal,)*) => {
        $(
            #[tokio::test(flavor = "multi_thread")]
            async fn $name() {
                host_cut_while_restoring_here($point).await
            }
        )*
    };
}

restore_here_cuts! {
    a_host_cut_while_a_restore_here_takes_the_agent_in_undoes_it: "store.agent.put.committed",
    a_host_cut_after_a_restore_here_named_this_mac_undoes_or_finishes_it: "store.session.put.committed",
    a_host_cut_after_a_restore_here_dropped_the_archive_row_finishes_it: "store.archive.remove.committed",
}
