//! The review's rules, without GPUI: where comments point and follow the
//! code to, the send queue's exactly-once delivery under retries, terminal
//! states and restarts, and that sessions share nothing.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use humata2_review_view::model::{
    AgentAction, AgentState, Anchor, ChangeKind, CommentBook, CommentId, ComposerAt, Content,
    DeliveryState, DiffLine, Failure, FileChange, FileKey, HighlightAt, Hold, Hunk, Kind,
    LineRange, ListRow, NoteAt, Placement, PreparedGroup, PreparedNote, PreparedReview,
    PullRequest, Review, ReviewMemory, ReviewSnapshot, SessionId, SessionReview, Side, Spot,
    Status, Switch, Target, TextDiff, Tool, Typed, Viewed, body_rows, list_rows, pixel_diff, place,
};
use humata2_review_view::{
    ActionError, Change, DeliveryError, MarkdownRenderer, PlainBlocks, ReviewError, rendered_diff,
};
use image::{Rgba, RgbaImage};

fn session(id: &str) -> SessionId {
    SessionId::new(id)
}

fn key(path: &str) -> FileKey {
    FileKey::new("app", path)
}

/// A text file whose new side is `new` in full, every line added.
fn text_file(path: &str, blob: &str, new: &[&str]) -> FileChange {
    let lines = new
        .iter()
        .enumerate()
        .map(|(index, text)| DiffLine::added(index as u32 + 1, *text))
        .collect();
    FileChange {
        key: key(path),
        change: ChangeKind::Modified,
        base: format!("base-{blob}"),
        blob: blob.to_owned(),
        content: Content::Text(TextDiff {
            hunks: vec![Hunk {
                header: "@@".to_owned(),
                lines,
            }],
            old_text: None,
            new_text: Some(Arc::from(new.join("\n"))),
        }),
    }
}

fn lines<'a>(texts: &[&'a str]) -> Vec<(u32, &'a str)> {
    texts
        .iter()
        .enumerate()
        .map(|(index, text)| (index as u32 + 1, *text))
        .collect()
}

fn anchor(path: &str, first: u32, last: u32, quoted: &[&str]) -> Anchor {
    Anchor {
        file: key(path),
        side: Side::New,
        lines: LineRange::new(first, last),
        base: "base".to_owned(),
        quoted: quoted.iter().map(|line| (*line).to_owned()).collect(),
        rendered: None,
    }
}

fn book_with(drafts: &[&str]) -> (CommentBook, Vec<CommentId>) {
    let mut book = CommentBook::new(session("s1"));
    let ids = drafts
        .iter()
        .enumerate()
        .map(|(index, body)| {
            let line = index as u32 + 1;
            book.add(anchor("src/lib.rs", line, line, &["x"]), body)
                .expect("a comment with text is added")
        })
        .collect();
    (book, ids)
}

fn status(book: &CommentBook, id: CommentId) -> Status {
    book.comment(id).expect("the comment is there").status
}

// Anchors.

#[test]
fn an_anchor_whose_lines_are_still_there_stays() {
    let now = lines(&["a", "fn main() {", "}", "b"]);
    assert_eq!(
        place(&["fn main() {".into(), "}".into()], LineRange::new(2, 3), &now),
        Placement::Unchanged
    );
}

#[test]
fn an_anchor_follows_its_lines_when_they_move() {
    let now = lines(&["new", "new", "a", "fn main() {", "}"]);
    assert_eq!(
        place(&["fn main() {".into(), "}".into()], LineRange::new(2, 3), &now),
        Placement::Moved {
            lines: LineRange::new(4, 5)
        }
    );
}

#[test]
fn an_anchor_moves_to_the_nearest_copy_of_its_lines() {
    let now = lines(&["x", "dup", "x", "x", "x", "x", "x", "x", "dup"]);
    assert_eq!(
        place(&["dup".into()], LineRange::line(7), &now),
        Placement::Moved {
            lines: LineRange::line(9)
        }
    );
}

#[test]
fn an_anchor_ignores_whitespace_changes() {
    let now = lines(&["    let  x = 1;"]);
    assert_eq!(
        place(&["let x = 1;".into()], LineRange::line(1), &now),
        Placement::Unchanged
    );
}

#[test]
fn an_anchor_whose_lines_changed_is_outdated() {
    let now = lines(&["fn main() {", "    run();", "}"]);
    assert_eq!(
        place(&["    walk();".into()], LineRange::line(2), &now),
        Placement::Outdated
    );
}

#[test]
fn a_comment_follows_its_code_when_the_review_changes() {
    let mut review = SessionReview::new(session("s1"));
    review.set_review(Review::new(vec![text_file("a.rs", "1", &["one", "two", "three"])]));
    let at = ComposerAt {
        file: key("a.rs"),
        side: Side::New,
        lines: LineRange::line(2),
    };
    let anchor = review.anchor(&at, None).expect("the file is in the review");
    assert_eq!(anchor.quoted, vec!["two".to_owned()]);
    let id = review.change(|book| book.add(anchor, "rename this")).expect("added");

    review.set_review(Review::new(vec![text_file(
        "a.rs",
        "2",
        &["zero", "one", "two", "three"],
    )]));
    let comment = review.book().comment(id).expect("still there");
    assert_eq!(comment.anchor.lines, LineRange::line(3));
    assert!(!comment.outdated);

    review.set_review(Review::new(vec![text_file("a.rs", "3", &["zero", "one", "three"])]));
    assert!(review.book().comment(id).expect("still there").outdated);
}

