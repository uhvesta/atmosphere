//! RV-31, VIEWS P1/P3: each repository of a session has its own base menu.
//! One menu is open at a time; it reads the repository's branches as the
//! clone last fetched them, filters them, and a pick gives that repository
//! its own base and closes the menu. The Diff header says the repositories
//! are compared per repository only when their bases really differ.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{
    DiffBase, FileDiff, RepoBase, RepoDiff, RepoDiffSpec, SessionId, Status,
};
use humata2_clients::{FakeDaemon, Operation, Workspace};
use humata2_review::{PushState, Review, ReviewAction as A, ReviewState};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("ledger")
}

/// One changed file in `repo`.
fn changed(repo: &str) -> RepoDiff {
    RepoDiff {
        session: session(),
        repo: repo.to_owned(),
        base: "main".to_owned(),
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

/// The design workspace, with the session idle in acme/api and changes in
/// acme/api and acme/web. The fake's clone of acme/api knows main and three
/// more branches; acme/web knows main alone.
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

struct Rig {
    store: TestStore<Review>,
    daemon: FakeDaemon,
}

/// The review page with the session open and its first read done.
fn rig() -> Rig {
    let clock = Arc::new(TestClock::new());
    let daemon = FakeDaemon::new(workspace(), clock.clone());
    let state = ReviewState {
        workspace: Arc::new(workspace()),
        ..ReviewState::default()
    };
    let store =
        TestStore::new(state, Review::new(Arc::new(daemon.clone()), clock)).non_exhaustive();
    let mut rig = Rig { store, daemon };
    rig.act(A::Open(session()));
    rig
}

impl Rig {
    fn act(&mut self, action: A) {
        self.store.send(action, |_| {});
        for _ in 0..6 {
            self.store.skip_received();
        }
    }

    fn open_repo(&self) -> Option<String> {
        self.store
            .state()
            .repo_picker
            .as_ref()
            .map(|picker| picker.repo.clone())
    }

    fn branches(&self) -> Vec<String> {
        self.store
            .state()
            .repo_picker
            .as_ref()
            .map(|picker| picker.branches().into_iter().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

fn toggle(repo: &str) -> A {
    A::ToggleRepoBase {
        repo: repo.to_owned(),
    }
}

#[test]
fn opening_a_repository_menu_reads_its_branches_and_closes_the_header_menu() {
    let mut rig = rig();
    rig.act(A::ToggleBaseMenu);
    assert!(rig.store.state().base_menu);

    rig.act(toggle("acme/api"));
    assert!(!rig.store.state().base_menu, "one menu at a time");
    assert_eq!(rig.open_repo().as_deref(), Some("acme/api"));
    let branches = rig.branches();
    assert!(branches.len() > 1, "the clone's branches: {branches:?}");
    assert!(branches.iter().any(|branch| branch == "main"));
    assert!(
        rig.daemon
            .calls()
            .iter()
            .any(|call| call == "bases ledger acme/api")
    );

    rig.act(A::ToggleBaseMenu);
    assert_eq!(rig.open_repo(), None, "the header menu closes it");
}

#[test]
fn opening_another_repository_menu_closes_the_first_and_toggling_closes_it() {
    let mut rig = rig();
    rig.act(toggle("acme/api"));
    rig.act(toggle("acme/web"));
    assert_eq!(rig.open_repo().as_deref(), Some("acme/web"));
    assert_eq!(rig.branches(), vec!["main".to_owned()]);
    rig.act(toggle("acme/web"));
    assert_eq!(rig.open_repo(), None);
}

#[test]
fn the_filter_keeps_matching_branches_and_a_new_menu_starts_unfiltered() {
    let mut rig = rig();
    rig.act(toggle("acme/api"));
    let all = rig.branches();
    rig.act(A::FilterRepoBases("RELEASE".to_owned()));
    let kept = rig.branches();
    assert!(!kept.is_empty() && kept.len() < all.len(), "{kept:?} of {all:?}");
    assert!(kept.iter().all(|branch| branch.contains("release")));

    rig.act(A::CloseRepoBase);
    rig.act(toggle("acme/api"));
    assert_eq!(rig.branches(), all);
}

#[test]
fn a_failed_branch_read_shows_in_the_menu_and_reading_again_fills_it() {
    let mut rig = rig();
    rig.daemon
        .fail_next(Operation::Bases, "The daemon closed the connection.");
    rig.act(toggle("acme/api"));
    let failed = rig
        .store
        .state()
        .repo_picker
        .as_ref()
        .and_then(|picker| picker.bases.clone());
    assert!(matches!(failed, Some(Err(_))), "{failed:?}");

    rig.act(A::ReloadRepoBases);
    assert!(rig.branches().len() > 1);
}

#[test]
fn picking_a_branch_gives_only_that_repository_its_base_and_closes_the_menu() {
    let mut rig = rig();
    rig.act(toggle("acme/api"));
    rig.act(A::PickRepoBase {
        session: session(),
        repo: "acme/api".to_owned(),
        base: RepoBase::Branch("release/2.4".to_owned()),
    });
    assert_eq!(rig.open_repo(), None);
    let targets = rig.store.state().targets_of(&session());
    assert_eq!(targets.of("acme/api"), RepoBase::Branch("release/2.4".to_owned()));
    assert_eq!(targets.of("acme/web"), RepoBase::DefaultBranch);
    assert!(rig.store.state().compares_per_repo(&session()));
    let asked = rig.daemon.calls();
    let expected = format!(
        "diffs ledger {:?}",
        DiffBase::Spec(vec![
            RepoDiffSpec {
                repo: "acme/api".to_owned(),
                base: RepoBase::Branch("release/2.4".to_owned()),
                options: Default::default(),
            },
            RepoDiffSpec {
                repo: "acme/web".to_owned(),
                base: RepoBase::DefaultBranch,
                options: Default::default(),
            },
        ])
    );
    assert!(asked.contains(&expected), "{asked:?}");
}

#[test]
fn the_header_compares_per_repo_only_when_the_bases_differ() {
    let mut rig = rig();
    assert!(!rig.store.state().compares_per_repo(&session()));
    // Both repositories on the same base, one through an override: not per
    // repository.
    rig.act(A::PickBase(DiffBase::UncommittedOnly));
    rig.act(A::PickRepoBase {
        session: session(),
        repo: "acme/api".to_owned(),
        base: RepoBase::Uncommitted,
    });
    assert!(!rig.store.state().compares_per_repo(&session()));

    rig.act(A::PickRepoBase {
        session: session(),
        repo: "acme/web".to_owned(),
        base: RepoBase::Rev("v1.2".to_owned()),
    });
    assert!(rig.store.state().compares_per_repo(&session()));

    rig.act(A::PickBase(DiffBase::OriginMain));
    assert!(!rig.store.state().compares_per_repo(&session()));
}

#[test]
fn use_for_every_repo_gives_all_repositories_that_base_and_closes_the_menu() {
    let mut rig = rig();
    rig.act(A::PickRepoBase {
        session: session(),
        repo: "acme/api".to_owned(),
        base: RepoBase::Uncommitted,
    });
    assert!(rig.store.state().compares_per_repo(&session()));
    rig.act(toggle("acme/api"));
    rig.act(A::UseRepoBaseForEvery {
        session: session(),
        repo: "acme/api".to_owned(),
    });
    assert_eq!(rig.open_repo(), None);
    let targets = rig.store.state().targets_of(&session());
    assert_eq!(targets.of("acme/api"), RepoBase::Uncommitted);
    assert_eq!(targets.of("acme/web"), RepoBase::Uncommitted);
    assert!(!rig.store.state().compares_per_repo(&session()));
}

#[test]
fn pushing_a_repository_on_its_own_branch_asks_for_that_base() {
    let mut rig = rig();
    rig.act(A::PickRepoBase {
        session: session(),
        repo: "acme/api".to_owned(),
        base: RepoBase::Branch("origin/release/2.4".to_owned()),
    });
    rig.act(A::Push {
        repo: "acme/api".to_owned(),
    });
    let pushes = rig.daemon.pushes();
    let choice = pushes
        .iter()
        .find(|(_, choice)| choice.repo == "acme/api")
        .map(|(_, choice)| choice.clone());
    assert_eq!(
        choice.map(|choice| choice.base_branch),
        Some("release/2.4".to_owned())
    );
}

#[test]
fn a_pull_request_left_on_another_base_says_so_on_its_repository() {
    let mut rig = rig();
    rig.daemon.set_open_pull_request(&session(), "acme/api", 3);
    rig.act(A::PickRepoBase {
        session: session(),
        repo: "acme/api".to_owned(),
        base: RepoBase::Branch("release/2.4".to_owned()),
    });
    rig.act(A::Push {
        repo: "acme/api".to_owned(),
    });
    let state = rig
        .store
        .state()
        .pushes
        .get(&(session(), "acme/api".to_owned()))
        .cloned();
    match state {
        Some(PushState::Pushed {
            pull_request: Some(3),
            note,
        }) => assert!(!note.is_empty(), "the push says the PR wasn't retargeted"),
        other => panic!("expected PR #3 with a note, got {other:?}"),
    }
}
