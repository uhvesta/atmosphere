//! D800, DS-60 to DS-65: a plan with a stack is stepped through layer by
//! layer. Each pick reads only that layer's repositories, the person's own
//! diff targets are never touched, and the base menus stay shut.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{DiffBase, RepoBase, RepoDiffSpec, SessionId, Status};
use humata2_clients::views::{LayerDrift, LayerRepoView, LayerView, PlanView, StackView};
use humata2_clients::{FakeDaemon, Workspace};
use humata2_review::stack::{StackHeader, pull_request_label};
use humata2_review::{Review, ReviewAction as A, ReviewState};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("ledger")
}

fn workspace() -> Workspace {
    let mut workspace = fixture::design();
    for each in &mut workspace.sessions {
        if each.id == session() {
            each.status = Status::Idle;
            each.repo = "acme/api".to_owned();
        }
    }
    workspace.diffs = Vec::new();
    workspace
}

fn range(repo: &str, base: &str, head: &str) -> RepoDiffSpec {
    RepoDiffSpec {
        repo: repo.to_owned(),
        base: RepoBase::Range {
            base: base.to_owned(),
            head: head.to_owned(),
        },
    }
}

fn part(repo: &str, spec: RepoDiffSpec, pull_request: &str, files: u32) -> LayerRepoView {
    LayerRepoView {
        repo: repo.to_owned(),
        spec,
        pull_request: pull_request.to_owned(),
        files,
        drift: None,
    }
}

fn layer(title: &str, purpose: &str, repos: Vec<LayerRepoView>) -> LayerView {
    LayerView {
        title: title.to_owned(),
        purpose: purpose.to_owned(),
        repos,
        ..LayerView::default()
    }
}

/// Three layers over api and web: schema (api), client (api and web, web
/// drifted), screen (web).
fn stack() -> StackView {
    let mut client_web = part("acme/web", range("acme/web", "w1", "w2"), "acme/web#88", 1);
    client_web.drift = Some(LayerDrift::Moved);
    StackView {
        layers: vec![
            layer(
                "schema",
                "The tables",
                vec![part("acme/api", range("acme/api", "a0", "a1"), "acme/api#412", 3)],
            ),
            layer(
                "client",
                "Call them",
                vec![
                    part("acme/api", range("acme/api", "a1", "a2"), "acme/api#412", 2),
                    client_web,
                ],
            ),
            layer(
                "screen",
                "Show them",
                vec![part("acme/web", range("acme/web", "w2", "w3"), "acme/web#88", 4)],
            ),
        ],
        whole: vec![range("acme/api", "a0", "a2"), range("acme/web", "w1", "w3")],
    }
}

fn spec_of(stack: &StackView, title: &str) -> DiffBase {
    let Some(found) = stack.layers.iter().find(|each| each.title == title) else {
        panic!("the stack has {title}");
    };
    DiffBase::Spec(found.repos.iter().map(|repo| repo.spec.clone()).collect())
}

fn stacked() -> ReviewState {
    let mut state = ReviewState {
        workspace: Arc::new(workspace()),
        session: Some(session()),
        ..ReviewState::default()
    };
    state.plans.insert(
        session(),
        PlanView {
            stack: Some(stack()),
            ..PlanView::default()
        },
    );
    state
}

fn page(state: ReviewState) -> TestStore<Review> {
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(workspace(), clock.clone());
    TestStore::new(state, Review::new(Arc::new(daemon), clock)).non_exhaustive()
}

fn read_of(store: &mut TestStore<Review>, base: DiffBase) {
    store.receive_matching(
        "a read of the picked layer",
        move |action| matches!(action, A::Fetched { base: read, .. } if *read == base),
        |_| {},
    );
}

fn showing(store: &TestStore<Review>) -> Option<String> {
    store.state().layer_of(&session()).map(str::to_owned)
}

#[test]
fn a_pick_reads_only_that_layers_repositories() {
    let mut store = page(stacked());
    store.send(A::PickLayer(Some("screen".to_owned())), |_| {});
    read_of(&mut store, spec_of(&stack(), "screen"));
    assert_eq!(showing(&store).as_deref(), Some("screen"));

    store.send(A::PickLayer(None), |_| {});
    read_of(&mut store, DiffBase::Spec(stack().whole));
    assert_eq!(showing(&store), None);
}

