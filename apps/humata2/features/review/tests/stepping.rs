//! DV-15: stepping through the open session's files and changes, and the
//! narrow file list that folds into the file bar.
use std::sync::Arc;

use humata2_clients::model::{DiffLine, FileDiff, LineKind, RepoDiff, SessionId};
use humata2_clients::{FakeDaemon, Workspace};
use humata2_review::{Review, ReviewAction as A, ReviewState};
use tca::{Reducer, TestClock};

fn session() -> SessionId {
    SessionId::new("ledger")
}

/// A file with `hunks` hunks of one added line each.
fn file(path: &str, hunks: usize) -> FileDiff {
    let mut lines = Vec::new();
    for hunk in 0..hunks {
        lines.push(DiffLine {
            kind: LineKind::Hunk,
            old: None,
            new: None,
            text: format!("@@ hunk {hunk} @@"),
        });
        lines.push(DiffLine {
            kind: LineKind::Added,
            old: None,
            new: Some(1),
            text: "added".to_owned(),
        });
    }
    FileDiff {
        path: path.to_owned(),
        lines,
        ..FileDiff::default()
    }
}

fn repo(name: &str, files: Vec<FileDiff>) -> RepoDiff {
    RepoDiff {
        session: session(),
        repo: name.to_owned(),
        base: "origin/main".to_owned(),
        files,
        comments: Vec::new(),
    }
}

/// Two repos: `acme/api` with a.rs (1 hunk) and b.rs (3 hunks), `acme/web`
/// with c.rs.
fn page() -> ReviewState {
    let workspace = Workspace {
        diffs: vec![
            repo("acme/api", vec![file("a.rs", 1), file("b.rs", 3)]),
            repo("acme/web", vec![file("c.rs", 1)]),
        ],
        ..Workspace::default()
    };
    ReviewState {
        session: Some(session()),
        workspace: Arc::new(workspace),
        ..ReviewState::default()
    }
}

fn send(state: &mut ReviewState, action: A) {
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(Workspace::default(), clock.clone());
    let review = Review::new(Arc::new(daemon), clock);
    let _effect = review.reduce(state, action);
}

fn open(state: &ReviewState) -> Option<(String, String)> {
    state.saved.selected.get(&session()).cloned()
}

fn at(repo: &str, path: &str) -> Option<(String, String)> {
    Some((repo.to_owned(), path.to_owned()))
}

#[test]
fn next_and_previous_file_walk_the_files_across_repos_and_stop_at_the_ends() {
    let mut state = page();
    send(&mut state, A::PrevFile);
    assert_eq!(open(&state), None, "nothing before the first file");

    send(&mut state, A::NextFile);
    assert_eq!(open(&state), at("acme/api", "b.rs"));
    send(&mut state, A::NextFile);
    assert_eq!(open(&state), at("acme/web", "c.rs"));
    send(&mut state, A::NextFile);
    assert_eq!(open(&state), at("acme/web", "c.rs"));

    send(&mut state, A::PrevFile);
    send(&mut state, A::PrevFile);
    assert_eq!(open(&state), at("acme/api", "a.rs"));
    send(&mut state, A::PrevFile);
    assert_eq!(open(&state), at("acme/api", "a.rs"));
}

#[test]
fn stepping_onto_a_file_in_a_collapsed_repo_opens_the_repo() {
    let mut state = page();
    state.saved.selected.insert(session(), ("acme/api".to_owned(), "b.rs".to_owned()));
    state.collapsed.insert((session(), "acme/web".to_owned()));

    send(&mut state, A::NextFile);
    assert_eq!(open(&state), at("acme/web", "c.rs"));
    assert!(!state.collapsed.contains(&(session(), "acme/web".to_owned())));
}

#[test]
fn stepping_with_no_open_session_changes_nothing() {
    let mut state = page();
    state.session = None;
    let before = state.clone();
    send(&mut state, A::NextFile);
    send(&mut state, A::NextChange);
    assert_eq!(state, before);
}

#[test]
fn next_and_previous_change_stay_within_the_files_hunks() {
    let mut state = page();
    send(&mut state, A::SelectFile {
        repo: "acme/api".to_owned(),
        path: "b.rs".to_owned(),
    });
    assert_eq!(state.current_change(&session()), 0);
    send(&mut state, A::PrevChange);
    assert_eq!(state.current_change(&session()), 0);
    for _ in 0..5 {
        send(&mut state, A::NextChange);
    }
    assert_eq!(state.current_change(&session()), 2);
    send(&mut state, A::PrevChange);
    assert_eq!(state.current_change(&session()), 1);
}

#[test]
fn a_file_with_one_hunk_has_one_change() {
    let mut state = page();
    send(&mut state, A::NextChange);
    assert_eq!(state.current_change(&session()), 0);
}

#[test]
fn the_change_starts_over_when_the_file_changes() {
    let mut state = page();
    send(&mut state, A::SelectFile {
        repo: "acme/api".to_owned(),
        path: "b.rs".to_owned(),
    });
    send(&mut state, A::NextChange);
    send(&mut state, A::NextChange);
    assert_eq!(state.current_change(&session()), 2);

    send(&mut state, A::NextFile);
    send(&mut state, A::PrevFile);
    assert_eq!(open(&state), at("acme/api", "b.rs"));
    assert_eq!(state.current_change(&session()), 0);
}

#[test]
fn toggling_the_file_list_opens_and_folds_it() {
    let mut state = page();
    send(&mut state, A::ToggleFileList);
    assert!(state.file_list_open.contains(&session()));
    send(&mut state, A::ToggleFileList);
    assert!(!state.file_list_open.contains(&session()));
}

#[test]
fn picking_a_file_folds_the_list_and_so_does_stepping() {
    let mut state = page();
    send(&mut state, A::ToggleFileList);
    send(&mut state, A::SelectFile {
        repo: "acme/web".to_owned(),
        path: "c.rs".to_owned(),
    });
    assert!(!state.file_list_open.contains(&session()));
    assert_eq!(open(&state), at("acme/web", "c.rs"));

    send(&mut state, A::ToggleFileList);
    send(&mut state, A::PrevFile);
    assert!(!state.file_list_open.contains(&session()));
}
