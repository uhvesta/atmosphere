//! DS-84: the file list driven by keys alone: a cursor that moves over
//! repository and file rows, folds and unfolds, and opens a file.
use std::sync::Arc;

use humata2_clients::model::{FileDiff, RepoDiff, SessionId};
use humata2_clients::{FakeDaemon, Workspace};
use humata2_review::{ListCursor, Review, ReviewAction as A, ReviewState};
use tca::{Reducer, TestClock};

fn session() -> SessionId {
    SessionId::new("ledger")
}

fn repo(name: &str, paths: &[&str]) -> RepoDiff {
    RepoDiff {
        session: session(),
        repo: name.to_owned(),
        base: "origin/main".to_owned(),
        files: paths
            .iter()
            .map(|path| FileDiff {
                path: (*path).to_owned(),
                ..FileDiff::default()
            })
            .collect(),
        comments: Vec::new(),
    }
}

/// Rows: api, a.rs, b.rs, web, c.rs.
fn page() -> ReviewState {
    let workspace = Workspace {
        diffs: vec![repo("api", &["a.rs", "b.rs"]), repo("web", &["c.rs"])],
        ..Workspace::default()
    };
    ReviewState {
        session: Some(session()),
        workspace: Arc::new(workspace),
        ..ReviewState::default()
    }
}

fn send(state: &mut ReviewState, actions: &[A]) {
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(Workspace::default(), clock.clone());
    let review = Review::new(Arc::new(daemon), clock);
    for action in actions {
        let _effect = review.reduce(state, action.clone());
    }
}

fn file(repo: &str, path: &str) -> Option<ListCursor> {
    Some(ListCursor::File {
        repo: repo.to_owned(),
        path: path.to_owned(),
    })
}

fn on_repo(repo: &str) -> Option<ListCursor> {
    Some(ListCursor::Repo(repo.to_owned()))
}

fn cursor(state: &ReviewState) -> Option<ListCursor> {
    state.list_cursor(&session()).cloned()
}

fn collapsed(state: &ReviewState, repo: &str) -> bool {
    state.collapsed.contains(&(session(), repo.to_owned()))
}

#[test]
fn the_first_press_only_places_the_cursor_on_the_open_file() {
    let mut state = page();
    send(&mut state, &[A::ListDown]);
    assert_eq!(cursor(&state), file("api", "a.rs"), "the first file is open until one is picked");

    let mut state = page();
    send(&mut state, &[A::SelectFile { repo: "api".into(), path: "b.rs".into() }, A::ListUp]);
    assert_eq!(cursor(&state), file("api", "b.rs"));
}

#[test]
fn down_and_up_walk_the_rows_and_stop_at_the_ends() {
    let mut state = page();
    send(&mut state, &[A::ListUp, A::ListUp, A::ListUp]);
    assert_eq!(cursor(&state), on_repo("api"), "stops on the first row");

    send(&mut state, &[A::ListDown]);
    assert_eq!(cursor(&state), file("api", "a.rs"));
    send(&mut state, &[A::ListDown, A::ListDown, A::ListDown]);
    assert_eq!(cursor(&state), file("web", "c.rs"));
    send(&mut state, &[A::ListDown, A::ListDown]);
    assert_eq!(cursor(&state), file("web", "c.rs"), "stops on the last row");
    send(&mut state, &[A::ListUp]);
    assert_eq!(cursor(&state), on_repo("web"));
}

#[test]
fn folded_repositories_hide_their_files_from_the_walk() {
    let mut state = page();
    send(&mut state, &[A::ListDown, A::ListUp, A::ListCollapse, A::ListDown]);
    assert!(collapsed(&state, "api"));
    assert_eq!(cursor(&state), on_repo("web"));
}

#[test]
fn collapse_from_a_file_goes_to_its_repository_then_folds_it() {
    let mut state = page();
    send(&mut state, &[A::ListDown]);
    assert_eq!(cursor(&state), file("api", "a.rs"));

    send(&mut state, &[A::ListCollapse]);
    assert_eq!(cursor(&state), on_repo("api"));
    assert!(!collapsed(&state, "api"));

    send(&mut state, &[A::ListCollapse]);
    assert!(collapsed(&state, "api"));
    assert_eq!(cursor(&state), on_repo("api"));

    send(&mut state, &[A::ListCollapse]);
    assert!(collapsed(&state, "api"), "a folded repository stays folded");
    assert_eq!(cursor(&state), on_repo("api"));
}

#[test]
fn expand_reverses_collapse() {
    let mut state = page();
    send(&mut state, &[A::ListDown, A::ListUp, A::ListCollapse]);
    assert!(collapsed(&state, "api"));

    send(&mut state, &[A::ListExpand]);
    assert!(!collapsed(&state, "api"));
    assert_eq!(cursor(&state), on_repo("api"));

    send(&mut state, &[A::ListExpand]);
    assert_eq!(cursor(&state), file("api", "a.rs"), "an open repository enters its files");

    send(&mut state, &[A::ListExpand]);
    assert_eq!(cursor(&state), file("api", "a.rs"), "a file stays put");
}

#[test]
fn open_on_a_file_selects_it() {
    let mut state = page();
    send(&mut state, &[A::ListDown, A::ListDown, A::ListOpen]);
    assert_eq!(
        state.saved.selected.get(&session()),
        Some(&("api".to_owned(), "b.rs".to_owned()))
    );
}

#[test]
fn open_on_a_repository_folds_and_unfolds_it() {
    let mut state = page();
    send(&mut state, &[A::ListDown, A::ListUp, A::ListOpen]);
    assert!(collapsed(&state, "api"));
    assert!(state.saved.selected.get(&session()).is_none());
    send(&mut state, &[A::ListOpen]);
    assert!(!collapsed(&state, "api"));
}

#[test]
fn folding_a_repository_from_elsewhere_moves_a_cursor_off_its_hidden_file() {
    let mut state = page();
    send(&mut state, &[A::ListDown]);
    assert_eq!(cursor(&state), file("api", "a.rs"));
    send(&mut state, &[A::ToggleRepo { repo: "api".into() }]);
    assert_eq!(cursor(&state), on_repo("api"));
}

#[test]
fn the_keys_do_nothing_without_an_open_session() {
    let mut state = page();
    state.session = None;
    send(&mut state, &[A::ListDown, A::ListOpen, A::ListCollapse, A::ListExpand]);
    assert!(state.list_cursor.is_empty());
    assert!(state.collapsed.is_empty());
}
