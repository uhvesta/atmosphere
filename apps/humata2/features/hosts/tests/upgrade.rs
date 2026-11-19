//! UP-1, D281 (frames 80–82): the machine's page while its daemon waits for
//! a Migrate item. While the item waits, the page is read-only: New session
//! and Open a shell are refused, and a banner points to the item with Open
//! in Inbox. It shows Migrating while the item migrates, the failure when
//! it failed, and "Migrated to <version> at <time>" once it is approved. A
//! migration_needed refusal from an action routes to the item too.
//!
//! The parent feeds the page the Inbox's Migrate items, the app's own
//! included, through `Migrations`; the page keeps them across machines.

use std::cell::RefCell;
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::machines::{FakeMachines, GhLogin, MachineDetail, MachineHealth, Reach};
use humata2_clients::model::{
    Answer, ChannelId, HoldersMoved, MachineId, MigrateCard, MigrateFailure, MigratePhase, MigrateThing,
    Preview, Release, Request, RequestId, RequestKind, SessionId,
};
use humata2_hosts::page::{Delegate, Refused, Upgrade};
use humata2_hosts::{MachinePage, MachinePageAction as A, MachinePageState};
use tca::TestStore;

const DEVBOX: &str = "devbox-1";

fn id(text: &str) -> MachineId {
    MachineId::new(text)
}

fn to() -> Release {
    Release { version: "2.0.5".to_owned(), bundle: "a1b2c3d4e5f60718293a".to_owned() }
}

fn detail(machine: &str) -> MachineDetail {
    MachineDetail {
        machine: id(machine),
        os: "Ubuntu 24.04".to_owned(),
        arch: "x86_64".to_owned(),
        address: format!("{machine}.lan"),
        reach: Reach::Ssh,
        daemon_version: "2.0.4".to_owned(),
        health: MachineHealth {
            gh: Some(GhLogin { user: "uhvesta".to_owned(), scopes: vec!["repo".to_owned()] }),
            agents: vec![],
            disk_free: 11,
            disk_total: 500,
            variables: vec![],
            skills_problem: None,
            git_version: "2.43.0".to_owned(),
            gh_version: "2.83.0".to_owned(),
        },
        folders: vec![],
        sessions: vec![SessionId::new("s1")],
        rules: vec![],
        stop: None,
    }
}