#[test]
fn a_sent_comment_whose_lines_changed_is_addressed() {
    let mut review = SessionReview::new(session("s1"));
    review.set_review(Review::new(vec![text_file("a.rs", "1", &["one", "two"])]));
    let at = ComposerAt {
        file: key("a.rs"),
        side: Side::New,
        lines: LineRange::line(2),
    };
    let anchor = review.anchor(&at, None).expect("anchored");
    let id = review.change(|book| book.add(anchor, "fix")).expect("added");
    review.change(|book| {
        book.set_agent(AgentState::Idle);
        let delivery = book.queue(&[id]).expect("queued");
        let dispatch = book.next_dispatch().expect("idle agent takes it");
        book.finish(delivery, dispatch.attempt, Ok(Typed::Typed));
    });
    review.set_review(Review::new(vec![text_file("a.rs", "2", &["one", "deux"])]));
    let comment = review.book().comment(id).expect("still there");
    assert_eq!(comment.status, Status::Addressed);
    assert!(!comment.outdated);
}

#[test]
fn an_outdated_comment_can_be_reattached() {
    let mut review = SessionReview::new(session("s1"));
    review.set_review(Review::new(vec![text_file("a.rs", "1", &["one", "two"])]));
    let first = ComposerAt {
        file: key("a.rs"),
        side: Side::New,
        lines: LineRange::line(2),
    };
    let anchor = review.anchor(&first, None).expect("anchored");
    let id = review.change(|book| book.add(anchor, "fix")).expect("added");
    review.set_review(Review::new(vec![text_file("a.rs", "2", &["one", "deux"])]));
    assert!(review.book().comment(id).expect("there").outdated);

    let anchor = review.anchor(&first, None).expect("anchored");
    review.change(|book| book.reattach(id, anchor)).expect("reattached");
    let comment = review.book().comment(id).expect("there");
    assert!(!comment.outdated);
    assert_eq!(comment.anchor.quoted, vec!["deux".to_owned()]);
}

#[test]
fn a_comment_needs_text() {
    let mut book = CommentBook::new(session("s1"));
    assert_eq!(
        book.add(anchor("a.rs", 1, 1, &["x"]), "   \n"),
        Err(ReviewError::EmptyComment)
    );
    assert!(book.comments().is_empty());
}

// The send queue.

#[test]
fn the_preview_is_exactly_what_is_sent() {
    let (mut book, ids) = book_with(&["first", "second"]);
    let preview = book.preview(&ids).expect("drafts preview");
    book.set_agent(AgentState::Idle);
    book.queue(&ids).expect("queued");
    let dispatch = book.next_dispatch().expect("sent");
    assert_eq!(dispatch.text, preview);
}

#[test]
fn sending_all_puts_every_draft_in_one_message() {
    let (mut book, ids) = book_with(&["first", "second", "third"]);
    let delivery = book.queue_all().expect("queued");
    assert_eq!(book.delivery(delivery).expect("there").comments, ids);
    assert_eq!(book.deliveries().len(), 1);
    assert!(book.drafts().is_empty());
}

#[test]
fn pressing_send_twice_queues_once() {
    let (mut book, ids) = book_with(&["first"]);
    let a = book.queue(&ids).expect("queued");
    let b = book.queue(&ids).expect("already queued");
    assert_eq!(a, b);
    assert_eq!(book.deliveries().len(), 1);
}

#[test]
fn deliveries_go_one_at_a_time_in_order() {
    let (mut book, ids) = book_with(&["first", "second"]);
    book.set_agent(AgentState::Idle);
    let a = book.queue(&ids[..1]).expect("queued");
    let b = book.queue(&ids[1..]).expect("queued");
    let first = book.next_dispatch().expect("first goes");
    assert_eq!(first.delivery, a);
    assert_eq!(book.next_dispatch(), None, "one at a time");
    book.finish(a, first.attempt, Ok(Typed::Typed));
    assert_eq!(book.next_dispatch().expect("second goes").delivery, b);
}

#[test]
fn a_busy_agent_holds_the_queue_until_its_turn_ends() {
    let (mut book, ids) = book_with(&["first"]);
    book.set_agent(AgentState::Working);
    let delivery = book.queue(&ids).expect("queued");
    assert_eq!(book.next_dispatch(), None);
    assert_eq!(
        book.delivery(delivery).expect("there").state,
        DeliveryState::Queued {
            hold: Hold::Busy,
            send_now: false
        }
    );
    book.set_agent(AgentState::Idle);
    assert!(book.next_dispatch().is_some());
}

#[test]
fn send_now_types_while_the_agent_may_be_busy() {
    let (mut book, ids) = book_with(&["first"]);
    book.set_agent(AgentState::ProbablyIdle);
    let delivery = book.queue(&ids).expect("queued");
    assert_eq!(book.next_dispatch(), None);
    book.send_now(delivery).expect("there");
    assert!(book.next_dispatch().is_some());
}

#[test]
fn nothing_is_sent_before_the_terminal_says_the_agent_is_free() {
    let (mut book, ids) = book_with(&["first"]);
    let delivery = book.queue(&ids).expect("queued");
    assert_eq!(book.agent(), AgentState::Unknown);
    book.send_now(delivery).expect("there");
    assert_eq!(book.next_dispatch(), None);
}

#[test]
fn a_disconnected_machine_holds_the_queue_and_it_goes_on_reconnect() {
    let (mut book, ids) = book_with(&["first"]);
    book.set_agent(AgentState::Idle);
    let delivery = book.queue(&ids).expect("queued");
    let dispatch = book.next_dispatch().expect("sent");
    book.finish(
        delivery,
        dispatch.attempt,
        Err(DeliveryError::Disconnected {
            session: session("s1"),
        }),
    );
    assert_eq!(book.agent(), AgentState::Disconnected);
    assert_eq!(status(&book, ids[0]), Status::Queued);
    assert_eq!(book.next_dispatch(), None);
    book.set_agent(AgentState::Idle);
    let again = book.next_dispatch().expect("goes on reconnect");
    assert_eq!(again.key, dispatch.key);
    book.finish(delivery, again.attempt, Ok(Typed::Typed));
    assert_eq!(status(&book, ids[0]), Status::Sent);
}

