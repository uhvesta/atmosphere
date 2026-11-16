//! Moving a channel session to another machine (MV-1..MV-6, MV-13, MV-14,
//! flow 26.6; D195): the agent pauses, the session's folder, clones and
//! conversation are copied and checked, the agent resumes there, and only
//! then is the old copy put in the old machine's trash. Any failure before
//! the switch leaves the session running where it was, nothing deleted.
//!
//! Every wait is on the host's feeds (the rig's `wait_session`, `ended`);
//! `LONG` is only a safety net.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{git, holder_answers};
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::Code;

const REPO: (&str, &str) = ("acme", "api");

fn text(path: &Path) -> String {
    path.display().to_string()
}

async fn sessions(rig: &Rig) -> pb::session_service_client::SessionServiceClient<tonic::transport::Channel> {
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
}

fn request(id: &pb::SessionId, to: &pb::MachineId, key: &str) -> pb::MoveSessionRequest {
    pb::MoveSessionRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
        id: Some(id.clone()),
        to: Some(to.clone()),
    }
}

/// Starts a move and waits for its operation to end.
async fn move_to(rig: &Rig, id: &pb::SessionId, to: &pb::MachineId) -> pb::Operation {
    let started = sessions(rig)
        .await
        .move_session(request(id, to, &format!("move-{}-{}", id.value, to.value)))
        .await
        .expect("the move starts")
        .into_inner();
    assert_eq!(started.kind, pb::OperationKind::MoveSession as i32);
    rig.ended(&started).await
}

/// Where "Cache warmup"'s clone sits in its channel's folder.
const CLONE: &str = "sessions/cache-warmup/api";

/// The session's clone on `machine`'s own disk.
fn clone_on(rig: &Rig, machine: &str, channel: &pb::Channel) -> PathBuf {
    rig.the(machine).path(channel, CLONE)
}

/// A dirty file, an untracked note in the session folder outside the clone,
/// and one commit the remote doesn't have (MV-13).
fn make_work(clone: &Path) -> String {
    std::fs::write(clone.join("README.md"), "changed before the move\n").unwrap();
    std::fs::write(clone.join("committed.txt"), "kept\n").unwrap();
    git(&["-C", &text(clone), "add", "committed.txt"]);
    git(&["-C", &text(clone), "commit", "-m", "work before the move"]);
    std::fs::write(clone.parent().unwrap().join("PLAN.md"), "# the plan\n").unwrap();
    git(&["-C", &text(clone), "rev-parse", "HEAD"])
}

/// Says `line` to the session's agent and waits for its answer, so the
/// conversation file holds it.
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

/// MV-2, MV-5, MV-13: the same session, on box-2, with the branch, the
/// unpushed commit, the dirty file, the folder's own files and the
/// conversation, its agent resumed there; box-1's copy in box-1's trash.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_moved_box_1_to_box_2_keeps_id_branch_work_and_conversation_and_resumes() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let on = [rig.host_id().await, one.clone(), two.clone()];
    let channel = rig.create_channel("Payments", &[REPO], &on).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    let before = rig.wait_session(&id, "live", is_live).await;
    let head = make_work(&clone_on(&rig, "box-1", &channel));
    say(&rig, &id, "remember the cache").await;
    let conversation = rig.the("box-1").conversation_id(&id);
    let talk = std::fs::read(rig.the("box-1").conversation(&conversation)).unwrap();

    let ended = move_to(&rig, &id, &two).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "the move failed: {:?}", ended.error);

    let after = rig.wait_session(&id, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s)).await;
    assert_eq!(after.id, before.id);
    assert_eq!(after.title, before.title);
    assert_eq!(after.channel, before.channel);
    assert_eq!(after.repos[0].branch, before.repos[0].branch);
    let clone = clone_on(&rig, "box-2", &channel);
    assert_eq!(git(&["-C", &text(&clone), "rev-parse", "HEAD"]), head, "the unpushed commit came along");
    assert_eq!(std::fs::read_to_string(clone.join("README.md")).unwrap(), "changed before the move\n");
    assert_eq!(std::fs::read_to_string(clone.parent().unwrap().join("PLAN.md")).unwrap(), "# the plan\n");

    let two_box = rig.the("box-2");
    let moved = two_box.conversation_id(&id);
    assert!(std::fs::read(two_box.conversation(&moved)).unwrap().starts_with(&talk), "the conversation came along");
    assert_eq!(two_box.tabs().len(), 1);
    assert!(holder_answers(&two_box.tabs()[0]), "the agent runs on box-2");
    say(&rig, &id, "still here").await;

    let one_box = rig.the("box-1");
    assert!(one_box.tabs().is_empty(), "nothing runs on box-1");
    assert!(!clone_on(&rig, "box-1", &channel).exists(), "box-1's copy is gone from its folder");
    assert!(std::fs::read_dir(one_box.trash()).unwrap().next().is_some(), "box-1's copy is in its trash");
    assert!(staged(&rig, "box-2", &channel).is_empty(), "box-2 keeps nothing staged for the move");
}

