//! CH-6, CH-18, CH-19 (frame 20): the "Add a GitHub repo…" sheet of a
//! channel. It lists the library first, a click adds the repo to the channel
//! and closes the sheet, a pasted GitHub address is read as owner/name, and
//! a refusal stays in the sheet. Flows on the fake daemon.
use std::sync::Arc;

use humata2_app::{
    App, AppAction as A, AppState, ChannelAddRepo, ChannelAddRepoAction as C, Overlay, RepoGroup,
    RepoPicker, RepoSearchAction as S, Route,
};
use humata2_clients::model::ChannelId;
use humata2_clients::repos::{LibraryRepo, RepoPlace};
use humata2_clients::{Clients, Fakes, Operation, fixture};
use tca::{TestClock, TestStore};

const FROM_LIBRARY: &str = "bazelbuild/rules_jvm_external";

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

fn setup() -> (TestStore<App>, Fakes, ChannelId) {
    let workspace = fixture::design();
    let Some(channel) = workspace.channels.first().map(|channel| channel.id.clone()) else {
        panic!("the design workspace has a channel");
    };
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    fakes
        .daemon
        .set_github_repos(["octo/notes", "octo/infra"].map(str::to_owned).to_vec());
    fakes.daemon.set_repo_library(vec![held(FROM_LIBRARY, 20)]);
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    state.route = Route::Channel(channel.clone());
    let store = TestStore::new(state, App::new(clients)).non_exhaustive();
    (store, fakes, channel)
}

/// Lets each effect and the one it starts run.
fn settle(store: &mut TestStore<App>) {
    for _ in 0..3 {
        store.skip_received();
    }
}

fn open(store: &mut TestStore<App>, channel: &ChannelId) {
    store.send(A::ChannelAddRepo(C::Open(channel.clone())), |_| {});
    settle(store);
}

fn sheet(store: &TestStore<App>) -> Option<&ChannelAddRepo> {
    match store.state().overlay() {
        Some(Overlay::ChannelAddRepo(sheet)) => Some(sheet),
        _ => None,
    }
}

fn repos(store: &TestStore<App>, channel: &ChannelId) -> Vec<String> {
    store
        .state()
        .workspace
        .channel(channel)
        .map(|found| found.repos.clone())
        .unwrap_or_default()
}

fn adds(fakes: &Fakes) -> usize {
    fakes
        .daemon
        .calls()
        .iter()
        .filter(|call| call.starts_with("add repo"))
        .count()
}

#[test]
fn opening_lists_the_library_then_github_and_embeds_the_one_search() {
    let (mut store, _, channel) = setup();
    open(&mut store, &channel);
    let sheet = sheet(&store).expect("the sheet is open");
    assert_eq!(sheet.channel, channel);
    let (rows, _) = sheet.search.rows(&[], 50);
    let library: Vec<&str> = rows
        .iter()
        .filter(|row| row.group == Some(RepoGroup::Library))
        .map(|row| row.repo.as_str())
        .collect();
    assert_eq!(library, vec![FROM_LIBRARY]);
    assert!(rows.iter().any(|row| row.repo == "octo/notes" && row.group == Some(RepoGroup::GitHub)));
}

#[test]
fn the_pickers_search_actions_reach_this_sheet() {
    let (mut store, _, channel) = setup();
    open(&mut store, &channel);
    store.send(
        A::RepoSearch(RepoPicker::ChannelAddRepo, S::AddOwnerOpened),
        |_| {},
    );
    assert!(sheet(&store).is_some_and(|sheet| sheet.search.adding_owner.is_some()));
}

#[test]
fn picking_a_row_adds_the_repo_to_the_channel_and_closes_the_sheet() {
    let (mut store, fakes, channel) = setup();
    open(&mut store, &channel);
    store.send(A::ChannelAddRepo(C::Pick(FROM_LIBRARY.into())), |_| {});
    settle(&mut store);
    assert!(sheet(&store).is_none(), "the sheet closed");
    assert!(repos(&store, &channel).contains(&FROM_LIBRARY.to_owned()));
    assert_eq!(adds(&fakes), 1);
}