#[test]
fn an_exited_agent_fails_the_queue_and_retry_sends_it_after_resume() {
    let (mut book, ids) = book_with(&["first"]);
    let delivery = book.queue(&ids).expect("queued");
    book.set_agent(AgentState::Exited);
    assert_eq!(
        book.delivery(delivery).expect("there").state,
        DeliveryState::Failed {
            failure: Failure::Exited
        }
    );
    assert_eq!(status(&book, ids[0]), Status::Failed);
    assert_eq!(book.next_dispatch(), None);

    book.set_agent(AgentState::Idle);
    assert_eq!(book.next_dispatch(), None, "a failed send waits for Retry");
    book.retry(delivery).expect("there");
    book.retry(delivery).expect("a second press does nothing more");
    let dispatch = book.next_dispatch().expect("goes after retry");
    assert_eq!(dispatch.attempt, 1);
    book.finish(delivery, dispatch.attempt, Ok(Typed::Typed));
    assert_eq!(status(&book, ids[0]), Status::Sent);
}

#[test]
fn a_terminal_that_exits_mid_send_fails_it_with_the_agent_exited() {
    let (mut book, ids) = book_with(&["first"]);
    book.set_agent(AgentState::Idle);
    let delivery = book.queue(&ids).expect("queued");
    let dispatch = book.next_dispatch().expect("sent");
    book.finish(
        delivery,
        dispatch.attempt,
        Err(DeliveryError::Exited {
            session: session("s1"),
        }),
    );
    assert_eq!(book.agent(), AgentState::Exited);
    assert_eq!(status(&book, ids[0]), Status::Failed);
}

#[test]
fn a_late_failure_after_success_leaves_it_sent() {
    let (mut book, ids) = book_with(&["first"]);
    book.set_agent(AgentState::Idle);
    let delivery = book.queue(&ids).expect("queued");
    let dispatch = book.next_dispatch().expect("sent");
    book.finish(delivery, dispatch.attempt, Ok(Typed::Typed));
    book.finish(
        delivery,
        dispatch.attempt,
        Err(DeliveryError::Refused {
            session: session("s1"),
            reason: "late".into(),
        }),
    );
    assert_eq!(status(&book, ids[0]), Status::Sent);
    assert_eq!(book.next_dispatch(), None);
}

#[test]
fn a_stale_attempts_failure_does_not_touch_the_current_attempt() {
    let (mut book, ids) = book_with(&["first"]);
    book.set_agent(AgentState::Idle);
    let delivery = book.queue(&ids).expect("queued");
    let first = book.next_dispatch().expect("sent");
    book.finish(
        delivery,
        first.attempt,
        Err(DeliveryError::Interrupted {
            session: session("s1"),
        }),
    );
    book.set_agent(AgentState::Idle);
    let second = book.next_dispatch().expect("resent");
    book.finish(
        delivery,
        first.attempt,
        Err(DeliveryError::Refused {
            session: session("s1"),
            reason: "stale".into(),
        }),
    );
    assert!(matches!(
        book.delivery(delivery).expect("there").state,
        DeliveryState::Sending { attempt } if attempt == second.attempt
    ));
}

#[test]
fn an_interrupted_send_is_in_doubt_and_cannot_be_taken_back() {
    let (mut book, ids) = book_with(&["first"]);
    book.set_agent(AgentState::Idle);
    let delivery = book.queue(&ids).expect("queued");
    let dispatch = book.next_dispatch().expect("sent");
    book.finish(
        delivery,
        dispatch.attempt,
        Err(DeliveryError::Interrupted {
            session: session("s1"),
        }),
    );
    assert!(book.delivery(delivery).expect("there").in_doubt);
    assert_eq!(
        book.cancel(delivery),
        Err(ReviewError::DeliveryInFlight { delivery })
    );
    assert_eq!(
        book.edit(ids[0], "changed"),
        Err(ReviewError::Sending { id: ids[0] })
    );
}

#[test]
fn editing_a_queued_comment_takes_it_out_of_its_message() {
    let (mut book, ids) = book_with(&["first", "second"]);
    let delivery = book.queue(&ids).expect("queued");
    book.edit(ids[0], "first, changed").expect("edited");
    assert_eq!(status(&book, ids[0]), Status::Draft);
    let left = book.delivery(delivery).expect("still queued");
    assert_eq!(left.comments, vec![ids[1]]);
    assert_eq!(Some(left.text.clone()), {
        let mut alone = CommentBook::new(session("s1"));
        let id = alone
            .add(anchor("src/lib.rs", 2, 2, &["x"]), "second")
            .expect("added");
        alone.preview(&[id])
    });
}

#[test]
fn a_sending_comment_cannot_be_changed_or_dropped() {
    let (mut book, ids) = book_with(&["first"]);
    book.set_agent(AgentState::Idle);
    let delivery = book.queue(&ids).expect("queued");
    book.next_dispatch().expect("sending");
    assert_eq!(book.edit(ids[0], "x"), Err(ReviewError::Sending { id: ids[0] }));
    assert_eq!(book.delete(ids[0]), Err(ReviewError::Sending { id: ids[0] }));
    assert_eq!(
        book.cancel(delivery),
        Err(ReviewError::DeliveryInFlight { delivery })
    );
}

