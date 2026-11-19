//! The comments reducer against a scripted fake daemon: opening a session,
//! writing and saving drafts, sending, resolving, archiving and handing
//! over drafts saved by older builds.
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::DaemonError;
use humata2_clients::comments::{
    Anchor, Call, CommentEvent, CommentId, CommentsClient, FakeComments, Listing, Side, Status,
    Typing, Which,
};
use humata2_clients::model::{Comment as LegacyComment, CommentContext, SessionId};
use humata2_review::comments::{
    Comments, CommentsAction as A, CommentsDelegate as D, CommentsState, HandOver, SessionComments,
};
use tca::{TestClock, TestStore};

fn session() -> SessionId {
    SessionId::new("s1")
}

fn anchor(path: &str, line: u32) -> Anchor {
    Anchor {
        repo: "notes".to_owned(),
        path: path.to_owned(),
        side: Side::New,
        first: line,
        last: line,
        quoted: vec![format!("line {line}")],
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
    let clock = TestClock::new();
    let reducer = Comments::new(Arc::new(fake.clone()), Arc::new(clock.clone()));
    let store = TestStore::new(CommentsState::default(), reducer).non_exhaustive();
    Rig { store, fake, clock }
}

impl Rig {
    /// Reduces whatever effects have sent so far, and what that sends.
    fn pump(&mut self) {
        for _ in 0..6 {
            self.store.skip_received();
        }
    }

    fn act(&mut self, action: A) {
        self.store.send(action, |_| {});
        self.pump();
    }

    /// A draft the daemon already has.
    fn seed(&self, path: &str, line: u32, text: &str) -> CommentId {
        let _ = self.fake.create(&session(), anchor(path, line), text, &format!("seed-{path}-{line}-{text}"));
        self.fake
            .comments(&session())
            .last()
            .map(|c| c.id.clone())
            .unwrap_or_else(|| CommentId::new("none"))
    }

    /// What a list answers, without following changes.
    fn load(&mut self) {
        let comments: Vec<_> = self
            .fake
            .comments(&session())
            .into_iter()
            .filter(|c| !c.archived)
            .collect();
        let draft_count = u32::try_from(comments.iter().filter(|c| c.counts_as_draft()).count()).unwrap_or(0);
        self.act(A::Listed(session(), Which::WithResolved, Ok(Listing { comments, draft_count })));
    }

    fn held(&self) -> SessionComments {
        self.store.state().sessions.get(&session()).cloned().unwrap_or_default()
    }

    fn advance(&mut self, millis: u64) {
        self.clock.advance(Duration::from_millis(millis));
        self.pump();
    }

    fn count(&self, wanted: fn(&Call) -> bool) -> usize {
        self.fake.calls().iter().filter(|c| wanted(c)).count()
    }

    fn type_text(&mut self, text: &str) {
        self.act(A::Typed(session(), text.to_owned()));
    }
}

fn is_edit(call: &Call) -> bool {
    matches!(call, Call::Edit(..))
}
fn is_create(call: &Call) -> bool {
    matches!(call, Call::Create(..))
}
fn is_list(call: &Call) -> bool {
    matches!(call, Call::List(..))
}

fn toasted(rig: &mut Rig) {
    rig.store
        .receive_matching("a toast", |a| matches!(a, A::Delegate(D::Toast(_))), |_| {});
}

#[test]
fn opening_a_session_lists_and_follows_its_comments() {
    let mut rig = rig();
    let first = rig.seed("a.md", 3, "one");
    rig.act(A::Open(session()));
    assert!(rig.fake.calls().contains(&Call::Watch(session())));
    assert!(rig.fake.calls().contains(&Call::List(session(), Which::WithResolved)));
    let held = rig.held();
    assert_eq!(held.comments.len(), 1);
    assert_eq!(held.draft_count, 1);

    // Another window's comment arrives over the watch, in made order.
    let second = rig.seed("b.md", 4, "two");
    rig.pump();
    let ids: Vec<_> = rig.held().comments.iter().map(|c| c.id.clone()).collect();
    assert_eq!(ids, vec![first.clone(), second.clone()]);
    assert_eq!(rig.held().draft_count, 2);

    // The daemon asks for a fresh list, and a drop elsewhere shows.
    let _ = rig.fake.drop_events();
    rig.pump();
    assert_eq!(rig.count(is_list), 2);
    let _ = rig.fake.drop_comment(&first);
    rig.pump();
    assert_eq!(rig.held().comments.len(), 1);
    assert_eq!(rig.held().draft_count, 1);
}

#[test]
fn an_older_revision_does_not_replace_a_newer_one() {
    let mut rig = rig();
    rig.seed("a.md", 3, "one");
    rig.load();
    let mut newer = rig.held().comments[0].clone();
    newer.revision = 5;
    newer.text = "newer".to_owned();
    let mut older = newer.clone();
    older.revision = 2;
    older.text = "older".to_owned();
    rig.act(A::Event(session(), CommentEvent::Changed(newer)));
    rig.act(A::Event(session(), CommentEvent::Changed(older)));
    assert_eq!(rig.held().comments[0].text, "newer");
}

#[test]
fn opening_a_box_on_the_same_file_stretches_the_open_one_and_another_file_waits() {
    let mut rig = rig();
    rig.act(A::OpenBox(session(), anchor("a.md", 3)));
    let mut range = anchor("a.md", 3);
    range.last = 5;
    rig.act(A::OpenBox(session(), range));
    let lines = |rig: &Rig| rig.held().editing.map(|b| (b.anchor.path.clone(), b.anchor.first, b.anchor.last));
    assert_eq!(lines(&rig), Some(("a.md".to_owned(), 3, 5)));
    rig.act(A::OpenBox(session(), anchor("b.md", 1)));
    assert_eq!(lines(&rig), Some(("a.md".to_owned(), 3, 5)));
}

#[test]
fn another_line_keeps_a_written_box_as_a_draft_and_opens_an_empty_one_there() {
    let mut rig = rig();
    rig.act(A::OpenBox(session(), anchor("a.md", 2)));
    rig.store.send(A::Typed(session(), "Name this error.".to_owned()), |_| {});
    // Clicked while the first text's save is still in flight.
    rig.store.send(A::OpenBox(session(), anchor("a.md", 4)), |_| {});
    rig.pump();
    let held = rig.held();
    let open = held.editing.as_ref().map(|b| (b.anchor.first, b.anchor.last, b.text.clone(), b.comment.clone()));
    assert_eq!(open, Some((4, 4, String::new(), None)));
    let kept: Vec<_> = rig.fake.comments(&session()).into_iter().map(|c| (c.anchor.first, c.text, c.status)).collect();
    assert_eq!(kept, [(2, "Name this error.".to_owned(), Status::Draft)]);
}

#[test]
fn the_first_text_makes_the_draft_and_blank_text_makes_nothing() {
    let mut rig = rig();
    rig.act(A::OpenBox(session(), anchor("a.md", 3)));
    rig.type_text("   ");
    assert_eq!(rig.count(is_create), 0);
    rig.type_text("fix this");
    assert_eq!(rig.count(is_create), 1);
    let held = rig.held();
    assert_eq!(held.editing.as_ref().and_then(|b| b.comment.clone()), Some(CommentId::new("s1/c-1")));
    assert_eq!(held.comments.len(), 1);
    assert_eq!(rig.fake.comments(&session())[0].status, Status::Draft);
}

#[test]
fn later_text_is_saved_once_after_the_pause() {
    let mut rig = rig();
    rig.act(A::OpenBox(session(), anchor("a.md", 3)));
    rig.type_text("a");
    // Typed while the create is still in flight, and again before the pause.
    rig.store.send(A::Typed(session(), "ab".to_owned()), |_| {});
    rig.pump();
    rig.type_text("abc");
    assert_eq!(rig.count(is_create), 1);
    assert_eq!(rig.count(is_edit), 0);
    rig.advance(399);
    assert_eq!(rig.count(is_edit), 0);
    rig.advance(1);
    let edits: Vec<_> = rig.fake.calls().into_iter().filter(is_edit).collect();
    assert_eq!(edits, vec![Call::Edit(CommentId::new("s1/c-1"), "abc".to_owned(), 1)]);
    assert_eq!(rig.held().comments[0].text, "abc");
}

#[test]
fn comment_saves_what_was_typed_and_sends_that_one() {
    let mut rig = rig();
    rig.act(A::OpenBox(session(), anchor("a.md", 3)));
    rig.store.send(A::Typed(session(), "fix".to_owned()), |_| {});
    rig.store.send(A::Typed(session(), "fix more".to_owned()), |_| {});
    rig.store.send(A::Comment(session()), |_| {});
    rig.pump();
    let typed = rig.fake.typed(&session());
    assert_eq!(typed.len(), 1);
    assert!(typed[0].contains("fix more"), "{typed:?}");
    let held = rig.held();
    assert!(held.editing.is_none());
    assert_eq!(held.comments[0].status, Status::Sent);
}

#[test]
fn cancel_drops_only_a_draft_the_box_made() {
    let mut rig = rig();
    rig.act(A::OpenBox(session(), anchor("a.md", 3)));
    rig.type_text("fix");
    rig.act(A::Cancel(session()));
    assert!(rig.fake.comments(&session()).is_empty());
    assert!(rig.held().comments.is_empty());
    assert!(rig.held().editing.is_none());

    // A blank box closes without a call.
    let calls = rig.fake.calls().len();
    rig.act(A::OpenBox(session(), anchor("a.md", 3)));
    rig.act(A::Close(session()));
    assert_eq!(rig.fake.calls().len(), calls);
    assert!(rig.held().editing.is_none());
}

#[test]
fn cancel_gives_an_existing_comment_back_its_opening_text() {
    let mut rig = rig();
    let id = rig.seed("a.md", 3, "original");
    rig.load();
    rig.act(A::EditDraft(session(), id.clone()));
    rig.type_text("changed");
    rig.advance(400);
    assert_eq!(rig.fake.comments(&session())[0].text, "changed");
    rig.act(A::Cancel(session()));
    assert_eq!(rig.fake.comments(&session())[0].text, "original");
    assert_eq!(rig.held().comments[0].text, "original");
    assert!(rig.held().editing.is_none());
}

#[test]
fn a_stale_refusal_is_retried_once_with_the_reloaded_revision() {
    let mut rig = rig();
    let id = rig.seed("a.md", 3, "original");
    rig.load();
    rig.act(A::EditDraft(session(), id.clone()));
    // Another window changed it behind the app's back.
    let _ = rig.fake.archive(&id);
    let _ = rig.fake.unarchive(&id);
    let revision = rig.fake.comments(&session())[0].revision;
    assert!(revision > 1);
    rig.type_text("mine");
    rig.advance(400);
    let edits: Vec<_> = rig.fake.calls().into_iter().filter(is_edit).collect();
    assert_eq!(
        edits,
        vec![
            Call::Edit(id.clone(), "mine".to_owned(), 1),
            Call::Edit(id, "mine".to_owned(), revision),
        ]
    );
    assert_eq!(rig.fake.comments(&session())[0].text, "mine");
}

#[test]
fn a_stale_refusal_on_an_archived_comment_keeps_the_text_and_says_so() {
    let mut rig = rig();
    let id = rig.seed("a.md", 3, "original");
    rig.load();
    rig.act(A::EditDraft(session(), id.clone()));
    let _ = rig.fake.archive(&id);
    rig.type_text("mine");
    rig.store.send(A::SaveDue(session()), |_| {});
    toasted(&mut rig);
    rig.pump();
    assert_eq!(rig.count(is_edit), 1);
    let held = rig.held();
    assert_eq!(held.editing.as_ref().map(|b| b.text.clone()), Some("mine".to_owned()));
    assert!(held.error.is_some());
}

#[test]
fn a_failed_create_keeps_the_text_and_retries_with_the_same_key() {
    let mut rig = rig();
    rig.fake.fail("create", DaemonError::Offline { machine: humata2_clients::model::MachineId::new("m") });
    rig.act(A::OpenBox(session(), anchor("a.md", 3)));
    rig.store.send(A::Typed(session(), "fix".to_owned()), |_| {});
    toasted(&mut rig);
    let held = rig.held();
    assert_eq!(held.editing.as_ref().map(|b| b.text.clone()), Some("fix".to_owned()));
    assert!(held.editing.as_ref().is_some_and(|b| b.comment.is_none()));
    assert!(held.error.is_some());

    rig.fake.clear_failure("create");
    rig.type_text("fix it");
    let keys: Vec<String> = rig
        .fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Create(_, _, _, key) => Some(key),
            _ => None,
        })
        .collect();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0], keys[1]);
    assert_eq!(rig.fake.comments(&session()).len(), 1);
}