/// What a move left staged in `channel`'s folder on `machine`.
fn staged(rig: &Rig, machine: &str, channel: &pb::Channel) -> Vec<PathBuf> {
    std::fs::read_dir(rig.the(machine).path(channel, ".carry"))
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

/// MV-3: box-2 can't take the copy. The session runs on box-1 as before,
/// with the same agent, its work untouched, and the failure names the step
/// and says nothing was deleted.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_whose_copy_fails_leaves_it_running_on_the_source_and_names_the_step() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let on = [rig.host_id().await, one.clone(), two.clone()];
    let channel = rig.create_channel("Payments", &[REPO], &on).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    let head = make_work(&clone_on(&rig, "box-1", &channel));
    let holder = rig.the("box-1").tabs()[0].holder_pid;
    let sessions_on_two = rig.the("box-2").path(&channel, "sessions");
    std::fs::create_dir_all(&sessions_on_two).unwrap();
    std::fs::set_permissions(&sessions_on_two, std::fs::Permissions::from_mode(0o555)).unwrap();

    let ended = move_to(&rig, &id, &two).await;
    std::fs::set_permissions(&sessions_on_two, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(ended.state, pb::OperationState::Failed as i32);
    let error = ended.error.expect("a failed move says why");
    assert!(!error.message.is_empty() && !error.fix.is_empty(), "{error:?}");
    let failed: Vec<_> = ended.steps.iter().filter(|step| step.state == pb::StepState::Failed as i32).collect();
    assert_eq!(failed.len(), 1, "one step is named as failed: {:?}", ended.steps);

    let after = rig.wait_session(&id, "live on box-1", is_live).await;
    assert_eq!(after.machine, Some(one));
    let clone = clone_on(&rig, "box-1", &channel);
    assert_eq!(git(&["-C", &text(&clone), "rev-parse", "HEAD"]), head);
    assert_eq!(std::fs::read_to_string(clone.join("README.md")).unwrap(), "changed before the move\n");
    let tabs = rig.the("box-1").tabs();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].holder_pid, holder, "the same agent runs on box-1");
    say(&rig, &id, "still on box-1").await;
    assert!(std::fs::read_dir(&sessions_on_two).unwrap().next().is_none(), "nothing is left on box-2");
    assert!(rig.the("box-2").tabs().is_empty());
}

