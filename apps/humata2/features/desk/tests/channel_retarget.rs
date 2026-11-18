//! A channel Desk session whose target is gone (CD-5): the Desk knows it from
//! the sessions it lists, and the person picks a new target or has the
//! session look after the whole channel, on the fake daemon.
use std::sync::Arc;

use humata2_clients::desk::Closed;
use humata2_clients::model::{ChannelId, SessionId, Status};
use humata2_clients::{FakeDaemon, Operation, Workspace, fixture};
use humata2_desk::{Change, Desk, DeskAction as A, DeskState, Menu};
use tca::{TestClock, TestStore};

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

fn backfill() -> SessionId {
    SessionId::new("cdesk-backfill")
}

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

fn workspace_with(change: impl FnOnce(&mut Workspace)) -> Workspace {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    change(&mut workspace);
    workspace
}

fn store(workspace: Workspace) -> (TestStore<Desk>, FakeDaemon) {
    let fake = FakeDaemon::new(workspace.clone(), Arc::new(TestClock::new()));
    let state = DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    };
    let store = TestStore::new(state, Desk::new(Arc::new(fake.clone()))).non_exhaustive();
    (store, fake)
}

fn show(store: &mut TestStore<Desk>, fake: &FakeDaemon) {
    store.send(A::Show(Arc::new(fake.workspace())), |_| {});
}

fn with_notes(workspace: &mut Workspace) {
    if let Some(record) = workspace.desk.channel_sessions.get_mut(&backfill()) {
        record.documents = vec!["notes.md".to_owned()];
    }
}

fn gone(state: &DeskState) -> Option<bool> {
    state.channel_record(&backfill()).map(|record| record.target_gone)
}

fn changed(store: &mut TestStore<Desk>, session: &SessionId) {
    store.receive(
        A::Changed {
            session: session.clone(),
            change: Change::Target,
            result: Ok(()),
        },
        |_| {},
    );
}

#[test]
fn a_target_that_is_there_is_not_gone() {
    let (store, _fake) = store(workspace_with(|_| {}));
    assert_eq!(gone(store.state()), Some(false));
}

#[test]
fn a_deleted_target_stops_a_session_holding_markdown_and_marks_it_gone() {
    let (mut store, fake) = store(workspace_with(with_notes));
    fake.remove_session(&ledger());
    show(&mut store, &fake);
    assert_eq!(gone(store.state()), Some(true));
    assert_eq!(
        store.state().workspace.session(&backfill()).map(|session| session.status),
        Some(Status::Stopped)
    );
}

#[test]
fn a_deleted_target_closes_a_session_with_no_markdown() {
    let (mut store, fake) = store(workspace_with(|_| {}));
    fake.remove_session(&ledger());
    show(&mut store, &fake);
    assert_eq!(gone(store.state()), None);
    assert!(store
        .state()
        .channel_rows(&payments())
        .iter()
        .all(|row| row.session.id != backfill()));
}

#[test]
fn a_target_moved_out_of_the_channel_is_gone_as_a_deleted_one_is() {
    let (mut store, fake) = store(workspace_with(with_notes));
    fake.relocate_session(&ledger(), &ChannelId::new("web"));
    show(&mut store, &fake);
    assert_eq!(gone(store.state()), Some(true));
}

#[test]
fn another_session_going_leaves_the_target_alone() {
    let (mut store, fake) = store(workspace_with(with_notes));
    fake.remove_session(&SessionId::new("brin"));
    show(&mut store, &fake);
    assert_eq!(gone(store.state()), Some(false));
}

#[test]
fn picking_a_new_target_looks_after_it_ends_the_gone_state_and_runs_the_session_again() {
    let (mut store, fake) = store(workspace_with(with_notes));
    fake.remove_session(&ledger());
    show(&mut store, &fake);
    let brin = SessionId::new("brin");
    store.send(A::SetTarget(backfill(), Some(brin.clone())), |_| {});
    changed(&mut store, &backfill());
    show(&mut store, &fake);
    let state = store.state();
    let Some(record) = state.channel_record(&backfill()) else {
        panic!("the session is still on the Desk")
    };
    assert_eq!(record.target, Some(brin));
    assert!(!record.target_gone);
    assert_ne!(
        state.workspace.session(&backfill()).map(|session| session.status),
        Some(Status::Stopped)
    );
}

#[test]
fn clearing_the_target_makes_the_session_look_after_the_whole_channel() {
    let (mut store, fake) = store(workspace_with(|_| {}));
    store.send(A::Menu(Some(Menu::Target(backfill()))), |_| {});
    assert_eq!(store.state().menu, Some(Menu::Target(backfill())));
    store.send(A::SetTarget(backfill(), None), |_| {});
    assert_eq!(store.state().menu, None, "the pop-up closes with the pick");
    changed(&mut store, &backfill());
    show(&mut store, &fake);
    assert!(store
        .state()
        .channel_record(&backfill())
        .is_some_and(|record| record.target.is_none() && !record.target_gone));
}

#[test]
fn only_other_sessions_of_the_channel_are_offered_as_targets() {
    let (store, _fake) = store(workspace_with(|_| {}));
    let ids: Vec<SessionId> = store
        .state()
        .target_choices(&backfill())
        .into_iter()
        .map(|session| session.id.clone())
        .collect();
    assert!(ids.contains(&SessionId::new("brin")));
    assert!(!ids.contains(&ledger()), "the current target is not offered again");
    assert!(
        ids.iter().all(|id| !id.as_str().starts_with("cdesk-")),
        "a Desk session can't look after another"
    );
    assert!(!ids.contains(&SessionId::new("s-login")), "that one is in another channel");
}

#[test]
fn a_session_of_another_channel_is_not_picked() {
    let (mut store, fake) = store(workspace_with(|_| {}));
    store.send(A::SetTarget(backfill(), Some(SessionId::new("s-login"))), |_| {});
    assert!(store.state().pending.is_empty(), "nothing was asked of the daemon");
    assert_eq!(
        fake.workspace()
            .desk
            .channel_sessions
            .get(&backfill())
            .and_then(|record| record.target.clone()),
        Some(ledger())
    );
}

#[test]
fn a_closed_session_keeps_its_target() {
    let (mut store, _fake) = store(workspace_with(|workspace| {
        if let Some(record) = workspace.desk.channel_sessions.get_mut(&backfill()) {
            record.desk.closed = Some(Closed {
                when: "today".to_owned(),
                days_left: None,
            });
        }
    }));
    store.send(A::SetTarget(backfill(), Some(SessionId::new("brin"))), |_| {});
    assert!(store.state().pending.is_empty());
}

#[test]
fn a_refused_change_says_so_and_leaves_the_target() {
    let (mut store, fake) = store(workspace_with(|_| {}));
    fake.fail_next(
        Operation::Desk(humata2_clients::desk::Operation::SetTarget),
        "no luck",
    );
    store.send(A::SetTarget(backfill(), Some(SessionId::new("brin"))), |_| {});
    store.receive_matching(
        "Changed(Err(_))",
        |a| matches!(a, A::Changed { result: Err(_), .. }),
        |_| {},
    );
    assert!(store.state().error.is_some());
    assert!(store.state().pending.is_empty());
    assert_eq!(
        fake.workspace()
            .desk
            .channel_sessions
            .get(&backfill())
            .and_then(|record| record.target.clone()),
        Some(ledger())
    );
}
