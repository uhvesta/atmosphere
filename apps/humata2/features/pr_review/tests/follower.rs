//! PR-24, D59, D88, D286 (pick 8): the review agent's view changes show only
//! after the person has been still for a second, on a clock the test moves.
//! Each change can be undone, Back returns to where the person was, queued
//! focus moves collapse to the latest, and a change the code made has no Undo.
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::DaemonError;
use humata2_clients::comments::{Anchor, CommentId, Side};
use humata2_clients::model::SessionId;
use humata2_clients::review_chat::fake::Call;
use humata2_clients::review_chat::{
    FakeReviewChat, FocusRequest, LineSpan, Note, Place, ReviewChange, ReviewChangeKind,
    ReviewChatClient,
};
use humata2_pr_review::follower::{
    Delegate, Focus, Follower, FollowerAction as A, FollowerState, Interaction,
};
use tca::{TestClock, TestStore};
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn session() -> SessionId {
    SessionId::new("se_review")
}

fn place(path: &str, line: u32) -> Place {
    Place {
        repo: "acme/api".to_owned(),
        path: path.to_owned(),
        visible: Some(LineSpan { first: line, last: line + 20 }),
        selection: None,
        side: Side::New,
    }
}

fn note(id: &str, line: u32) -> Note {
    Note {
        id: CommentId::new(id),
        anchor: Anchor {
            repo: "acme/api".to_owned(),
            path: "src/refund.rs".to_owned(),
            side: Side::New,
            first: line,
            last: line,
            quoted: vec!["let rounded = cents - fee;".to_owned()],
            base: None,
            detached: false,
            outdated: false,
        },
        text: "Rounds down; is that intended?".to_owned(),
    }
}

fn focus_request(path: &str, line: u32) -> FocusRequest {
    FocusRequest {
        repo: "acme/api".to_owned(),
        path: path.to_owned(),
        lines: Some(LineSpan { first: line, last: line }),
        side: Side::New,
        highlights: Vec::new(),
        layer: None,
        whole_stack: false,
    }
}

/// The agent moves the view, through the fake, which also tells the feed.
fn agent_focuses(fake: &FakeReviewChat, path: &str, line: u32) -> ReviewChange {
    block_on(fake.focus(&session(), focus_request(path, line))).expect("the agent focuses")
}

/// A focus change as the feed would carry it, for tests that act on the
/// follower directly.
fn focus_change(seq: u64, path: &str, line: u32) -> ReviewChange {
    ReviewChange {
        undo_id: format!("u-{seq}"),
        seq,
        tool: "review_focus".to_owned(),
        summary: format!("{path}:{line}"),
        change: Some(ReviewChangeKind::Focus(place(path, line))),
        highlights: Vec::new(),
        undid: String::new(),
        at_unix_ms: 0,
    }
}

struct Rig {
    fake: FakeReviewChat,
    clock: TestClock,
    store: TestStore<Follower>,
}

impl Rig {
    fn new() -> Self {
        let fake = FakeReviewChat::new();
        let clock = TestClock::new();
        let store = TestStore::new(
            FollowerState::default(),
            Follower::new(Arc::new(fake.clone()), Arc::new(clock.clone())),
        )
        .non_exhaustive();
        Self { fake, clock, store }
    }

    /// Following the review, with the feed open.
    fn following() -> Self {
        let mut rig = Self::new();
        rig.store.send(A::Follow(session()), |_| {});
        rig.store.settle();
        rig
    }

    fn state(&self) -> &FollowerState {
        self.store.state()
    }

    /// Waits for the next change on the feed to reach the follower.
    fn heard(&mut self) {
        self.store.receive_matching(
            "the agent's change arrived",
            |action| matches!(action, A::Changed(_)),
            |_| {},
        );
    }

    /// Lets `millis` pass on the clock; what waits on it wakes if it is due.
    fn pass(&mut self, millis: u64) {
        self.store.settle();
        self.clock.advance(Duration::from_millis(millis));
        self.store.settle();
    }

    /// The wait for stillness ended and the follower acted on it.
    fn quiet_ended(&mut self) {
        self.store.receive(A::Quiet, |_| {});
    }

    /// Runs what the last step sent, such as the parent being told to show.
    fn flush(&mut self) {
        self.store.skip_received();
    }