#[test]
fn cancelling_a_queued_message_makes_its_comments_drafts() {
    let (mut book, ids) = book_with(&["first", "second"]);
    let delivery = book.queue(&ids).expect("queued");
    book.cancel(delivery).expect("cancelled");
    assert_eq!(book.drafts(), ids);
    assert!(book.deliveries().is_empty());
}

#[test]
fn a_reopened_comment_is_sent_again_under_a_new_key() {
    let (mut book, ids) = book_with(&["first"]);
    book.set_agent(AgentState::Idle);
    let delivery = book.queue(&ids).expect("queued");
    let first = book.next_dispatch().expect("sent");
    book.finish(delivery, first.attempt, Ok(Typed::Typed));
    book.reopen(ids[0]).expect("reopened");
    assert_eq!(status(&book, ids[0]), Status::Draft);
    book.queue(&ids).expect("queued again");
    let second = book.next_dispatch().expect("sent again");
    assert_ne!(first.key, second.key);
}

// Restarts (PER-25, PER-27).

fn round_trip(book: &CommentBook) -> CommentBook {
    let json = serde_json::to_string(book).expect("serialises");
    serde_json::from_str::<CommentBook>(&json)
        .expect("deserialises")
        .restored()
}

#[test]
fn a_saved_review_round_trips() {
    let mut review = SessionReview::new(session("s1"));
    review.set_review(Review::new(vec![text_file("a.rs", "1", &["one"])]));
    review.change(|book| book.add(anchor("a.rs", 1, 1, &["one"]), "draft")).expect("added");
    review.remember(|memory| memory.set_viewed(&key("a.rs"), "1", true));
    review.open_composer(
        ComposerAt {
            file: key("a.rs"),
            side: Side::New,
            lines: LineRange::line(1),
        },
        None,
    );
    review.set_composer_text("half a tho".into());
    let snapshot = review.snapshot();
    let json = serde_json::to_vec(&snapshot).expect("serialises");
    let back: ReviewSnapshot = serde_json::from_slice(&json).expect("deserialises");
    assert_eq!(back, snapshot);
    let restored = SessionReview::restore(back);
    assert_eq!(restored.composer().map(|c| c.text.as_str()), Some("half a tho"));
    assert_eq!(restored.book().drafts().len(), 1);
}

#[test]
fn a_restart_forgets_the_agents_state_until_the_terminal_says() {
    let (mut book, ids) = book_with(&["first"]);
    book.set_agent(AgentState::Idle);
    book.queue(&ids).expect("queued");
    let mut back = round_trip(&book);
    assert_eq!(back.agent(), AgentState::Unknown);
    assert_eq!(back.next_dispatch(), None);
}

#[test]
fn a_send_in_flight_at_a_restart_is_resent_under_its_key() {
    let (mut book, ids) = book_with(&["first"]);
    book.set_agent(AgentState::Idle);
    let delivery = book.queue(&ids).expect("queued");
    let before = book.next_dispatch().expect("sending");

    let mut back = round_trip(&book);
    let after = back.delivery(delivery).expect("kept");
    assert!(after.in_doubt);
    assert_eq!(status(&back, ids[0]), Status::Sending, "it may be with the agent");
    assert_eq!(
        back.cancel(delivery),
        Err(ReviewError::DeliveryInFlight { delivery })
    );

    back.set_agent(AgentState::Idle);
    let resend = back.next_dispatch().expect("resent");
    assert_eq!(resend.key, before.key);
    assert_eq!(resend.text, before.text);
    back.finish(delivery, resend.attempt, Ok(Typed::AlreadyTyped));
    assert_eq!(status(&back, ids[0]), Status::Sent);
    assert!(!back.delivery(delivery).expect("kept").in_doubt);
}

/// A terminal that types each key at most once, remembered across restarts
/// of the app.
#[derive(Default)]
struct Terminal {
    typed: HashMap<String, usize>,
    log: Vec<String>,
}

impl Terminal {
    fn take(&mut self, key: &str, text: &str) -> Typed {
        let count = self.typed.entry(key.to_owned()).or_default();
        *count += 1;
        if *count == 1 {
            self.log.push(text.to_owned());
            Typed::Typed
        } else {
            Typed::AlreadyTyped
        }
    }
}

/// The pane's send protocol, step by step: save the book as sending, type,
/// settle, save. `kill` stops the app after that many steps; what was last
/// saved is what the next run starts from.
fn run(saved: &mut String, terminal: &mut Terminal, kill: Option<usize>) -> bool {
    let mut book = serde_json::from_str::<CommentBook>(saved)
        .expect("saved book reads")
        .restored();
    book.set_agent(AgentState::Idle);
    let mut steps = 0usize;
    let step = |steps: &mut usize| {
        *steps += 1;
        kill.is_some_and(|kill| *steps > kill)
    };
    while let Some(dispatch) = book.next_dispatch() {
        if step(&mut steps) {
            return false;
        }
        *saved = serde_json::to_string(&book).expect("saves");
        if step(&mut steps) {
            return false;
        }
        let typed = terminal.take(&dispatch.key, &dispatch.text);
        if step(&mut steps) {
            return false;
        }
        book.finish(dispatch.delivery, dispatch.attempt, Ok(typed));
        if step(&mut steps) {
            return false;
        }
        *saved = serde_json::to_string(&book).expect("saves");
    }
    true
}