/// MV-1: only an online machine of the session's channel can take it; any
/// other is refused at once and nothing changes.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_to_an_offline_machine_or_one_outside_the_channel_is_refused_and_changes_nothing() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone()]).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    let holder = rig.the("box-1").tabs()[0].holder_pid;

    let outside = sessions(&rig).await.move_session(request(&id, &two, "move-outside")).await.unwrap_err();
    assert_eq!(outside.code(), Code::FailedPrecondition, "{outside:?}");

    let channel_two = rig.create_channel("Ledger", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
    let other = rig.start_on(&channel_two, &one, "Ledger work").await;
    rig.wait_session(&other, "live", is_live).await;
    rig.link_lost("box-2").await;
    let offline = sessions(&rig).await.move_session(request(&other, &two, "move-offline")).await.unwrap_err();
    assert_eq!(offline.code(), Code::FailedPrecondition, "{offline:?}");

    let same = sessions(&rig).await.move_session(request(&id, &one, "move-same")).await.unwrap_err();
    assert_eq!(same.code(), Code::InvalidArgument, "{same:?}");

    let after = rig.session(&id).await;
    assert_eq!(after.machine, Some(one));
    assert!(is_live(&after));
    assert_eq!(rig.the("box-1").tabs().iter().filter(|tab| tab.holder_pid == holder).count(), 1);
}

/// MV-2 from this Mac: the host's own folder goes to its trash only after
/// the session runs on box-1.
#[tokio::test(flavor = "multi_thread")]
async fn moving_from_this_mac_frees_its_folder_only_after_it_runs_on_box_1() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let id = rig.start_on(&channel, &host, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    let mine = rig.host_path(&channel, CLONE);
    let head = make_work(&mine);
    say(&rig, &id, "remember the cache").await;

    let ended = move_to(&rig, &id, &one).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "the move failed: {:?}", ended.error);

    rig.wait_session(&id, "live on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s)).await;
    assert_eq!(git(&["-C", &text(&clone_on(&rig, "box-1", &channel)), "rev-parse", "HEAD"]), head);
    let moved = rig.the("box-1").conversation_id(&id);
    let talk = std::fs::read_to_string(rig.the("box-1").conversation(&moved)).unwrap();
    assert!(talk.contains("remember the cache"), "the conversation came along: {talk}");
    assert!(!mine.exists(), "this Mac's copy is gone from its folder");
    assert!(common::stored_tabs(&rig.host.state).is_empty(), "nothing runs on this Mac");
}

/// MV-2, MV-5: a move is safe to ask again with the same key; it answers
/// the same operation and moves once.
#[tokio::test(flavor = "multi_thread")]
async fn asking_the_same_move_again_answers_the_same_operation() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;

    let mut client = sessions(&rig).await;
    let first = client.move_session(request(&id, &two, "move-once")).await.unwrap().into_inner();
    let again = client.move_session(request(&id, &two, "move-once")).await.unwrap().into_inner();
    assert_eq!(first.id, again.id);
    let ended = rig.ended(&first).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    rig.wait_session(&id, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s)).await;
    assert_eq!(rig.the("box-2").tabs().len(), 1, "moved once");
}

/// Starts a move under a host started with `HUMATA2_STOP_AT=<at>`, waits for
/// the host to die there, and starts it again with no stop.
async fn move_killed_at(rig: &mut Rig, id: &pb::SessionId, to: &pb::MachineId, at: &str) {
    rig.restart_host(&[("HUMATA2_STOP_AT", at)]).await;
    let _ = sessions(rig).await.move_session(request(id, to, &format!("move-killed-{at}"))).await;
    rig.host_stopped().await;
    rig.restart_host(&[]).await;
}

fn not_moving(session: &pb::Session) -> bool {
    session.status != pb::SessionStatus::Moving as i32
}

/// MV-14: a host killed after the copy reached box-2 but before the agent
/// moved undoes the move when it starts again: the session runs on box-1,
/// one agent, its work there; box-2 keeps nothing, so moving again works.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_cut_short_after_the_copy_is_undone_at_restart() {
    let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    let head = make_work(&clone_on(&rig, "box-1", &channel));

    move_killed_at(&mut rig, &id, &two, "move-copied").await;

    rig.wait_session(&id, "back on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s) && not_moving(s)).await;
    assert_eq!(git(&["-C", &text(&clone_on(&rig, "box-1", &channel)), "rev-parse", "HEAD"]), head);
    assert_eq!(rig.the("box-1").tabs().len(), 1);
    assert!(holder_answers(&rig.the("box-1").tabs()[0]), "the agent runs on box-1");
    assert!(rig.the("box-2").tabs().is_empty(), "no agent on box-2");
    assert!(!clone_on(&rig, "box-2", &channel).exists(), "box-2's copy was taken back");

    let ended = move_to(&rig, &id, &two).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "moving again failed: {:?}", ended.error);
}

