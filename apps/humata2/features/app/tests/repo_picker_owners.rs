//! RL-5, RL-8, DK-14: New desk session picks repos with the one repo search;
//! **+ Org** suggests the user's GitHub orgs after the library's owners; and
//! "used … ago" is worked out from the time the daemon gives, not read from
//! the screen.
use std::sync::Arc;

use humata2_app::{
    App, AppAction as A, AppState, Overlay, NewChannel, RepoPicker, RepoSearchAction as S,
    RepoSearchState, Route, used_line,
};
use humata2_clients::repos::{LibraryRepo, RepoPlace};
use humata2_clients::{Clients, Fakes, Workspace, fixture};
use humata2_desk::{DeskAction as D, NewAction as N};
use tca::{TestClock, TestStore};

fn held(repo: &str, last_used_at_ms: i64, place: RepoPlace) -> LibraryRepo {
    LibraryRepo {
        repo: repo.to_owned(),
        remote: format!("git@github.com:{repo}.git"),
        default_branch: "main".to_owned(),
        added_at_ms: 1,
        last_used_at_ms,
        last_used_in: place,
    }
}

fn setup() -> (TestStore<App>, Fakes) {
    let workspace = fixture::design();
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    fakes.daemon.set_repo_library(vec![
        held("bazelbuild/rules_jvm_external", 20, RepoPlace::Desk),
        held("acme/api", 10, RepoPlace::Desk),
    ]);
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace.clone());
    state.desk.workspace = Arc::new(workspace);
    state.route = Route::Home;
    let store = TestStore::new(state, App::new(clients)).non_exhaustive();
    (store, fakes)
}

fn open_new_desk_search(store: &mut TestStore<App>) {
    store.send(A::StartNewDeskSession, |_| {});
    store.send(A::Desk(D::New(N::AddingRepo(true))), |_| {});
    store.skip_received();
}

fn new_desk(store: &TestStore<App>) -> &RepoSearchState {
    &store.state().new_desk_repos
}

fn ticked(store: &TestStore<App>) -> Vec<String> {
    store
        .state()
        .desk
        .new
        .as_ref()
        .map(|form| form.repos.clone())
        .unwrap_or_default()
}

fn names(search: &RepoSearchState) -> Vec<String> {
    search.rows(&[], 50).0.into_iter().map(|row| row.repo).collect()
}

#[test]
fn the_new_desk_session_search_opens_on_the_library_and_ticks_repos() {
    let (mut store, _) = setup();
    open_new_desk_search(&mut store);
    assert_eq!(
        RepoPicker::on_screen(store.state()),
        Some(RepoPicker::NewDeskSession)
    );
    // The Library group comes first; GitHub's group (Your repos, RL-7)
    // follows it.
    let (rows, _) = new_desk(&store).rows(&[], 50);
    let library: Vec<&str> = rows
        .iter()
        .take_while(|row| row.group == Some(humata2_app::RepoGroup::Library))
        .map(|row| row.repo.as_str())
        .collect();
    assert_eq!(library, vec!["bazelbuild/rules_jvm_external", "acme/api"]);

    store.send(A::Desk(D::New(N::ToggleRepo("acme/api".into()))), |_| {});
    assert_eq!(ticked(&store), vec!["acme/api"]);
    let (rows, _) = new_desk(&store).rows(&ticked(&store), 50);
    assert!(rows.first().is_some_and(|row| row.repo == "acme/api" && row.ticked));
}

#[test]
fn typing_narrows_the_library_at_once() {
    let (mut store, _) = setup();
    open_new_desk_search(&mut store);
    store.send(
        A::RepoSearch(RepoPicker::NewDeskSession, S::QueryChanged("bazel".into())),
        |_| {},
    );
    assert_eq!(names(new_desk(&store)), vec!["bazelbuild/rules_jvm_external"]);
    store.skip_in_flight();
}

#[test]
fn a_search_for_a_form_that_is_not_open_is_dropped() {
    let (mut store, _) = setup();
    store.send(
        A::RepoSearch(RepoPicker::NewDeskSession, S::QueryChanged("acme".into())),
        |_| {},
    );
    store.skip_received();
    assert_eq!(new_desk(&store).query, "");
}

#[test]
fn org_suggestions_list_the_library_owners_then_the_github_orgs() {
    let (mut store, fakes) = setup();
    fakes
        .daemon
        .set_repo_owners(vec!["octo-org".into(), "acme".into(), "bazelbuild".into()]);
    open_new_desk_search(&mut store);
    store.send(A::RepoSearch(RepoPicker::NewDeskSession, S::AddOwnerOpened), |_| {});
    store.skip_received();
    assert_eq!(
        new_desk(&store).owner_suggestions(),
        vec!["bazelbuild", "acme", "octo-org"]
    );

    store.send(A::RepoSearch(RepoPicker::NewDeskSession, S::OwnerAdded("acme".into())), |_| {});
    store.skip_received();
    assert_eq!(new_desk(&store).owner_suggestions(), vec!["bazelbuild", "octo-org"]);
}

#[test]
fn with_no_github_orgs_only_the_library_owners_are_suggested() {
    let (mut store, _) = setup();
    open_new_desk_search(&mut store);
    store.send(A::RepoSearch(RepoPicker::NewDeskSession, S::AddOwnerOpened), |_| {});
    store.skip_received();
    assert_eq!(new_desk(&store).owner_suggestions(), vec!["bazelbuild", "acme"]);
}

#[test]
fn the_search_takes_the_time_from_the_daemon() {
    let (mut store, fakes) = setup();
    fakes.daemon.set_repo_now_ms(777);
    store.send(A::StartNewChannel, |_| {});
    store.skip_received();
    match store.state().overlay() {
        Some(Overlay::NewChannel(NewChannel { search, .. })) => assert_eq!(search.now_ms, 777),
        other => panic!("the New channel sheet is open, not {other:?}"),
    }
}

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

fn used(last_used_at_ms: i64, now_ms: i64, place: RepoPlace) -> String {
    used_line(&held("acme/api", last_used_at_ms, place), now_ms, &Workspace::default())
}

#[test]
fn the_age_is_the_difference_between_the_times_it_is_given() {
    let then = 1_000_000_000;
    let two_days = used(then, then + 2 * DAY, RepoPlace::Desk);
    let three_hours = used(then, then + 3 * HOUR, RepoPlace::Desk);
    let just_now = used(then, then + 10_000, RepoPlace::Desk);
    assert!(two_days.contains("2 days ago"), "{two_days}");
    assert!(three_hours.contains("3 hours ago"), "{three_hours}");
    assert!(just_now.contains("just now"), "{just_now}");
}

#[test]
fn a_clock_behind_the_last_use_does_not_make_a_negative_age() {
    let line = used(5_000, 1_000, RepoPlace::Desk);
    assert!(line.contains("just now"), "{line}");
}

#[test]
fn the_same_times_always_give_the_same_line() {
    assert_eq!(
        used(10, 10 + 5 * MINUTE, RepoPlace::Desk),
        used(10, 10 + 5 * MINUTE, RepoPlace::Desk)
    );
}
