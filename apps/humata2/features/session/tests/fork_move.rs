//! The Fork, Move and Archive dialogs' reducer paths (FK-1 to FK-5, MV-1 to
//! MV-3, MV-7 to MV-9).
use humata2_clients::{
    FakeDaemon, fixture,
    model::{Agent, ForkKind, MachineId, SessionId},
};
use humata2_session::{ForkPicker, SessionAction as A, SessionState, Sessions};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn store(open: &str) -> (TestStore<Sessions>, FakeDaemon) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let state = SessionState {
        workspace: Arc::new(fakes.daemon.workspace()),
        open: Some(SessionId::new(open)),
        ..Default::default()
    };
    (
        TestStore::new(state, Sessions::new(clients)).non_exhaustive(),
        fakes.daemon,
    )
}

#[test]
fn opening_the_fork_dialog_suggests_a_branch_beside_the_parents() {
    let (mut store, _) = store("ledger");
    store.send(A::StartFork(ForkKind::Worktree), |_| {});
    let draft = store.state().fork.clone().expect("the dialog opened");
    assert!(!draft.branch.is_empty());
    assert!(draft.branch.contains("ledger-schema-migration"));
    assert_eq!(draft.before_turn, None);
    assert_eq!(draft.machine, None);
}

#[test]
fn the_fork_dialog_opens_collapsed_and_more_options_toggles() {
    let (mut store, _) = store("ledger");
    store.send(A::StartFork(ForkKind::Worktree), |_| {});
    assert!(!store.state().fork.as_ref().expect("open").more);
    store.send(A::ForkMoreOptions, |_| {});
    assert!(store.state().fork.as_ref().expect("open").more);
    store.send(A::ForkMoreOptions, |_| {});
    assert!(!store.state().fork.as_ref().expect("open").more);
}

#[test]
fn a_preset_other_than_code_and_context_opens_the_dialog_expanded() {
    for kind in [ForkKind::Conversation, ForkKind::Shared] {
        let (mut store, _) = store("ledger");
        store.send(A::StartFork(kind), |_| {});
        assert!(store.state().fork.as_ref().expect("open").more);
    }
}

#[test]
fn an_agent_or_machine_unlike_the_parents_expands_and_the_parents_own_does_not() {
    let (mut first, _) = store("ledger");
    first.send(A::StartFork(ForkKind::Worktree), |_| {});
    first.send(A::ForkAgent(Agent::Claude), |_| {});
    first.send(A::ForkMachine(MachineId::new("devbox-1")), |_| {});
    assert!(!first.state().fork.as_ref().expect("open").more);
    first.send(A::ForkAgent(Agent::Codex), |_| {});
    assert!(first.state().fork.as_ref().expect("open").more);

    let (mut second, _) = store("ledger");
    second.send(A::StartFork(ForkKind::Worktree), |_| {});
    second.send(A::ForkMachine(MachineId::new("this-mac")), |_| {});
    assert!(second.state().fork.as_ref().expect("open").more);
}

#[test]
fn a_turn_preset_alone_keeps_the_dialog_collapsed() {
    let (mut store, _) = store("ledger");
    store.send(A::StartFork(ForkKind::Worktree), |_| {});
    store.send(A::ForkBeforeTurn(Some(2)), |_| {});
    assert!(!store.state().fork.as_ref().expect("open").more);
}

#[test]
fn more_options_is_not_remembered_across_opens() {
    let (mut store, _) = store("ledger");
    store.send(A::StartFork(ForkKind::Worktree), |_| {});
    store.send(A::ForkMoreOptions, |_| {});
    store.send(A::CancelFork, |_| {});
    store.send(A::StartFork(ForkKind::Worktree), |_| {});
    assert!(!store.state().fork.as_ref().expect("open").more);
}

#[test]
fn the_fork_sends_what_the_dialog_holds() {
    let (mut store, daemon) = store("ledger");
    store.send(A::StartFork(ForkKind::Worktree), |_| {});
    store.send(A::ForkBeforeTurn(Some(2)), |_| {});
    store.send(A::ForkMachine(MachineId::new("this-mac")), |_| {});
    store.send(A::ForkAgent(Agent::Codex), |_| {});
    store.send(A::ForkBranch(" uhvesta/other ".into()), |_| {});
    store.send(A::ConfirmFork, |_| {});
    store.receive_matching("fork", |a| matches!(a, A::Forked(Ok(_))), |_| {});
    let sent = daemon.last_fork().expect("the fork was sent");
    assert_eq!(sent.kind, ForkKind::Worktree);
    assert_eq!(sent.before_turn, Some(2));
    assert_eq!(sent.machine, Some(MachineId::new("this-mac")));
    assert_eq!(sent.agent, Some(Agent::Codex));
    assert_eq!(sent.branch, "uhvesta/other");
}

