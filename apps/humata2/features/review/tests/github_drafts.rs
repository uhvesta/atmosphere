//! PR-11 and PR-23: in a pull request review the comment box also makes a
//! draft for GitHub, and a note from the review chat becomes one. Neither is
//! typed into a terminal.
use std::sync::Arc;

use humata2_clients::comments::{
    Anchor, Author, CommentId, Destination, FakeComments, Side, Status,
};
use humata2_clients::fixture;
use humata2_clients::model::{PullRequest, SessionId};
use humata2_review::comments::{
    Comments, CommentsAction as A, CommentsState, SessionComments, github_target,
};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("ledger")
}

fn anchor() -> Anchor {
    Anchor {
        repo: "acme/api".to_owned(),
        path: "src/refund.rs".to_owned(),
        side: Side::New,
        first: 12,
        last: 12,
        quoted: vec!["let total = round(amount);".to_owned()],
        base: None,
        detached: false,
        outdated: false,
    }
}

struct Rig {
    store: TestStore<Comments>,
    fake: FakeComments,
}

fn rig(reviewing: bool) -> Rig {
    let fake = FakeComments::new();
    if reviewing {
        fake.set_review_pull_requests(&session(), vec![("acme/api".to_owned(), 482)]);
    }
    let clock = TestClock::new();
    let reducer = Comments::new(Arc::new(fake.clone()), Arc::new(clock));
    let store = TestStore::new(CommentsState::default(), reducer).non_exhaustive();
    Rig { store, fake }
}

impl Rig {
    fn act(&mut self, action: A) {
        self.store.send(action, |_| {});
        for _ in 0..6 {
            self.store.skip_received();
        }
    }

    fn held(&self) -> SessionComments {
        self.store.state().sessions.get(&session()).cloned().unwrap_or_default()
    }

    fn write(&mut self, text: &str) {
        self.act(A::OpenBox(session(), anchor()));
        self.act(A::Typed(session(), text.to_owned()));
    }
}

fn workspace(reviewing: bool) -> humata2_clients::Workspace {
    let mut workspace = fixture::design();
    for each in &mut workspace.sessions {
        if each.id == session() {
            each.pull_request_review = reviewing;
        }
    }
    workspace.pulls.push(PullRequest {
        number: 482,
        title: "Round refunds".to_owned(),
        repo: "acme/api".to_owned(),
        session: Some(session()),
        checks: String::new(),
    });
    workspace
}

#[test]
fn draft_for_github_makes_a_user_draft_for_the_pull_request_and_closes_the_box() {
    let mut rig = rig(true);
    rig.write("Why round here?");
    rig.act(A::DraftForGitHub(session(), "acme/api".to_owned(), 482));

    let made = rig.fake.comments(&session());
    assert_eq!(made.len(), 1, "the box's own comment for the agent is gone: {made:?}");
    assert_eq!(made[0].author, Author::User);
    assert_eq!(made[0].status, Status::Draft);
    assert_eq!(made[0].text, "Why round here?");
    assert_eq!(
        made[0].destination,
        Destination::PullRequest { repo: "acme/api".to_owned(), number: 482 }
    );
    let held = rig.held();
    assert!(held.editing.is_none());
    assert_eq!(held.comments.len(), 1);
    assert_eq!(held.draft_count, 0, "a pull request draft is not typed, so it isn't counted");
}

#[test]
fn a_refused_draft_keeps_the_box_and_its_text() {
    // This session reviews no pull request, so the daemon refuses.
    let mut rig = rig(false);
    rig.write("Why round here?");
    rig.act(A::DraftForGitHub(session(), "acme/api".to_owned(), 482));

    let held = rig.held();
    assert!(held.error.is_some());
    assert_eq!(held.editing.map(|b| b.text), Some("Why round here?".to_owned()));
    assert!(
        rig.fake
            .comments(&session())
            .iter()
            .all(|c| !matches!(c.destination, Destination::PullRequest { .. }))
    );
}

#[test]
fn a_blank_box_makes_no_draft_for_github() {
    let mut rig = rig(true);
    rig.act(A::OpenBox(session(), anchor()));
    rig.act(A::DraftForGitHub(session(), "acme/api".to_owned(), 482));
    assert!(rig.fake.comments(&session()).is_empty());
}

#[test]
fn only_a_pull_request_review_offers_draft_for_github() {
    let review = workspace(true);
    assert_eq!(github_target(&review, &session(), "acme/api"), Some(("acme/api".to_owned(), 482)));
    assert_eq!(github_target(&review, &session(), "api"), Some(("acme/api".to_owned(), 482)));

    let plain = workspace(false);
    assert_eq!(github_target(&plain, &session(), "acme/api"), None);
}

#[test]
fn make_it_a_draft_turns_the_agents_note_into_a_pull_request_draft_in_place() {
    let mut rig = rig(true);
    let note = rig
        .fake
        .seed_note(&session(), anchor(), "Is the rounding intended?", Author::Agent("claude".to_owned()));
    rig.act(A::MakeDraft(session(), note.id.clone(), None));

    let made = rig.fake.comments(&session());
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].id, note.id);
    assert_eq!(made[0].author, Author::Agent("claude".to_owned()));
    assert_eq!(
        made[0].destination,
        Destination::PullRequest { repo: "acme/api".to_owned(), number: 482 }
    );
    let held = rig.held();
    assert_eq!(held.comments.len(), 1);
    assert_eq!(held.comments[0].destination, made[0].destination);
}

#[test]
fn make_it_a_draft_for_a_pull_request_the_review_lacks_is_refused_and_shown() {
    let mut rig = rig(true);
    let note = rig.fake.seed_note(&session(), anchor(), "Note", Author::Agent("claude".to_owned()));
    rig.act(A::MakeDraft(session(), note.id.clone(), Some(("acme/web".to_owned(), 9))));
    assert_eq!(rig.fake.comments(&session())[0].destination, Destination::Note);
    assert!(rig.held().error.is_some());
}

#[test]
fn make_it_a_draft_for_a_missing_note_says_so() {
    let mut rig = rig(true);
    rig.act(A::MakeDraft(session(), CommentId::new("ledger/c-99"), None));
    assert!(rig.held().error.is_some());
}
