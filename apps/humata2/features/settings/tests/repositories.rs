//! ST-21, RL-4, RL-7: Settings › General › Repositories lists the repo
//! library, removes one repo, forgets all only after a confirm, and edits
//! the default scope. Flows on the fake daemon.
use humata2_clients::repos::{LibraryRepo, RepoPlace, RepoScope};
use humata2_clients::{Clients, FakeDaemon, fixture};
use humata2_settings::{
    RepositoriesAction as R, RepositoriesState, Settings, SettingsAction as A, SettingsState,
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn held(repo: &str, last_used_at_ms: i64) -> LibraryRepo {
    LibraryRepo {
        repo: repo.to_owned(),
        remote: format!("git@github.com:{repo}.git"),
        default_branch: "main".to_owned(),
        added_at_ms: 1,
        last_used_at_ms,
        last_used_in: RepoPlace::Desk,
    }
}

fn setup() -> (TestStore<Settings>, FakeDaemon) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    fakes.daemon.set_repo_library(vec![
        held("acme/ledger", 10),
        held("acme/api", 30),
        held("octo/notes", 20),
    ]);
    let store = TestStore::new(SettingsState::default(), Settings::new(clients)).non_exhaustive();
    (store, fakes.daemon)
}

fn section(store: &TestStore<Settings>) -> &RepositoriesState {
    &store.state().repos
}

fn names(state: &RepositoriesState) -> Vec<&str> {
    state
        .library
        .iter()
        .flatten()
        .map(|repo| repo.repo.as_str())
        .collect()
}

fn open(store: &mut TestStore<Settings>) {
    store.send(A::Repos(R::Open), |_| {});
    store.skip_received();
}

#[test]
fn opening_lists_the_library_most_recently_used_first_and_the_default_scope() {
    let (mut store, _) = setup();
    open(&mut store);
    assert_eq!(names(section(&store)), vec!["acme/api", "octo/notes", "acme/ledger"]);
    assert_eq!(section(&store).scope, Some(RepoScope::YourRepos));
    assert!(section(&store).include_me());
}

#[test]
fn remove_takes_one_repo_out_of_the_library_and_nothing_else() {
    let (mut store, _) = setup();
    open(&mut store);
    store.send(A::Repos(R::Remove("octo/notes".into())), |_| {});
    store.skip_received();
    assert_eq!(names(section(&store)), vec!["acme/api", "acme/ledger"]);
    // The host holds it that way: reading again gives the same.
    open(&mut store);
    assert_eq!(names(section(&store)), vec!["acme/api", "acme/ledger"]);
}

#[test]
fn forget_all_waits_for_the_confirm_and_a_dismissed_confirm_forgets_nothing() {
    let (mut store, _) = setup();
    open(&mut store);
    store.send(A::Repos(R::ForgetAsked), |_| {});
    assert!(section(&store).confirming_forget);
    assert_eq!(section(&store).library_len(), 3, "asking forgets nothing");
    store.send(A::Repos(R::ForgetCancelled), |_| {});
    assert!(!section(&store).confirming_forget);
    assert_eq!(section(&store).library_len(), 3);
    // Confirming without having asked does nothing either.
    store.send(A::Repos(R::ForgetConfirmed), |_| {});
    store.skip_received();
    assert_eq!(section(&store).library_len(), 3);
}

#[test]
fn confirming_forget_all_empties_the_library() {
    let (mut store, _) = setup();
    open(&mut store);
    store.send(A::Repos(R::ForgetAsked), |_| {});
    store.send(A::Repos(R::ForgetConfirmed), |_| {});
    store.skip_received();
    assert_eq!(section(&store).library_len(), 0);
    assert!(!section(&store).confirming_forget);
    open(&mut store);
    assert_eq!(section(&store).library_len(), 0, "the host forgot them too");
}

#[test]
fn adding_an_owner_saves_the_default_scope_with_it() {
    let (mut store, _) = setup();
    open(&mut store);
    store.send(A::Repos(R::OwnerAdded("bazelbuild".into())), |_| {});
    store.skip_received();
    assert_eq!(
        section(&store).scope,
        Some(RepoScope::Owners {
            owners: vec!["bazelbuild".to_owned()],
            include_me: true
        })
    );
    // The host keeps it: opening again reads the same scope.
    open(&mut store);
    assert_eq!(section(&store).owners(), ["bazelbuild".to_owned()]);
}

#[test]
fn the_switch_and_the_chips_edit_the_saved_scope() {
    let (mut store, _) = setup();
    open(&mut store);
    store.send(A::Repos(R::OwnerAdded("bazelbuild".into())), |_| {});
    store.skip_received();
    store.send(A::Repos(R::ToggleIncludeMe), |_| {});
    store.skip_received();
    assert!(!section(&store).include_me());
    store.send(A::Repos(R::OwnerRemoved("bazelbuild".into())), |_| {});
    store.skip_received();
    // Nothing listed and the switch off is searching nothing, not Your repos.
    assert!(section(&store).owners().is_empty());
    assert!(!section(&store).include_me());
    store.send(A::Repos(R::ToggleIncludeMe), |_| {});
    store.skip_received();
    assert_eq!(section(&store).scope, Some(RepoScope::YourRepos));
}

#[test]
fn owners_to_suggest_are_the_librarys_owners_the_scope_lacks() {
    let (mut store, _) = setup();
    open(&mut store);
    assert_eq!(section(&store).owner_suggestions(), vec!["acme".to_owned(), "octo".to_owned()]);
    store.send(A::Repos(R::OwnerAdded("acme".into())), |_| {});
    store.skip_received();
    assert_eq!(section(&store).owner_suggestions(), vec!["octo".to_owned()]);
}
