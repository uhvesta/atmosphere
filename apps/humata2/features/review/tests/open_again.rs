//! DS-76: opening the Diff panel again, closing it or going full screen
//! does not read the session's diff again while the one held is fresh. A
//! base pick, Try again, the end of a turn or an older diff still read it.
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::fixture;
use humata2_clients::model::{DiffBase, DiffLine, FileDiff, LineKind, RepoDiff, SessionId};
use humata2_clients::{FakeDaemon, Operation};
use humata2_review::{Review, ReviewAction as A, ReviewState};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("ledger")
}

fn change() -> RepoDiff {
    RepoDiff {
        session: session(),
        repo: "api".to_owned(),
        base: "origin/main".to_owned(),
        files: vec![FileDiff {
            path: "src/client.rs".to_owned(),
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
    clock: Arc<TestClock>,
}

fn rig() -> Rig {
    let clock = Arc::new(TestClock::new());
    let workspace = fixture::design();
    let daemon = FakeDaemon::new(workspace.clone(), clock.clone());
    daemon.set_diffs(&session(), vec![change()]);
    let state = ReviewState {
        workspace: Arc::new(workspace),
        ..ReviewState::default()
    };
    let store =
        TestStore::new(state, Review::new(Arc::new(daemon.clone()), clock.clone())).non_exhaustive();
    Rig { store, daemon, clock }
}

impl Rig {
    fn act(&mut self, action: A) {
        self.store.send(action, |_| {});
        for _ in 0..6 {
            self.store.skip_received();
        }
    }

    /// How many times the session's diff was read.
    fn reads(&self) -> usize {
        self.daemon
            .calls()
            .iter()
            .filter(|call| call.starts_with("diffs "))
            .count()
    }
}

#[test]
fn opening_again_with_the_diff_held_reads_nothing() {
    let mut rig = rig();
    rig.act(A::Open(session()));
    assert_eq!(rig.reads(), 1);
    rig.clock.advance(Duration::from_secs(5));
    rig.act(A::Open(session()));
    rig.act(A::Open(session()));
    assert_eq!(rig.reads(), 1, "the held diff is kept");
}

#[test]
fn try_again_and_a_base_pick_still_read() {
    let mut rig = rig();
    rig.act(A::Open(session()));
    rig.act(A::Retry);
    assert_eq!(rig.reads(), 2);
    rig.act(A::PickBase(DiffBase::UncommittedOnly));
    assert_eq!(rig.reads(), 3);
}

#[test]
fn a_failed_read_or_an_old_diff_is_read_on_open() {
    let mut rig = rig();
    rig.daemon.fail_next(Operation::Diffs, "git broke");
    rig.act(A::Open(session()));
    rig.act(A::Open(session()));
    assert_eq!(rig.reads(), 2, "a failed read is tried again");
    rig.clock.advance(Duration::from_secs(120));
    rig.act(A::Open(session()));
    assert_eq!(rig.reads(), 3, "a diff read two minutes ago is read again");
}
