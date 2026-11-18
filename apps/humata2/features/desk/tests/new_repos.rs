//! DK-14, RL-5: the repos of a New desk session are ticked from the one repo
//! search: ticking again unticks, a blank is ignored, and the search stays
//! open until the person is done.
use std::sync::Arc;

use humata2_clients::model::Agent;
use humata2_clients::{FakeDaemon, Workspace, fixture};
use humata2_desk::{Desk, DeskAction as A, DeskState, NewAction as N};
use tca::{TestClock, TestStore};

fn store(workspace: Workspace) -> TestStore<Desk> {
    let fake = FakeDaemon::new(workspace.clone(), Arc::new(TestClock::new()));
    let state = DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    };
    TestStore::new(state, Desk::new(Arc::new(fake))).non_exhaustive()
}

fn repos(store: &TestStore<Desk>) -> Vec<String> {
    match &store.state().new {
        Some(form) => form.repos.clone(),
        None => panic!("the New desk session form is open"),
    }
}

fn open_form() -> TestStore<Desk> {
    let mut store = store(fixture::design());
    store.send(A::StartNew { agent: Agent::Shell }, |_| {});
    store.send(A::New(N::AddingRepo(true)), |_| {});
    store
}

#[test]
fn ticking_a_repo_adds_it_and_ticking_it_again_removes_it() {
    let mut store = open_form();
    store.send(A::New(N::ToggleRepo("acme/api".into())), |_| {});
    store.send(A::New(N::ToggleRepo("acme/web".into())), |_| {});
    assert_eq!(repos(&store), vec!["acme/api", "acme/web"]);
    store.send(A::New(N::ToggleRepo("acme/api".into())), |_| {});
    assert_eq!(repos(&store), vec!["acme/web"]);
}

#[test]
fn ticking_keeps_the_search_open() {
    let mut store = open_form();
    store.send(A::New(N::ToggleRepo("acme/api".into())), |_| {});
    assert!(store.state().new.as_ref().is_some_and(|form| form.adding_repo));
}

#[test]
fn a_blank_is_not_a_repo() {
    let mut store = open_form();
    store.send(A::New(N::ToggleRepo("   ".into())), |_| {});
    assert!(repos(&store).is_empty());
}