#[test]
fn send_n_types_every_held_draft_once_and_counts_them() {
    let mut rig = rig();
    rig.seed("a.md", 3, "one");
    rig.seed("b.md", 4, "two");
    let gone = rig.seed("c.md", 5, "archived one");
    let _ = rig.fake.archive(&gone);
    rig.act(A::Open(session()));
    assert_eq!(rig.held().draft_count, 2);
    rig.act(A::SendAll(session()));
    let typed = rig.fake.typed(&session());
    assert_eq!(typed.len(), 1);
    assert_eq!(typed[0].matches("[comment on ").count(), 2);
    let held = rig.held();
    assert!(held.comments.iter().all(|c| c.status == Status::Sent));
    assert_eq!(held.draft_count, 0);
}

#[test]
fn send_all_saves_an_open_box_first() {
    let mut rig = rig();
    rig.act(A::OpenBox(session(), anchor("a.md", 3)));
    rig.type_text("one");
    rig.type_text("one two");
    rig.act(A::SendAll(session()));
    let typed = rig.fake.typed(&session());
    assert_eq!(typed.len(), 1);
    assert!(typed[0].contains("one two"));
    assert!(rig.held().editing.is_none());
}

#[test]
fn the_send_answer_decides_the_status_and_take_back_restores_drafts() {
    let mut rig = rig();
    rig.seed("a.md", 3, "one");
    rig.seed("b.md", 4, "two");
    rig.load();
    rig.fake.set_typing(&session(), Typing::NotTyped);
    rig.act(A::SendAll(session()));
    assert!(rig.fake.typed(&session()).is_empty());
    assert!(rig.held().comments.iter().all(|c| c.status == Status::Waiting));
    rig.act(A::TakeBack(session(), CommentId::new("s1/c-1")));
    assert!(rig.held().comments[0].status == Status::Draft);
    assert_eq!(rig.held().draft_count, 1);
}

