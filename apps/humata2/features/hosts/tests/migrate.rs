//! An add blocked because This Mac's daemon is behind this app: the dialog
//! offers Migrate This Mac only while a Migrate item for that daemon exists,
//! approves that item through its parent, and runs the same add again once
//! the item is migrated. A failed migrate shows its failure and offers the
//! action again.
use std::sync::Arc;

use humata2_clients::machines::{
    AddEvent, AddOutcome, AddRequest, AddTarget, Call, FakeMachines, OperationId, StepKind,
};
use humata2_clients::model::{
    Answer, ChannelId, MachineId, MigrateCard, MigrateFailure, MigratePhase, MigrateThing, Preview,
    Release, Request, RequestId, RequestKind, SessionId,
};
use humata2_hosts::add::{Choice, Delegate, MigrateStage};
use humata2_hosts::{AddMachine, AddMachineAction as A, AddMachineState};
use tca::{StreamEnd, TestStore};

fn store() -> (TestStore<AddMachine>, FakeMachines) {
    let fake = FakeMachines::new();
    let state = AddMachineState {
        open: true,
        choice: Some(Choice::Host("devbox-2".to_owned())),
        name: "build-box".to_owned(),
        ..AddMachineState::default()
    };
    let store = TestStore::new(state, AddMachine::new(Arc::new(fake.clone()))).non_exhaustive();
    (store, fake)
}

fn started() -> AddEvent {
    AddEvent::Started {
        operation: OperationId::new("op-1"),
        steps: vec![StepKind::Connect, StepKind::Check, StepKind::Install],
    }
}

fn blocked_events() -> Vec<AddEvent> {
    vec![
        started(),
        AddEvent::Outcome(AddOutcome::NeedsMigrate {
            platform: "linux-x86_64".to_owned(),
            what: "Migrate the daemon on This Mac first.".to_owned(),
        }),
    ]
}

fn same_add() -> Call {
    Call::Add(AddRequest {
        target: AddTarget::Ssh { host: "devbox-2".to_owned() },
        name: Some("build-box".to_owned()),
        trusted_fingerprint: None,
    })
}

fn item(thing: MigrateThing, phase: MigratePhase, answer: Option<Answer>) -> Request {
    Request {
        id: RequestId::new("m-this-mac"),
        session: SessionId::new(""),
        channel: ChannelId::new(""),
        kind: RequestKind::Migrate,
        title: "Migrate the daemon on This Mac".to_owned(),
        reason: String::new(),
        preview: Preview::Migrate(MigrateCard {
            thing,
            machine: MachineId::new("this-mac"),
            from: Release { version: "2.0.4".to_owned(), bundle: "9f3e2d1c0b1a".to_owned() },
            to: Release { version: "2.0.5".to_owned(), bundle: "a1b2c3d4e5f60718293a".to_owned() },
            changes: vec!["Daemon".to_owned()],
            keeps_running: Vec::new(),
            phase,
            waiting_sessions: Vec::new(),
            migrated: answer.as_ref().map(|_| "just now".to_owned()),
            steps: Vec::new(),
            holders: Vec::new(),
            moved: None,
        }),
        doing: String::new(),
        rule: String::new(),
        undoable: false,
        asked: "1m".to_owned(),
        answer,
        operation: String::new(),
        steps: Vec::new(),
        why_you: String::new(),
        asked_by: None,
        undo_note: None,
        choices: Vec::new(),
        on_screen: None,
        dismiss_only: false,
        facts: Vec::new(),
        affects: Vec::new(),
        undo_text: String::new(),
        working_on: String::new(),
        asked_why: None,
        rule_label: String::new(),
    }
}

fn waiting() -> Request {
    item(MigrateThing::Daemon, MigratePhase::Waiting, None)
}

fn migrating() -> Request {
    item(MigrateThing::Daemon, MigratePhase::Migrating, None)
}

fn migrated() -> Request {
    item(MigrateThing::Daemon, MigratePhase::Migrating, Some(Answer::Approved))
}

fn failure() -> MigrateFailure {
    MigrateFailure {
        message: "The daemon did not restart.".to_owned(),
        fix: "Click Migrate again.".to_owned(),
    }
}

fn adds(fake: &FakeMachines) -> usize {
    fake.calls().into_iter().filter(|call| matches!(call, Call::Add(_))).count()
}

/// The dialog after an add that stopped behind This Mac's daemon.
fn blocked() -> (TestStore<AddMachine>, FakeMachines) {
    let (mut store, fake) = store();
    fake.script_add(blocked_events());
    store.send(A::Start, |_| {});
    store.receive_matching(
        "the outcome",
        |a| matches!(a, A::Progress(AddEvent::Outcome(AddOutcome::NeedsMigrate { .. }))),
        |_| {},
    );
    store.receive(A::ProgressEnded(StreamEnd::Finished), |_| {});
    (store, fake)
}