#[test]
fn next_walks_whole_stack_then_the_layers_bottom_up_and_stops_at_the_top() {
    let mut store = page(stacked());
    let mut seen = Vec::new();
    for _ in 0..5 {
        store.send(A::NextLayer, |_| {});
        seen.push(showing(&store));
    }
    assert_eq!(
        seen,
        vec![
            Some("schema".to_owned()),
            Some("client".to_owned()),
            Some("screen".to_owned()),
            Some("screen".to_owned()),
            Some("screen".to_owned()),
        ]
    );
}

#[test]
fn previous_walks_back_and_stops_at_whole_stack() {
    let mut state = stacked();
    state.layer.insert(session(), Some("client".to_owned()));
    let mut store = page(state);
    let mut seen = Vec::new();
    for _ in 0..4 {
        store.send(A::PrevLayer, |_| {});
        seen.push(showing(&store));
    }
    assert_eq!(seen, vec![Some("schema".to_owned()), None, None, None]);
}

#[test]
fn picking_layers_leaves_the_persons_own_targets_alone() {
    let mut state = stacked();
    state.targets.insert(
        session(),
        humata2_clients::model::DiffTargets::default(),
    );
    let before = state.targets.clone();
    let saved = state.saved.clone();
    let mut store = page(state);
    store.send(A::PickLayer(Some("client".to_owned())), |_| {});
    assert_eq!(store.state().targets, before);
    store.send(A::PickLayer(None), |_| {});
    assert_eq!(store.state().targets, before);
    assert_eq!(store.state().saved, saved);
}

#[test]
fn the_base_menus_stay_shut_while_a_stack_shows() {
    let mut store = page(stacked());
    store.send(A::ToggleBaseMenu, |_| {});
    assert!(!store.state().base_menu);
    store.send(
        A::ToggleRepoBase {
            repo: "acme/api".to_owned(),
        },
        |_| {},
    );
    assert!(store.state().repo_picker.is_none());
    store.send(A::PickBase(DiffBase::UncommittedOnly), |_| {});
    store.send(
        A::PickRepoBase {
            session: session(),
            repo: "acme/api".to_owned(),
            base: RepoBase::Uncommitted,
        },
        |_| {},
    );
    assert!(store.state().targets.is_empty());
}

#[test]
fn the_base_menu_still_opens_without_a_stack() {
    let mut state = stacked();
    state.plans.clear();
    let mut store = page(state);
    store.send(A::ToggleBaseMenu, |_| {});
    assert!(store.state().base_menu);
}

#[test]
fn skim_files_open_and_fold_per_repository() {
    let mut store = page(stacked());
    let open = |store: &TestStore<Review>, repo: &str| {
        store
            .state()
            .skim_open
            .contains(&(session(), repo.to_owned()))
    };
    store.send(A::ToggleSkim { repo: "acme/api".to_owned() }, |_| {});
    assert!(open(&store, "acme/api"));
    assert!(!open(&store, "acme/web"));
    store.send(A::ToggleSkim { repo: "acme/api".to_owned() }, |_| {});
    assert!(!open(&store, "acme/api"));
}

#[test]
fn rows_run_whole_stack_then_the_top_layer_down_to_one() {
    let state = stacked();
    let Some(rows) = state.stack_rows() else {
        panic!("a stack has rows");
    };
    let numbers: Vec<Option<usize>> = rows.rows.iter().map(|row| row.number).collect();
    assert_eq!(numbers, vec![None, Some(3), Some(2), Some(1)]);
    assert_eq!(rows.picked, 0);
    assert_eq!(rows.header, StackHeader::Whole { layers: 3 });
    assert_eq!(rows.shown_repos, vec!["acme/api", "acme/web"]);
    assert!(rows.untouched_repos.is_empty());
}

#[test]
fn only_the_picked_row_shows_its_purpose_and_the_header_names_it() {
    let mut state = stacked();
    state.layer.insert(session(), Some("client".to_owned()));
    let Some(rows) = state.stack_rows() else {
        panic!("a stack has rows");
    };
    assert_eq!(rows.picked, 2);
    let purposes: Vec<&str> = rows.rows.iter().map(|row| row.purpose.as_str()).collect();
    assert_eq!(purposes, vec!["", "", "Call them", ""]);
    assert_eq!(
        rows.header,
        StackHeader::Layer {
            number: 2,
            of: 3,
            title: "client".to_owned()
        }
    );
    assert_eq!(rows.rows[2].pull_requests, vec!["api #412", "web #88"]);
    assert_eq!(rows.rows[2].files, 3);
}