#[test]
fn an_unconfirmed_comment_can_be_sent_again_or_taken_back_with_its_batch() {
    let mut rig = rig();
    rig.seed("a.md", 3, "one");
    rig.seed("b.md", 4, "two");
    rig.load();
    rig.fake.set_typing(&session(), Typing::Unknown);
    rig.act(A::SendAll(session()));
    assert!(rig.held().comments.iter().all(|c| c.unconfirmed));

    rig.fake.set_typing(&session(), Typing::Typed);
    rig.act(A::SendAgain(session(), CommentId::new("s1/c-1")));
    assert_eq!(rig.fake.typed(&session()).len(), 1);
    assert!(rig.held().comments.iter().all(|c| c.status == Status::Sent));
}

#[test]
fn a_comment_the_person_resolves_stays_listed_as_done_and_reopen_brings_back_a_draft() {
    let mut rig = rig();
    let id = rig.seed("a.md", 3, "one");
    rig.load();
    rig.act(A::SendAll(session()));
    rig.act(A::Resolve(session(), id.clone()));
    assert_eq!(rig.held().comments.len(), 1);
    assert_eq!(rig.held().comments[0].status, Status::Resolved);

    rig.act(A::Reopen(session(), id));
    assert_eq!(rig.held().comments[0].status, Status::Draft);
    assert_eq!(rig.held().draft_count, 1);
}

