//! D190: a passage that changed addresses a sent comment at the lines it
//! moved to; a passage that is gone returns a sent comment to a detached
//! draft, which can't be sent until it is re-attached.

use humata2_review_diff::{
    CommentAnchor, CommentAuthor, CommentError, CommentLedger, CommentStatus, FileRef, Placement,
    Side, Typed, place,
};

fn q(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| (*l).to_owned()).collect()
}

fn anchor(path: &str) -> CommentAnchor {
    CommentAnchor {
        file: FileRef {
            repo: "api".to_owned(),
            path: path.to_owned(),
        },
        side: Side::New,
        first: 3,
        last: 4,
        quoted: q(&["a", "b"]),
        base: String::new(),
        blob: String::new(),
        detached: false,
    }
}

/// c-1 and c-2 drafts, c-1 sent.
fn sent_one() -> CommentLedger {
    let mut l = CommentLedger::new();
    l.add(anchor("a.rs"), "first", 1);
    l.add(anchor("b.rs"), "second", 2);
    let key = l.begin(&q(&["c-1"]), "msg".to_owned(), 10).unwrap().key.clone();
    l.settle(&key, Typed::Yes, 11).unwrap();
    l
}

#[test]
fn a_line_edited_in_place_is_changed_there() {
    let file = ["x", "let size = 48 * KB;", "y"];
    match place(&q(&["let size = 32 * KB;"]), 2, &file) {
        Placement::Changed { first, last, now } => {
            assert_eq!((first, last), (2, 2));
            assert_eq!(now, q(&["let size = 48 * KB;"]));
        }
        _ => panic!("expected Changed"),
    }
}

#[test]
fn an_edited_passage_with_lines_added_above_is_found_where_it_went() {
    let file = ["new one", "new two", "p", "fn run(pages: u32) {", "    scan(pages * 2)", "}"];
    match place(&q(&["fn run(pages: u32) {", "    scan(pages)"]), 2, &file) {
        Placement::Changed { first, last, now } => {
            assert_eq!((first, last), (4, 5));
            assert_eq!(now, q(&["fn run(pages: u32) {", "    scan(pages * 2)"]));
        }
        _ => panic!("expected Changed"),
    }
}

#[test]
fn a_deleted_passage_with_other_text_at_its_lines_is_gone() {
    let file = ["intro", "totally different words here", "and more unrelated text"];
    assert!(matches!(
        place(&q(&["rollback plan: drop the index", "then vacuum"]), 2, &file),
        Placement::Gone
    ));
}

#[test]
fn every_passage_in_an_empty_file_is_gone() {
    assert!(matches!(place(&q(&["anything"]), 1, &[]), Placement::Gone));
}

#[test]
fn same_and_moved_are_unchanged() {
    assert!(matches!(place(&q(&["a b"]), 2, &["x", "a  b"]), Placement::Same));
    assert!(matches!(
        place(&q(&["a", "b"]), 1, &["p", "a", "b"]),
        Placement::Moved { first: 2, last: 3 }
    ));
}

#[test]
fn of_two_equal_windows_the_nearest_wins() {
    let file = ["alpha beta 1", "x", "x", "x", "x", "alpha beta 2"];
    match place(&q(&["alpha beta 0"]), 5, &file) {
        Placement::Changed { first, .. } => assert_eq!(first, 6),
        _ => panic!("expected Changed"),
    }
}

#[test]
fn a_quote_without_words_is_changed_in_range_and_gone_past_the_end() {
    assert!(matches!(
        place(&q(&["}"]), 2, &["a", ")"]),
        Placement::Changed { first: 2, last: 2, .. }
    ));
    assert!(matches!(place(&q(&["}"]), 5, &["a", ")"]), Placement::Gone));
}

#[test]
fn a_sent_comment_whose_passage_is_gone_is_a_detached_draft_again() {
    let mut l = sent_one();
    let before = l.comment("c-1").unwrap().revision;
    assert!(l.gone("c-1"));
    let c = l.comment("c-1").unwrap();
    assert_eq!(c.status, CommentStatus::Draft);
    assert!(c.anchor.detached);
    assert_eq!(c.text, "first");
    assert!(c.sent_at.is_none());
    assert!(c.addressed.is_none());
    assert!(c.revision > before);
    // A pass after a restart finds it gone again and changes nothing.
    assert!(!l.gone("c-1"));
}

#[test]
fn a_detached_draft_is_refused_until_re_attached_then_sends_anew() {
    let mut l = sent_one();
    l.gone("c-1");
    assert_eq!(l.draft_count(), 1);
    assert!(matches!(
        l.begin(&q(&["c-1"]), "m".to_owned(), 20),
        Err(CommentError::Detached { .. })
    ));
    l.reattach("c-1", anchor("a.rs"), None).unwrap();
    assert_eq!(l.draft_count(), 2);
    let key = l.begin(&q(&["c-1"]), "m".to_owned(), 21).unwrap().key.clone();
    l.settle(&key, Typed::Yes, 22).unwrap();
    assert_eq!(l.comment("c-1").unwrap().status, CommentStatus::Sent);
}

#[test]
fn a_sent_comment_whose_passage_changed_is_addressed_at_the_new_lines() {
    let mut l = sent_one();
    assert!(l.changed("c-1", 7, 8, q(&["a2", "b"]), 30));
    let c = l.comment("c-1").unwrap();
    assert_eq!(c.status, CommentStatus::Addressed);
    assert_eq!((c.anchor.first, c.anchor.last), (7, 8));
    assert_eq!(c.anchor.quoted, q(&["a", "b"]));
    assert_eq!(c.addressed.as_ref().unwrap().now, q(&["a2", "b"]));
}

#[test]
fn addressed_resolved_and_archived_comments_ignore_both() {
    let mut l = sent_one();
    l.add(anchor("c.rs"), "third", 3);
    l.changed("c-1", 3, 4, q(&["z"]), 30);
    l.resolve("c-2", CommentAuthor::User, None, 31, None).unwrap();
    l.archive("c-3", 32).unwrap();
    for id in ["c-1", "c-2", "c-3"] {
        let before = l.comment(id).unwrap().clone();
        assert!(!l.gone(id));
        assert!(!l.changed(id, 1, 1, q(&["y"]), 33));
        let after = l.comment(id).unwrap();
        assert_eq!((after.status, after.anchor.detached), (before.status, before.anchor.detached));
    }
}

#[test]
fn a_waiting_comment_that_is_gone_detaches_and_returns_to_draft_once_sent() {
    let mut l = sent_one();
    let key = l.begin(&q(&["c-2"]), "m".to_owned(), 40).unwrap().key.clone();
    assert!(l.gone("c-2"));
    assert_eq!(l.comment("c-2").unwrap().status, CommentStatus::Waiting);
    assert!(l.comment("c-2").unwrap().anchor.detached);
    l.settle(&key, Typed::Yes, 41).unwrap();
    assert_eq!(l.comment("c-2").unwrap().status, CommentStatus::Sent);
    assert!(l.gone("c-2"));
    assert_eq!(l.comment("c-2").unwrap().status, CommentStatus::Draft);
}