#[test]
fn picking_the_parents_own_machine_or_agent_clears_the_override() {
    let (mut store, _) = store("ledger");
    store.send(A::StartFork(ForkKind::Worktree), |_| {});
    store.send(A::ForkMachine(MachineId::new("this-mac")), |_| {});
    store.send(A::ForkMachine(MachineId::new("devbox-1")), |_| {});
    store.send(A::ForkAgent(Agent::Codex), |_| {});
    store.send(A::ForkAgent(Agent::Claude), |_| {});
    let draft = store.state().fork.clone().expect("open");
    assert_eq!(draft.machine, None);
    assert_eq!(draft.agent, None);
}

#[test]
fn code_only_forks_from_now() {
    let (mut store, _) = store("ledger");
    store.send(A::StartFork(ForkKind::Worktree), |_| {});
    store.send(A::ForkBeforeTurn(Some(3)), |_| {});
    store.send(A::ForkKind(ForkKind::Shared), |_| {});
    assert_eq!(store.state().fork.as_ref().expect("open").before_turn, None);
}

#[test]
fn the_turns_are_read_once_when_the_picker_first_opens() {
    let (mut store, daemon) = store("ledger");
    store.send(A::StartFork(ForkKind::Worktree), |_| {});
    store.send(A::ForkPicker(Some(ForkPicker::From)), |_| {});
    store.receive_matching("turns", |a| matches!(a, A::ForkTurns(Ok(_))), |_| {});
    assert!(store.state().fork.as_ref().expect("open").turns.is_some());
    store.send(A::ForkPicker(Some(ForkPicker::From)), |_| {});
    store.send(A::ForkPicker(Some(ForkPicker::From)), |_| {});
    let reads = daemon
        .calls()
        .iter()
        .filter(|call| call.starts_with("list turns"))
        .count();
    assert_eq!(reads, 1);
}

#[test]
fn only_an_online_other_machine_can_be_picked_to_move_to() {
    let (mut store, daemon) = store("rate");
    store.send(A::StartMove, |_| {});
    store.send(A::MovePick(MachineId::new("devbox-1")), |_| {});
    assert_eq!(store.state().moving.as_ref().expect("open").to, None);
    store.send(A::MovePick(MachineId::new("gpu-box")), |_| {});
    assert_eq!(store.state().moving.as_ref().expect("open").to, None);
    // Nothing picked: Move does nothing.
    store.send(A::ConfirmMove, |_| {});
    assert_eq!(daemon.last_move(), None);
    store.send(A::MovePick(MachineId::new("this-mac")), |_| {});
    store.send(A::ConfirmMove, |_| {});
    store.receive(A::Moved(Ok(())), |_| {});
    assert_eq!(
        daemon.last_move(),
        Some((SessionId::new("rate"), MachineId::new("this-mac")))
    );
    assert!(store.state().moving.is_none());
}

#[test]
fn a_failed_move_keeps_the_dialog_and_says_why() {
    let (mut store, daemon) = store("rate");
    daemon.fail_next(humata2_clients::Operation::MoveSession, "link unavailable");
    store.send(A::StartMove, |_| {});
    store.send(A::MovePick(MachineId::new("this-mac")), |_| {});
    store.send(A::ConfirmMove, |_| {});
    store.receive_matching("moved", |a| matches!(a, A::Moved(Err(_))), |_| {});
    let moving = store.state().moving.clone().expect("still open");
    assert_eq!(moving.to, Some(MachineId::new("this-mac")));
    assert!(!moving.sending);
    assert!(store.state().error.is_some());
}

#[test]
fn unpushed_work_asks_before_archiving() {
    let (mut store, daemon) = store("ledger");
    let ledger = SessionId::new("ledger");
    daemon.set_unpushed(&ledger, vec![("api".into(), "1 commit not pushed.".into())]);
    store.send(A::Archive, |_| {});
    store.receive_matching("plan", |a| matches!(a, A::ArchivePlanned { .. }), |_| {});
    let asking = store.state().archiving.clone().expect("it asks");
    assert_eq!(asking.plan.unpushed.len(), 1);
    assert_eq!(daemon.last_archive(), None);

    store.send(A::CancelArchive, |_| {});
    assert!(store.state().archiving.is_none());
    assert_eq!(daemon.last_archive(), None);

    store.send(A::Archive, |_| {});
    store.receive_matching("plan", |a| matches!(a, A::ArchivePlanned { .. }), |_| {});
    store.send(A::ConfirmArchive(true), |_| {});
    store.receive_matching("archived", |a| matches!(a, A::Archived { result: Ok(_), .. }), |_| {});
    assert_eq!(daemon.last_archive(), Some((ledger, true)));
    assert!(store.state().archiving.is_none());
    assert_eq!(store.state().open, None);
}

#[test]
fn a_session_with_nothing_unpushed_archives_without_asking() {
    let (mut store, daemon) = store("ledger");
    store.send(A::Archive, |_| {});
    store.receive_matching("plan", |a| matches!(a, A::ArchivePlanned { .. }), |_| {});
    assert!(store.state().archiving.is_none());
    store.receive_matching("archived", |a| matches!(a, A::Archived { result: Ok(_), .. }), |_| {});
    assert_eq!(daemon.last_archive(), Some((SessionId::new("ledger"), false)));
}
