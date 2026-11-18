//! D5: each session's picked diff bases survive a restart. A repository's
//! own base is saved with the review page's state, a restored page reads it
//! back, and the next read of the session asks the daemon for it.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{
    DiffBase, DiffTargets, FileDiff, RepoBase, RepoDiff, RepoDiffSpec, SessionId, Status,
};
use humata2_clients::{FakeDaemon, Workspace};
use humata2_review::{Review, ReviewAction as A, ReviewSaved, ReviewState};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("ledger")
}

/// One changed file in `repo`.
fn changed(repo: &str) -> RepoDiff {
    RepoDiff {
        session: session(),
        repo: repo.to_owned(),
        base: "origin/main".to_owned(),
        files: vec![FileDiff {
            path: "src/lib.rs".to_owned(),
            added: 1,
            removed: 0,
            lines: Vec::new(),
            ..FileDiff::default()
        }],
        comments: Vec::new(),
    }
}

/// The design workspace, with the session working in acme/api and changes
/// in acme/api and acme/web.
fn workspace() -> Workspace {
    let mut workspace = fixture::design();
    for each in &mut workspace.sessions {
        if each.id == session() {
            each.status = Status::Idle;
            each.repo = "acme/api".to_owned();
        }
    }
    workspace.diffs = vec![changed("acme/api"), changed("acme/web")];
    workspace
}

/// The review page as the app holds it on launch, before any pick.
fn fresh() -> ReviewState {
    ReviewState {
        workspace: Arc::new(workspace()),
        ..ReviewState::default()
    }
}

/// The review page with `state`, on a daemon that knows the same workspace.
fn page(state: ReviewState) -> TestStore<Review> {
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(workspace(), clock.clone());
    TestStore::new(state, Review::new(Arc::new(daemon), clock)).non_exhaustive()
}

/// Receives the next read of the session, which must be against `base`.
fn read_of(store: &mut TestStore<Review>, base: DiffBase) {
    store.receive_matching(
        "a read of the session's changes",
        move |action| matches!(action, A::Fetched { base: read, .. } if *read == base),
        |_| {},
    );
}

/// acme/api against its uncommitted changes, acme/web against its default
/// branch.
fn api_uncommitted() -> DiffBase {
    DiffBase::Spec(vec![
        RepoDiffSpec {
            repo: "acme/api".to_owned(),
            base: RepoBase::Uncommitted,
        },
        RepoDiffSpec {
            repo: "acme/web".to_owned(),
            base: RepoBase::DefaultBranch,
        },
    ])
}

#[test]
fn a_repository_pick_is_saved_and_a_restored_page_reads_it_back() {
    let mut store = page(fresh());
    store.send(
        A::PickRepoBase {
            session: session(),
            repo: "acme/api".to_owned(),
            base: RepoBase::Uncommitted,
        },
        |_| {},
    );
    read_of(&mut store, api_uncommitted());

    let saved = store.state().saved.clone();
    assert_eq!(
        saved
            .targets
            .get(&session())
            .and_then(|targets| targets.repos.get("acme/api")),
        Some(&RepoBase::Uncommitted)
    );
    let json = serde_json::to_string(&saved).expect("a saved review page turns into JSON");
    let restored: ReviewSaved = serde_json::from_str(&json).expect("the saved JSON reads back");

    let mut store = page(fresh());
    store.send(A::Restore(restored), |_| {});
    store.send(A::Open(session()), |_| {});
    read_of(&mut store, api_uncommitted());
}

#[test]
fn a_saved_file_from_before_targets_loads_with_the_default_targets() {
    let before = r#"{"selected":{},"folded":{}}"#;
    let saved: ReviewSaved =
        serde_json::from_str(before).expect("a file saved before targets reads");
    let mut store = page(fresh());
    store.send(A::Restore(saved), |_| {});
    assert_eq!(store.state().targets_of(&session()), DiffTargets::default());
    store.send(A::Open(session()), |_| {});
    read_of(&mut store, DiffBase::OriginMain);
}

#[test]
fn picking_one_base_for_every_repository_clears_the_repository_pick() {
    let mut store = page(fresh());
    store.send(A::Open(session()), |_| {});
    store.skip_received();
    store.send(
        A::PickRepoBase {
            session: session(),
            repo: "acme/api".to_owned(),
            base: RepoBase::Uncommitted,
        },
        |_| {},
    );
    read_of(&mut store, api_uncommitted());

    store.send(A::PickBase(DiffBase::UncommittedOnly), |_| {});
    read_of(&mut store, DiffBase::UncommittedOnly);
    assert!(!store.state().targets_of(&session()).per_repo());
}