#[test]
fn resolve_addressed_works_per_file_and_per_session() {
    let mut rig = rig();
    let a = rig.seed("a.md", 3, "one");
    let b = rig.seed("b.md", 4, "two");
    let c = rig.seed("b.md", 9, "three");
    rig.load();
    rig.act(A::SendAll(session()));
    for id in [&a, &b, &c] {
        let _ = rig.fake.lines_changed(id, &["changed"]);
    }
    rig.act(A::ResolveAddressed(session(), Some(("notes".to_owned(), "a.md".to_owned()))));
    let status = |rig: &Rig, id: &CommentId| {
        rig.fake.comments(&session()).into_iter().find(|x| &x.id == id).map(|x| x.status)
    };
    assert_eq!(status(&rig, &a), Some(Status::Resolved));
    assert_eq!(status(&rig, &b), Some(Status::Addressed));
    rig.act(A::ResolveAddressed(session(), None));
    assert_eq!(status(&rig, &b), Some(Status::Resolved));
    assert_eq!(status(&rig, &c), Some(Status::Resolved));
    assert!(rig.held().comments.iter().all(|x| x.status == Status::Resolved));
}

#[test]
fn archive_saves_the_box_then_offers_undo_and_a_failed_undo_says_so() {
    let mut rig = rig();
    let id = rig.seed("a.md", 3, "original");
    rig.load();
    rig.act(A::EditDraft(session(), id.clone()));
    rig.type_text("edited");
    rig.store.send(A::Archive(session(), id.clone()), |_| {});
    let expected = A::Delegate(D::ArchivedToast {
        comment: id.clone(),
        undo: Box::new(A::Unarchive(session(), id.clone())),
    });
    rig.store.receive(expected, |_| {});
    let stored = rig.fake.comments(&session());
    assert!(stored[0].archived);
    assert_eq!(stored[0].text, "edited");
    assert!(rig.held().comments.is_empty());
    assert!(rig.held().editing.is_none());

    rig.fake.fail("unarchive", DaemonError::NotFound { what: "The comment".to_owned() });
    rig.store.send(A::Unarchive(session(), id.clone()), |_| {});
    toasted(&mut rig);
    rig.fake.clear_failure("unarchive");
    rig.act(A::Unarchive(session(), id));
    assert_eq!(rig.held().comments.len(), 1);
    assert!(!rig.fake.comments(&session())[0].archived);
}