#[test]
fn every_row_names_the_repositories_it_touches() {
    let state = stacked();
    let Some(rows) = state.stack_rows() else {
        panic!("a stack has rows");
    };
    let repos: Vec<&[String]> = rows.rows.iter().map(|row| row.repos.as_slice()).collect();
    // Whole stack, then screen (web), client (api and web), schema (api).
    assert_eq!(
        repos,
        vec![
            &["acme/api".to_owned(), "acme/web".to_owned()][..],
            &["acme/web".to_owned()][..],
            &["acme/api".to_owned(), "acme/web".to_owned()][..],
            &["acme/api".to_owned()][..],
        ]
    );
}

#[test]
fn drifted_repositories_are_listed_and_untouched_ones_noted() {
    let mut state = stacked();
    state.layer.insert(session(), Some("schema".to_owned()));
    let Some(rows) = state.stack_rows() else {
        panic!("a stack has rows");
    };
    // schema touches api only; the session also works in web (its diffs).
    assert_eq!(rows.shown_repos, vec!["acme/api"]);
    assert_eq!(rows.rows[2].drifted, vec!["acme/web"]);
    assert!(rows.rows[3].drifted.is_empty());
    assert_eq!(rows.rows[0].drifted, vec!["acme/web"]);
}

#[test]
fn a_session_repository_a_layer_skips_is_untouched() {
    let mut state = stacked();
    state.workspace = Arc::new({
        let mut workspace = workspace();
        workspace.diffs = vec![humata2_clients::model::RepoDiff {
            session: session(),
            repo: "acme/web".to_owned(),
            base: String::new(),
            files: Vec::new(),
            comments: Vec::new(),
        }];
        workspace
    });
    state.layer.insert(session(), Some("schema".to_owned()));
    let Some(rows) = state.stack_rows() else {
        panic!("a stack has rows");
    };
    assert_eq!(rows.untouched_repos, vec!["acme/web"]);
}

#[test]
fn a_pull_request_reads_as_its_repository_and_number() {
    assert_eq!(pull_request_label("owner/api#412").as_deref(), Some("api #412"));
    assert_eq!(pull_request_label(""), None);
}

#[test]
fn a_repository_the_whole_stack_names_is_untouched_by_a_layer_that_skips_it() {
    // The session's own diffs list only api; the Whole stack also spans web.
    let mut state = stacked();
    state.layer.insert(session(), Some("schema".to_owned()));
    let Some(rows) = state.stack_rows() else {
        panic!("a stack has rows");
    };
    assert_eq!(rows.untouched_repos, vec!["acme/web"]);
}

#[test]
fn the_whole_stack_keeps_its_file_count_while_a_layer_shows() {
    // The session works in web too, so the daemon reads the Whole stack.
    let mut both = workspace();
    both.diffs = vec![humata2_clients::model::RepoDiff {
        session: session(),
        repo: "acme/web".to_owned(),
        base: String::new(),
        files: Vec::new(),
        comments: Vec::new(),
    }];
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(both.clone(), clock.clone());
    let mut state = stacked();
    state.workspace = Arc::new(both);
    let mut store = TestStore::new(state, Review::new(Arc::new(daemon), clock)).non_exhaustive();
    store.send(A::PickLayer(Some("schema".to_owned())), |_| {});
    read_of(&mut store, spec_of(&stack(), "schema"));
    store.send(A::PickLayer(None), |_| {});
    read_of(&mut store, DiffBase::Spec(stack().whole));
    store.send(A::PickLayer(Some("schema".to_owned())), |_| {});
    read_of(&mut store, spec_of(&stack(), "schema"));
    // The fake daemon has no files for these ranges, so the Whole stack
    // read listed none; the layers alone would add up to 10.
    assert_eq!(store.state().stack_rows().map(|rows| rows.rows[0].files), Some(0));
}

#[test]
fn a_picked_layer_shows_the_whole_stack_count_last_read_not_the_layers_sum() {
    let mut state = stacked();
    state.layer.insert(session(), Some("schema".to_owned()));
    state.fetch.entry(session()).or_default().whole_files = Some(7);
    let Some(rows) = state.stack_rows() else {
        panic!("a stack has rows");
    };
    assert_eq!(rows.rows[0].files, 7, "the layers add up to 10");
}
