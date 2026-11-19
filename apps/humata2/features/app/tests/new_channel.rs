//! CH-1 to CH-4 and CH-10: the New channel sheet lists the repos `gh` sees
//! on the host, keeps the ticked repos and machines, and makes the channel
//! with them. Flows on the fake daemon.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, NewChannel, NewChannelAction as N, Overlay, Route};
use humata2_clients::model::{Channel, MachineId};
use humata2_clients::repos::{RepoSearchProblem, RepoSearchProblemKind};
use humata2_clients::{Clients, Fakes, fixture};
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<App>, Fakes) {
    let workspace = fixture::design();
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    fakes.daemon.set_github_repos(
        [
            "acme/ledger",
            "acme/api",
            "acme/web",
            "acme/infra",
            "acme/docs",
            "octo/notes",
        ]
        .map(str::to_owned)
        .to_vec(),
    );
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    state.route = Route::Home;
    let store = TestStore::new(state, App::new(clients)).non_exhaustive();
    (store, fakes)
}

/// What the host says when `gh` there is signed out (RL-10).
fn signed_out() -> RepoSearchProblem {
    RepoSearchProblem {
        kind: RepoSearchProblemKind::GhSignedOut,
        message: "gh is not signed in; run `gh auth login`".to_owned(),
    }
}

/// Opens the sheet and lets the repo list arrive.
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

fn made(fakes: &Fakes, name: &str) -> Option<Channel> {
    fakes
        .daemon
        .workspace()
        .channels
        .into_iter()
        .find(|channel| channel.name == name)
}

fn ids(names: &[&str]) -> Vec<MachineId> {
    names.iter().map(|name| MachineId::new(*name)).collect()
}

#[test]
fn opening_the_sheet_lists_the_repos_gh_sees_and_ticks_this_mac() {
    let (mut store, _) = setup();
    open(&mut store);
    let sheet = sheet(&store);
    assert_eq!(
        sheet.shown_repos(),
        vec!["acme/ledger", "acme/api", "acme/web", "acme/infra", "acme/docs", "octo/notes"]
    );
    assert!(sheet.repos.is_empty());
    assert_eq!(sheet.machines, ids(&["this-mac"]));
}

#[test]
fn typing_narrows_the_list_by_fuzzy_match() {
    let (mut store, _) = setup();
    open(&mut store);
    store.send(A::NewChannel(N::Query("acme".into())), |_| {});
    assert_eq!(
        sheet(&store).shown_repos(),
        vec!["acme/ledger", "acme/api", "acme/web", "acme/infra", "acme/docs"]
    );
    store.send(A::NewChannel(N::Query("ldgr".into())), |_| {});
    assert_eq!(sheet(&store).shown_repos(), vec!["acme/ledger"]);
    store.send(A::NewChannel(N::Query("LEDGER".into())), |_| {});
    assert_eq!(sheet(&store).shown_repos(), vec!["acme/ledger"]);
}

#[test]
fn whole_word_matches_come_before_scattered_ones() {
    let (mut store, _) = setup();
    open(&mut store);
    // "es" is in octo/notes as typed, and only scattered in acme/docs, which
    // gh lists first.
    store.send(A::NewChannel(N::Query("es".into())), |_| {});
    assert_eq!(sheet(&store).shown_repos(), vec!["octo/notes", "acme/docs"]);
}

#[test]
fn ticked_repos_show_first_even_when_they_do_not_match() {
    let (mut store, _) = setup();
    open(&mut store);
    store.send(A::NewChannel(N::ToggleRepo("octo/notes".into())), |_| {});
    store.send(A::NewChannel(N::Query("ldgr".into())), |_| {});
    assert_eq!(sheet(&store).shown_repos(), vec!["octo/notes", "acme/ledger"]);
    assert_eq!(sheet(&store).repos, vec!["octo/notes".to_owned()]);
}

#[test]
fn the_create_button_says_how_many_repos_it_will_clone() {
    let (mut store, _) = setup();
    open(&mut store);
    assert_eq!(sheet(&store).create_label(), "Create channel");
    store.send(A::NewChannel(N::ToggleRepo("acme/ledger".into())), |_| {});
    assert_eq!(sheet(&store).create_label(), "Create channel and clone 1 repo");
    store.send(A::NewChannel(N::ToggleRepo("acme/api".into())), |_| {});
    assert_eq!(sheet(&store).create_label(), "Create channel and clone 2 repos");
    store.send(A::NewChannel(N::ToggleRepo("acme/ledger".into())), |_| {});
    assert_eq!(sheet(&store).repos, vec!["acme/api".to_owned()]);
}

#[test]
fn the_folder_follows_the_name() {
    let (mut store, _) = setup();
    open(&mut store);
    store.send(A::NewChannel(N::Name("Billing exports".into())), |_| {});
    assert_eq!(sheet(&store).folder(), "billing-exports");
    store.send(A::NewChannel(N::Name("  # Q4 / Billing!! ".into())), |_| {});
    assert_eq!(sheet(&store).folder(), "q4-billing");
}