#[test]
fn a_kill_at_any_step_neither_loses_nor_repeats_a_comment() {
    let (mut book, ids) = book_with(&["first", "second", "third"]);
    book.queue(&ids[..1]).expect("queued");
    book.queue(&ids[1..]).expect("queued");
    let start = serde_json::to_string(&book).expect("saves");

    let mut clean = Terminal::default();
    let mut saved = start.clone();
    assert!(run(&mut saved, &mut clean, None));
    let expected = clean.log.clone();
    assert_eq!(expected.len(), 2);

    for first_kill in 0..12 {
        for second_kill in [Some(0), Some(1), Some(3), None] {
            let mut terminal = Terminal::default();
            let mut saved = start.clone();
            run(&mut saved, &mut terminal, Some(first_kill));
            run(&mut saved, &mut terminal, second_kill);
            run(&mut saved, &mut terminal, None);
            assert_eq!(
                terminal.log, expected,
                "killed after {first_kill} then {second_kill:?} steps"
            );
            let book = serde_json::from_str::<CommentBook>(&saved).expect("reads");
            for id in &ids {
                assert_eq!(status(&book, *id), Status::Sent);
            }
        }
    }
}

// Sessions.

#[test]
fn sessions_share_no_comments_or_drafts() {
    let mut one = SessionReview::new(session("one"));
    let mut two = SessionReview::new(session("two"));
    let review = Review::new(vec![text_file("a.rs", "1", &["one"])]);
    one.set_review(review.clone());
    two.set_review(review);
    one.change(|book| book.add(anchor("a.rs", 1, 1, &["one"]), "only in one")).expect("added");
    one.open_composer(
        ComposerAt {
            file: key("a.rs"),
            side: Side::New,
            lines: LineRange::line(1),
        },
        None,
    );
    assert!(two.book().comments().is_empty());
    assert!(two.composer().is_none());
}

#[test]
fn keys_never_repeat_across_sessions_or_a_book_made_again() {
    let mut keys = HashSet::new();
    for id in ["one", "two", "one"] {
        let mut book = CommentBook::new(session(id));
        let comment = book.add(anchor("a.rs", 1, 1, &["x"]), "hi").expect("added");
        book.set_agent(AgentState::Idle);
        book.queue(&[comment]).expect("queued");
        let dispatch = book.next_dispatch().expect("sent");
        assert!(keys.insert(dispatch.key), "a key repeated");
    }
}

// The file list and body.

fn three_files() -> Review {
    Review::new(vec![
        text_file("a.rs", "1", &["a"]),
        text_file("b.rs", "1", &["b"]),
        text_file("c.rs", "1", &["c"]),
    ])
}

fn listed(rows: &[ListRow]) -> Vec<usize> {
    rows.iter()
        .filter_map(|row| match row {
            ListRow::File(index) => Some(*index),
            ListRow::Repo { .. } | ListRow::Group { .. } => None,
        })
        .collect()
}

#[test]
fn filters_narrow_the_file_list() {
    let review = three_files();
    let mut memory = ReviewMemory::default();
    let mut book = CommentBook::new(session("s1"));
    memory.set_viewed(&key("a.rs"), "1", true);
    memory.set_viewed(&key("b.rs"), "old", true);
    book.add(anchor("c.rs", 1, 1, &["c"]), "look").expect("added");

    assert_eq!(listed(&list_rows(&review, &memory, &book)), vec![0, 1, 2]);
    memory.filters.unviewed = true;
    assert_eq!(listed(&list_rows(&review, &memory, &book)), vec![1, 2]);
    memory.filters.unviewed = false;
    memory.filters.changed = true;
    assert_eq!(listed(&list_rows(&review, &memory, &book)), vec![1]);
    memory.filters.changed = false;
    memory.filters.commented = true;
    assert_eq!(listed(&list_rows(&review, &memory, &book)), vec![2]);
}

#[test]
fn a_file_viewed_then_changed_is_not_viewed() {
    let mut memory = ReviewMemory::default();
    memory.set_viewed(&key("a.rs"), "1", true);
    assert_eq!(memory.viewed(&key("a.rs"), "1"), Viewed::Yes);
    assert_eq!(memory.viewed(&key("a.rs"), "2"), Viewed::ChangedSince);
    assert_eq!(memory.viewed(&key("b.rs"), "1"), Viewed::No);
}

#[test]
fn a_collapsed_file_shows_only_its_header() {
    let review = three_files();
    let mut memory = ReviewMemory::default();
    let book = CommentBook::new(session("s1"));
    let open = body_rows(&review, &memory, &book, &Default::default(), None).len();
    memory.toggle_collapsed(&key("b.rs"));
    let closed = body_rows(&review, &memory, &book, &Default::default(), None).len();
    // b.rs loses its hunk header and its one line.
    assert_eq!(open - closed, 2);
}

#[test]
fn a_comment_sits_under_the_line_it_ends_on() {
    use humata2_review_view::model::BodyRow;
    let review = Review::new(vec![text_file("a.rs", "1", &["one", "two", "three"])]);
    let memory = ReviewMemory::default();
    let mut book = CommentBook::new(session("s1"));
    let id = book.add(anchor("a.rs", 1, 2, &["one", "two"]), "here").expect("added");
    let rows = body_rows(&review, &memory, &book, &Default::default(), None);
    let at = rows
        .iter()
        .position(|row| *row == BodyRow::Comment { file: 0, id })
        .expect("shown");
    assert_eq!(
        rows[at - 1],
        BodyRow::Line {
            file: 0,
            hunk: 0,
            line: 1
        }
    );
}

// Rendered markdown.

