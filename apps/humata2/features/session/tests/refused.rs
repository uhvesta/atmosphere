//! D377: a session action the daemon refuses because something must be
//! migrated first leaves a structured refusal on the session state, naming
//! what, where and the Migrate item that fixes it. The next session action
//! that succeeds clears it, and so does the Inbox showing that item
//! migrated.
use humata2_clients::{
    Clients, DaemonError, Fakes, Operation, fixture,
    model::{
        Agent, Answer, ChannelId, MachineId, MigrateCard, MigratePhase, MigrateThing, Preview,
        Release, Request, RequestId, RequestKind, SessionId, Turn,
    },
};
use humata2_session::{SessionAction as A, SessionRefused, SessionState, SessionView, Sessions};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

/// The design workspace with ledger open. Its agent takes a message only
/// when told to send it, so each send is one explicit SendQueued.
fn setup() -> (TestStore<Sessions>, Fakes) {
    let mut workspace = fixture::design();
    let session = workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == ledger())
        .expect("the design fixture has ledger");
    session.agent = Agent::Codex;
    session.turn = Turn::Ended;
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = SessionState {
        workspace: Arc::new(workspace),
        open: Some(ledger()),
        ..Default::default()
    };
    state.views.insert(ledger(), SessionView::default());
    (
        TestStore::new(state, Sessions::new(clients)).non_exhaustive(),
        fakes,
    )
}

fn needed(request: Option<&str>) -> DaemonError {
    DaemonError::MigrationNeeded {
        thing: MigrateThing::Daemon,
        machine: MachineId::new("this-mac"),
        request: request.map(RequestId::new),
    }
}

fn refusal(request: Option<&str>) -> SessionRefused {
    SessionRefused::MigrationNeeded {
        thing: MigrateThing::Daemon,
        machine: MachineId::new("this-mac"),
        request: request.map(RequestId::new),
    }
}

/// Queues `text` (when given) and sends the first queued message.
fn send(store: &mut TestStore<Sessions>, text: Option<&str>) {
    if let Some(text) = text {
        store.send(A::Draft(text.into()), |_| {});
        store.send(A::Submit, |_| {});
    }
    store.send(A::SendQueued(0), |_| {});
    store.receive_matching("the send's result", |a| matches!(a, A::Sent { .. }), |_| {});
}

fn resume(store: &mut TestStore<Sessions>) {
    store.send(A::Resume, |_| {});
    store.receive_matching("the resume's result", |a| matches!(a, A::Done(_)), |_| {});
}