#[test]
fn the_machines_line_names_the_ticked_machines_in_tick_order_and_is_empty_with_none() {
    let (mut store, _fakes) = setup();
    open(&mut store);
    let machines = store.state().workspace.machines.clone();
    let name = |id: &str| {
        machines
            .iter()
            .find(|machine| machine.id == MachineId::new(id))
            .map(|machine| machine.name.clone())
            .expect("the design workspace has the machine")
    };
    assert_eq!(sheet(&store).machines_line(&machines), Some(name("this-mac")));

    store.send(A::NewChannel(N::ToggleMachine(MachineId::new("devbox-1"))), |_| {});
    assert_eq!(
        sheet(&store).machines_line(&machines),
        Some(format!("{}, {}", name("this-mac"), name("devbox-1")))
    );

    store.send(A::NewChannel(N::ToggleMachine(MachineId::new("this-mac"))), |_| {});
    store.send(A::NewChannel(N::ToggleMachine(MachineId::new("devbox-1"))), |_| {});
    assert_eq!(sheet(&store).machines_line(&machines), None);
}

#[test]
fn an_offline_machine_cannot_be_ticked() {
    let (mut store, _) = setup();
    open(&mut store);
    store.send(A::NewChannel(N::ToggleMachine(MachineId::new("gpu-box"))), |_| {});
    assert_eq!(sheet(&store).machines, ids(&["this-mac"]));
    store.send(A::NewChannel(N::ToggleMachine(MachineId::new("devbox-1"))), |_| {});
    assert_eq!(sheet(&store).machines, ids(&["this-mac", "devbox-1"]));
}

#[test]
fn create_clones_the_ticked_repos_on_the_ticked_machines() {
    let (mut store, fakes) = setup();
    open(&mut store);
    store.send(A::NewChannel(N::Name("Billing exports".into())), |_| {});
    store.send(A::NewChannel(N::ToggleRepo("acme/ledger".into())), |_| {});
    store.send(A::NewChannel(N::ToggleRepo("acme/api".into())), |_| {});
    store.send(A::NewChannel(N::ToggleMachine(MachineId::new("devbox-1"))), |_| {});
    store.send(A::NewChannel(N::Create), |_| {});
    store.skip_received();
    let channel = made(&fakes, "billing-exports").expect("the channel is made");
    assert_eq!(channel.repos, vec!["acme/ledger".to_owned(), "acme/api".to_owned()]);
    assert_eq!(channel.machines, ids(&["this-mac", "devbox-1"]));
}

#[test]
fn a_channel_with_no_repos_and_without_this_mac_is_made() {
    let (mut store, fakes) = setup();
    open(&mut store);
    store.send(A::NewChannel(N::Name("Notes".into())), |_| {});
    store.send(A::NewChannel(N::ToggleMachine(MachineId::new("this-mac"))), |_| {});
    store.send(A::NewChannel(N::ToggleMachine(MachineId::new("devbox-1"))), |_| {});
    store.send(A::NewChannel(N::Create), |_| {});
    store.skip_received();
    let channel = made(&fakes, "notes").expect("the channel is made");
    assert!(channel.repos.is_empty());
    assert_eq!(channel.machines, ids(&["devbox-1"]));
}

#[test]
fn create_with_no_machine_says_to_pick_one_and_makes_nothing() {
    let (mut store, fakes) = setup();
    open(&mut store);
    store.send(A::NewChannel(N::Name("Notes".into())), |_| {});
    store.send(A::NewChannel(N::ToggleMachine(MachineId::new("this-mac"))), |_| {});
    store.send(A::NewChannel(N::Create), |_| {});
    store.skip_received();
    assert!(sheet(&store).error.is_some());
    assert!(!sheet(&store).creating);
    assert!(made(&fakes, "notes").is_none());
}

#[test]
fn when_gh_cannot_list_repos_the_sheet_says_why_and_still_makes_a_channel() {
    let (mut store, fakes) = setup();
    fakes.daemon.set_repo_search_problem(Some(signed_out()));
    open(&mut store);
    assert!(sheet(&store).search.problem().is_some());
    assert!(sheet(&store).shown_repos().is_empty());
    store.send(A::NewChannel(N::Name("Notes".into())), |_| {});
    store.send(A::NewChannel(N::Create), |_| {});
    store.skip_received();
    assert!(made(&fakes, "notes").is_some());
}

#[test]
fn retry_lists_the_repos_again_after_a_failure() {
    let (mut store, fakes) = setup();
    fakes.daemon.set_repo_search_problem(Some(signed_out()));
    open(&mut store);
    assert!(sheet(&store).search.problem().is_some());
    fakes.daemon.set_repo_search_problem(None);
    store.send(A::NewChannel(N::RetryRepos), |_| {});
    store.skip_received();
    assert!(sheet(&store).search.problem().is_none());
    assert_eq!(sheet(&store).shown_repos().len(), 6);
}
