//! DK-11, NV-27: Clean up… deletes only the ticked sessions, and only from
//! the list it showed.
use std::sync::Arc;

use humata2_clients::desk::CleanUp;
use humata2_clients::model::SessionId;
use humata2_clients::{FakeDaemon, fixture};
use humata2_desk::{Change, Delegate, Desk, DeskAction as A, DeskState};
use tca::{TestClock, TestStore};

fn id(text: &str) -> SessionId {
    SessionId::new(text)
}

fn store() -> (TestStore<Desk>, FakeDaemon) {
    let mut workspace = fixture::design();
    fixture::clean_up(&mut workspace);
    let fake = FakeDaemon::new(workspace.clone(), Arc::new(TestClock::new()));
    let state = DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    };
    let store = TestStore::new(state, Desk::new(Arc::new(fake.clone()))).non_exhaustive();
    (store, fake)
}

fn open_listed(store: &mut TestStore<Desk>) {
    store.send(A::OpenCleanUp, |_| {});
    assert!(store.state().clean_up.as_ref().is_some_and(|d| d.loading));
    store.receive_matching(
        "CleanUpListed(Ok(_))",
        |a| matches!(a, A::CleanUpListed(Ok(_))),
        |_| {},
    );
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
fn the_list_is_least_recently_used_first_with_the_safe_folders_ticked() {
    let (mut store, _) = store();
    open_listed(&mut store);
    let Some(dialog) = store.state().clean_up.as_ref() else {
        panic!("the dialog is open");
    };
    assert!(!dialog.loading);
    let rows: Vec<&str> = dialog.rows.iter().map(|r| r.session.as_str()).collect();
    assert_eq!(
        rows,
        [
            "desk-onboarding-doc",
            "desk-port-cron",
            "desk-notes-cleanup",
            "desk-release-checklist"
        ]
    );
    // Unpushed work and a picked folder are not ticked.
    assert!(dialog.ticked.contains(&id("desk-onboarding-doc")));
    assert!(dialog.ticked.contains(&id("desk-release-checklist")));
    assert!(!dialog.ticked.contains(&id("desk-port-cron")));
    assert!(!dialog.ticked.contains(&id("desk-notes-cleanup")));
}

#[test]
fn freed_bytes_follow_the_ticks() {
    let (mut store, _) = store();
    open_listed(&mut store);
    let freed = |store: &TestStore<Desk>| {
        store
            .state()
            .clean_up
            .as_ref()
            .map_or(0, |dialog| dialog.freed_bytes())
    };
    let before = freed(&store);
    assert!(before > 0);

    store.send(A::ToggleCleanUp(id("desk-port-cron")), |_| {});
    let with_port_cron = freed(&store);
    assert!(with_port_cron > before);

    store.send(A::ToggleCleanUp(id("desk-port-cron")), |_| {});
    assert_eq!(freed(&store), before);

    // A picked folder frees nothing.
    store.send(A::ToggleCleanUp(id("desk-notes-cleanup")), |_| {});
    assert_eq!(freed(&store), before);

    // Only a listed row can be ticked.
    store.send(A::ToggleCleanUp(id("desk-plan-q4")), |_| {});
    assert!(
        store
            .state()
            .clean_up
            .as_ref()
            .is_some_and(|d| !d.ticked.contains(&id("desk-plan-q4")))
    );
}

#[test]
fn confirming_deletes_exactly_the_ticked_sessions_and_closes_the_dialog() {
    let (mut store, fake) = store();
    open_listed(&mut store);
    store.send(A::ConfirmCleanUp, |_| {});
    store.receive_matching(
        "CleanedUp(Ok(Deleted))",
        |a| {
            matches!(
                a,
                A::CleanedUp(Ok(CleanUp::Deleted { sessions: 2, .. }))
            )
        },
        |_| {},
    );
    store.receive_matching(
        "Delegate(Toast(_))",
        |a| matches!(a, A::Delegate(Delegate::Toast(_))),
        |_| {},
    );
    assert!(store.state().clean_up.is_none());

    store.send(A::Show(Arc::new(fake.workspace())), |_| {});
    let mut closed = closed_ids(&store);
    closed.sort();
    assert_eq!(closed, ["desk-notes-cleanup", "desk-port-cron"]);
}

#[test]
fn a_desk_that_changed_under_the_dialog_deletes_nothing_and_asks_again() {
    let (mut store, fake) = store();
    open_listed(&mut store);
    // Another window closes a session while the dialog is open.
    let other = id("desk-plan-q4");
    store.send(A::Close(other.clone()), |_| {});
    store.receive(
        A::Changed {
            session: other.clone(),
            change: Change::Close,
            result: Ok(()),
        },
        |_| {},
    );
    store.receive_matching(
        "Delegate(Toast(_))",
        |a| matches!(a, A::Delegate(Delegate::Toast(_))),
        |_| {},
    );

    store.send(A::ConfirmCleanUp, |_| {});
    store.receive_matching(
        "CleanedUp(Ok(Changed))",
        |a| matches!(a, A::CleanedUp(Ok(CleanUp::Changed(_)))),
        |_| {},
    );

    let Some(dialog) = store.state().clean_up.as_ref() else {
        panic!("the dialog stays open");
    };
    assert!(dialog.changed);
    assert!(!dialog.deleting);
    assert!(dialog.rows.iter().any(|row| row.session == other));
    assert!(!dialog.ticked.contains(&other));
    // The unchanged rows keep their ticks.
    assert!(dialog.ticked.contains(&id("desk-onboarding-doc")));

    store.send(A::Show(Arc::new(fake.workspace())), |_| {});
    assert_eq!(closed_ids(&store).len(), 5);
}

#[test]
fn confirming_with_nothing_ticked_does_nothing() {
    let (mut store, fake) = store();
    open_listed(&mut store);
    store.send(A::ToggleCleanUp(id("desk-onboarding-doc")), |_| {});
    store.send(A::ToggleCleanUp(id("desk-release-checklist")), |_| {});
    store.send(A::ConfirmCleanUp, |_| {});
    store.skip_received();

    let Some(dialog) = store.state().clean_up.as_ref() else {
        panic!("the dialog stays open");
    };
    assert!(!dialog.deleting);
    assert!(dialog.ticked.is_empty());
    store.send(A::Show(Arc::new(fake.workspace())), |_| {});
    assert_eq!(closed_ids(&store).len(), 4);
}

#[test]
fn cancel_closes_the_dialog() {
    let (mut store, _) = store();
    open_listed(&mut store);
    store.send(A::CancelDialog, |_| {});
    assert!(store.state().clean_up.is_none());
    assert!(!store.state().dialog_open());
}
