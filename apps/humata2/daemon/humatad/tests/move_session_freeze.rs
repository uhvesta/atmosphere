//! A cross-machine move freezes the old agent instead of letting it overlap
//! the new one (D801 J): its program is stopped (SIGSTOP) from the start of
//! the move, and a move that fails or is cancelled thaws it again, the same
//! program under the same holder, never restarted.
//!
//! Every wait is on the host's feeds; `LONG` is only a safety net.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};

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

/// The state letters `ps` gives each process whose parent is `holder`: the
/// agent's program runs as its child. A stopped process starts with `T`.
fn program_states(holder: u32) -> Vec<(u32, String)> {
    let listed = std::process::Command::new("/bin/ps")
        .args(["-A", "-o", "pid=,ppid=,stat="])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid: u32 = fields.next()?.parse().ok()?;
            let parent: u32 = fields.next()?.parse().ok()?;
            let state = fields.next()?.to_owned();
            (parent == holder).then_some((pid, state))
        })
        .collect()
}

fn programs_stopped(holder: u32) -> bool {
    let states = program_states(holder);
    !states.is_empty() && states.iter().all(|(_, state)| state.starts_with('T'))
}

/// Polls until `programs_stopped(holder)` is `stopped`: the program takes
/// the signal a moment after the holder says it sent it.
async fn wait_stopped(holder: u32, stopped: bool) {
    let deadline = std::time::Instant::now() + remote_rig::LONG;
    while programs_stopped(holder) != stopped {
        assert!(
            std::time::Instant::now() < deadline,
            "the agent's program never became {} ({:?})",
            if stopped { "stopped" } else { "runnable" },
            program_states(holder)
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Says `line` to the session's agent and waits for its answer: it can only
/// answer if its program runs.
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

fn done(operation: &pb::Operation, key: &str) -> bool {
    operation.steps.iter().any(|step| step.key == key && step.state == pb::StepState::Done as i32)
}

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

/// Restarts the host so its move holds at `at` until `hold`, a FIFO, is
/// opened for writing.
async fn hold_moves_at(rig: &mut Rig, at: &str) -> PathBuf {
    let hold = rig.host.dir.path().join(format!("hold-{at}.fifo"));
    assert!(std::process::Command::new("mkfifo").arg(&hold).status().unwrap().success());
    rig.restart_host(&[("HUMATA2_HOLD_AT", at), ("HUMATA2_HOLD_FIFO", &text(&hold))]).await;
    hold
}

async fn release(hold: &Path) {
    let hold = hold.to_owned();
    let opened = tokio::task::spawn_blocking(move || std::fs::OpenOptions::new().write(true).open(hold).map(drop));
    tokio::time::timeout(remote_rig::LONG, opened)
        .await
        .expect("the move never got to where it is held")
        .unwrap()
        .expect("the held move's FIFO opens");
}

fn not_moving(session: &pb::Session) -> bool {
    session.status != pb::SessionStatus::Moving as i32
}

/// The old agent is frozen while the copy is made, and a move cancelled
/// before the new agent starts thaws it: same holder, same program, never
/// stopped for good, and it answers again.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_freezes_the_old_agent_while_it_copies_and_a_cancel_thaws_it() {
    let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    let holder = rig.the("box-1").tabs()[0].holder_pid;
    assert!(!programs_stopped(holder), "the agent runs before the move");

    let hold = hold_moves_at(&mut rig, "move-copied").await;
    rig.wait_session(&id, "live again", is_live).await;
    let started = sessions(&rig).await.move_session(request(&id, &two, "move-frozen")).await.unwrap().into_inner();
    wait_operation(&rig, &started, |operation| done(operation, "copy")).await;
    wait_stopped(holder, true).await;

    pb::operation_service_client::OperationServiceClient::new(rig.channel().await)
        .cancel_operation(pb::CancelOperationRequest { id: started.id.clone(), idempotency_key: None })
        .await
        .expect("a running move takes a cancel");
    release(&hold).await;
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Cancelled as i32, "{:?}", ended.error);

    wait_stopped(holder, false).await;
    rig.wait_session(&id, "on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s) && not_moving(s)).await;
    let tabs = rig.the("box-1").tabs();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].holder_pid, holder, "the same agent runs on box-1");
    say(&rig, &id, "thawed after the cancel").await;
    assert!(rig.the("box-2").tabs().is_empty(), "no agent on box-2");
}

/// A move whose target refuses the copy fails after the old agent was
/// frozen: the old agent runs again under the same holder, and nothing
/// ran beside it.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_that_fails_after_the_freeze_leaves_the_old_agent_running_with_its_pid_unchanged() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let on = [rig.host_id().await, one.clone(), two.clone()];
    let channel = rig.create_channel("Payments", &[REPO], &on).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    let holder = rig.the("box-1").tabs()[0].holder_pid;
    let program_before = program_states(holder);
    assert_eq!(program_before.len(), 1, "one program under the holder: {program_before:?}");
    let sessions_on_two = rig.the("box-2").path(&channel, "sessions");
    std::fs::create_dir_all(&sessions_on_two).unwrap();
    std::fs::set_permissions(&sessions_on_two, std::fs::Permissions::from_mode(0o555)).unwrap();

    let started = sessions(&rig).await.move_session(request(&id, &two, "move-refused")).await.unwrap().into_inner();
    let ended = rig.ended(&started).await;
    std::fs::set_permissions(&sessions_on_two, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(ended.state, pb::OperationState::Failed as i32);
    rig.wait_session(&id, "live on box-1", |s| s.machine.as_ref() == Some(&one) && is_live(s) && not_moving(s)).await;
    wait_stopped(holder, false).await;
    let program_after = program_states(holder);
    assert_eq!(
        program_after.iter().map(|(pid, _)| *pid).collect::<Vec<_>>(),
        program_before.iter().map(|(pid, _)| *pid).collect::<Vec<_>>(),
        "the agent's program is the same process"
    );
    let tabs = rig.the("box-1").tabs();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].holder_pid, holder, "the same holder");
    say(&rig, &id, "still on box-1").await;
    assert!(rig.the("box-2").tabs().is_empty());
}