fn migrate(request: &str, thing: MigrateThing, machine: &str, phase: MigratePhase) -> Request {
    Request {
        id: RequestId::new(request),
        session: SessionId::new(""),
        channel: ChannelId::new(""),
        kind: RequestKind::Migrate,
        title: format!("Migrate {thing:?} on {machine}"),
        reason: String::new(),
        preview: Preview::Migrate(MigrateCard {
            thing,
            machine: id(machine),
            from: Release { version: "2.0.4".to_owned(), bundle: "9f3e2d1c0b1a".to_owned() },
            to: to(),
            changes: vec!["Daemon".to_owned()],
            keeps_running: vec![],
            phase,
            waiting_sessions: vec![SessionId::new("s1")],
            migrated: None,
            steps: Vec::new(),
            holders: Vec::new(),
            moved: None,
        }),
        doing: String::new(),
        rule: String::new(),
        undoable: false,
        asked: "1m".to_owned(),
        answer: None,
        operation: String::new(),
        steps: vec![],
        why_you: String::new(),
        asked_by: None,
        undo_note: None,
        choices: vec![],
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

fn devbox_daemon(phase: MigratePhase) -> Request {
    migrate("m-devbox", MigrateThing::Daemon, DEVBOX, phase)
}

fn migrated(mut request: Request, at: &str) -> Request {
    request.answer = Some(Answer::Approved);
    if let Preview::Migrate(card) = &mut request.preview {
        card.migrated = Some(at.to_owned());
    }
    request
}

/// `machine`'s page, loaded, after the parent fed it `migrations`.
fn page(machine: &str, migrations: Vec<Request>) -> (TestStore<MachinePage>, FakeMachines) {
    let fake = FakeMachines::new();
    fake.set_detail(Ok(detail(machine)));
    let mut store =
        TestStore::new(MachinePageState::default(), MachinePage::new(Arc::new(fake.clone())))
            .non_exhaustive();
    store.send(A::Migrations(migrations), |_| {});
    store.send(A::Show(id(machine)), |_| {});
    store.receive_matching("loaded", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
    (store, fake)
}

/// The first delegate the page hands its parent, if one comes.
fn delegate(store: &mut TestStore<MachinePage>) -> Option<Delegate> {
    let got = RefCell::new(None);
    let found = store.try_receive_matching(
        "a delegate",
        |a| match a {
            A::Delegate(delegate) => {
                *got.borrow_mut() = Some(delegate.clone());
                true
            }
            _ => false,
        },
        |_| {},
    );
    found.ok().and_then(|()| got.into_inner())
}

fn waiting_request() -> RequestId {
    RequestId::new("m-devbox")
}

// Read-only while the daemon's Migrate item waits (frame 80).

#[test]
fn a_waiting_daemon_migrate_makes_the_page_read_only() {
    let (store, _fake) = page(DEVBOX, vec![devbox_daemon(MigratePhase::Waiting)]);
    assert_eq!(
        store.state().upgrade(),
        Upgrade::ReadOnly { request: waiting_request(), to: to() }
    );
    assert!(store.state().upgrade().read_only());
}

#[test]
fn while_read_only_new_session_and_open_shell_are_refused() {
    for action in [A::NewSession, A::OpenShell] {
        let (mut store, _fake) = page(DEVBOX, vec![devbox_daemon(MigratePhase::Waiting)]);
        store.send(action, |_| {});
        assert_eq!(
            store.state().refused,
            Some(Refused::MigrationNeeded(Some(waiting_request())))
        );
        assert_eq!(delegate(&mut store), None, "nothing goes to the parent");
    }
}

#[test]
fn opening_a_session_still_works_while_read_only() {
    // Transcripts stay readable; only new work is held back.
    let (mut store, _fake) = page(DEVBOX, vec![devbox_daemon(MigratePhase::Waiting)]);
    store.send(A::OpenSession(SessionId::new("s1")), |_| {});
    assert_eq!(delegate(&mut store), Some(Delegate::OpenSession(SessionId::new("s1"))));
}

#[test]
fn open_in_inbox_hands_the_item_to_the_parent() {
    let (mut store, _fake) = page(DEVBOX, vec![devbox_daemon(MigratePhase::Waiting)]);
    store.send(A::OpenInInbox, |_| {});
    assert_eq!(delegate(&mut store), Some(Delegate::OpenInInbox(waiting_request())));
}

#[test]
fn only_this_machines_daemon_or_store_items_make_it_read_only() {
    let others = vec![
        migrate("m-gpu", MigrateThing::Daemon, "gpu-box", MigratePhase::Waiting),
        migrate("m-cli", MigrateThing::Cli, DEVBOX, MigratePhase::Waiting),
        migrate("m-holders", MigrateThing::Holders, DEVBOX, MigratePhase::Waiting),
    ];
    let (mut store, _fake) = page(DEVBOX, others);
    assert_eq!(store.state().upgrade(), Upgrade::Current);
    assert!(!store.state().upgrade().read_only());
    store.send(A::NewSession, |_| {});
    assert_eq!(store.state().refused, None);
    assert_eq!(delegate(&mut store), Some(Delegate::NewSession(id(DEVBOX))));

    let store_item = migrate("m-store", MigrateThing::Store, DEVBOX, MigratePhase::Waiting);
    store.send(A::Migrations(vec![store_item]), |_| {});
    assert!(store.state().upgrade().read_only());
}

#[test]
fn the_items_are_kept_when_another_machine_is_shown() {
    let (mut store, _fake) = page("gpu-box", vec![devbox_daemon(MigratePhase::Waiting)]);
    assert_eq!(store.state().upgrade(), Upgrade::Current, "gpu-box has no item");

    store.send(A::Show(id(DEVBOX)), |_| {});
    store.receive_matching("loaded", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
    assert_eq!(
        store.state().upgrade(),
        Upgrade::ReadOnly { request: waiting_request(), to: to() }
    );
}

// Migrating, failed and migrated (frame 81).

#[test]
fn while_migrating_the_page_shows_migrating_and_stays_read_only() {
    let (store, _fake) = page(DEVBOX, vec![devbox_daemon(MigratePhase::Migrating)]);
    assert_eq!(
        store.state().upgrade(),
        Upgrade::Migrating { request: waiting_request(), to: to() }
    );
    assert!(store.state().upgrade().read_only());
}

#[test]
fn after_a_failure_the_page_shows_it_and_stays_read_only() {
    let failure = MigrateFailure {
        message: "2.0.5 failed to start twice, so 2.0.4 is running again".to_owned(),
        fix: "Fix what the log names, then Migrate again".to_owned(),
    };
    let (mut store, _fake) = page(DEVBOX, vec![devbox_daemon(MigratePhase::Waiting)]);
    store.send(A::Migrations(vec![devbox_daemon(MigratePhase::Failed(failure.clone()))]), |_| {});
    assert_eq!(
        store.state().upgrade(),
        Upgrade::Failed { request: waiting_request(), to: to(), failure }
    );
    assert!(store.state().upgrade().read_only());
    store.send(A::OpenInInbox, |_| {});
    assert_eq!(delegate(&mut store), Some(Delegate::OpenInInbox(waiting_request())));
}

#[test]
fn after_approval_the_page_shows_migrated_and_works_again() {
    let (mut store, _fake) = page(DEVBOX, vec![devbox_daemon(MigratePhase::Migrating)]);
    let done = migrated(devbox_daemon(MigratePhase::Migrating), "2 min ago");
    store.send(A::Migrations(vec![done]), |_| {});

    assert_eq!(
        store.state().upgrade(),
        Upgrade::Migrated { to: to(), at: "2 min ago".to_owned() }
    );
    assert!(!store.state().upgrade().read_only());
    store.send(A::OpenShell, |_| {});
    assert_eq!(store.state().refused, None);
    assert_eq!(delegate(&mut store), Some(Delegate::OpenShell(id(DEVBOX))));
}

#[test]
fn a_withdrawn_item_leaves_the_page_current() {
    let (mut store, _fake) = page(DEVBOX, vec![devbox_daemon(MigratePhase::Waiting)]);
    store.send(A::Migrations(vec![]), |_| {});
    assert_eq!(store.state().upgrade(), Upgrade::Current);
    store.send(A::NewSession, |_| {});
    assert_eq!(delegate(&mut store), Some(Delegate::NewSession(id(DEVBOX))));
}

// A migration_needed refusal from an action (frame 82).

fn send_variable(store: &mut TestStore<MachinePage>) {
    store.send(A::BeginVariable(Some("STRIPE_KEY".to_owned())), |_| {});
    store.send(A::VariableValue("sk_test".to_owned()), |_| {});
    store.send(A::SendVariable, |_| {});
    store.receive_matching("refused", |a| matches!(a, A::VariableSent(Err(_))), |_| {});
}

#[test]
fn a_migration_needed_refusal_routes_to_its_item() {
    // The refusal can come before the item reaches the Inbox.
    let (mut store, fake) = page(DEVBOX, vec![]);
    fake.fail(
        "set_variable",
        DaemonError::MigrationNeeded {
            thing: MigrateThing::Daemon,
            machine: id(DEVBOX),
            request: Some(RequestId::new("m-named")),
        },
    );
    send_variable(&mut store);

    assert_eq!(
        store.state().refused,
        Some(Refused::MigrationNeeded(Some(RequestId::new("m-named"))))
    );
    store.send(A::OpenInInbox, |_| {});
    assert_eq!(delegate(&mut store), Some(Delegate::OpenInInbox(RequestId::new("m-named"))));
}

#[test]
fn a_refusal_that_names_no_item_routes_to_the_machines_item() {
    let (mut store, fake) = page(DEVBOX, vec![]);
    fake.fail(
        "set_variable",
        DaemonError::MigrationNeeded {
            thing: MigrateThing::Daemon,
            machine: id(DEVBOX),
            request: None,
        },
    );
    send_variable(&mut store);
    assert_eq!(store.state().refused, Some(Refused::MigrationNeeded(None)));

    store.send(A::Migrations(vec![devbox_daemon(MigratePhase::Waiting)]), |_| {});
    store.send(A::OpenInInbox, |_| {});
    assert_eq!(delegate(&mut store), Some(Delegate::OpenInInbox(waiting_request())));
}

#[test]
fn open_in_inbox_with_no_item_hands_nothing_on() {
    let (mut store, _fake) = page(DEVBOX, vec![]);
    store.send(A::OpenInInbox, |_| {});
    assert_eq!(delegate(&mut store), None);
}

// A host waiting on Migrate never claims it is gone (owner, 0.2.1): its old
// daemon answering a call it doesn't know isn't the machine being deleted,
// and the read-only banner already says why the page can't load.

#[test]
fn a_failed_load_while_a_migrate_waits_shows_no_error_of_its_own() {
    let (mut store, fake) = page(DEVBOX, vec![devbox_daemon(MigratePhase::Waiting)]);
    fake.set_detail(Err(DaemonError::NotFound { what: "The machine".to_owned() }));
    store.send(A::Reload, |_| {});
    store.receive_matching("load failed", |a| matches!(a, A::Loaded(_, Err(_))), |_| {});
    assert_eq!(store.state().shown_load_error(), None);
}

#[test]
fn a_failed_load_on_a_current_machine_still_shows_its_error() {
    let (mut store, fake) = page(DEVBOX, Vec::new());
    fake.set_detail(Err(DaemonError::NotFound { what: "The machine".to_owned() }));
    store.send(A::Reload, |_| {});
    store.receive_matching("load failed", |a| matches!(a, A::Loaded(_, Err(_))), |_| {});
    assert!(store.state().shown_load_error().is_some());
}

// The Holders Migrate's count of moved terminals (frame 125).

fn holders_item(machine: &str, phase: MigratePhase, done: u32) -> Request {
    let mut request = migrate("m-holders", MigrateThing::Holders, machine, phase);
    if let Preview::Migrate(card) = &mut request.preview {
        card.moved = Some(HoldersMoved { done, total: 7, started_ms: 5 });
    }
    request
}

#[test]
fn a_migrating_holders_item_shows_how_many_terminals_moved() {
    let (store, _fake) = page(DEVBOX, vec![holders_item(DEVBOX, MigratePhase::Migrating, 3)]);
    assert_eq!(store.state().holders_moving(), Some(HoldersMoved { done: 3, total: 7, started_ms: 5 }));
}

#[test]
fn a_waiting_or_other_machines_holders_item_shows_no_count() {
    let (store, _fake) = page(DEVBOX, vec![holders_item(DEVBOX, MigratePhase::Waiting, 3)]);
    assert_eq!(store.state().holders_moving(), None);
    let (store, _fake) = page(DEVBOX, vec![holders_item("gpu-box", MigratePhase::Migrating, 3)]);
    assert_eq!(store.state().holders_moving(), None);
}