    fn scroll(&mut self) {
        self.store.send(A::Interacted(Interaction::Scroll), |_| {});
    }

    fn shown_path(&self) -> Option<&str> {
        self.state().shown.as_ref().map(|shown| shown.path.as_str())
    }
}

#[test]
fn a_change_waits_until_the_person_has_been_still_for_a_second() {
    let mut rig = Rig::following();
    rig.scroll();
    agent_focuses(&rig.fake, "src/refund.rs", 140);
    rig.heard();
    assert_eq!(rig.state().waiting(), 1);
    assert_eq!(rig.shown_path(), None, "nothing moves under the person's hands");

    rig.pass(999);
    rig.flush();
    assert_eq!(rig.state().waiting(), 1, "a millisecond short of a second");

    rig.pass(1);
    rig.quiet_ended();
    assert_eq!(rig.state().waiting(), 0);
    assert_eq!(rig.shown_path(), Some("src/refund.rs"));
}

#[test]
fn a_change_arriving_after_the_person_was_still_for_a_while_applies_at_once() {
    let mut rig = Rig::following();
    rig.scroll();
    rig.pass(5000);
    agent_focuses(&rig.fake, "src/refund.rs", 140);
    rig.heard();
    assert_eq!(rig.shown_path(), Some("src/refund.rs"));
}

#[test]
fn a_person_who_has_done_nothing_yet_is_not_made_to_wait() {
    let mut rig = Rig::following();
    agent_focuses(&rig.fake, "src/refund.rs", 140);
    rig.heard();
    assert_eq!(rig.shown_path(), Some("src/refund.rs"));
}

#[test]
fn every_scroll_key_or_selection_starts_the_wait_again() {
    let mut rig = Rig::following();
    rig.scroll();
    agent_focuses(&rig.fake, "src/refund.rs", 140);
    rig.heard();

    rig.pass(800);
    rig.store.send(A::Interacted(Interaction::Key), |_| {});
    rig.pass(800);
    rig.flush();
    assert_eq!(rig.state().waiting(), 1, "1600 ms after the first event, 800 after the key");

    rig.store.send(A::Interacted(Interaction::Selection), |_| {});
    rig.pass(999);
    rig.flush();
    assert_eq!(rig.state().waiting(), 1);

    rig.pass(1);
    rig.quiet_ended();
    assert_eq!(rig.shown_path(), Some("src/refund.rs"));
}

#[test]
fn a_drag_blocks_changes_until_it_ends_and_then_the_wait_runs() {
    let mut rig = Rig::following();
    rig.store.send(A::DragStarted, |_| {});
    agent_focuses(&rig.fake, "src/refund.rs", 140);
    rig.heard();

    rig.pass(60_000);
    rig.flush();
    assert_eq!(rig.state().waiting(), 1, "the drag is still going");

    rig.store.send(A::DragEnded, |_| {});
    rig.pass(999);
    rig.flush();
    assert_eq!(rig.state().waiting(), 1);
    rig.pass(1);
    rig.quiet_ended();
    assert_eq!(rig.shown_path(), Some("src/refund.rs"));
}

#[test]
fn queued_focus_moves_collapse_to_the_latest_while_notes_apply_in_order() {
    let mut rig = Rig::following();
    rig.scroll();
    agent_focuses(&rig.fake, "src/a.rs", 1);
    rig.fake.add_note(&session(), note("c-1", 140));
    agent_focuses(&rig.fake, "src/b.rs", 2);
    rig.fake.add_note(&session(), note("c-2", 150));
    agent_focuses(&rig.fake, "src/c.rs", 3);
    for _ in 0..5 {
        rig.heard();
    }
    assert_eq!(rig.state().waiting(), 5);

    rig.pass(1000);
    rig.quiet_ended();
    assert_eq!(rig.shown_path(), Some("src/c.rs"));
    let notes: Vec<_> = rig.state().notes.iter().map(|note| note.id.clone()).collect();
    assert_eq!(notes, vec![CommentId::new("c-1"), CommentId::new("c-2")]);
    let tools: Vec<_> = rig
        .state()
        .applied
        .iter()
        .map(|applied| applied.change.tool.as_str())
        .collect();
    assert_eq!(tools, vec!["review_note", "review_note", "review_focus"]);
}