#[test]
fn a_github_row_not_yet_in_the_library_is_added_and_joins_it() {
    let (mut store, fakes, channel) = setup();
    open(&mut store, &channel);
    store.send(A::ChannelAddRepo(C::Pick("octo/notes".into())), |_| {});
    settle(&mut store);
    assert!(repos(&store, &channel).contains(&"octo/notes".to_owned()));
    assert!(library_names(fakes.daemon.clone()).contains(&"octo/notes".to_owned()));
}

/// The repos the fake's library holds.
fn library_names(daemon: humata2_clients::FakeDaemon) -> Vec<String> {
    use humata2_clients::Daemon as _;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime starts");
    runtime
        .block_on(daemon.repo_library())
        .expect("the library lists")
        .into_iter()
        .map(|repo| repo.repo)
        .collect()
}

#[test]
fn a_pasted_github_address_adds_the_repo_it_names() {
    let (mut store, _, channel) = setup();
    open(&mut store, &channel);
    store.send(
        A::ChannelAddRepo(C::Query("https://github.com/octo/infra.git".into())),
        |_| {},
    );
    settle(&mut store);
    store.send(A::ChannelAddRepo(C::Submit), |_| {});
    settle(&mut store);
    assert!(sheet(&store).is_none());
    assert!(repos(&store, &channel).contains(&"octo/infra".to_owned()));
}

#[test]
fn what_is_not_a_repo_stays_in_the_sheet_with_a_reason_and_calls_nothing() {
    let (mut store, fakes, channel) = setup();
    open(&mut store, &channel);
    for text in ["", "acme", "~/code/api", "/etc/passwd", "../api"] {
        store.send(A::ChannelAddRepo(C::Query(text.into())), |_| {});
        settle(&mut store);
        store.send(A::ChannelAddRepo(C::Submit), |_| {});
        let sheet = sheet(&store).expect("the sheet stays open");
        assert!(sheet.error.is_some(), "{text:?} says why");
        assert!(sheet.adding.is_none());
    }
    assert_eq!(adds(&fakes), 0);
}

#[test]
fn a_repo_the_channel_has_is_not_added_again() {
    let (mut store, fakes, channel) = setup();
    let Some(have) = repos(&store, &channel).first().cloned() else {
        panic!("the design channel has a repo");
    };
    open(&mut store, &channel);
    store.send(A::ChannelAddRepo(C::Pick(have)), |_| {});
    assert!(sheet(&store).is_some_and(|sheet| sheet.error.is_some()));
    assert_eq!(adds(&fakes), 0);
}

#[test]
fn a_refusal_stays_inline_and_the_next_try_works() {
    let (mut store, fakes, channel) = setup();
    open(&mut store, &channel);
    fakes
        .daemon
        .fail_next(Operation::AddRepoToChannel, "the clone failed");
    let before = repos(&store, &channel);
    store.send(A::ChannelAddRepo(C::Pick(FROM_LIBRARY.into())), |_| {});
    settle(&mut store);
    let open_sheet = sheet(&store).expect("the sheet stays open");
    assert!(open_sheet.error.is_some());
    assert!(open_sheet.adding.is_none(), "it can be tried again");
    assert_eq!(repos(&store, &channel), before);

    store.send(A::ChannelAddRepo(C::DismissError), |_| {});
    assert!(sheet(&store).is_some_and(|sheet| sheet.error.is_none()));
    store.send(A::ChannelAddRepo(C::Pick(FROM_LIBRARY.into())), |_| {});
    settle(&mut store);
    assert!(sheet(&store).is_none());
    assert!(repos(&store, &channel).contains(&FROM_LIBRARY.to_owned()));
}

#[test]
fn a_second_pick_while_one_is_out_is_ignored() {
    let (mut store, fakes, channel) = setup();
    open(&mut store, &channel);
    store.send(A::ChannelAddRepo(C::Pick(FROM_LIBRARY.into())), |_| {});
    store.send(A::ChannelAddRepo(C::Pick("octo/notes".into())), |_| {});
    settle(&mut store);
    assert_eq!(adds(&fakes), 1);
}

#[test]
fn escape_closes_the_sheet_and_a_late_search_answer_is_dropped() {
    let (mut store, _, channel) = setup();
    open(&mut store, &channel);
    store.send(A::Escape, |_| {});
    assert!(sheet(&store).is_none());
    store.send(
        A::RepoSearch(RepoPicker::ChannelAddRepo, S::AddOwnerOpened),
        |_| {},
    );
    assert!(sheet(&store).is_none());
}
