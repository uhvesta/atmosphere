//! The Review a pull request dialog and Submit review hold the keys while
//! they show, and Escape closes them without starting or posting anything.
use humata2_app::{App, AppAction as A, AppState};
use humata2_clients::fixture;
use humata2_clients::model::{Agent, SessionId};
use humata2_clients::Clients;
use humata2_pr_review::dialog::DialogAction;
use humata2_pr_review::submit::{Step, SubmitAction};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn rig() -> TestStore<App> {
    let workspace = fixture::design();
    let (clients, _fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(workspace)), |_| {});
    store
}

#[test]
fn the_review_dialog_holds_the_keys_and_escape_closes_it() {
    let mut store = rig();
    assert!(!store.state().dialog_holds_keys());
    store.send(
        A::PrDialog(DialogAction::Open {
            machine: None,
            agent: Agent::Claude,
        }),
        |_| {},
    );
    assert!(store.state().dialog_holds_keys());
    store.send(A::Escape, |_| {});
    assert!(!store.state().pr_dialog.open);
    assert!(!store.state().dialog_holds_keys());
}

#[test]
fn submit_review_holds_the_keys_and_escape_closes_it_without_posting() {
    let mut store = rig();
    store.send(A::PrSubmit(SubmitAction::Open(SessionId::new("ledger"))), |_| {});
    assert!(store.state().dialog_holds_keys());
    store.send(A::Escape, |_| {});
    assert_eq!(store.state().pr_submit.step, Step::Closed);
    assert!(!store.state().dialog_holds_keys());
}
