//! The header's ⋯ session menu and its Rename dialog (SE-7, FC-17).
use humata2_clients::{
    Clients, FakeDaemon, Operation, fixture,
    model::{Session, SessionId, Status},
};
use humata2_session::{MenuItem, Panel, SessionAction as A, SessionState, Sessions};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn store(open: &str) -> (TestStore<Sessions>, FakeDaemon) {
    let (clients, fakes) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
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

fn on_daemon(daemon: &FakeDaemon, id: &str) -> Session {
    daemon
        .workspace()
        .session(&SessionId::new(id))
        .cloned()
        .expect("the daemon keeps the session")
}

#[test]
fn picking_rename_closes_the_menu_and_opens_the_dialog_on_the_current_title() {
    let (mut store, daemon) = store("ledger");
    store.send(A::Menu(true), |_| {});
    assert!(store.state().menu);
    store.send(A::MenuPick(MenuItem::Rename), |_| {});
    assert!(!store.state().menu);
    let draft = store.state().renaming.clone().expect("the dialog opened");
    assert_eq!(draft.title, on_daemon(&daemon, "ledger").title);
    assert!(!draft.rename_branch);
}

#[test]
fn renaming_changes_the_title_and_keeps_the_branch() {
    let (mut store, daemon) = store("ledger");
    let branch = on_daemon(&daemon, "ledger").branch;
    store.send(A::StartRename, |_| {});
    store.send(A::RenameTitle("  Ledger v2 ".into()), |_| {});
    store.send(A::ConfirmRename, |_| {});
    store.receive_matching("renamed", |a| matches!(a, A::Renamed(Ok(()))), |_| {});
    assert!(store.state().renaming.is_none());
    let renamed = on_daemon(&daemon, "ledger");
    assert_eq!(renamed.title, "Ledger v2");
    assert_eq!(renamed.branch, branch);
}

#[test]
fn renaming_the_branch_too_names_it_for_the_new_title() {
    let (mut store, daemon) = store("ledger");
    let branch = on_daemon(&daemon, "ledger").branch;
    store.send(A::StartRename, |_| {});
    store.send(A::RenameTitle("Ledger v2".into()), |_| {});
    store.send(A::RenameBranch(true), |_| {});
    store.send(A::ConfirmRename, |_| {});
    store.receive_matching("renamed", |a| matches!(a, A::Renamed(Ok(()))), |_| {});
    let renamed = on_daemon(&daemon, "ledger");
    assert_ne!(renamed.branch, branch);
    assert!(renamed.branch.ends_with("ledger-v2"));
}

#[test]
fn a_blank_title_is_not_sent() {
    let (mut store, daemon) = store("ledger");
    let title = on_daemon(&daemon, "ledger").title;
    store.send(A::StartRename, |_| {});
    store.send(A::RenameTitle("   ".into()), |_| {});
    store.send(A::ConfirmRename, |_| {});
    let draft = store.state().renaming.clone().expect("the dialog stays open");
    assert!(!draft.sending);
    assert_eq!(on_daemon(&daemon, "ledger").title, title);
}

#[test]
fn an_unchanged_title_closes_the_dialog() {
    let (mut store, daemon) = store("ledger");
    let before = on_daemon(&daemon, "ledger");
    store.send(A::StartRename, |_| {});
    store.send(A::ConfirmRename, |_| {});
    assert!(store.state().renaming.is_none());
    assert_eq!(on_daemon(&daemon, "ledger"), before);
}

#[test]
fn a_failed_rename_keeps_what_was_typed_and_says_why() {
    let (mut store, daemon) = store("ledger");
    daemon.fail_next(Operation::RenameSession, "link unavailable");
    store.send(A::StartRename, |_| {});
    store.send(A::RenameTitle("Ledger v2".into()), |_| {});
    store.send(A::ConfirmRename, |_| {});
    store.receive_matching("refused", |a| matches!(a, A::Renamed(Err(_))), |_| {});
    let draft = store.state().renaming.clone().expect("the dialog stays open");
    assert_eq!(draft.title, "Ledger v2");
    assert!(!draft.sending);
    assert!(store.state().error.is_some());
    store.send(A::CancelRename, |_| {});
    assert!(store.state().renaming.is_none());
}

#[test]
fn stopping_ends_the_agent_and_keeps_the_session() {
    let (mut store, daemon) = store("ledger");
    store.send(A::MenuPick(MenuItem::Stop), |_| {});
    store.receive_matching("stopped", |a| matches!(a, A::Done(Ok(()))), |_| {});
    let stopped = on_daemon(&daemon, "ledger");
    assert_eq!(stopped.status, Status::AgentStopped, "it reads as stopped, not exited (D784)");
    assert!(!stopped.archived);
    assert!(store.state().error.is_none());
}

#[test]
fn an_agent_that_already_exited_is_left_as_it_ended() {
    let (mut store, daemon) = store("postgres");
    let before = on_daemon(&daemon, "postgres").status;
    store.send(A::StopAgent, |_| {});
    assert_eq!(on_daemon(&daemon, "postgres").status, before);
}

#[test]
fn archive_and_push_are_handed_to_the_app() {
    let (mut store, _) = store("ledger");
    store.send(A::MenuPick(MenuItem::Archive), |_| {});
    store.receive_matching("archive", |a| matches!(a, A::Archive), |_| {});
    store.send(A::MenuPick(MenuItem::PushAndPullRequest), |_| {});
    store.receive_matching(
        "the Changes panel",
        |a| matches!(a, A::SetPanel(Some(Panel::Changes))),
        |_| {},
    );
}

#[test]
fn the_menu_opens_only_over_a_session() {
    let (mut store, _) = store("ledger");
    store.send(A::Open(SessionId::new("no-such-session")), |_| {});
    store.send(A::Menu(true), |_| {});
    assert!(!store.state().menu);
}