fn migrate_item(
    id: &str,
    thing: MigrateThing,
    machine: &str,
    phase: MigratePhase,
    answer: Option<Answer>,
) -> Request {
    Request {
        id: RequestId::new(id),
        session: SessionId::new(""),
        channel: ChannelId::new(""),
        kind: RequestKind::Migrate,
        title: format!("Migrate {thing:?} on {machine}"),
        reason: String::new(),
        preview: Preview::Migrate(MigrateCard {
            thing,
            machine: MachineId::new(machine),
            from: Release { version: "2.0.4".to_owned(), bundle: "9f3e2d1c0b1a".to_owned() },
            to: Release { version: "2.0.5".to_owned(), bundle: "a1b2c3d4e5f6".to_owned() },
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
        to_do: None,
    }
}

fn waiting(id: &str, thing: MigrateThing, machine: &str) -> Request {
    migrate_item(id, thing, machine, MigratePhase::Waiting, None)
}

fn migrating(id: &str, thing: MigrateThing, machine: &str) -> Request {
    migrate_item(id, thing, machine, MigratePhase::Migrating, None)
}

fn migrated(id: &str, thing: MigrateThing, machine: &str) -> Request {
    migrate_item(id, thing, machine, MigratePhase::Waiting, Some(Answer::Approved))
}

// Set by a refused action.

#[test]
fn a_send_refused_for_a_migration_records_what_and_where() {
    let (mut store, fakes) = setup();
    assert_eq!(store.state().refused, None);
    fakes.daemon.fail_next_with(Operation::Send, needed(Some("m-local")));

    send(&mut store, Some("run the tests"));

    assert_eq!(store.state().refused, Some(refusal(Some("m-local"))));
    // The message is kept to send again once the migrate is done.
    assert_eq!(
        store.state().view().map(|view| view.queue.len()),
        Some(1),
        "the refused message stays queued"
    );
}

#[test]
fn a_refusal_that_names_no_item_is_still_recorded() {
    let (mut store, fakes) = setup();
    fakes.daemon.fail_next_with(Operation::Send, needed(None));
    send(&mut store, Some("run the tests"));
    assert_eq!(store.state().refused, Some(refusal(None)));
}

#[test]
fn a_refused_resume_records_it() {
    let (mut store, fakes) = setup();
    fakes.daemon.fail_next_with(Operation::Resume, needed(Some("m-local")));
    resume(&mut store);
    assert_eq!(store.state().refused, Some(refusal(Some("m-local"))));
}

#[test]
fn a_refused_start_records_it() {
    // The terminals client can't be told to fail, so the result arrives as
    // the daemon would send it.
    let (mut store, _fakes) = setup();
    store.send(
        A::TerminalOpened {
            session: ledger(),
            group: 0,
            split: false,
            result: Err(needed(Some("m-local"))),
        },
        |_| {},
    );
    store.settle();
    store.skip_received();
    assert_eq!(store.state().refused, Some(refusal(Some("m-local"))));
}

#[test]
fn any_other_failure_is_not_a_refusal() {
    let (mut store, fakes) = setup();
    fakes.daemon.fail_next(Operation::Send, "the link dropped");
    send(&mut store, Some("run the tests"));
    assert_eq!(store.state().refused, None);
    assert!(store.state().error.is_some(), "it is still an error");
}

// Cleared by the next action that succeeds.

#[test]
fn the_next_successful_send_clears_the_refusal() {
    let (mut store, fakes) = setup();
    fakes.daemon.fail_next_with(Operation::Send, needed(Some("m-local")));
    send(&mut store, Some("run the tests"));
    assert!(store.state().refused.is_some());

    // The same message, sent again.
    send(&mut store, None);
    assert_eq!(store.state().refused, None);
    assert_eq!(store.state().view().map(|view| view.queue.len()), Some(0));
}

#[test]
fn a_failed_retry_keeps_the_refusal() {
    let (mut store, fakes) = setup();
    fakes.daemon.fail_next_with(Operation::Send, needed(Some("m-local")));
    send(&mut store, Some("run the tests"));
    fakes.daemon.fail_next(Operation::Send, "the link dropped");
    send(&mut store, None);
    assert_eq!(store.state().refused, Some(refusal(Some("m-local"))));
}

#[test]
fn a_successful_resume_clears_the_refusal() {
    let (mut store, fakes) = setup();
    fakes.daemon.fail_next_with(Operation::Resume, needed(Some("m-local")));
    resume(&mut store);
    assert!(store.state().refused.is_some());
    resume(&mut store);
    assert_eq!(store.state().refused, None);
}

#[test]
fn a_terminal_that_opens_clears_the_refusal() {
    let (mut store, fakes) = setup();
    fakes.daemon.fail_next_with(Operation::Send, needed(Some("m-local")));
    send(&mut store, Some("run the tests"));
    assert!(store.state().refused.is_some());

    store.send(A::OpenTerminal { agent: Agent::Shell, split: false }, |_| {});
    store.receive_matching(
        "the terminal opened",
        |a| matches!(a, A::TerminalOpened { result: Ok(_), .. }),
        |_| {},
    );
    assert_eq!(store.state().refused, None);
}

// Cleared when the Inbox shows the fix migrated.

#[test]
fn the_named_item_migrated_clears_the_refusal_but_waiting_or_migrating_does_not() {
    let (mut store, fakes) = setup();
    fakes.daemon.fail_next_with(Operation::Send, needed(Some("m-local")));
    send(&mut store, Some("run the tests"));

    store.send(A::Migrations(vec![waiting("m-local", MigrateThing::Daemon, "this-mac")]), |_| {});
    assert_eq!(store.state().refused, Some(refusal(Some("m-local"))));
    store.send(
        A::Migrations(vec![migrating("m-local", MigrateThing::Daemon, "this-mac")]),
        |_| {},
    );
    assert_eq!(store.state().refused, Some(refusal(Some("m-local"))));
    // Another machine's item migrating changes nothing here.
    store.send(
        A::Migrations(vec![
            migrated("m-gpu", MigrateThing::Daemon, "gpu-box"),
            migrating("m-local", MigrateThing::Daemon, "this-mac"),
        ]),
        |_| {},
    );
    assert_eq!(store.state().refused, Some(refusal(Some("m-local"))));

    store.send(A::Migrations(vec![migrated("m-local", MigrateThing::Daemon, "this-mac")]), |_| {});
    assert_eq!(store.state().refused, None);
}

#[test]
fn a_refusal_naming_no_item_clears_once_that_thing_on_that_machine_is_migrated() {
    let (mut store, fakes) = setup();
    fakes.daemon.fail_next_with(Operation::Send, needed(None));
    send(&mut store, Some("run the tests"));

    // This Mac's CLI, or gpu-box's daemon, isn't what was behind.
    store.send(
        A::Migrations(vec![
            migrated("m-cli", MigrateThing::Cli, "this-mac"),
            migrated("m-gpu", MigrateThing::Daemon, "gpu-box"),
        ]),
        |_| {},
    );
    assert_eq!(store.state().refused, Some(refusal(None)));

    store.send(
        A::Migrations(vec![migrated("m-local", MigrateThing::Daemon, "this-mac")]),
        |_| {},
    );
    assert_eq!(store.state().refused, None);
}

// Typing into a terminal and attaching to write are the terminal view's
// own calls, so their results arrive as `Typed`.

#[test]
fn typing_refused_for_a_migration_records_what_and_where() {
    let (mut store, _fakes) = setup();
    assert_eq!(store.state().refused, None);
    store.send(A::Typed(Err(needed(Some("m-local")))), |_| {});
    assert_eq!(store.state().refused, Some(refusal(Some("m-local"))));
}

#[test]
fn typing_that_fails_for_another_reason_is_not_a_refusal() {
    let (mut store, _fakes) = setup();
    store.send(
        A::Typed(Err(DaemonError::Refused {
            what: "Typing into the terminal".to_owned(),
            reason: "its connection ended".to_owned(),
        })),
        |_| {},
    );
    assert_eq!(store.state().refused, None);
    assert_eq!(store.state().error, None, "the terminal view says it itself");
}

#[test]
fn the_next_successful_write_clears_the_typing_refusal() {
    let (mut store, _fakes) = setup();
    store.send(A::Typed(Err(needed(None))), |_| {});
    assert_eq!(store.state().refused, Some(refusal(None)));
    store.send(A::Typed(Ok(())), |_| {});
    assert_eq!(store.state().refused, None);
}

#[test]
fn a_typing_refusal_clears_when_its_item_is_migrated() {
    let (mut store, _fakes) = setup();
    store.send(A::Typed(Err(needed(Some("m-local")))), |_| {});
    store.send(A::Migrations(vec![waiting("m-local", MigrateThing::Daemon, "this-mac")]), |_| {});
    assert_eq!(store.state().refused, Some(refusal(Some("m-local"))));
    store.send(A::Migrations(vec![migrated("m-local", MigrateThing::Daemon, "this-mac")]), |_| {});
    assert_eq!(store.state().refused, None);
}