#[test]
fn undoing_a_move_puts_the_view_back_where_it_was_and_asks_the_daemon() {
    let mut rig = Rig::following();
    rig.store.send(A::UserWentTo(place("src/mine.rs", 10)), |_| {});
    rig.pass(1000);
    agent_focuses(&rig.fake, "src/refund.rs", 140);
    rig.heard();
    assert_eq!(rig.shown_path(), Some("src/refund.rs"));
    let undo_id = rig.state().applied[0].change.undo_id.clone();
    assert!(rig.state().applied[0].can_undo());

    rig.store.send(A::Undo(undo_id.clone()), |_| {});
    rig.store.receive_matching("the daemon undid it", |action| matches!(action, A::Undone { result: Ok(_), .. }), |_| {});
    assert_eq!(rig.shown_path(), Some("src/mine.rs"));
    assert!(rig.state().applied[0].undone);
    assert!(!rig.state().applied[0].can_undo());
    assert!(rig.fake.calls().contains(&Call::Undo(session(), undo_id)));

    // The feed then repeats the undo; it changes nothing more.
    rig.flush();
    assert_eq!(rig.shown_path(), Some("src/mine.rs"));
    assert_eq!(rig.state().applied.len(), 1);
}

#[test]
fn undoing_a_note_removes_it() {
    let mut rig = Rig::following();
    rig.fake.add_note(&session(), note("c-1", 140));
    rig.heard();
    assert_eq!(rig.state().notes.len(), 1);
    let undo_id = rig.state().applied[0].change.undo_id.clone();
    rig.store.send(A::Undo(undo_id), |_| {});
    rig.store.receive_matching("undone", |action| matches!(action, A::Undone { result: Ok(_), .. }), |_| {});
    assert!(rig.state().notes.is_empty());
}

#[test]
fn an_undo_the_daemon_refuses_leaves_the_change_applied_and_says_why() {
    let mut rig = Rig::following();
    agent_focuses(&rig.fake, "src/refund.rs", 140);
    rig.heard();
    rig.fake.fail(
        "undo",
        DaemonError::Refused {
            what: "This change".to_owned(),
            reason: "it was undone already".to_owned(),
        },
    );
    let undo_id = rig.state().applied[0].change.undo_id.clone();
    rig.store.send(A::Undo(undo_id), |_| {});
    rig.store.receive_matching("refused", |action| matches!(action, A::Undone { result: Err(_), .. }), |_| {});
    assert!(rig.state().error.is_some());
    assert!(!rig.state().applied[0].undone);
    assert_eq!(rig.shown_path(), Some("src/refund.rs"));
    rig.store.send(A::DismissError, |_| {});
    assert!(rig.state().error.is_none());
}

#[test]
fn back_returns_to_where_the_person_was_before_the_agent_moved_the_view() {
    let mut rig = Rig::following();
    rig.store.send(A::UserWentTo(place("src/mine.rs", 10)), |_| {});
    rig.pass(1000);
    agent_focuses(&rig.fake, "src/a.rs", 1);
    rig.heard();
    agent_focuses(&rig.fake, "src/b.rs", 2);
    rig.heard();
    assert_eq!(rig.shown_path(), Some("src/b.rs"));

    rig.store.send(A::Back, |_| {});
    assert_eq!(rig.shown_path(), Some("src/mine.rs"), "back to the person's own place, past both moves");
    rig.store.receive(A::Delegate(Delegate::Show(place("src/mine.rs", 10))), |_| {});

    rig.store.send(A::Back, |_| {});
    assert_eq!(rig.shown_path(), Some("src/mine.rs"), "there is nowhere further back");
}

#[test]
fn once_the_person_moves_themselves_back_has_nothing_to_return_to() {
    let mut rig = Rig::following();
    rig.store.send(A::UserWentTo(place("src/mine.rs", 10)), |_| {});
    rig.pass(1000);
    agent_focuses(&rig.fake, "src/a.rs", 1);
    rig.heard();
    rig.store.send(A::UserWentTo(place("src/other.rs", 5)), |_| {});
    rig.store.send(A::Back, |_| {});
    assert_eq!(rig.shown_path(), Some("src/other.rs"));
}

