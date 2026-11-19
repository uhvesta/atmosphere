//! D852: the host keeps the looks, so they outlive the app and the daemon.
//! Opening a session reads them, so a file ticks with no Looks good pressed;
//! each change of the looks is saved; a save that fails keeps the tick and
//! says so.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{
    DiffLine, FileDiff, LineKind, LookedFile, RepoDiff, SessionId, Status,
};
use humata2_clients::{Daemon, FakeDaemon, Operation, Workspace};
use humata2_review::looks::{fingerprint, is_looked};
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
        }
    }
    workspace
}

fn file(path: &str, text: &str) -> FileDiff {
    FileDiff {
        path: path.to_owned(),
        added: 1,
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
        ..FileDiff::default()
    }
}

fn diffs() -> Vec<RepoDiff> {
    vec![RepoDiff {
        session: session(),
        repo: "api".to_owned(),
        base: "origin/main".to_owned(),
        files: vec![file("src/a.rs", "one"), file("src/b.rs", "two")],
        comments: Vec::new(),
    }]
}

struct Rig {
    store: TestStore<Review>,
    daemon: FakeDaemon,
}

impl Rig {
    fn act(&mut self, action: A) {
        self.store.send(action, |_| {});
        for _ in 0..8 {
            self.store.skip_received();
        }
    }

    fn looked(&self, path: &str) -> bool {
        let state = self.store.state();
        let shown = state.fetch[&session()].diffs.clone().unwrap_or_default();
        shown
            .iter()
            .flat_map(|diff| &diff.files)
            .filter(|each| each.path == path)
            .any(|each| is_looked(state, &session(), "api", path, each))
    }

    fn saved(&self) -> Vec<LookedFile> {
        let reply = self.daemon.looks(&session());
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime starts")
            .block_on(reply)
            .expect("the looks read")
    }
}

fn rig(daemon: FakeDaemon) -> Rig {
    let clock = Arc::new(TestClock::new());
    daemon.set_diffs(&session(), diffs());
    let mut shown = workspace();
    shown.diffs.clear();
    let state = ReviewState {
        workspace: Arc::new(shown),
        ..ReviewState::default()
    };
    let store =
        TestStore::new(state, Review::new(Arc::new(daemon.clone()), clock)).non_exhaustive();
    Rig { store, daemon }
}

fn daemon() -> FakeDaemon {
    FakeDaemon::new(workspace(), Arc::new(TestClock::new()))
}

#[test]
fn opening_a_session_ticks_the_files_the_host_kept() {
    let daemon = daemon();
    let kept = fingerprint(&file("src/a.rs", "one"));
    let reply = daemon.save_looks(
        &session(),
        vec![LookedFile {
            repo: "api".to_owned(),
            path: "src/a.rs".to_owned(),
            fingerprint: kept,
        }],
    );
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime starts")
        .block_on(reply)
        .expect("saved");
    let mut rig = rig(daemon);
    rig.act(A::Open(session()));
    assert!(rig.looked("src/a.rs"));
    assert!(!rig.looked("src/b.rs"));
}

#[test]
fn a_look_kept_for_a_diff_that_changed_since_is_not_a_tick() {
    let daemon = daemon();
    let reply = daemon.save_looks(
        &session(),
        vec![LookedFile {
            repo: "api".to_owned(),
            path: "src/a.rs".to_owned(),
            fingerprint: fingerprint(&file("src/a.rs", "an older version")),
        }],
    );
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime starts")
        .block_on(reply)
        .expect("saved");
    let mut rig = rig(daemon);
    rig.act(A::Open(session()));
    assert!(!rig.looked("src/a.rs"));
}

#[test]
fn looks_good_a_tick_and_a_clear_are_each_saved() {
    let mut rig = rig(daemon());
    rig.act(A::Open(session()));
    rig.act(A::LooksGood);
    assert_eq!(rig.saved().len(), 2);
    rig.act(A::ToggleLooked {
        repo: "api".to_owned(),
        path: "src/a.rs".to_owned(),
    });
    let paths: Vec<String> = rig.saved().into_iter().map(|look| look.path).collect();
    assert_eq!(paths, ["src/b.rs"]);
    rig.act(A::ClearLooks);
    assert!(rig.saved().is_empty());
}

#[test]
fn a_save_that_fails_keeps_the_tick_and_shows_an_error() {
    let mut rig = rig(daemon());
    rig.act(A::Open(session()));
    rig.daemon.fail_next(Operation::SaveLooks, "the host is unreachable");
    rig.act(A::ToggleLooked {
        repo: "api".to_owned(),
        path: "src/a.rs".to_owned(),
    });
    assert!(rig.looked("src/a.rs"));
    assert!(rig.store.state().fetch[&session()].error.is_some());
}

#[test]
fn a_known_diff_has_a_known_fingerprint_in_every_process() {
    // Saved looks are compared across restarts, so this number must never
    // depend on a per-process random seed.
    assert_eq!(fingerprint(&file("src/a.rs", "one")), 0x2c5a_d53a_5b49_3cff);
    assert_ne!(
        fingerprint(&file("src/a.rs", "one")),
        fingerprint(&file("src/a.rs", "two"))
    );
}