/// MV-14: a host killed once the agent runs on box-2 finishes the move when
/// it starts again: the session is on box-2 with one agent, and box-1's
/// copy is in box-1's trash. Never two agents.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_cut_short_after_the_agent_moved_is_finished_at_restart() {
    for at in ["move-resumed", "move-switched", "move-released"] {
        let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
        let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
        let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
        let id = rig.start_on(&channel, &one, "Cache warmup").await;
        rig.wait_session(&id, "live", is_live).await;
        let head = make_work(&clone_on(&rig, "box-1", &channel));

        move_killed_at(&mut rig, &id, &two, at).await;

        rig.wait_session(&id, "on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s) && not_moving(s)).await;
        assert_eq!(git(&["-C", &text(&clone_on(&rig, "box-2", &channel)), "rev-parse", "HEAD"]), head, "{at}");
        assert_eq!(rig.the("box-2").tabs().len(), 1, "{at}");
        assert!(holder_answers(&rig.the("box-2").tabs()[0]), "{at}: the agent runs on box-2");
        assert!(rig.the("box-1").tabs().is_empty(), "{at}: an agent still runs on box-1");
        assert!(!clone_on(&rig, "box-1", &channel).exists(), "{at}: box-1's copy is still in its folder");
        assert!(std::fs::read_dir(rig.the("box-1").trash()).unwrap().next().is_some(), "{at}: box-1's copy is in its trash");
        let journals = std::fs::read_dir(rig.host.state.join("carry"))
            .map(|entries| entries.flatten().filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json")).count())
            .unwrap_or(0);
        assert_eq!(journals, 0, "{at}: the move's journal is done with");
        assert!(staged(&rig, "box-2", &channel).is_empty(), "{at}: box-2 keeps nothing staged for the move");
    }
}

/// MV-4: while a session moves, its terminal is read-only and says so; what
/// is typed then reaches no agent. The host holds the move after the copy
/// until the test opens `hold`, a FIFO, so the state is seen mid-move with
/// no timing. Once for a session on this Mac (its own tab) and once for one
/// on box-1 (the host's relay).
#[tokio::test(flavor = "multi_thread")]
async fn a_moving_session_s_terminal_is_read_only_until_it_resumes() {
    for from in ["host", "box-1"] {
        let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
        let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
        let host = rig.host_id().await;
        let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone(), two.clone()]).await;
        let (on, to) = if from == "host" { (host.clone(), one.clone()) } else { (one.clone(), two.clone()) };
        let id = rig.start_on(&channel, &on, "Cache warmup").await;
        rig.wait_session(&id, "live", is_live).await;
        let hold = rig.host.dir.path().join("hold.fifo");
        assert!(std::process::Command::new("mkfifo").arg(&hold).status().unwrap().success());
        rig.restart_host(&[("HUMATA2_HOLD_AT", "move-copied"), ("HUMATA2_HOLD_FIFO", &text(&hold))]).await;
        rig.wait_session(&id, "live again", is_live).await;

        let tab = common::agent_tab(&rig.host, &id).await;
        let open = pb::AttachOpen { session: Some(id.clone()), ..common::open(&tab, 0) };
        let mut attach = common::Attach::start(&rig.host, open).await;
        attach.wait_for("agent up").await;
        let started = sessions(&rig)
            .await
            .move_session(request(&id, &to, "move-read-only"))
            .await
            .expect("the move starts")
            .into_inner();
        let moving = pb::terminal_state::Writable::Moving as i32;
        attach.wait_until(|seen| seen.states.last().is_some_and(|state| state.writable == moving)).await;
        attach.type_text("typed while moving\n").await;

        let release = hold.clone();
        tokio::task::spawn_blocking(move || std::fs::OpenOptions::new().write(true).open(release).map(drop))
            .await
            .unwrap()
            .expect("the held move opens the FIFO");
        let ended = rig.ended(&started).await;
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{from}: {:?}", ended.error);
        let place = if from == "host" { "box-1" } else { "box-2" };
        let conversation = rig.the(place).conversation_id(&id);
        let talk = std::fs::read_to_string(rig.the(place).conversation(&conversation)).unwrap();
        assert!(!talk.contains("typed while moving"), "{from}: input during the move reached the agent");
        say(&rig, &id, "typed once it resumed").await;
    }
}

