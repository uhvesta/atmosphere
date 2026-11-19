//! ST-21, RL-8: Settings › General › Repositories suggests the orgs the
//! user's GitHub login belongs to as well as the library's owners, and knows
//! what time it is from the daemon, not from the screen.
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
    fakes
        .daemon
        .set_repo_library(vec![held("acme/api", 30), held("octo/notes", 20)]);
    fakes
        .daemon
        .set_repo_owners(vec!["bazelbuild".to_owned(), "acme".to_owned(), "zeta".to_owned()]);
    let store = TestStore::new(SettingsState::default(), Settings::new(clients)).non_exhaustive();
    (store, fakes.daemon)
}

fn section(store: &TestStore<Settings>) -> &RepositoriesState {
    &store.state().repos
}

fn open(store: &mut TestStore<Settings>) {
    store.send(A::Repos(R::Open), |_| {});
    store.skip_received();
}

#[test]
fn library_owners_are_suggested_first_then_github_orgs_without_repeats() {
    let (mut store, _) = setup();
    open(&mut store);
    assert_eq!(
        section(&store).owner_suggestions(),
        vec!["acme", "octo", "bazelbuild", "zeta"]
    );
}

#[test]
fn an_owner_added_to_the_scope_stops_being_suggested() {
    let (mut store, _) = setup();
    open(&mut store);
    store.send(A::Repos(R::OwnerAdded("bazelbuild".into())), |_| {});
    store.skip_received();
    assert_eq!(
        section(&store).scope,
        Some(RepoScope::Owners { owners: vec!["bazelbuild".into()], include_me: true })
    );
    assert_eq!(section(&store).owner_suggestions(), vec!["acme", "octo", "zeta"]);
}

#[test]
fn the_section_takes_the_time_from_the_daemon() {
    let (mut store, daemon) = setup();
    daemon.set_repo_now_ms(123_456);
    open(&mut store);
    assert_eq!(section(&store).now_ms, 123_456);
}
