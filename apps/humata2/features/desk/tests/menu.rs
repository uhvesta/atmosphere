//! Pop-ups on the Desk: a pick closes its pop-up, none opens over a dialog,
//! and Escape closes the innermost thing first.
use std::sync::Arc;

use humata2_clients::model::{Agent, MachineId, SessionId};
use humata2_clients::{FakeDaemon, fixture};
use humata2_desk::{Desk, DeskAction as A, DeskState, Menu, NewAction as N, Sort};
use tca::{TestClock, TestStore};

fn store() -> TestStore<Desk> {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let fake = FakeDaemon::new(workspace.clone(), Arc::new(TestClock::new()));
    let state = DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    };
    TestStore::new(state, Desk::new(Arc::new(fake))).non_exhaustive()
}

#[test]
fn picking_a_sort_closes_its_pop_up() {
    let mut store = store();
    store.send(A::Menu(Some(Menu::Sort)), |_| {});
    assert_eq!(store.state().menu, Some(Menu::Sort));
    store.send(A::SetSort(Sort::Title), |_| {});
    assert_eq!(store.state().sort, Sort::Title);
    assert_eq!(store.state().menu, None);
}

#[test]
fn moving_a_session_closes_the_machine_picker() {
    let mut store = store();
    let id = SessionId::new("desk-plan-q4");
    store.send(A::Menu(Some(Menu::Machine(id.clone()))), |_| {});
    store.send(A::Move(id.clone(), MachineId::new("devbox-1")), |_| {});
    assert_eq!(store.state().menu, None);
    assert!(store.state().pending.contains(&id));
    store.skip_received();
}

#[test]
fn escape_closes_a_pop_up_and_leaves_the_page() {
    let mut store = store();
    store.send(A::Menu(Some(Menu::Sort)), |_| {});
    assert!(store.state().dialog_open());
    store.send(A::CancelDialog, |_| {});
    assert!(!store.state().dialog_open());
}

#[test]
fn no_pop_up_opens_over_a_dialog_and_a_dialog_closes_the_pop_up() {
    let mut store = store();
    store.send(A::Menu(Some(Menu::Sort)), |_| {});
    store.send(
        A::StartNew {
            agent: Agent::Claude,
        },
        |_| {},
    );
    assert!(store.state().new.is_some());
    assert_eq!(store.state().menu, None);
    store.send(A::Menu(Some(Menu::Sort)), |_| {});
    assert_eq!(store.state().menu, None);
}

#[test]
fn escape_closes_the_forms_machine_list_before_the_form() {
    let mut store = store();
    store.send(
        A::StartNew {
            agent: Agent::Claude,
        },
        |_| {},
    );
    store.send(A::New(N::Machines(true)), |_| {});
    store.send(A::CancelDialog, |_| {});
    assert!(
        store
            .state()
            .new
            .as_ref()
            .is_some_and(|form| !form.machines_open)
    );
    store.send(A::CancelDialog, |_| {});
    assert!(store.state().new.is_none());
}

#[test]
fn picking_a_machine_or_adding_a_repo_closes_its_field() {
    let mut store = store();
    store.send(
        A::StartNew {
            agent: Agent::Claude,
        },
        |_| {},
    );
    store.send(A::New(N::Machines(true)), |_| {});
    store.send(A::New(N::Machine(MachineId::new("devbox-1"))), |_| {});
    store.send(A::New(N::AddingRepo(true)), |_| {});
    store.send(A::New(N::AddRepo("acme/api".to_owned())), |_| {});
    let Some(form) = store.state().new.as_ref() else {
        panic!("the form is open");
    };
    assert_eq!(form.machine, Some(MachineId::new("devbox-1")));
    assert!(!form.machines_open);
    assert!(!form.adding_repo);
    assert_eq!(form.repos, vec!["acme/api".to_owned()]);
}
