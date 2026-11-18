//! Frame 98: a repository whose clone has no default branch can't be read
//! against it. The page says which one, Find the default branch has Humata
//! ask the remote, and the changes read again; a failed find keeps its
//! reason so the person can try again or pick a branch instead.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{DiffLine, FileDiff, LineKind, RepoDiff, SessionId};
use humata2_clients::{FakeDaemon, Operation};
use humata2_review::{Fetch, Review, ReviewAction as A, ReviewState};
use tca::{TestClock, TestStore};

const REPO: &str = "acme/ledger";

fn session() -> SessionId {
    SessionId::new("ledger")
}

fn change() -> RepoDiff {
    RepoDiff {
        session: session(),
        repo: REPO.to_owned(),
        base: "origin/main".to_owned(),
        files: vec![FileDiff {
            path: "src/ledger.rs".to_owned(),
            added: 1,
            removed: 0,
            lines: vec![DiffLine {
                kind: LineKind::Added,
                old: None,
                new: Some(1),
                text: "fn a() {}".to_owned(),
            }],
        }],
        comments: Vec::new(),
    }
}

struct Rig {
    store: TestStore<Review>,
    daemon: FakeDaemon,
}

/// The review page on a daemon whose clone of `REPO` has no default branch.
fn rig() -> Rig {
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(fixture::design(), clock.clone());
    daemon.set_diffs(&session(), vec![change()]);
    daemon.set_no_default_branch(&session(), REPO);
    let mut shown = fixture::design();
    shown.diffs.clear();
    let state = ReviewState {
        workspace: Arc::new(shown),
        ..ReviewState::default()
    };
    let store = TestStore::new(state, Review::new(Arc::new(daemon.clone()), clock)).non_exhaustive();
    Rig { store, daemon }
}

impl Rig {
    fn act(&mut self, action: A) {
        self.store.send(action, |_| {});
        for _ in 0..6 {
            self.store.skip_received();
        }
    }

    fn fetch(&self) -> Fetch {
        self.store.state().fetch.get(&session()).cloned().unwrap_or_default()
    }
}

#[test]
fn a_repo_with_no_default_branch_is_named_so_the_page_can_offer_to_find_it() {
    let mut rig = rig();
    rig.act(A::Open(session()));
    let fetch = rig.fetch();
    assert_eq!(fetch.no_default.as_deref(), Some(REPO));
    assert!(fetch.diffs.is_none());
}

#[test]
fn finding_the_default_branch_reads_the_changes_again() {
    let mut rig = rig();
    rig.act(A::Open(session()));
    rig.act(A::FindDefaultBranch { repo: REPO.to_owned() });
    let fetch = rig.fetch();
    assert_eq!(fetch.no_default, None);
    assert!(!fetch.finding);
    assert_eq!(fetch.error, None);
    assert!(fetch.diffs.is_some_and(|diffs| diffs.iter().any(|diff| diff.repo == REPO)));
}

#[test]
fn a_failed_find_keeps_its_reason_and_the_repo_until_a_retry_works() {
    let mut rig = rig();
    rig.act(A::Open(session()));
    rig.daemon
        .fail_next(Operation::FindDefaultBranch, "The remote couldn't be reached.");
    rig.act(A::FindDefaultBranch { repo: REPO.to_owned() });
    let fetch = rig.fetch();
    assert!(fetch.find_error.is_some());
    assert!(!fetch.finding);
    assert_eq!(fetch.no_default.as_deref(), Some(REPO));

    rig.act(A::FindDefaultBranch { repo: REPO.to_owned() });
    let fetch = rig.fetch();
    assert_eq!(fetch.find_error, None);
    assert_eq!(fetch.no_default, None);
    assert!(fetch.diffs.is_some());
}
