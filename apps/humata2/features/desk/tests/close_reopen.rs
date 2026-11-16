//! DK-5, DK-11: Close, Reopen and Keep on Desk sessions.
use std::sync::Arc;

use humata2_clients::desk;
use humata2_clients::model::SessionId;
use humata2_clients::{FakeDaemon, Operation, fixture};
use humata2_desk::{Change, Delegate, Desk, DeskAction as A, DeskState};
use tca::{TestClock, TestStore};

fn store() -> (TestStore<Desk>, FakeDaemon) {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
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

fn changed(store: &mut TestStore<Desk>, id: &SessionId, change: Change) {
    store.receive(
        A::Changed {
            session: id.clone(),
            change,
            result: Ok(()),
        },
        |_| {},
    );
}

fn open_ids(store: &TestStore<Desk>) -> Vec<String> {
    store
        .state()
        .open()
        .iter()
        .map(|row| row.session.id.to_string())
        .collect()
}

fn closed_ids(store: &TestStore<Desk>) -> Vec<String> {
    store
        .state()
        .closed()
        .iter()
        .map(|row| row.session.id.to_string())
        .collect()
}

#[test]
fn closing_moves_a_session_from_open_to_closed() {
    let (mut store, fake) = store();
    let id = SessionId::new("desk-plan-q4");
    store.send(A::Close(id.clone()), |_| {});
    assert!(store.state().pending.contains(&id));
    changed(&mut store, &id, Change::Close);
    store.receive_matching(
        "Delegate(Toast(_))",
        |a| matches!(a, A::Delegate(Delegate::Toast(_))),
        |_| {},
    );
    assert!(store.state().pending.is_empty());

    show(&mut store, &fake);
    assert!(!open_ids(&store).contains(&id.to_string()));
    assert!(closed_ids(&store).contains(&id.to_string()));
}

#[test]
fn reopening_makes_the_session_open_and_first_and_opens_it() {
    let (mut store, fake) = store();
    let id = SessionId::new("desk-release-checklist");
    assert!(closed_ids(&store).contains(&id.to_string()));
    store.send(A::Reopen(id.clone()), |_| {});
    changed(&mut store, &id, Change::Reopen);
    store.receive(A::Delegate(Delegate::OpenSession(id.clone())), |_| {});

    show(&mut store, &fake);
    assert_eq!(open_ids(&store).first(), Some(&id.to_string()));
    assert!(!closed_ids(&store).contains(&id.to_string()));
}

#[test]
fn keeping_a_closed_scratch_folder_clears_its_days() {
    let (mut store, fake) = store();
    let id = SessionId::new("desk-release-checklist");
    store.send(A::Keep(id.clone()), |_| {});
    changed(&mut store, &id, Change::Keep);
    store.receive_matching(
        "Delegate(Toast(_))",
        |a| matches!(a, A::Delegate(Delegate::Toast(_))),
        |_| {},
    );

    show(&mut store, &fake);
    let Some(record) = store.state().record(&id) else {
        panic!("the session still has a Desk record");
    };
    assert!(!record.folder.expires());
    let Some(closed) = record.closed.as_ref() else {
        panic!("the session is still closed");
    };
    assert_eq!(closed.days_left, None);
}

#[test]
fn a_second_close_while_the_first_is_pending_is_ignored() {
    let (mut store, fake) = store();
    let id = SessionId::new("desk-plan-q4");
    store.send(A::Close(id.clone()), |_| {});
    store.send(A::Close(id.clone()), |_| {});
    assert_eq!(store.state().pending.len(), 1);
    changed(&mut store, &id, Change::Close);
    store.skip_received();
    assert!(store.state().pending.is_empty());
    assert!(store.state().error.is_none());

    show(&mut store, &fake);
    assert!(closed_ids(&store).contains(&id.to_string()));
}

#[test]
fn a_failed_close_sets_the_error_and_clears_pending() {
    let (mut store, fake) = store();
    let id = SessionId::new("desk-plan-q4");
    fake.fail_next(Operation::Desk(desk::Operation::Close), "no luck");
    store.send(A::Close(id.clone()), |_| {});
    store.receive_matching(
        "Changed(Err(_))",
        |a| matches!(a, A::Changed { result: Err(_), .. }),
        |_| {},
    );
    assert!(store.state().error.is_some());
    assert!(store.state().pending.is_empty());

    show(&mut store, &fake);
    assert!(open_ids(&store).contains(&id.to_string()));

    store.send(A::DismissError, |_| {});
    assert!(store.state().error.is_none());
}
