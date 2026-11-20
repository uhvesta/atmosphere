//! RL-3 and RL-5 to RL-10, D475 to D478: the one repo search in the New
//! channel sheet. The library group comes first and is narrowed as you type;
//! GitHub is asked once typing pauses; the scope is shown as chips, widened
//! for one search and saved only on request; a GitHub failure leaves the
//! library answering. Flows on the fake daemon.
use std::sync::Arc;
use std::time::Duration;

use humata2_app::{
    App, AppAction as A, AppState, NewChannel, NewChannelAction as N, Overlay, PickRow,
    RepoChip, RepoGroup, RepoPicker, RepoSearchAction as S, RepoSearchState, Route,
};
use humata2_clients::repos::{
    LibraryRepo, RepoPlace, RepoScope, RepoSearchProblem, RepoSearchProblemKind,
};
use humata2_clients::{Fakes, fixture};
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

fn setup() -> (TestStore<App>, Fakes, Arc<TestClock>) {
    let workspace = fixture::design();
    let clock = Arc::new(TestClock::new());
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), clock.clone());
    fakes.daemon.set_github_repos(
        ["acme/ledger", "acme/api", "acme/web", "octo/notes", "octo/infra"]
            .map(str::to_owned)
            .to_vec(),
    );
    fakes.daemon.set_repo_library(vec![
        held("bazelbuild/rules_jvm_external", 20),
        held("acme/api", 10),
    ]);
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    state.route = Route::Home;
    let store = TestStore::new(state, App::new(clients)).non_exhaustive();
    (store, fakes, clock)
}

fn open(store: &mut TestStore<App>) {
    store.send(A::StartNewChannel, |_| {});
    store.skip_received();
}

fn sheet(store: &TestStore<App>) -> &NewChannel {
    match store.state().overlay() {
        Some(Overlay::NewChannel(sheet)) => sheet,
        other => panic!("the New channel sheet is open, not {other:?}"),
    }
}

fn search(store: &TestStore<App>) -> &RepoSearchState {
    &sheet(store).search
}

fn repo_search(store: &mut TestStore<App>, action: S) {
    store.send(A::RepoSearch(RepoPicker::NewChannel, action), |_| {});
    store.skip_received();
}

fn names(rows: &[PickRow], group: Option<RepoGroup>) -> Vec<&str> {
    rows.iter()
        .filter(|row| row.group == group)
        .map(|row| row.repo.as_str())
        .collect()
}

fn searches(fakes: &Fakes) -> usize {
    fakes
        .daemon
        .calls()
        .iter()
        .filter(|call| call.starts_with("search repos"))
        .count()
}

/// Saves "acme" and "octo" with the user as the default scope, through the
/// field's own actions.
fn save_default_scope(store: &mut TestStore<App>) {
    repo_search(store, S::OwnerAdded("acme".into()));
    repo_search(store, S::OwnerAdded("octo".into()));
    repo_search(store, S::SaveScopeAsDefault);
}

fn the_pause(action: &A) -> bool {
    matches!(action, A::RepoSearch(_, S::Paused(_)))
}

#[test]
fn the_library_group_comes_first_without_typing_and_is_not_repeated_under_github() {
    let (mut store, _, _) = setup();
    open(&mut store);
    let (rows, _) = search(&store).rows(&[], 50);
    assert_eq!(
        names(&rows, Some(RepoGroup::Library)),
        vec!["bazelbuild/rules_jvm_external", "acme/api"],
        "most recently used first"
    );
    assert_eq!(
        names(&rows, Some(RepoGroup::GitHub)),
        vec!["acme/ledger", "acme/web", "octo/notes", "octo/infra"]
    );
    let first_github = rows.iter().position(|row| row.group == Some(RepoGroup::GitHub));
    let last_library = rows.iter().rposition(|row| row.group == Some(RepoGroup::Library));
    assert!(last_library < first_github, "Library is listed before GitHub");
}

#[test]
fn typing_narrows_the_library_group_at_once_before_github_is_asked() {
    let (mut store, fakes, _) = setup();
    open(&mut store);
    let asked = searches(&fakes);
    store.send(A::NewChannel(N::Query("rules".into())), |_| {});
    let (rows, _) = search(&store).rows(&[], 50);
    assert_eq!(
        names(&rows, Some(RepoGroup::Library)),
        vec!["bazelbuild/rules_jvm_external"]
    );
    assert_eq!(searches(&fakes), asked, "GitHub waits for the pause");
}

#[test]
fn typing_waits_300_ms_after_the_last_key_then_searches_once() {
    let (mut store, fakes, clock) = setup();
    open(&mut store);
    let asked = searches(&fakes);
    for typed in ["a", "ac", "acm", "acme"] {
        store.send(A::NewChannel(N::Query(typed.into())), |_| {});
        clock.advance(Duration::from_millis(100));
    }
    store.settle();
    assert_eq!(searches(&fakes), asked, "keys 100 ms apart never pause");

    // 100 ms have passed since the last key.
    clock.advance(Duration::from_millis(199));
    store.settle();
    assert_eq!(searches(&fakes), asked, "299 ms after the last key is not yet");

    clock.advance(Duration::from_millis(1));
    store.receive_matching("the pause ending", the_pause, |_| {});
    store.skip_received();
    assert_eq!(searches(&fakes), asked + 1, "one search for the whole word");
    assert!(
        fakes.daemon.calls().last().is_some_and(|call| call.contains("acme")),
        "it searches what was typed last"
    );
}