/// Restarts the host so its move holds at `at` until `hold`, a FIFO, is
/// opened for writing.
async fn hold_moves_at(rig: &mut Rig, at: &str) -> PathBuf {
    let hold = rig.host.dir.path().join(format!("hold-{at}.fifo"));
    assert!(std::process::Command::new("mkfifo").arg(&hold).status().unwrap().success());
    rig.restart_host(&[("HUMATA2_HOLD_AT", at), ("HUMATA2_HOLD_FIFO", &text(&hold))]).await;
    hold
}

/// Lets a move held at the FIFO `hold` go on; call it only once the move is
/// there, or it waits for a reader that never comes.
async fn release(hold: &Path) {
    let hold = hold.to_owned();
    let opened = tokio::task::spawn_blocking(move || std::fs::OpenOptions::new().write(true).open(hold).map(drop));
    tokio::time::timeout(remote_rig::LONG, opened)
        .await
        .expect("the move never got to where it is held")
        .unwrap()
        .expect("the held move's FIFO opens");
}

/// Whether `operation`'s step `key` is done.
fn done(operation: &pb::Operation, key: &str) -> bool {
    operation.steps.iter().any(|step| step.key == key && step.state == pb::StepState::Done as i32)
}

/// Watches `operation` until `reached` holds for it; the watch pushes
/// each change, so nothing sleeps.
async fn wait_operation(rig: &Rig, operation: &pb::Operation, reached: impl Fn(&pb::Operation) -> bool) {
    let mut watch = pb::operation_service_client::OperationServiceClient::new(rig.channel().await)
        .watch_operation(pb::WatchOperationRequest { id: operation.id.clone(), from_version: 0 })
        .await
        .expect("the operation can be watched")
        .into_inner();
    let mut last = operation.clone();
    while !reached(&last) {
        match tokio::time::timeout(remote_rig::LONG, watch.message()).await {
            Ok(Ok(Some(next))) => last = next,
            other => panic!("the operation never got there ({other:?}); it last was {last:?}"),
        }
    }
}

async fn cancel(rig: &Rig, operation: &pb::Operation) -> Result<pb::Operation, tonic::Status> {
    pb::operation_service_client::OperationServiceClient::new(rig.channel().await)
        .cancel_operation(pb::CancelOperationRequest { id: operation.id.clone(), idempotency_key: None })
        .await
        .map(tonic::Response::into_inner)
}

