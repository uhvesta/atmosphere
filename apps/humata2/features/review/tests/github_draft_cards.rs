//! PR-9, PR-11: a draft for GitHub on its card in the Diff panel can be
//! edited and dropped, and neither types it into a terminal.
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::comments::{
    Anchor, Author, Call, CommentId, Destination, FakeComments, Listing, Side, Status, Which,
};
use humata2_clients::model::SessionId;
use humata2_review::comments::{Comments, CommentsAction as A, CommentsState, SessionComments};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("review")
}

fn anchor() -> Anchor {
    Anchor {
        repo: "acme/api".to_owned(),
        path: "src/refund.rs".to_owned(),
        side: Side::New,
        first: 140,
        last: 140,
        quoted: Vec::new(),
        base: None,
        detached: false,
        outdated: false,
    }
}

struct Rig {
    store: TestStore<Comments>,
    fake: FakeComments,
    clock: TestClock,
}

fn rig() -> Rig {
    let fake = FakeComments::new();
    fake.set_review_pull_requests(&session(), vec![("acme/api".to_owned(), 482)]);
    let clock = TestClock::new();
    let reducer = Comments::new(Arc::new(fake.clone()), Arc::new(clock.clone()));
    let store = TestStore::new(CommentsState::default(), reducer).non_exhaustive();
    Rig { store, fake, clock }
}

impl Rig {
    fn act(&mut self, action: A) {
        self.store.send(action, |_| {});
        self.pump();
    }

    fn pump(&mut self) {
        for _ in 0..6 {
            self.store.skip_received();
        }
    }

    fn draft(&mut self, text: &str, author: Author) -> CommentId {
        let made = self
            .fake
            .seed_pull_request_draft(&session(), anchor(), text, author, "acme/api", 482);
        let comments = self.fake.comments(&session());
        self.act(A::Listed(session(), Which::Open, Ok(Listing { comments, draft_count: 0 })));
        made.id
    }

    fn held(&self) -> SessionComments {
        self.store.state().sessions.get(&session()).cloned().unwrap_or_default()
    }
}

#[test]
fn editing_a_draft_for_github_opens_its_text_and_saves_the_change() {
    let mut rig = rig();
    let id = rig.draft("round() over-refunds", Author::Agent("claude".to_owned()));
    rig.act(A::EditDraft(session(), id.clone()));
    let held = rig.held();
    assert_eq!(held.editing.as_ref().and_then(|b| b.comment.clone()), Some(id.clone()));
    assert_eq!(held.editing.as_ref().map(|b| b.text.clone()), Some("round() over-refunds".to_owned()));

    rig.act(A::Typed(session(), "Use floor() on each part.".to_owned()));
    rig.clock.advance(Duration::from_millis(400));
    rig.pump();
    let stored = rig.fake.comments(&session());
    assert_eq!(stored[0].text, "Use floor() on each part.");
    assert_eq!(stored[0].author, Author::Agent("claude".to_owned()));
    assert_eq!(
        stored[0].destination,
        Destination::PullRequest { repo: "acme/api".to_owned(), number: 482 }
    );
}

#[test]
fn closing_the_edit_keeps_the_draft_for_github_and_sends_nothing() {
    let mut rig = rig();
    let id = rig.draft("round() over-refunds", Author::User);
    rig.act(A::EditDraft(session(), id));
    rig.act(A::Typed(session(), "changed".to_owned()));
    rig.act(A::Comment(session()));
    rig.clock.advance(Duration::from_millis(400));
    rig.pump();

    let sent = rig.fake.calls().into_iter().filter(|call| matches!(call, Call::Send(..))).count();
    assert_eq!(sent, 0, "a draft for GitHub is never typed into a terminal");
    let stored = rig.fake.comments(&session());
    assert_eq!(stored[0].status, Status::Draft);
    assert_eq!(stored[0].text, "changed");
    assert!(rig.held().editing.is_none());
}

#[test]
fn cancelling_the_edit_gives_the_draft_back_its_text() {
    let mut rig = rig();
    let id = rig.draft("original", Author::User);
    rig.act(A::EditDraft(session(), id));
    rig.act(A::Typed(session(), "changed".to_owned()));
    rig.clock.advance(Duration::from_millis(400));
    rig.pump();
    rig.act(A::Cancel(session()));
    assert_eq!(rig.fake.comments(&session())[0].text, "original");
    assert_eq!(rig.fake.comments(&session()).len(), 1);
}

#[test]
fn dropping_a_draft_for_github_removes_it_from_the_list_and_the_daemon() {
    let mut rig = rig();
    let keep = rig.draft("keep", Author::User);
    let gone = rig.draft("drop me", Author::Agent("claude".to_owned()));
    rig.act(A::Drop(session(), gone.clone()));
    assert!(rig.fake.comments(&session()).iter().all(|c| c.id != gone));
    let held = rig.held();
    assert_eq!(held.comments.len(), 1);
    assert_eq!(held.comments[0].id, keep);
}

#[test]
fn dropping_a_draft_made_from_a_note_removes_the_note_too() {
    let mut rig = rig();
    let note = rig
        .fake
        .seed_note(&session(), anchor(), "Is this intended?", Author::Agent("claude".to_owned()));
    rig.act(A::MakeDraft(session(), note.id.clone(), None));
    rig.act(A::Drop(session(), note.id.clone()));
    assert!(rig.fake.comments(&session()).is_empty());
}

#[test]
fn the_drafts_for_github_are_counted_by_who_wrote_them() {
    let mut rig = rig();
    rig.draft("a", Author::Agent("claude".to_owned()));
    rig.draft("b", Author::Agent("claude".to_owned()));
    rig.draft("c", Author::User);
    // A note and a comment for the agent are not drafts for GitHub.
    rig.fake.seed_note(&session(), anchor(), "n", Author::Agent("claude".to_owned()));
    let comments = rig.fake.comments(&session());
    rig.act(A::Listed(session(), Which::Open, Ok(Listing { comments, draft_count: 0 })));

    let state = rig.store.state();
    let drafts = state.github_drafts(&session());
    assert_eq!(drafts.len(), 3);
    assert_eq!(drafts.iter().filter(|c| matches!(c.author, Author::Agent(_))).count(), 2);
}

#[test]
fn sending_a_note_makes_a_sent_comment_of_the_person_and_removes_the_note() {
    let mut rig = rig();
    let note = rig
        .fake
        .seed_note(&session(), anchor(), "Read it through tx.", Author::Agent("copilot".to_owned()));
    let comments = rig.fake.comments(&session());
    rig.act(A::Listed(session(), Which::Open, Ok(Listing { comments, draft_count: 0 })));
    rig.act(A::SendNote(session(), note.id.clone()));

    let held = rig.fake.comments(&session());
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].author, Author::User);
    assert_eq!(held[0].destination, Destination::Terminal);
    assert_eq!(held[0].text, "Read it through tx.");
    assert_eq!(held[0].status, Status::Sent);
    assert!(rig.held().comments.iter().all(|c| c.id != note.id));
}