#[test]
fn a_blocked_add_stops_and_keeps_what_was_asked_for() {
    let (store, fake) = blocked();
    let state = store.state();
    assert!(!state.running);
    let Some(migrate) = &state.migrate else {
        panic!("the add should be blocked on the migrate");
    };
    assert_eq!(migrate.platform, "linux-x86_64");
    assert_eq!(migrate.stage, MigrateStage::Blocked);
    assert_eq!(adds(&fake), 1);
}

#[test]
fn the_action_shows_only_while_a_migrate_item_for_the_daemon_exists() {
    let (mut store, _fake) = blocked();
    assert_eq!(store.state().migrate_offer(), None);
    store.send(A::Migrations(vec![item(MigrateThing::Store, MigratePhase::Waiting, None)]), |_| {});
    assert_eq!(store.state().migrate_offer(), None, "a store item is not the daemon's");
    store.send(A::Migrations(vec![waiting()]), |_| {});
    assert_eq!(store.state().migrate_offer(), Some(RequestId::new("m-this-mac")));
}

#[test]
fn without_an_item_the_action_does_nothing() {
    let (mut store, fake) = blocked();
    store.send(A::MigrateThisMac, |_| {});
    assert_eq!(store.state().migrate.as_ref().map(|m| m.stage.clone()), Some(MigrateStage::Blocked));
    assert_eq!(adds(&fake), 1);
}

#[test]
fn migrating_asks_the_parent_to_approve_then_the_same_add_runs_again() {
    let (mut store, fake) = blocked();
    fake.script_add(vec![
        started(),
        AddEvent::Outcome(AddOutcome::Added(MachineId::new("build-box"))),
    ]);
    store.send(A::Migrations(vec![waiting()]), |_| {});
    store.send(A::MigrateThisMac, |_| {});
    store.receive(A::Delegate(Delegate::Migrate(RequestId::new("m-this-mac"))), |_| {});
    assert_eq!(
        store.state().migrate.as_ref().map(|m| m.stage.clone()),
        Some(MigrateStage::Migrating(RequestId::new("m-this-mac")))
    );
    assert_eq!(store.state().migrate_offer(), None, "no second click while it migrates");

    // The item is still migrating: nothing runs yet.
    store.send(A::Migrations(vec![migrating()]), |_| {});
    assert_eq!(adds(&fake), 1);
    assert!(!store.state().running);

    // It was migrated: the same add runs again, from the same inputs.
    store.send(A::Migrations(vec![migrated()]), |_| {});
    assert!(store.state().running);
    assert_eq!(store.state().migrate, None);
    store.receive_matching("the machine is added", |a| matches!(a, A::Delegate(Delegate::Added(_))), |_| {});
    store.receive_matching("a toast", |a| matches!(a, A::Delegate(Delegate::Toast(_))), |_| {});
    let calls: Vec<Call> = fake.calls().into_iter().filter(|call| matches!(call, Call::Add(_))).collect();
    assert_eq!(calls, vec![same_add(), same_add()]);
}

#[test]
fn an_item_migrated_before_the_click_does_not_run_the_add() {
    let (mut store, fake) = blocked();
    store.send(A::Migrations(vec![migrated()]), |_| {});
    assert_eq!(adds(&fake), 1);
    assert!(!store.state().running);
    assert_eq!(store.state().migrate.as_ref().map(|m| m.stage.clone()), Some(MigrateStage::Blocked));
}

#[test]
fn a_failed_migrate_shows_its_failure_and_offers_the_action_again() {
    let (mut store, fake) = blocked();
    store.send(A::Migrations(vec![waiting()]), |_| {});
    store.send(A::MigrateThisMac, |_| {});
    store.receive(A::Delegate(Delegate::Migrate(RequestId::new("m-this-mac"))), |_| {});
    store.send(
        A::Migrations(vec![item(MigrateThing::Daemon, MigratePhase::Failed(failure()), None)]),
        |_| {},
    );
    assert_eq!(
        store.state().migrate.as_ref().map(|m| m.stage.clone()),
        Some(MigrateStage::Failed(failure()))
    );
    assert_eq!(store.state().migrate_offer(), Some(RequestId::new("m-this-mac")));
    assert_eq!(adds(&fake), 1, "the add does not run again after a failure");
}

