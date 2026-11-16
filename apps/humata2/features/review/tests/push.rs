//! RV-2, flow 26.3 step 8: each repo group pushes its branch and opens its
//! pull request on its own; the other repos stay as they are.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::{SessionId, Status};
use humata2_clients::{FakeDaemon, Operation};
use humata2_review::{PushState, Review, ReviewAction as A, ReviewState};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("ledger")
}

struct Rig {
    store: TestStore<Review>,
    daemon: FakeDaemon,
}

fn rig() -> Rig {
    let clock = Arc::new(TestClock::new());
    let mut workspace = fixture::design();
    for each in &mut workspace.sessions {
        if each.id == session() {
            each.status = Status::Idle;
        }
    }
    let daemon = FakeDaemon::new(workspace.clone(), clock.clone());
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

    fn push_state(&self, repo: &str) -> Option<PushState> {
        self.store
            .state()
            .pushes
            .get(&(session(), repo.to_owned()))
            .cloned()
    }
}

#[test]
fn pushing_one_repo_pushes_it_and_opens_its_pull_request_only() {
    let mut rig = rig();
    rig.act(A::Open(session()));
    rig.act(A::Push { repo: "api".into() });
    let pushes = rig.daemon.pushes();
    assert_eq!(pushes.len(), 1, "one push, for api only");
    let (pushed_session, choice) = &pushes[0];
    assert_eq!(pushed_session, &session());
    assert_eq!(choice.repo, "api");
    assert!(choice.open_pull_request);
    assert_eq!(choice.title, "Ledger schema migration");
    assert!(matches!(
        rig.push_state("api"),
        Some(PushState::Pushed { pull_request: Some(_) })
    ));
    assert_eq!(rig.push_state("ledger"), None, "the other repo is left alone");
}

#[test]
fn a_repo_that_already_has_a_pull_request_is_only_pushed() {
    let mut rig = rig();
    rig.daemon.set_open_pull_request(&session(), "api", 3);
    rig.act(A::Open(session()));
    rig.act(A::Push { repo: "api".into() });
    let pushes = rig.daemon.pushes();
    assert_eq!(pushes.len(), 1);
    assert!(!pushes[0].1.open_pull_request);
    assert_eq!(
        rig.push_state("api"),
        Some(PushState::Pushed { pull_request: Some(3) })
    );
}

#[test]
fn a_failed_push_says_why_and_can_be_tried_again() {
    let mut rig = rig();
    rig.act(A::Open(session()));
    rig.daemon.fail_next(Operation::Push, "gh is signed out");
    rig.act(A::Push { repo: "api".into() });
    assert!(matches!(rig.push_state("api"), Some(PushState::Failed(_))));
    rig.act(A::Push { repo: "api".into() });
    assert!(matches!(
        rig.push_state("api"),
        Some(PushState::Pushed { .. })
    ));
}

#[test]
fn a_failed_plan_is_shown_on_that_repo() {
    let mut rig = rig();
    rig.act(A::Open(session()));
    rig.daemon.fail_next(Operation::PlanPush, "the clone is missing");
    rig.act(A::Push { repo: "api".into() });
    assert!(matches!(rig.push_state("api"), Some(PushState::Failed(_))));
    assert!(rig.daemon.pushes().is_empty());
}