#[test]
fn a_change_the_code_made_shows_without_undo_and_refreshes_the_notes() {
    let mut rig = Rig::following();
    rig.fake.add_note(&session(), note("c-1", 140));
    rig.heard();
    rig.fake.diff_changed(&session(), "a later diff moved refund.rs:140");
    rig.heard();
    let diff = rig.state().applied.last().expect("the diff change shows").clone();
    assert_eq!(diff.change.tool, "diff");
    assert!(!diff.can_undo());

    rig.store.send(A::Undo(diff.change.undo_id.clone()), |_| {});
    assert!(
        !rig.fake.calls().iter().any(|call| matches!(call, Call::Undo(..))),
        "the daemon is never asked to undo it"
    );
    rig.store.receive_matching(
        "the notes were read again",
        |action| matches!(action, A::ContextRead(Ok(_))),
        |_| {},
    );
    assert_eq!(rig.state().notes.len(), 1);
}

#[test]
fn a_change_the_feed_repeats_after_a_restart_is_ignored() {
    let mut rig = Rig::new();
    let change = focus_change(7, "src/refund.rs", 140);
    rig.store.send(A::Changed(change.clone()), |_| {});
    rig.store.send(A::Changed(change), |_| {});
    assert_eq!(rig.state().applied.len(), 1);
}

#[test]
fn a_feed_that_ends_is_followed_again() {
    let mut rig = Rig::following();
    rig.store.send(A::WatchEnded, |_| {});
    rig.store.settle();
    agent_focuses(&rig.fake, "src/refund.rs", 140);
    rig.heard();
    assert_eq!(rig.shown_path(), Some("src/refund.rs"));
}

#[test]
fn the_agents_focus_stands_until_undone_or_the_person_moves() {
    let mut rig = Rig::following();
    assert!(rig.state().focus.is_none());
    rig.store.send(A::Changed(focus_change(1, "src/refund.rs", 140)), |_| {});
    rig.flush();
    let focus = rig.state().focus.clone().expect("the agent focused");
    assert_eq!(focus.line(), Some(140));
    assert_eq!(focus.seq, 1);
    assert_eq!(focus.place.path, "src/refund.rs");

    // A later focus replaces it, and undoing that brings the first back.
    rig.store.send(A::Changed(focus_change(2, "src/refund.rs", 212)), |_| {});
    rig.flush();
    assert_eq!(rig.state().focus.as_ref().and_then(Focus::line), Some(212));
    rig.store.send(A::Undone { undo_id: "u-2".to_owned(), result: Ok(focus_change(3, "src/refund.rs", 212)) }, |_| {});
    assert_eq!(rig.state().focus.as_ref().and_then(Focus::line), Some(140));

    // The person going somewhere themselves ends it.
    rig.store.send(A::UserWentTo(place("src/mine.rs", 10)), |_| {});
    assert!(rig.state().focus.is_none());
}

#[test]
fn a_review_plan_change_delegates_to_parent_and_undo_clears_it() {
    use humata2_clients::views::PlanView;
    let mut rig = Rig::following();
    let plan = PlanView {
        order: vec![("acme/api".to_owned(), "src/refund.rs".to_owned())],
        ..PlanView::default()
    };
    let change = ReviewChange {
        undo_id: "u-plan".to_owned(),
        seq: 1,
        tool: "review_prepare".to_owned(),
        summary: "prepared plan".to_owned(),
        change: Some(ReviewChangeKind::Plan(plan.clone())),
        highlights: Vec::new(),
        undid: String::new(),
        at_unix_ms: 0,
    };
    rig.store.send(A::Changed(change), |_| {});
    rig.store
        .receive(A::Delegate(Delegate::Plan(Some(plan.clone()))), |_| {});
    assert_eq!(rig.state().plan, Some(plan));

    rig.store.send(
        A::Undone {
            undo_id: "u-plan".to_owned(),
            result: Ok(ReviewChange {
                undo_id: String::new(),
                seq: 2,
                tool: "undo".to_owned(),
                summary: "undid".to_owned(),
                change: None,
                highlights: Vec::new(),
                undid: "u-plan".to_owned(),
                at_unix_ms: 0,
            }),
        },
        |_| {},
    );
    rig.store.receive(A::Delegate(Delegate::Plan(None)), |_| {});
    assert!(rig.state().plan.is_none());
}