#[test]
fn an_item_that_goes_back_to_waiting_after_the_click_is_a_failure() {
    let (mut store, fake) = blocked();
    store.send(A::Migrations(vec![waiting()]), |_| {});
    store.send(A::MigrateThisMac, |_| {});
    store.receive(A::Delegate(Delegate::Migrate(RequestId::new("m-this-mac"))), |_| {});
    store.send(A::Migrations(vec![waiting()]), |_| {});
    assert!(matches!(
        store.state().migrate.as_ref().map(|m| &m.stage),
        Some(MigrateStage::Failed(_))
    ));
    assert_eq!(adds(&fake), 1);
}

// This Mac's daemon waiting for its Migrate item is at another release than
// this app. An add then would run across releases, and the daemon could
// answer in words this app doesn't know (a first-seen host key, say), so the
// add waits behind the same Migrate This Mac as a blocked add (force-upgrade).

#[test]
fn an_add_while_this_macs_daemon_waits_to_migrate_does_not_run_and_offers_migrate() {
    let (mut store, fake) = store();
    store.send(A::Migrations(vec![waiting()]), |_| {});
    store.send(A::Start, |_| {});
    assert_eq!(adds(&fake), 0);
    assert!(!store.state().running);
    assert_eq!(store.state().migrate.as_ref().map(|m| m.stage.clone()), Some(MigrateStage::Blocked));
    assert_eq!(store.state().migrate_offer(), Some(RequestId::new("m-this-mac")));
}

#[test]
fn an_add_held_behind_this_macs_migrate_runs_once_it_is_migrated() {
    let (mut store, fake) = store();
    fake.script_add(vec![started(), AddEvent::Outcome(AddOutcome::Added(MachineId::new("build-box")))]);
    store.send(A::Migrations(vec![waiting()]), |_| {});
    store.send(A::Start, |_| {});
    store.send(A::MigrateThisMac, |_| {});
    store.receive(A::Delegate(Delegate::Migrate(RequestId::new("m-this-mac"))), |_| {});
    store.send(A::Migrations(vec![migrated()]), |_| {});
    store.receive_matching("the machine is added", |a| matches!(a, A::Delegate(Delegate::Added(_))), |_| {});
    store.receive_matching("a toast", |a| matches!(a, A::Delegate(Delegate::Toast(_))), |_| {});
    let calls: Vec<Call> = fake.calls().into_iter().filter(|call| matches!(call, Call::Add(_))).collect();
    assert_eq!(calls, vec![same_add()]);
}

#[test]
fn an_add_while_this_mac_is_migrating_waits_and_runs_once_it_is_migrated() {
    let (mut store, fake) = store();
    fake.script_add(vec![started(), AddEvent::Outcome(AddOutcome::Added(MachineId::new("build-box")))]);
    store.send(A::Migrations(vec![migrating()]), |_| {});
    store.send(A::Start, |_| {});
    assert_eq!(adds(&fake), 0);
    assert_eq!(
        store.state().migrate.as_ref().map(|m| m.stage.clone()),
        Some(MigrateStage::Migrating(RequestId::new("m-this-mac")))
    );
    store.send(A::Migrations(vec![migrated()]), |_| {});
    store.receive_matching("the machine is added", |a| matches!(a, A::Delegate(Delegate::Added(_))), |_| {});
    store.receive_matching("a toast", |a| matches!(a, A::Delegate(Delegate::Toast(_))), |_| {});
    assert_eq!(adds(&fake), 1);
}

#[test]
fn an_add_after_this_mac_was_migrated_runs_at_once() {
    let (mut store, fake) = store();
    fake.script_add(vec![started(), AddEvent::Outcome(AddOutcome::Added(MachineId::new("build-box")))]);
    store.send(A::Migrations(vec![migrated()]), |_| {});
    store.send(A::Start, |_| {});
    assert_eq!(store.state().migrate, None);
    store.receive_matching("the machine is added", |a| matches!(a, A::Delegate(Delegate::Added(_))), |_| {});
    store.receive_matching("a toast", |a| matches!(a, A::Delegate(Delegate::Toast(_))), |_| {});
    assert_eq!(adds(&fake), 1);
}

#[test]
fn a_store_item_waiting_does_not_hold_an_add() {
    let (mut store, fake) = store();
    fake.script_add(vec![started(), AddEvent::Outcome(AddOutcome::Added(MachineId::new("build-box")))]);
    store.send(A::Migrations(vec![item(MigrateThing::Store, MigratePhase::Waiting, None)]), |_| {});
    store.send(A::Start, |_| {});
    assert_eq!(store.state().migrate, None);
    store.receive_matching("the machine is added", |a| matches!(a, A::Delegate(Delegate::Added(_))), |_| {});
    store.receive_matching("a toast", |a| matches!(a, A::Delegate(Delegate::Toast(_))), |_| {});
    assert_eq!(adds(&fake), 1);
}