#[test]
fn the_rendered_diff_marks_added_changed_and_removed_blocks() {
    let old = "# Title\n\nKeep this.\n\nChange this.\n\n- gone\n";
    let new = "# Title\n\nKeep this.\n\nChanged this.\n\n- new item\n- another\n";
    let blocks = rendered_diff(&PlainBlocks, old, new);
    let changes: Vec<(String, Change)> = blocks
        .iter()
        .map(|block| (block.block.text.clone(), block.change))
        .collect();
    assert_eq!(
        changes,
        vec![
            ("Title".to_owned(), Change::Same),
            ("Keep this.".to_owned(), Change::Same),
            ("Changed this.".to_owned(), Change::Changed),
            ("new item".to_owned(), Change::Changed),
            ("another".to_owned(), Change::Added),
        ]
    );
}

#[test]
fn rendered_blocks_know_their_source_lines() {
    let source = "# Title\n\nA paragraph\nover two lines.\n\n```\ncode\n```\n";
    let blocks = PlainBlocks.blocks(source);
    let ranges: Vec<LineRange> = blocks.iter().map(|block| block.lines).collect();
    assert_eq!(
        ranges,
        vec![LineRange::line(1), LineRange::new(3, 4), LineRange::new(6, 8)]
    );
    assert_eq!(blocks[1].text, "A paragraph over two lines.");
}

#[test]
fn a_removed_block_is_kept_struck_through() {
    let blocks = rendered_diff(&PlainBlocks, "One.\n\nTwo.\n", "One.\n");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[1].change, Change::Removed);
    assert_eq!(blocks[1].new_lines(), None);
}

// Images.

fn filled(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
    RgbaImage::from_pixel(width, height, Rgba(color))
}

#[test]
fn the_pixel_diff_counts_changed_pixels() {
    let old = filled(4, 4, [10, 10, 10, 255]);
    let mut new = old.clone();
    new.put_pixel(1, 1, Rgba([200, 10, 10, 255]));
    new.put_pixel(2, 2, Rgba([12, 10, 10, 255]));
    let diff = pixel_diff(&old, &new, 8);
    assert_eq!(diff.changed, 1, "a change under the threshold doesn't count");
    assert_eq!(diff.total, 16);
    assert!(!diff.resized);
}

#[test]
fn the_pixel_diff_counts_a_resize_as_changed_pixels() {
    let old = filled(2, 2, [0, 0, 0, 255]);
    let new = filled(3, 2, [0, 0, 0, 255]);
    let diff = pixel_diff(&old, &new, 8);
    assert!(diff.resized);
    assert_eq!(diff.changed, 2);
    assert_eq!(diff.picture.dimensions(), (3, 2));
}

// Kinds: comments, questions and GitHub drafts.

#[test]
fn questions_go_with_comments_in_one_message_marked_as_questions() {
    let mut book = CommentBook::new(session("s1"));
    let comment = book.add(anchor("a.rs", 1, 1, &["x"]), "Rename this").expect("added");
    let question = book
        .add_as(Kind::Question, None, anchor("a.rs", 2, 2, &["y"]), "Why is this here?")
        .expect("added");
    let as_comment = {
        let mut other = CommentBook::new(session("s1"));
        let id = other.add(anchor("a.rs", 2, 2, &["y"]), "Why is this here?").expect("added");
        other.preview(&[id]).expect("a message")
    };
    let as_question = book.preview(&[question]).expect("a message");
    assert_ne!(as_question, as_comment, "a question reads differently from a comment");
    book.set_agent(AgentState::Idle);
    let delivery = book.queue_all().expect("queued");
    let sent = book.delivery(delivery).expect("there");
    assert_eq!(sent.comments, vec![comment, question]);
}

#[test]
fn github_drafts_never_go_to_the_agent() {
    let mut book = CommentBook::new(session("s1"));
    let draft = book
        .add_as(Kind::GitHub, Some("Claude Code".into()), anchor("a.rs", 1, 1, &["x"]), "nit")
        .expect("added");
    assert!(book.drafts().is_empty());
    assert_eq!(book.unsent(), 0);
    assert_eq!(book.github_drafts(), vec![draft]);
    assert_eq!(book.preview(&[draft]), None);
    assert_eq!(book.queue(&[draft]), Err(ReviewError::NothingToSend));
}

#[test]
fn a_comment_made_a_github_draft_leaves_the_queue() {
    let mut review = pr_review();
    let id = review.change(|book| book.add(anchor("b.rs", 1, 1, &["b1"]), "fix")).expect("added");
    review.change(|book| book.queue(&[id])).expect("queued");
    review.edit_composer(id).expect("opens");
    review.set_composer_kind(Kind::GitHub);
    review.submit_composer().expect("saved");
    assert_eq!(review.book().github_drafts(), vec![id]);
    assert_eq!(review.book().unsent(), 0);
}

// The review chat's agent working the view (PR-22–PR-26).

fn pr_review() -> SessionReview {
    let mut review = SessionReview::new(session("pr"));
    review.set_review(
        Review::new(vec![
            text_file("a.rs", "1", &["a1", "a2"]),
            text_file("b.rs", "1", &["b1", "b2", "b3", "b4"]),
            text_file("c.rs", "1", &["c1"]),
        ])
        .with_pulls(vec![PullRequest {
            repo: "app".into(),
            number: 7,
            title: "Refunds".into(),
        }]),
    );
    review
}

fn act(id: &str, tool: Tool) -> AgentAction {
    AgentAction {
        id: id.into(),
        agent: "Claude Code".into(),
        tool,
    }
}

fn focus(id: &str, path: &str, first: u32, last: u32, label: &str) -> AgentAction {
    let target = Target::lines(key(path), Side::New, LineRange::new(first, last));
    act(
        id,
        Tool::Focus {
            target: target.clone(),
            select: true,
            highlights: vec![HighlightAt {
                target,
                label: label.into(),
            }],
        },
    )
}