/// GRPC.md §7: a move cancelled before the old agent stops is undone: it
/// ends CANCELLED, the session runs on box-1 as before and box-2 keeps
/// nothing, so moving again works. Once the old agent has stopped, a cancel
/// is refused and the move finishes.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_cancelled_before_the_agent_stops_is_undone_and_one_after_is_refused() {
    let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    let head = make_work(&clone_on(&rig, "box-1", &channel));

    let hold = hold_moves_at(&mut rig, "move-copied").await;
    rig.wait_session(&id, "live again", is_live).await;
    let started = sessions(&rig).await.move_session(request(&id, &two, "move-cancelled")).await.unwrap().into_inner();
    assert!(started.cancellable, "a move starts cancellable");
    wait_operation(&rig, &started, |operation| done(operation, "copy")).await;
    cancel(&rig, &started).await.expect("a running move takes a cancel");
    release(&hold).await;
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Cancelled as i32, "{:?}", ended.error);
    rig.wait_session(&id, "on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s) && not_moving(s)).await;
    assert_eq!(git(&["-C", &text(&clone_on(&rig, "box-1", &channel)), "rev-parse", "HEAD"]), head);
    assert!(holder_answers(&rig.the("box-1").tabs()[0]), "the agent runs on box-1");
    assert!(rig.the("box-2").tabs().is_empty(), "no agent on box-2");
    assert!(!clone_on(&rig, "box-2", &channel).exists(), "box-2's copy was taken back");

    let hold = hold_moves_at(&mut rig, "move-resumed").await;
    rig.wait_session(&id, "live again", is_live).await;
    let started = sessions(&rig).await.move_session(request(&id, &two, "move-past-no-return")).await.unwrap().into_inner();
    wait_operation(&rig, &started, |operation| !operation.cancellable).await;
    let refused = cancel(&rig, &started).await.expect_err("a cancel after the agent stopped is refused");
    assert_eq!(refused.code(), tonic::Code::FailedPrecondition, "{refused:?}");
    release(&hold).await;
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    rig.wait_session(&id, "on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s) && not_moving(s)).await;
}

/// A session the user stopped before the move stays stopped when a move cut
/// short at `unpacked` is undone: the undo starts again only an agent the
/// move itself stopped.
#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_session_whose_move_is_undone_at_restart_stays_stopped() {
    let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    sessions(&rig)
        .await
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(id.clone()) })
        .await
        .expect("the session stops");
    assert!(rig.the("box-1").tabs().iter().all(|tab| !holder_answers(tab)), "the agent stopped");

    move_killed_at(&mut rig, &id, &two, "move-unpacked").await;

    rig.wait_session(&id, "back on box-1", |s| s.machine.as_ref() == Some(&one) && not_moving(s)).await;
    assert!(clone_on(&rig, "box-1", &channel).exists(), "box-1 keeps its copy");
    assert!(!clone_on(&rig, "box-2", &channel).exists(), "box-2's copy was taken back");
    assert!(rig.the("box-1").tabs().iter().all(|tab| !holder_answers(tab)), "the undo started no agent on box-1");
    assert!(rig.the("box-2").tabs().is_empty(), "no agent on box-2");
}

/// D207: a move undone while box-2 is offline leaves box-2 a stopped copy
/// at first; the session runs on box-1 at once, and the copy is taken back
/// when box-2's link is up again.
#[tokio::test(flavor = "multi_thread")]
async fn a_copy_left_on_an_offline_target_is_taken_back_when_its_link_is_up() {
    let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;

    let hold = hold_moves_at(&mut rig, "move-unpacked").await;
    rig.wait_session(&id, "live again", is_live).await;
    let started = sessions(&rig).await.move_session(request(&id, &two, "move-target-lost")).await.unwrap().into_inner();
    wait_operation(&rig, &started, |operation| done(operation, "verify")).await;
    rig.the_mut("box-2").kill();
    rig.link_lost("box-2").await;
    release(&hold).await;
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Failed as i32, "{ended:?}");
    rig.wait_session(&id, "on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s) && not_moving(s)).await;
    assert!(holder_answers(&rig.the("box-1").tabs()[0]), "the agent runs on box-1");
    assert!(clone_on(&rig, "box-2", &channel).exists(), "box-2 was offline, so its copy is still there");

    rig.the_mut("box-2").relaunch(&[]).await;
    rig.link_back("box-2").await;
    let box_two = clone_on(&rig, "box-2", &channel);
    rig.wait_session(&id, "box-2's copy taken back", |_| !box_two.exists()).await;
    assert!(rig.the("box-2").store().session(&id.value).unwrap().is_none(), "box-2 has no record of it");
    assert!(staged(&rig, "box-2", &channel).is_empty(), "box-2 keeps nothing staged");
    assert!(holder_answers(&rig.the("box-1").tabs()[0]), "the agent still runs on box-1");
}
