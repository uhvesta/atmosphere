//! The review chat client on its scripted fake: what the app reports, what
//! the agent changes and what the person can undo (PR-22 to PR-26, D59,
//! D283, D288).

use humata2_clients::DaemonError;
use humata2_clients::comments::{Anchor, CommentId, Side};
use humata2_clients::model::SessionId;
use humata2_clients::review_chat::{
    AgentSwitches, FakeReviewChat, FocusRequest, Highlight, LineSpan, Note, Place, ReviewChatClient,
    ReviewChange, ReviewChangeKind, ReviewView,
};
use humata2_clients::model::{RepoBase, RepoDiffSpec};
use humata2_clients::views::PlanGroup;
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

fn refund_view() -> ReviewView {
    ReviewView {
        targets: vec![RepoDiffSpec {
            repo: "acme/api".to_owned(),
            base: RepoBase::Branch("main".to_owned()),
        }],
        open: Some(Place {
            repo: "acme/api".to_owned(),
            path: "src/refund.rs".to_owned(),
            visible: Some(LineSpan { first: 120, last: 160 }),
            selection: None,
            side: Side::New,
        }),
        order: vec![("acme/api".to_owned(), "src/refund.rs".to_owned())],
        groups: vec![PlanGroup {
            title: "Money".to_owned(),
            note: "Start here".to_owned(),
            first: ("acme/api".to_owned(), "src/refund.rs".to_owned()),
        }],
        viewed: Vec::new(),
        highlights: Vec::new(),
    }
}

fn focus_on(path: &str, first: u32) -> FocusRequest {
    FocusRequest {
        repo: "acme/api".to_owned(),
        path: path.to_owned(),
        lines: Some(LineSpan { first, last: first }),
        side: Side::New,
        highlights: vec![Highlight {
            repo: "acme/api".to_owned(),
            path: path.to_owned(),
            lines: Some(LineSpan { first, last: first + 2 }),
            side: Side::New,
            label: "rounds here".to_owned(),
        }],
    }
}

fn note(id: &str, text: &str) -> Note {
    Note {
        id: CommentId::new(id),
        anchor: Anchor {
            repo: "acme/api".to_owned(),
            path: "src/refund.rs".to_owned(),
            side: Side::New,
            first: 140,
            last: 140,
            quoted: vec!["    let rounded = cents - fee;".to_owned()],
            base: None,
            detached: false,
            outdated: false,
        },
        text: text.to_owned(),
    }
}

#[test]
fn the_context_shows_the_view_the_app_reported() {
    let fake = FakeReviewChat::new();
    block_on(fake.report_view(&session(), refund_view())).expect("reported");
    let context = block_on(fake.context(&session())).expect("the context is read");
    assert_eq!(context.view, refund_view());
    assert_eq!(context.switches, AgentSwitches::default());
}

#[test]
fn a_focus_changes_the_view_with_an_undo_and_the_feed_hears_it() {
    let fake = FakeReviewChat::new();
    let mut feed = block_on(fake.watch_changes(&session(), 0)).expect("followed");
    let change = block_on(fake.focus(&session(), focus_on("src/refund.rs", 140))).expect("focused");
    assert_eq!(change.tool, "review_focus");
    assert!(!change.undo_id.is_empty(), "the chat line can undo a focus");
    assert_eq!(change.highlights.len(), 1);
    let heard = feed.try_recv().expect("the feed carries the change");
    assert_eq!(heard, change);
}

#[test]
fn undo_takes_a_focus_back_once_and_says_what_it_undid() {
    let fake = FakeReviewChat::new();
    let focused = block_on(fake.focus(&session(), focus_on("src/refund.rs", 140))).expect("focused");
    let undone = block_on(fake.undo(&session(), &focused.undo_id)).expect("undone");
    assert_eq!(undone.tool, "undo");
    assert_eq!(undone.undid, focused.undo_id);
    let twice = block_on(fake.undo(&session(), &focused.undo_id));
    assert!(matches!(twice, Err(DaemonError::Refused { .. })));
}

#[test]
fn switching_focus_off_refuses_the_agents_focus_with_a_fix() {
    let fake = FakeReviewChat::new();
    let switches = AgentSwitches {
        focus: false,
        ..AgentSwitches::default()
    };
    let kept = block_on(fake.set_switches(&session(), switches.clone())).expect("set");
    assert_eq!(kept, switches);
    let refused = block_on(fake.focus(&session(), focus_on("src/refund.rs", 140)));
    assert!(matches!(refused, Err(DaemonError::Refused { .. })));
    // Reading the context is never gated by the switches (PR-25).
    assert!(block_on(fake.context(&session())).is_ok());
}

#[test]
fn a_note_the_agent_leaves_shows_in_the_context_and_the_feed() {
    let fake = FakeReviewChat::new();
    let mut feed = block_on(fake.watch_changes(&session(), 0)).expect("followed");
    let made = fake.add_note(&session(), note("n-1", "Rounds half the wrong way"));
    let heard = feed.try_recv().expect("the note's change is pushed");
    assert_eq!(heard.tool, "review_note");
    assert_eq!(heard, made);
    assert!(matches!(&heard.change, Some(ReviewChangeKind::Note(kept)) if kept.text == "Rounds half the wrong way"));
    let context = block_on(fake.context(&session())).expect("read");
    assert_eq!(context.notes.len(), 1);
    assert_eq!(context.notes[0].id, CommentId::new("n-1"));
}

#[test]
fn a_change_the_code_moved_has_no_undo() {
    let fake = FakeReviewChat::new();
    let moved = fake.diff_changed(&session(), "refund.rs:140 moved to 142");
    assert_eq!(moved.tool, "diff");
    assert!(moved.undo_id.is_empty());
}

#[test]
fn watching_from_a_sequence_number_sends_only_the_later_changes() {
    let fake = FakeReviewChat::new();
    let first = block_on(fake.focus(&session(), focus_on("src/a.rs", 1))).expect("focused");
    block_on(fake.focus(&session(), focus_on("src/b.rs", 2))).expect("focused");
    let mut feed = block_on(fake.watch_changes(&session(), first.seq)).expect("followed");
    let heard: Vec<ReviewChange> = std::iter::from_fn(|| feed.try_recv().ok()).collect();
    assert_eq!(heard.len(), 1);
    assert!(heard[0].seq > first.seq);
}