fn note(id: &str, path: &str, line: u32, text: &str) -> AgentAction {
    act(
        id,
        Tool::Note {
            target: Target::lines(key(path), Side::New, LineRange::line(line)),
            text: text.into(),
        },
    )
}

fn at_a() -> Spot {
    Spot {
        current: Some(key("a.rs")),
        selection: None,
    }
}

#[test]
fn focus_opens_selects_and_highlights_and_undo_puts_it_back() {
    let mut review = pr_review();
    review.remember(|memory| {
        memory.current = Some(key("a.rs"));
        memory.collapsed.insert(key("b.rs"));
    });
    let to = review
        .apply_agent(focus("f1", "b.rs", 2, 3, "the refund"), at_a())
        .expect("applied")
        .expect("the view moves");
    assert_eq!(to.current, Some(key("b.rs")));
    assert_eq!(to.selection.map(|s| s.lines), Some(LineRange::new(2, 3)));
    let memory = review.memory();
    assert_eq!(memory.current, Some(key("b.rs")));
    assert!(!memory.collapsed.contains(&key("b.rs")), "the file is opened");
    let lit: Vec<u32> = (1..=4)
        .filter(|line| memory.agent.highlights_at(&key("b.rs"), Side::New, *line).next().is_some())
        .collect();
    assert_eq!(lit, vec![2, 3]);

    let back = review.undo_agent("f1").expect("undone").expect("the view moves");
    assert_eq!(back, at_a());
    assert!(review.memory().agent.highlights.is_empty());
    assert_eq!(review.memory().current, Some(key("a.rs")));
    assert_eq!(review.undo_agent("f1"), Err(ActionError::NoChange { id: "f1".into() }));
}

#[test]
fn back_returns_to_where_the_user_was_once() {
    let mut review = pr_review();
    review.apply_agent(focus("f1", "b.rs", 1, 1, "here"), at_a()).expect("applied");
    assert_eq!(review.back(), Some(at_a()));
    assert_eq!(review.memory().current, Some(key("a.rs")));
    assert_eq!(review.back(), None);
}

#[test]
fn a_switch_turned_off_denies_the_tool_and_changes_nothing() {
    let mut review = pr_review();
    review.set_agent_may(Switch::Notes, false);
    let revision = review.revision();
    assert_eq!(
        review.apply_agent(note("n1", "b.rs", 2, "see"), at_a()),
        Err(ActionError::TurnedOff { switch: Switch::Notes })
    );
    assert!(review.memory().agent.notes.is_empty());
    assert_eq!(review.revision(), revision);
    // The others stay on.
    assert!(review.apply_agent(focus("f1", "b.rs", 1, 1, "x"), at_a()).is_ok());
}

#[test]
fn the_same_call_twice_applies_once() {
    let mut review = pr_review();
    review.apply_agent(note("n1", "b.rs", 2, "see"), at_a()).expect("applied");
    review.apply_agent(note("n1", "b.rs", 2, "see"), at_a()).expect("answered");
    assert_eq!(review.memory().agent.notes.len(), 1);
}

#[test]
fn targets_outside_the_review_are_refused() {
    let mut review = pr_review();
    assert!(matches!(
        review.apply_agent(focus("f1", "nope.rs", 1, 1, "x"), at_a()),
        Err(ActionError::NoFile { .. })
    ));
    assert!(matches!(
        review.apply_agent(focus("f2", "b.rs", 3, 9, "x"), at_a()),
        Err(ActionError::NoLines { .. })
    ));
    let hunk = act(
        "f3",
        Tool::Focus {
            target: Target {
                file: key("b.rs"),
                side: Side::New,
                lines: None,
                hunk: Some(4),
            },
            select: true,
            highlights: Vec::new(),
        },
    );
    assert!(matches!(review.apply_agent(hunk, at_a()), Err(ActionError::NoHunk { .. })));
    assert!(review.memory().agent.changes.is_empty());
}

#[test]
fn a_note_made_a_draft_is_a_github_draft_by_the_agent() {
    let mut review = pr_review();
    review.apply_agent(note("n1", "b.rs", 2, "Check the rounding"), at_a()).expect("applied");
    let id = review.memory().agent.notes[0].id;
    let draft = review.note_to_draft(id).expect("made a draft");
    let comment = review.book().comment(draft).expect("there").clone();
    assert_eq!(comment.kind, Kind::GitHub);
    assert_eq!(comment.by.as_deref(), Some("Claude Code"));
    assert_eq!(comment.anchor.lines, LineRange::line(2));
    assert_eq!(comment.anchor.quoted, vec!["b2".to_owned()]);
    assert!(review.memory().agent.notes.is_empty());
    assert_eq!(review.book().unsent(), 0, "it isn't sent to the agent");
}

#[test]
fn editing_a_note_opens_it_as_a_github_draft_and_adding_it_drops_the_note() {
    let mut review = pr_review();
    review.apply_agent(note("n1", "b.rs", 2, "Check the rounding"), at_a()).expect("applied");
    let id = review.memory().agent.notes[0].id;
    review.edit_note(id).expect("opens");
    let composer = review.composer().expect("open").clone();
    assert_eq!(composer.kind, Kind::GitHub);
    assert_eq!(composer.text, "Check the rounding");
    review.set_composer_text("Check the rounding of refunds".into());
    let draft = review.submit_composer().expect("added");
    let comment = review.book().comment(draft).expect("there");
    assert_eq!(comment.body, "Check the rounding of refunds");
    assert_eq!(comment.kind, Kind::GitHub);
    assert!(review.memory().agent.notes.is_empty());
}