#[test]
fn removing_a_chip_narrows_this_search_and_offers_save_as_default() {
    let (mut store, _, _) = setup();
    open(&mut store);
    save_default_scope(&mut store);
    assert!(!search(&store).can_save_default(), "saved, so nothing to save");

    repo_search(&mut store, S::ScopeChipRemoved(RepoChip::Owner("octo".into())));
    assert_eq!(
        search(&store).scope,
        Some(RepoScope::Owners { owners: vec!["acme".into()], include_me: true })
    );
    assert!(search(&store).can_save_default());
    let (rows, _) = search(&store).rows(&[], 50);
    assert!(
        names(&rows, Some(RepoGroup::GitHub)).iter().all(|repo| repo.starts_with("acme/")),
        "GitHub is searched inside the narrowed scope: {rows:?}"
    );
}

#[test]
fn save_as_default_writes_the_scope_and_is_no_longer_offered() {
    let (mut store, _, _) = setup();
    open(&mut store);
    save_default_scope(&mut store);
    repo_search(&mut store, S::ScopeChipRemoved(RepoChip::Owner("octo".into())));
    assert!(search(&store).can_save_default());

    repo_search(&mut store, S::SaveScopeAsDefault);
    assert!(!search(&store).can_save_default());
    assert_eq!(search(&store).scope, None, "now it is the default");

    // The host kept it: a new search starts on the narrowed scope.
    repo_search(&mut store, S::Open);
    let labels: Vec<String> = search(&store).chips().into_iter().map(|chip| chip.label).collect();
    assert_eq!(labels, vec!["acme".to_owned(), "you".to_owned()]);
}

#[test]
fn all_of_github_is_for_this_search_only() {
    let (mut store, fakes, _) = setup();
    open(&mut store);
    repo_search(&mut store, S::AllOfGitHubPicked);
    assert_eq!(search(&store).scope, Some(RepoScope::AllOfGitHub));
    assert!(!search(&store).can_save_default(), "All of GitHub cannot be saved");
    repo_search(&mut store, S::SaveScopeAsDefault);
    assert!(
        !fakes.daemon.calls().iter().any(|call| call.contains("save repo scope")),
        "nothing was sent to save"
    );

    repo_search(&mut store, S::Open);
    assert_eq!(search(&store).scope, None);
    assert_eq!(search(&store).scope(), &RepoScope::YourRepos);
}

#[test]
fn widening_with_an_owner_keeps_the_scope_and_adds_the_owner_to_it() {
    let (mut store, _, _) = setup();
    open(&mut store);
    repo_search(&mut store, S::OwnerAdded("buildfarm".into()));
    assert_eq!(
        search(&store).scope,
        Some(RepoScope::Owners { owners: vec!["buildfarm".into()], include_me: true })
    );
    assert!(search(&store).can_save_default());
    repo_search(&mut store, S::ScopeReset);
    assert_eq!(search(&store).scope, None);
    assert!(!search(&store).can_save_default());
}

#[test]
fn a_failing_github_search_keeps_the_library_group_and_says_so() {
    let (mut store, fakes, _) = setup();
    fakes.daemon.set_repo_search_problem(Some(RepoSearchProblem {
        kind: RepoSearchProblemKind::GitHubUnreachable,
        message: "The host cannot reach GitHub.".to_owned(),
    }));
    open(&mut store);
    assert!(search(&store).problem().is_some());
    let (rows, _) = search(&store).rows(&[], 50);
    assert_eq!(
        names(&rows, Some(RepoGroup::Library)),
        vec!["bazelbuild/rules_jvm_external", "acme/api"]
    );
    assert!(names(&rows, Some(RepoGroup::GitHub)).is_empty());
}

#[test]
fn a_query_too_short_for_the_scope_does_not_search_github_and_is_not_a_failure() {
    let (mut store, _, clock) = setup();
    open(&mut store);
    repo_search(&mut store, S::AllOfGitHubPicked);
    store.send(A::NewChannel(N::Query("ac".into())), |_| {});
    clock.advance(Duration::from_millis(300));
    store.receive_matching("the pause ending", the_pause, |_| {});
    store.skip_received();
    let state = search(&store);
    assert!(state.too_short().is_some());
    assert!(state.problem().is_none(), "too short is a hint, not a failure");
    assert!(state.result.as_ref().is_some_and(|result| result.github.is_empty()));
}

#[test]
fn ticking_a_library_repo_makes_the_channel_with_it() {
    let (mut store, fakes, _) = setup();
    open(&mut store);
    store.send(A::NewChannel(N::ToggleRepo("bazelbuild/rules_jvm_external".into())), |_| {});
    store.send(A::NewChannel(N::Name("Deps".into())), |_| {});
    store.send(A::NewChannel(N::Create), |_| {});
    store.skip_received();
    let channel = fakes
        .daemon
        .workspace()
        .channels
        .into_iter()
        .find(|channel| channel.name == "deps")
        .map(|channel| channel.repos);
    assert_eq!(channel, Some(vec!["bazelbuild/rules_jvm_external".to_owned()]));
}