#[test]
fn archived_comments_are_listed_on_demand() {
    let mut rig = rig();
    let id = rig.seed("a.md", 3, "one");
    rig.seed("b.md", 4, "two");
    let _ = rig.fake.archive(&id);
    rig.load();
    assert!(rig.held().archived.is_none());
    rig.act(A::ShowArchived(session()));
    assert!(rig.fake.calls().contains(&Call::List(session(), Which::Archived)));
    let archived = rig.held().archived.unwrap_or_default();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].id, id);
}

#[test]
fn drop_deletes_and_reattach_moves_a_detached_draft() {
    let mut rig = rig();
    let a = rig.seed("a.md", 3, "one");
    let b = rig.seed("b.md", 4, "two");
    let _ = rig.fake.lines_changed(&a, &[]);
    rig.load();
    assert!(rig.held().comments[0].anchor.detached);
    let moved = anchor("a.md", 8);
    let revision = rig.held().comments[0].revision;
    rig.act(A::Reattach(session(), a.clone(), moved.clone()));
    assert!(rig.fake.calls().contains(&Call::Reattach(a.clone(), moved, revision)));
    assert!(!rig.held().comments[0].anchor.detached);
    assert_eq!(rig.held().comments[0].anchor.first, 8);

    rig.act(A::Drop(session(), b.clone()));
    assert_eq!(rig.held().comments.len(), 1);
    assert_eq!(rig.fake.comments(&session()).len(), 1);
}

#[test]
fn a_failed_call_keeps_the_comment_and_toasts() {
    let mut rig = rig();
    rig.seed("a.md", 3, "one");
    rig.load();
    rig.fake.fail("send", DaemonError::Offline { machine: humata2_clients::model::MachineId::new("m") });
    rig.store.send(A::SendAll(session()), |_| {});
    toasted(&mut rig);
    assert_eq!(rig.held().comments[0].status, Status::Draft);
    assert!(rig.held().error.is_some());
}