#[test]
fn dropping_a_note_removes_it_and_undo_of_a_note_too() {
    let mut review = pr_review();
    review.apply_agent(note("n1", "b.rs", 2, "one"), at_a()).expect("applied");
    review.apply_agent(note("n2", "b.rs", 3, "two"), at_a()).expect("applied");
    let first = review.memory().agent.notes[0].id;
    review.drop_note(first).expect("dropped");
    assert_eq!(review.drop_note(first), Err(ReviewError::NoNote { note: first }));
    review.undo_agent("n2").expect("undone");
    assert!(review.memory().agent.notes.is_empty());
}

#[test]
fn notes_and_highlights_follow_their_lines_or_are_outdated() {
    let mut review = pr_review();
    review.apply_agent(focus("f1", "b.rs", 2, 2, "here"), at_a()).expect("applied");
    review.apply_agent(note("n1", "b.rs", 3, "and here"), at_a()).expect("applied");
    let pulls = review.review().pulls().to_vec();
    review.set_review(
        Review::new(vec![text_file("b.rs", "2", &["b0", "b1", "b2", "b3", "b4"])]).with_pulls(pulls.clone()),
    );
    let agent = &review.memory().agent;
    assert_eq!(agent.highlights[0].lines, LineRange::line(3));
    assert_eq!(agent.notes[0].at.as_ref().map(|at| at.lines), Some(LineRange::line(4)));
    review.set_review(
        Review::new(vec![text_file("b.rs", "3", &["b0", "b1", "B2!", "B3!", "b4"])]).with_pulls(pulls),
    );
    let agent = &review.memory().agent;
    assert!(agent.highlights[0].outdated);
    assert!(agent.notes[0].outdated, "kept, marked outdated");
}

#[test]
fn notes_highlights_order_and_switches_survive_a_restart() {
    let mut review = pr_review();
    review.apply_agent(focus("f1", "b.rs", 2, 3, "the refund"), at_a()).expect("applied");
    review.apply_agent(note("n1", "b.rs", 2, "see"), at_a()).expect("applied");
    review.apply_agent(act("p1", Tool::Prepare(prepared())), at_a()).expect("applied");
    review.set_agent_may(Switch::Focus, false);
    let json = serde_json::to_vec(&review.snapshot()).expect("serialises");
    let back: ReviewSnapshot = serde_json::from_slice(&json).expect("deserialises");
    let restored = SessionReview::restore(back);
    assert_eq!(restored.memory().agent, review.memory().agent);
    assert_eq!(restored.memory().prepared, review.memory().prepared);
    assert!(!restored.memory().agent.may.allows(Switch::Focus));
}

fn prepared() -> PreparedReview {
    PreparedReview {
        id: "view-1".into(),
        title: "Refunds first".into(),
        agent: "Claude Code".into(),
        summary: "The refund path, then the rest.".into(),
        groups: vec![PreparedGroup {
            title: "Refund path".into(),
            files: vec![key("c.rs"), key("b.rs")],
        }],
        notes: vec![PreparedNote {
            file: key("b.rs"),
            at: Some(NoteAt {
                side: Side::New,
                lines: LineRange::line(4),
                quoted: Vec::new(),
            }),
            text: "Start here".into(),
            outdated: false,
        }],
    }
}

#[test]
fn prepare_orders_the_files_and_undo_puts_the_old_order_back() {
    let mut review = pr_review();
    review.remember(|memory| memory.filters.unviewed = true);
    review.apply_agent(act("p1", Tool::Prepare(prepared())), at_a()).expect("applied");
    let order = review
        .memory()
        .prepared
        .as_ref()
        .expect("showing")
        .view
        .order(review.review());
    assert_eq!(order, vec![(Some(0), vec![2, 1]), (None, vec![0])]);
    assert_eq!(review.memory().current, Some(key("c.rs")));
    let notes = &review.memory().agent.notes;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].at.as_ref().map(|at| at.quoted.clone()), Some(vec!["b4".to_owned()]));
    review.undo_agent("p1").expect("undone");
    assert!(review.memory().prepared.is_none());
    assert!(review.memory().agent.notes.is_empty());
    assert!(review.memory().filters.unviewed, "the user's filter is back");
}

#[test]
fn reverting_a_prepared_view_the_user_opened_removes_its_notes_only() {
    let mut review = pr_review();
    review.apply_agent(note("n1", "a.rs", 1, "mine stays"), at_a()).expect("applied");
    review.open_prepared(prepared());
    assert_eq!(review.memory().agent.notes.len(), 2);
    let reverted = review.remember(|memory| memory.revert_prepared());
    assert_eq!(reverted.as_deref(), Some("view-1"));
    let notes = &review.memory().agent.notes;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].text, "mine stays");
}

#[test]
fn undoing_an_older_focus_leaves_the_newer_one() {
    let mut review = pr_review();
    review.apply_agent(focus("f1", "a.rs", 1, 1, "first"), at_a()).expect("applied");
    review.apply_agent(focus("f2", "b.rs", 2, 2, "second"), at_a()).expect("applied");
    assert_eq!(review.undo_agent("f1"), Ok(None), "the view stays");
    let memory = review.memory();
    assert_eq!(memory.current, Some(key("b.rs")));
    assert!(memory.agent.highlights_at(&key("b.rs"), Side::New, 2).next().is_some());
}

#[test]
fn a_note_the_user_took_over_has_no_undo() {
    let mut review = pr_review();
    review.apply_agent(note("n1", "b.rs", 2, "see"), at_a()).expect("applied");
    let id = review.memory().agent.notes[0].id;
    let draft = review.note_to_draft(id).expect("a draft");
    assert_eq!(review.undo_agent("n1"), Err(ActionError::NoChange { id: "n1".into() }));
    assert!(review.book().comment(draft).is_some(), "the draft stays");
}
