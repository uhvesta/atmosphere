//! DS-35: one repository of a session failing to read does not hide the
//! others; its reason is kept per repository, and Try again reads it anew.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{DiffLine, FileDiff, LineKind, RepoDiff, SessionId};
use humata2_clients::FakeDaemon;
use humata2_review::{Review, ReviewAction as A, ReviewState};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("ledger")
}

fn change(repo: &str) -> RepoDiff {
    RepoDiff {
        session: session(),
        repo: repo.to_owned(),
        base: "origin/main".to_owned(),
        files: vec![FileDiff {
            path: "src/lib.rs".to_owned(),
            added: 1,
            removed: 0,
            lines: vec![DiffLine {
                kind: LineKind::Added,
                old: None,
                new: Some(1),
                text: "fn a() {}".to_owned(),
            }],
            ..FileDiff::default()
        }],
        comments: Vec::new(),
    }
}

struct Rig {
    store: TestStore<Review>,
    daemon: FakeDaemon,
}

/// A session the page already knows works in `repos`, whose daemon holds a
/// change in each.
fn rig(repos: &[&str]) -> Rig {
    let clock = Arc::new(TestClock::new());
    let mut workspace = fixture::design();
    // It works in `repos` and nowhere else: its home is the first of them.
    for each in workspace.sessions.iter_mut().filter(|each| each.id == session()) {
        each.repo = repos[0].to_owned();
        each.clones.clear();
    }
    workspace.diffs = repos.iter().map(|repo| change(repo)).collect();
    let daemon = FakeDaemon::new(workspace.clone(), clock.clone());
    daemon.set_diffs(&session(), repos.iter().map(|repo| change(repo)).collect());
    let state = ReviewState {
        workspace: Arc::new(workspace),
        ..ReviewState::default()
    };
    let store =
        TestStore::new(state, Review::new(Arc::new(daemon.clone()), clock)).non_exhaustive();
    Rig { store, daemon }
}

impl Rig {
    fn act(&mut self, action: A) {
        self.store.send(action, |_| {});
        for _ in 0..6 {
            self.store.skip_received();
        }
    }

    fn repos_shown(&self) -> Vec<String> {
        self.store
            .state()
            .workspace
            .diffs
            .iter()
            .filter(|diff| diff.session == session())
            .map(|diff| diff.repo.clone())
            .collect()
    }

    fn repo_errors(&self) -> Vec<String> {
        self.store
            .state()
            .fetch
            .get(&session())
            .map(|fetch| fetch.repo_errors.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn whole_error(&self) -> Option<String> {
        self.store
            .state()
            .fetch
            .get(&session())
            .and_then(|fetch| fetch.error.clone())
    }
}

#[test]
fn a_failing_repository_keeps_its_own_error_and_the_others_still_show() {
    let mut rig = rig(&["api", "web"]);
    rig.daemon.fail_repo(&session(), "web", "git broke in web");
    rig.act(A::Open(session()));

    assert_eq!(rig.repos_shown(), vec!["api".to_owned()]);
    assert_eq!(rig.whole_error(), None);
    assert_eq!(rig.repo_errors(), vec!["web".to_owned()]);
}

#[test]
fn try_again_reads_everything_and_clears_the_repository_errors() {
    let mut rig = rig(&["api", "web"]);
    rig.daemon.fail_repo(&session(), "web", "git broke in web");
    rig.act(A::Open(session()));
    rig.daemon.heal_repo(&session(), "web");
    rig.act(A::Retry);

    assert!(rig.repo_errors().is_empty());
    assert_eq!(rig.whole_error(), None);
    assert_eq!(rig.repos_shown(), vec!["api".to_owned(), "web".to_owned()]);
}

#[test]
fn every_repository_failing_is_a_whole_error() {
    let mut rig = rig(&["api", "web"]);
    rig.daemon.fail_repo(&session(), "api", "git broke in api");
    rig.daemon.fail_repo(&session(), "web", "git broke in web");
    rig.act(A::Open(session()));

    assert!(rig.whole_error().is_some());
    assert!(rig.repo_errors().is_empty());
}

#[test]
fn a_session_with_one_repository_fails_whole_without_reading_by_repository() {
    let mut rig = rig(&["api"]);
    rig.daemon.fail_repo(&session(), "api", "git broke in api");
    rig.act(A::Open(session()));

    assert!(rig.whole_error().is_some());
    assert!(rig.repo_errors().is_empty());
    assert!(
        !rig.daemon
            .calls()
            .iter()
            .any(|call| call.starts_with("repo diff ")),
        "one repository is read whole, not by repository"
    );
}