#[test]
fn a_ticked_repo_stays_first_in_front_of_both_groups() {
    let (mut store, _, _) = setup();
    open(&mut store);
    store.send(A::NewChannel(N::ToggleRepo("octo/notes".into())), |_| {});
    let ticked = sheet(&store).repos.clone();
    let (rows, _) = search(&store).rows(&ticked, 50);
    assert_eq!(rows.first().map(|row| row.repo.as_str()), Some("octo/notes"));
    assert_eq!(
        rows.iter().filter(|row| row.repo == "octo/notes").count(),
        1,
        "shown once"
    );
}

#[test]
fn a_late_answer_for_a_closed_new_channel_sheet_does_not_touch_the_desk_search() {
    let (mut store, _, _) = setup();
    open(&mut store);
    store.send(A::Escape, |_| {});
    assert!(store.state().overlay().is_none(), "the sheet closed");
    let before = store.state().desk_repos.clone();
    // Seq 0 is the number the Desk's untouched search would accept.
    let late = humata2_clients::repos::RepoSearch {
        github: vec![humata2_clients::repos::FoundRepo {
            repo: "late/arrival".to_owned(),
            description: String::new(),
            is_private: false,
        }],
        ..Default::default()
    };
    store.send(A::RepoSearch(RepoPicker::NewChannel, S::Results { seq: 0, result: late }), |_| {});
    assert_eq!(store.state().desk_repos, before);
}

#[test]
fn highlight_moves_clamps_and_resets_on_query_owner_and_open() {
    let (mut store, _, _) = setup();
    open(&mut store);
    assert_eq!(search(&store).highlighted, 0);
    let count = search(&store).rows(&[], 50).0.len();
    assert!(count >= 3);

    repo_search(&mut store, S::MoveHighlight { delta: 1, count });
    assert_eq!(search(&store).highlighted, 1);
    assert_eq!(
        search(&store).highlighted_repo(&[], 50).as_deref(),
        Some("acme/api")
    );

    // Clamps at the bottom and top.
    repo_search(&mut store, S::MoveHighlight { delta: 100, count });
    assert_eq!(search(&store).highlighted, count - 1);
    repo_search(&mut store, S::MoveHighlight { delta: -100, count });
    assert_eq!(search(&store).highlighted, 0);

    // Resets on QueryChanged.
    repo_search(&mut store, S::MoveHighlight { delta: 2, count });
    assert_eq!(search(&store).highlighted, 2);
    store.send(A::NewChannel(N::Query("octo".into())), |_| {});
    assert_eq!(search(&store).highlighted, 0);

    // Resets on OwnerAdded.
    let octo_count = search(&store).rows(&[], 50).0.len();
    repo_search(&mut store, S::MoveHighlight { delta: 1, count: octo_count });
    assert_eq!(search(&store).highlighted, 1);
    repo_search(&mut store, S::OwnerAdded("acme".into()));
    assert_eq!(search(&store).highlighted, 0);

    // Resets on Open.
    repo_search(&mut store, S::ScopeReset);
    store.send(A::NewChannel(N::Query(String::new())), |_| {});
    store.skip_received();
    let full_count = search(&store).rows(&[], 50).0.len();
    assert!(full_count >= 2);
    repo_search(&mut store, S::MoveHighlight { delta: 1, count: full_count });
    assert_eq!(search(&store).highlighted, 1);
    repo_search(&mut store, S::Open);
    assert_eq!(search(&store).highlighted, 0);
}

#[test]
fn open_and_owner_added_load_full_scope_catalog_for_instant_local_filtering() {
    let (mut store, fakes, clock) = setup();
    open(&mut store);
    // Open loads the full scope catalog with an empty query.
    assert_eq!(
        fakes.daemon.calls().last().map(String::as_str),
        Some("search repos \"\"")
    );

    // Narrow with a query and let the 300 ms pause complete.
    store.send(A::NewChannel(N::Query("led".into())), |_| {});
    clock.advance(Duration::from_millis(300));
    store.receive_matching("the pause ending", the_pause, |_| {});
    store.skip_received();

    // Changing the query locally still filters the full cached catalog instantaneously
    // without waiting for another daemon round-trip.
    let before = searches(&fakes);
    store.send(A::NewChannel(N::Query("web".into())), |_| {});
    let (rows, _) = search(&store).rows(&[], 50);
    assert_eq!(names(&rows, Some(RepoGroup::GitHub)), vec!["acme/web"]);
    assert_eq!(searches(&fakes), before);

    // OwnerAdded immediately triggers an empty-query catalog load even while a query is typed.
    repo_search(&mut store, S::OwnerAdded("acme".into()));
    assert_eq!(
        fakes.daemon.calls().last().map(String::as_str),
        Some("search repos \"\"")
    );
}
