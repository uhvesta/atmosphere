use humata2_review_diff::{DiffContext, MessagePart, Placement, Side, bracketed_paste, compose, place};

fn plain<'a>(path: &'a str, first: u32, last: u32, text: &'a str) -> MessagePart<'a> {
    MessagePart { repo: None, path, first, last, diff: None, quoted: &[], text }
}

#[test]
fn plain_one_line() {
    assert_eq!(compose(&[plain("src/a.rs", 7, 7, "fix this")]), "[comment on src/a.rs:7]\nfix this");
}

#[test]
fn plain_range() {
    assert_eq!(compose(&[plain("a.rs", 7, 9, "x")]), "[comment on a.rs:7-9]\nx");
}

#[test]
fn whole_file() {
    assert_eq!(compose(&[plain("a.rs", 0, 0, "x")]), "[comment on a.rs]\nx");
}

#[test]
fn repo_prefix_and_trailing_whitespace() {
    let mut part = plain("a.rs", 1, 1, "x \n\n");
    part.repo = Some("api");
    assert_eq!(compose(&[part]), "[comment on api/a.rs:1]\nx");
}

#[test]
fn diff_comment_matches_app_format() {
    let quoted = vec!["let x = 1;".to_string()];
    let part = MessagePart {
        repo: Some("api"),
        path: "src/a.rs",
        first: 3,
        last: 3,
        diff: Some(DiffContext { side: Side::New, base: "main" }),
        quoted: &quoted,
        text: "why?",
    };
    assert_eq!(
        compose(&[part]),
        "[comment on api/src/a.rs:3 · new · base main]\n> let x = 1;\nwhy?"
    );
}

#[test]
fn diff_comment_empty_base() {
    let quoted = vec!["a".to_string(), "b".to_string()];
    let part = MessagePart {
        repo: None,
        path: "a.rs",
        first: 1,
        last: 2,
        diff: Some(DiffContext { side: Side::Old, base: "" }),
        quoted: &quoted,
        text: "t",
    };
    assert_eq!(compose(&[part]), "[comment on a.rs:1-2 · old]\n> a\n> b\nt");
}

#[test]
fn comments_are_joined_by_a_blank_line() {
    let parts = [plain("a.rs", 1, 1, "one"), plain("b.rs", 2, 2, "two")];
    assert_eq!(compose(&parts), "[comment on a.rs:1]\none\n\n[comment on b.rs:2]\ntwo");
}

#[test]
fn long_quotes_are_cut() {
    let quoted: Vec<String> = (1..=15).map(|n| format!("l{n}")).collect();
    let part = MessagePart {
        repo: None,
        path: "a.rs",
        first: 1,
        last: 15,
        diff: Some(DiffContext { side: Side::New, base: "" }),
        quoted: &quoted,
        text: "t",
    };
    let message = compose(&[part]);
    assert!(message.contains("> l12\n> (3 more lines)\nt"));
    assert!(!message.contains("l13"));
}

fn paste(message: &str) -> String {
    let bytes = bracketed_paste(message);
    let text = String::from_utf8(bytes).unwrap_or_default();
    text.strip_prefix("\x1b[200~")
        .and_then(|rest| rest.strip_suffix("\x1b[201~"))
        .unwrap_or("MISSING WRAPPER")
        .to_string()
}

#[test]
fn paste_wraps() {
    assert_eq!(bracketed_paste("hi"), b"\x1b[200~hi\x1b[201~".to_vec());
}

#[test]
fn paste_strips_early_end_and_controls() {
    assert_eq!(paste("a\x1b[201~b"), "a[201~b");
    assert_eq!(paste("a\x07b\x00c\x08d\x7fe\u{85}f\u{9b}g"), "abcdefg");
}

#[test]
fn paste_keeps_text_and_normalizes_newlines() {
    assert_eq!(paste("a\tb\nc é 日本"), "a\tb\nc é 日本");
    assert_eq!(paste("a\r\nb\rc"), "a\nb\nc");
}

fn q(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| (*l).to_string()).collect()
}

#[test]
fn place_same_even_with_whitespace_changes() {
    assert!(matches!(place(&q(&["a b", ""]), 2, &["x", "a  b", "  "]), Placement::Same));
}

#[test]
fn place_moved_up_and_down() {
    let file = ["p", "a", "b", "q"];
    assert!(matches!(place(&q(&["a", "b"]), 3, &file), Placement::Moved { first: 2, last: 3 }));
    assert!(matches!(place(&q(&["a", "b"]), 1, &file), Placement::Moved { first: 2, last: 3 }));
}

#[test]
fn place_nearest_of_two() {
    let file = ["a", "x", "x", "x", "x", "a"];
    assert!(matches!(place(&q(&["a"]), 5, &file), Placement::Moved { first: 6, last: 6 }));
    // Two matches as near as each other: the earlier one.
    assert!(matches!(place(&q(&["a"]), 2, &["a", "x", "a"]), Placement::Moved { first: 1, last: 1 }));
}

#[test]
fn place_no_shared_words_is_gone() {
    assert!(matches!(place(&q(&["a", "b"]), 2, &["1", "2", "3", "4"]), Placement::Gone));
}

#[test]
fn place_file_shorter_than_anchor_with_too_few_words_is_gone() {
    assert!(matches!(place(&q(&["a", "b"]), 5, &["1"]), Placement::Gone));
    assert!(matches!(place(&q(&["a", "b", "c"]), 1, &["a"]), Placement::Gone));
}

#[test]
fn place_empty_quoted_and_first_zero() {
    assert!(matches!(place(&[], 3, &["a"]), Placement::Same));
    assert!(matches!(place(&q(&["b"]), 0, &["a", "b"]), Placement::Moved { first: 2, last: 2 }));
}
