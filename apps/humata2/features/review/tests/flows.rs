//! The SPEC flows 26.3 and 26.20 through the comments reducer against the
//! fake daemon, following D151: an explicit send types at once and the
//! status comes from the Send answer.
use std::sync::Arc;

use humata2_clients::comments::{
    Anchor, CommentId, FakeComments, Side, Status, Typing,
};
use humata2_clients::model::SessionId;
use humata2_review::comments::{
    Comments, CommentsAction as A, CommentsState, SessionComments,
};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("s1")
}

fn anchor(repo: &str, path: &str, line: u32) -> Anchor {
    Anchor {
        repo: repo.to_owned(),
        path: path.to_owned(),
        side: Side::New,
        first: line,
        last: line,
        quoted: vec![format!("old line {line}")],
        base: None,
        detached: false,
        outdated: false,
    }
}

struct Rig {
    store: TestStore<Comments>,
    fake: FakeComments,
}

fn rig() -> Rig {
    let fake = FakeComments::new();
    let reducer = Comments::new(Arc::new(fake.clone()), Arc::new(TestClock::new()));
    let store = TestStore::new(CommentsState::default(), reducer).non_exhaustive();
    Rig { store, fake }
}

impl Rig {
    fn pump(&mut self) {
        for _ in 0..6 {
            self.store.skip_received();
        }
    }

    fn act(&mut self, action: A) {
        self.store.send(action, |_| {});
        self.pump();
    }

    fn held(&self) -> SessionComments {
        self.store.state().sessions.get(&session()).cloned().unwrap_or_default()
    }

    /// Writes a comment in a box and keeps it as a draft.
    fn draft(&mut self, anchor: Anchor, text: &str) {
        self.act(A::OpenBox(session(), anchor));
        self.act(A::Typed(session(), text.to_owned()));
        self.act(A::Close(session()));
    }

    fn status(&self, id: &str) -> Option<Status> {
        self.fake
            .comments(&session())
            .into_iter()
            .find(|c| c.id == CommentId::new(id))
            .map(|c| c.status)
    }

    fn held_status(&self, id: &str) -> Option<Status> {
        self.held().comments.into_iter().find(|c| c.id == CommentId::new(id)).map(|c| c.status)
    }
}

#[test]
fn flow_26_3_four_comments_across_three_repos_are_one_message() {
    let mut rig = rig();
    rig.fake.set_repos(&session(), &["api", "ledger", "web"]);
    rig.act(A::Open(session()));
    rig.draft(anchor("api", "src/routes.rs", 12), "name this");
    rig.draft(anchor("ledger", "src/post.rs", 40), "check the sign");
    rig.draft(anchor("web", "src/Pay.tsx", 7), "label");
    rig.draft(anchor("api", "src/auth.rs", 3), "why here");
    assert_eq!(rig.held().draft_count, 4);
    assert!(rig.fake.typed(&session()).is_empty());

    rig.act(A::SendAll(session()));

    let typed = rig.fake.typed(&session());
    assert_eq!(typed.len(), 1);
    assert_eq!(typed[0].matches("[comment on ").count(), 4);
    for header in [
        "[comment on api/src/routes.rs:12",
        "[comment on ledger/src/post.rs:40",
        "[comment on web/src/Pay.tsx:7",
        "[comment on api/src/auth.rs:3",
    ] {
        assert!(typed[0].contains(header), "{header} missing from {typed:?}");
    }
    assert!(rig.fake.comments(&session()).iter().all(|c| c.status == Status::Sent));
    assert!(rig.held().comments.iter().all(|c| c.status == Status::Sent));
    assert_eq!(rig.held().draft_count, 0);
}