#[test]
fn comments_for_a_file_sit_under_their_last_line_with_resolved_and_without_archived() {
    let mut rig = rig();
    let a = rig.seed("a.md", 3, "one");
    let b = rig.seed("a.md", 3, "two");
    let c = rig.seed("a.md", 9, "three");
    let d = rig.seed("other.md", 3, "elsewhere");
    let _ = (&b, &d);
    rig.load();
    rig.act(A::Resolve(session(), c.clone()));
    rig.act(A::Archive(session(), a.clone()));
    let lines = |rig: &Rig| {
        rig.store
            .state()
            .on_file(&session(), "notes", "a.md")
            .into_iter()
            .map(|(line, rows)| (line, rows.len()))
            .collect::<Vec<_>>()
    };
    assert_eq!(lines(&rig), vec![(3, 1), (9, 1)]);
}

fn legacy(path: &str, line: u32, body: &str, repo: Option<&str>) -> LegacyComment {
    LegacyComment {
        path: path.to_owned(),
        line,
        body: body.to_owned(),
        sent: false,
        context: repo.map(|repo| CommentContext {
            repo: repo.to_owned(),
            base: "origin/main".to_owned(),
            quoted: Some("let x = 1;".to_owned()),
        }),
    }
}

fn hand_over(comments: Vec<LegacyComment>) -> HandOver {
    HandOver {
        session: session(),
        comments,
        selected_repo: None,
        repos_with_path: [
            ("lib.rs".to_owned(), vec!["api".to_owned()]),
            ("shared.rs".to_owned(), vec!["api".to_owned(), "web".to_owned()]),
        ]
        .into_iter()
        .collect(),
    }
}

fn handed_over(rig: &mut Rig) {
    rig.store.receive_matching(
        "a handed-over draft",
        |a| matches!(a, A::Delegate(D::HandedOver { .. })),
        |_| {},
    );
}

#[test]
fn drafts_from_older_builds_become_daemon_drafts_once() {
    let mut rig = rig();
    let entries = vec![
        legacy("lib.rs", 4, "same", None),
        legacy("lib.rs", 4, "same", None),
        legacy("lib.rs", 4, "   ", None),
        legacy("shared.rs", 2, "unclear repo", None),
        legacy("x.rs", 1, "from context", Some("web")),
        // Already typed to the agent by an older build: not a draft again.
        LegacyComment { sent: true, ..legacy("lib.rs", 7, "already typed", None) },
    ];
    rig.store.send(A::HandOver(hand_over(entries.clone())), |_| {});
    for _ in 0..entries.len() {
        handed_over(&mut rig);
    }
    let made = rig.fake.comments(&session());
    assert_eq!(made.len(), 4);
    assert_eq!(made[0].anchor.repo, "api");
    assert!(!made[0].anchor.detached);
    assert_eq!(made[1].anchor.repo, "api");
    assert!(made[3].anchor.repo == "web" && made[3].anchor.base.as_deref() == Some("origin/main"));
    // The repo wasn't knowable for the third: handed over without one.
    assert_eq!(made[2].anchor.repo, "");
    assert!(made[2].anchor.detached);
    assert_eq!(rig.count(is_create), 4);

    // The same entries again make nothing more: the keys are the same.
    rig.store.send(A::HandOver(hand_over(entries.clone())), |_| {});
    for _ in 0..entries.len() {
        handed_over(&mut rig);
    }
    assert_eq!(rig.fake.comments(&session()).len(), 4);
}

#[test]
fn a_draft_stays_saved_when_its_create_fails() {
    let mut rig = rig();
    rig.fake.fail("create", DaemonError::Offline { machine: humata2_clients::model::MachineId::new("m") });
    rig.store.send(A::HandOver(hand_over(vec![legacy("lib.rs", 4, "keep", None)])), |_| {});
    toasted(&mut rig);
    assert!(rig.fake.comments(&session()).is_empty());
    let handed = rig.store.try_receive_matching(
        "a handed-over draft",
        |a| matches!(a, A::Delegate(D::HandedOver { .. })),
        |_| {},
    );
    assert!(handed.is_err());
}
