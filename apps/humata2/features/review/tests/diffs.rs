//! RV-1, RV-6 and D179: the review page reads the open session's changes
//! from the daemon when it opens, again when the session's turn ends, on a
//! new base and after Mark reviewed. A failed read says so and keeps the
//! last diff.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{
    DiffBase, DiffLine, FileDiff, LineKind, RepoDiff, SessionId, Status,
};
use humata2_clients::{FakeDaemon, Operation, Workspace};
use humata2_review::{Review, ReviewAction as A, ReviewState};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("ledger")
}

/// The design workspace with `status` on the session, as the app last
/// loaded it.
fn workspace(status: Status) -> Workspace {
    let mut workspace = fixture::design();
    for each in &mut workspace.sessions {
        if each.id == session() {
            each.status = status;
        }
    }
    workspace
}

/// One changed file in `repo`, adding `text` as line 1.
fn change(repo: &str, path: &str, text: &str) -> RepoDiff {
    RepoDiff {
        session: session(),
        repo: repo.to_owned(),
        base: "origin/main".to_owned(),
        files: vec![FileDiff {
            path: path.to_owned(),
            added: 1,
            removed: 0,
            lines: vec![
                DiffLine {
                    kind: LineKind::Hunk,
                    old: None,
                    new: None,
                    text: "@@ -0,0 +1 @@".to_owned(),
                },
                DiffLine {
                    kind: LineKind::Added,
                    old: None,
                    new: Some(1),
                    text: text.to_owned(),
                },
            ],
        }],
        comments: Vec::new(),
    }
}

struct Rig {
    store: TestStore<Review>,
    daemon: FakeDaemon,
}

/// The review page, on a daemon whose session has `diffs`, before the app
/// has read any of them.
fn rig(diffs: Vec<RepoDiff>) -> Rig {
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(workspace(Status::Working), clock.clone());
    daemon.set_diffs(&session(), diffs);
    let mut shown = workspace(Status::Working);
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

    /// The paths the page holds for the session, per repo.
    fn shown(&self) -> Vec<(String, Vec<String>)> {
        self.store
            .state()
            .workspace
            .diffs
            .iter()
            .filter(|diff| diff.session == session())
            .map(|diff| {
                (
                    diff.repo.clone(),
                    diff.files.iter().map(|file| file.path.clone()).collect(),
                )
            })
            .collect()
    }

    /// What the page says about its last read of the session.
    fn failed(&self) -> bool {
        self.store
            .state()
            .fetch
            .get(&session())
            .is_some_and(|fetch| fetch.error.is_some())
    }

    /// The app shows a newer workspace, with the session in `status`.
    fn status(&mut self, status: Status) {
        let mut next = (*self.store.state().workspace).clone();
        for each in &mut next.sessions {
            if each.id == session() {
                each.status = status;
            }
        }
        self.act(A::Show(Arc::new(next)));
    }
}

fn paths(repo: &str, paths: &[&str]) -> (String, Vec<String>) {
    (repo.to_owned(), paths.iter().map(|&path| path.to_owned()).collect())
}

#[test]
fn opening_a_session_reads_its_changes_from_the_daemon() {
    let mut rig = rig(vec![change("api", "src/client.rs", "fn a() {}")]);
    assert!(rig.shown().is_empty());
    rig.act(A::Open(session()));
    assert_eq!(rig.shown(), [paths("api", &["src/client.rs"])]);
    assert!(!rig.failed());
}

#[test]
fn the_end_of_a_turn_reads_the_changes_again() {
    let mut rig = rig(vec![change("api", "src/client.rs", "fn a() {}")]);
    rig.act(A::Open(session()));
    rig.daemon.set_diffs(
        &session(),
        vec![
            change("api", "src/client.rs", "fn a() {}"),
            change("web", "src/Cards.tsx", "export {}"),
        ],
    );
    rig.status(Status::Idle);
    assert_eq!(
        rig.shown(),
        [paths("api", &["src/client.rs"]), paths("web", &["src/Cards.tsx"])]
    );
}

#[test]
fn nothing_is_read_again_while_the_turn_goes_on() {
    let mut rig = rig(vec![change("api", "src/client.rs", "fn a() {}")]);
    rig.act(A::Open(session()));
    rig.daemon
        .set_diffs(&session(), vec![change("web", "src/Cards.tsx", "export {}")]);
    rig.status(Status::Working);
    assert_eq!(rig.shown(), [paths("api", &["src/client.rs"])]);
}

#[test]
fn since_i_last_reviewed_shows_only_what_changed_after_mark_reviewed() {
    let mut rig = rig(vec![change("api", "src/client.rs", "fn a() {}")]);
    rig.act(A::Open(session()));
    rig.act(A::MarkReviewed);
    rig.act(A::PickBase(DiffBase::SinceLastReviewed));
    assert_eq!(
        rig.store.state().base.get(&session()),
        Some(&DiffBase::SinceLastReviewed)
    );
    assert!(
        rig.shown().iter().all(|(_, files)| files.is_empty()),
        "nothing changed since the review: {:?}",
        rig.shown()
    );

    rig.daemon.set_diffs(
        &session(),
        vec![
            change("api", "src/client.rs", "fn a() {}"),
            change("api", "src/ledger.rs", "fn b() {}"),
        ],
    );
    rig.status(Status::Idle);
    assert_eq!(
        rig.shown().into_iter().filter(|(_, files)| !files.is_empty()).collect::<Vec<_>>(),
        [paths("api", &["src/ledger.rs"])]
    );

    rig.act(A::PickBase(DiffBase::OriginMain));
    assert_eq!(rig.shown(), [paths("api", &["src/client.rs", "src/ledger.rs"])]);
}

#[test]
fn a_failed_first_read_is_shown_and_try_again_reads_again() {
    let mut rig = rig(vec![change("api", "src/client.rs", "fn a() {}")]);
    rig.daemon.fail_next(Operation::Diffs, "git broke");
    rig.act(A::Open(session()));
    assert!(rig.failed());
    assert!(rig.shown().is_empty());
    rig.act(A::Retry);
    assert!(!rig.failed());
    assert_eq!(rig.shown(), [paths("api", &["src/client.rs"])]);
}

#[test]
fn a_failed_refresh_keeps_the_last_diff_and_says_when_it_was_read() {
    let mut rig = rig(vec![change("api", "src/client.rs", "fn a() {}")]);
    rig.act(A::Open(session()));
    rig.daemon
        .set_diffs(&session(), vec![change("web", "src/Cards.tsx", "export {}")]);
    rig.daemon.fail_next(Operation::Diffs, "git broke");
    rig.status(Status::Idle);
    assert!(rig.failed());
    assert_eq!(rig.shown(), [paths("api", &["src/client.rs"])]);
    assert!(
        rig.store
            .state()
            .fetch
            .get(&session())
            .is_some_and(|fetch| fetch.read_at.is_some()),
        "the page knows when the diff it shows was read"
    );
}

#[test]
fn a_failed_mark_reviewed_is_shown() {
    let mut rig = rig(vec![change("api", "src/client.rs", "fn a() {}")]);
    rig.act(A::Open(session()));
    rig.daemon.fail_next(Operation::MarkReviewed, "the store is full");
    rig.act(A::MarkReviewed);
    assert!(rig.failed());
}