#[test]
fn flow_26_20_one_comment_is_typed_at_once_then_a_batch_then_addressing() {
    let mut rig = rig();
    rig.act(A::Open(session()));

    // Comment on line 7: typed at once, and Sent because the answer says so.
    rig.act(A::OpenBox(session(), anchor("notes", "rfc-brin.md", 7)));
    rig.act(A::Typed(session(), "say why".to_owned()));
    rig.act(A::Comment(session()));
    let typed = rig.fake.typed(&session());
    assert_eq!(typed.len(), 1);
    assert!(typed[0].starts_with("[comment on rfc-brin.md:7]"), "{typed:?}");
    assert_eq!(rig.held_status("s1/c-1"), Some(Status::Sent));
    assert_eq!(rig.held().draft_count, 0);

    // Two drafts, Send 2: one paste.
    rig.draft(anchor("notes", "rfc-brin.md", 12), "second");
    rig.draft(anchor("notes", "rfc-brin.md", 20), "third");
    rig.draft(anchor("notes", "rfc-brin.md", 30), "fourth, stays a draft");
    assert_eq!(rig.held().draft_count, 3);
    // Send 2: the fourth stays behind, so send the first two by hand first.
    rig.act(A::Archive(session(), CommentId::new("s1/c-4")));
    rig.act(A::SendAll(session()));
    let typed = rig.fake.typed(&session());
    assert_eq!(typed.len(), 2);
    assert_eq!(typed[1].matches("[comment on ").count(), 2);
    rig.act(A::Unarchive(session(), CommentId::new("s1/c-4")));
    assert_eq!(rig.held().draft_count, 1);

    // The agent changes one comment's lines; another passage is gone.
    let _ = rig.fake.lines_changed(&CommentId::new("s1/c-2"), &["new line 12"]);
    let _ = rig.fake.lines_changed(&CommentId::new("s1/c-4"), &[]);
    rig.pump();
    let held = rig.held();
    let addressed = held.comments.iter().find(|c| c.id == CommentId::new("s1/c-2"));
    assert_eq!(addressed.map(|c| c.status), Some(Status::Addressed));
    assert_eq!(addressed.and_then(|c| c.addressed.clone()), Some(vec!["new line 12".to_owned()]));
    assert_eq!(addressed.map(|c| c.anchor.quoted.clone()), Some(vec!["old line 12".to_owned()]));
    let gone = held.comments.iter().find(|c| c.id == CommentId::new("s1/c-4"));
    assert_eq!(gone.map(|c| c.anchor.detached), Some(true));

    // Resolve the addressed one; it stays listed, as done (D855).
    rig.act(A::Resolve(session(), CommentId::new("s1/c-2")));
    assert_eq!(rig.status("s1/c-2"), Some(Status::Resolved));
    assert_eq!(rig.held_status("s1/c-2"), Some(Status::Resolved));

    // Re-attach the detached one and send it again.
    rig.act(A::Reattach(session(), CommentId::new("s1/c-4"), anchor("notes", "rfc-brin.md", 35)));
    rig.act(A::SendAll(session()));
    let typed = rig.fake.typed(&session());
    assert_eq!(typed.len(), 3);
    assert!(typed[2].starts_with("[comment on rfc-brin.md:35]"), "{typed:?}");
    assert_eq!(rig.held_status("s1/c-4"), Some(Status::Sent));
}

#[test]
fn flow_26_20_with_no_terminal_the_comment_waits_and_take_back_makes_it_a_draft() {
    let mut rig = rig();
    rig.fake.set_typing(&session(), Typing::NotTyped);
    rig.act(A::Open(session()));
    rig.act(A::OpenBox(session(), anchor("notes", "rfc-brin.md", 7)));
    rig.act(A::Typed(session(), "say why".to_owned()));
    rig.act(A::Comment(session()));
    assert!(rig.fake.typed(&session()).is_empty());
    assert_eq!(rig.held_status("s1/c-1"), Some(Status::Waiting));
    assert_eq!(rig.held().draft_count, 0);

    rig.act(A::TakeBack(session(), CommentId::new("s1/c-1")));
    assert_eq!(rig.held_status("s1/c-1"), Some(Status::Draft));
    assert_eq!(rig.held().draft_count, 1);

    // Back at the terminal, sending it types it.
    rig.fake.set_typing(&session(), Typing::Typed);
    rig.act(A::SendAll(session()));
    assert_eq!(rig.fake.typed(&session()).len(), 1);
    assert_eq!(rig.held_status("s1/c-1"), Some(Status::Sent));
}
