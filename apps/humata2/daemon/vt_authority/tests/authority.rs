//! A tab's authoritative screen: fed by what the holder printed, it
//! answers terminal queries, hands out checkpoints a viewer can install,
//! and never sends one answer twice, across a restart included.

use ghostty_vt::{Grid, Terminal};
use humata2_vt_authority::{Authority, AuthorityError, Config, ReplyRecord, Step};

const LIMIT: usize = 16 << 20;

fn config() -> Config {
    Config {
        cols: 40,
        rows: 6,
        scrollback_lines: 1000,
        scrollback_bytes: 4 << 20,
        checkpoint_limit: LIMIT,
    }
}

fn replies(steps: &[Step]) -> Vec<(u64, Vec<u8>)> {
    steps
        .iter()
        .filter_map(|step| match step {
            Step::Reply { seq, bytes } => Some((*seq, bytes.clone())),
            _ => None,
        })
        .collect()
}

/// What a plain terminal shows after the same bytes.
fn reference(bytes: &[u8]) -> Terminal {
    let mut terminal = Terminal::new(40, 6, 1000).expect("terminal");
    terminal.feed(bytes);
    terminal
}

fn rows(terminal: &mut Terminal) -> Vec<String> {
    terminal.sync();
    (0..terminal.size().rows)
        .map(|y| format!("{:?}", terminal.row(y)))
        .collect()
}

fn installed(authority: &mut Authority) -> Terminal {
    let snapshot = authority.snapshot().expect("snapshot");
    assert_eq!(snapshot.engine, ghostty_vt::CHECKPOINT_FORMAT);
    Terminal::from_checkpoint(&snapshot.bytes).expect("a viewer can install it")
}

#[test]
fn the_checkpoint_shows_what_the_holder_printed() {
    let output = b"\x1b[1mhello\x1b[0m\r\nworld \x1b[31mred\x1b[0m\r\n$ ";
    let mut authority = Authority::new(&config()).expect("authority");
    authority.feed(0, &output[..9]).expect("first chunk");
    authority.feed(9, &output[9..]).expect("second chunk");

    let snapshot = authority.snapshot().expect("snapshot");
    assert_eq!(snapshot.offset, output.len() as u64);
    assert_eq!((snapshot.cols, snapshot.rows), (40, 6));
    let mut viewer = Terminal::from_checkpoint(&snapshot.bytes).expect("install");
    assert_eq!(rows(&mut viewer), rows(&mut reference(output)));
}

#[test]
fn queries_are_answered_with_nobody_watching() {
    let mut authority = Authority::new(&config()).expect("authority");
    let steps = authority.feed(0, b"ab\x1b[6n").expect("feed");
    let answers = replies(&steps);
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].1, b"\x1b[1;3R");
}

#[test]
fn device_attributes_are_answered_with_nobody_watching() {
    let mut authority = Authority::new(&config()).expect("authority");
    let first = b"ab\x1b[6n";
    authority.feed(0, first).expect("feed");
    let steps = authority
        .feed(first.len() as u64, b"\x1b[c")
        .expect("feed");
    let answers = replies(&steps);
    assert_eq!(answers.len(), 1, "device attributes get one answer");
    assert!(answers[0].1.starts_with(b"\x1b[?"));
}

#[test]
fn each_answer_has_its_own_sequence_number() {
    let mut authority = Authority::new(&config()).expect("authority");
    let steps = authority.feed(0, b"\x1b[6n\x1b[6n").expect("feed");
    let seqs: Vec<u64> = replies(&steps).into_iter().map(|(seq, _)| seq).collect();
    assert_eq!(seqs.len(), 2);
    assert!(seqs[0] < seqs[1]);
}

#[test]
fn output_seen_twice_is_not_parsed_or_answered_twice() {
    let mut authority = Authority::new(&config()).expect("authority");
    authority.feed(0, b"one\x1b[6n").expect("feed");
    // The link resends from an older offset: only the new tail counts.
    let steps = authority.feed(0, b"one\x1b[6ntwo").expect("overlap");
    assert!(replies(&steps).is_empty());
    assert_eq!(authority.end(), 10);
    let mut viewer = installed(&mut authority);
    assert_eq!(rows(&mut viewer), rows(&mut reference(b"one\x1b[6ntwo")));
}

#[test]
fn a_gap_in_the_output_is_an_error_not_a_skip() {
    let mut authority = Authority::new(&config()).expect("authority");
    authority.feed(0, b"abc").expect("feed");
    let error = authority.feed(5, b"def").expect_err("bytes 3..5 are missing");
    assert!(matches!(error, AuthorityError::Gap { expected: 3, got: 5 }));
    assert_eq!(authority.end(), 3, "nothing past the gap was taken");
}

#[test]
fn an_answer_stays_pending_until_the_holder_wrote_it() {
    let mut authority = Authority::new(&config()).expect("authority");
    let steps = authority.feed(0, b"\x1b[6n").expect("feed");
    let (seq, _) = replies(&steps)[0].clone();
    assert_eq!(authority.record().pending, vec![seq]);
    authority.written(seq);
    assert!(authority.record().pending.is_empty());
    assert_eq!(authority.record().answered_through, 4);
}

#[test]
fn a_restart_never_answers_old_queries_again() {
    let output = b"x\x1b[6ny\x1b[6nz";
    let mut before = Authority::new(&config()).expect("authority");
    for (seq, _) in replies(&before.feed(0, output).expect("feed")) {
        before.written(seq);
    }
    let record = before.record();

    let (mut after, steps) = Authority::resume(&config(), 0, output, &record).expect("resume");
    assert!(replies(&steps).is_empty(), "both queries were answered before");
    assert!(!after.input_uncertain());
    assert_eq!(after.end(), output.len() as u64);
    let mut viewer = installed(&mut after);
    assert_eq!(rows(&mut viewer), rows(&mut reference(output)));
}

#[test]
fn a_restart_answers_queries_that_arrived_after_the_last_answer() {
    let first = b"a\x1b[6n";
    let mut before = Authority::new(&config()).expect("authority");
    for (seq, _) in replies(&before.feed(0, first).expect("feed")) {
        before.written(seq);
    }
    let record = before.record();
    // The daemon stopped after the holder printed a second query but
    // before it was parsed.
    let kept = b"a\x1b[6nbc\x1b[6n";

    let (after, steps) = Authority::resume(&config(), 0, kept, &record).expect("resume");
    let answers = replies(&steps);
    assert_eq!(answers.len(), 1, "only the unanswered query");
    assert_eq!(answers[0].1, b"\x1b[1;4R");
    assert!(!after.input_uncertain());
}

#[test]
fn a_restart_with_an_answer_in_flight_marks_input_uncertain() {
    let mut before = Authority::new(&config()).expect("authority");
    before.feed(0, b"\x1b[6n").expect("feed");
    let record = before.record();
    assert_eq!(record.pending.len(), 1, "written to the pty, not yet confirmed");

    let (mut after, steps) = Authority::resume(&config(), 0, b"\x1b[6n", &record).expect("resume");
    assert!(replies(&steps).is_empty(), "never resent");
    assert!(after.input_uncertain());
    // The old connection's sequence numbers mean nothing on the new one.
    after.written(record.pending[0]);
    assert!(after.input_uncertain());

    // Uncertainty survives a second restart until a resync clears it.
    let again = after.record();
    let (mut later, _) = Authority::resume(&config(), 0, b"\x1b[6n", &again).expect("resume");
    assert!(later.input_uncertain());
    later.resynced();
    assert!(!later.input_uncertain());
    assert!(!later.record().uncertain);
}

#[test]
fn a_restart_from_trimmed_output_says_the_history_is_partial() {
    let (whole, _) = Authority::resume(&config(), 0, b"abc", &ReplyRecord::default()).expect("resume");
    assert!(!whole.history_partial());
    let (trimmed, _) =
        Authority::resume(&config(), 100, b"abc", &ReplyRecord::default()).expect("resume");
    assert!(trimmed.history_partial());
    assert_eq!(trimmed.end(), 103);
}

#[test]
fn a_resize_takes_effect_at_the_current_offset_and_shows_in_the_checkpoint() {
    let mut authority = Authority::new(&config()).expect("authority");
    authority.feed(0, b"hello").expect("feed");
    let resized = authority.resize(60, 10, 8, 16).expect("resize");
    assert_eq!((resized.cols, resized.rows), (60, 10));
    assert_eq!((resized.cell_width, resized.cell_height), (8, 16));
    assert_eq!(resized.at, 5);
    let snapshot = authority.snapshot().expect("snapshot");
    assert_eq!((snapshot.cols, snapshot.rows), (60, 10));
}

#[test]
fn an_unusable_size_is_refused_and_the_screen_keeps_its_size() {
    let mut authority = Authority::new(&config()).expect("authority");
    assert!(authority.resize(0, 10, 8, 16).is_err());
    let snapshot = authority.snapshot().expect("snapshot");
    assert_eq!((snapshot.cols, snapshot.rows), (40, 6));
}

#[test]
fn it_reports_whether_the_screen_changed_since_the_last_checkpoint() {
    let mut authority = Authority::new(&config()).expect("authority");
    assert!(authority.changed(), "no checkpoint taken yet");
    authority.snapshot().expect("snapshot");
    assert!(!authority.changed());
    authority.feed(0, b"x").expect("feed");
    assert!(authority.changed());
    authority.snapshot().expect("snapshot");
    authority.resize(50, 6, 1, 1).expect("resize");
    assert!(authority.changed());
}

#[test]
fn titles_and_folders_the_program_sets_are_reported() {
    let mut authority = Authority::new(&config()).expect("authority");
    let steps = authority
        .feed(0, b"\x1b]2;build\x07\x1b]7;file:///tmp/work\x07")
        .expect("feed");
    assert!(steps.contains(&Step::Title("build".to_owned())));
    assert!(steps.iter().any(|step| matches!(step, Step::Pwd(pwd) if pwd.contains("/tmp/work"))));
}

#[test]
fn a_checkpoint_over_the_limit_trims_the_oldest_history_and_says_how_many_rows() {
    let history: Vec<u8> = (0..300)
        .flat_map(|n| format!("row {n:04} {}\r\n", "x".repeat(30)).into_bytes())
        .collect();
    let snapshot_under = |limit: usize, bytes: &[u8]| {
        let mut authority = Authority::new(&Config {
            checkpoint_limit: limit,
            ..config()
        })
        .expect("authority");
        authority.feed(0, bytes).expect("feed");
        authority.snapshot()
    };
    let full = snapshot_under(LIMIT, &history).expect("full").bytes.len();
    let bare = snapshot_under(LIMIT, b"row 0299\r\n").expect("bare").bytes.len();
    let limit = (full + bare) / 2;

    let snapshot = snapshot_under(limit, &history).expect("a trimmed checkpoint");

    assert!(snapshot.bytes.len() <= limit);
    assert!(snapshot.trimmed_rows > 0);
    let restored = Terminal::from_checkpoint(&snapshot.bytes).expect("restore");
    assert!(restored.scrollback_rows() + snapshot.trimmed_rows >= 290);
}

#[test]
fn a_checkpoint_over_the_limit_is_an_error() {
    let small = Config {
        checkpoint_limit: 64,
        ..config()
    };
    let mut authority = Authority::new(&small).expect("authority");
    authority.feed(0, b"hello").expect("feed");
    assert!(matches!(
        authority.snapshot(),
        Err(AuthorityError::Engine { .. })
    ));
}

#[test]
fn the_screen_text_is_what_the_active_area_shows_now_without_history() {
    let mut authority = Authority::new(&config()).expect("authority");
    // Ten lines on a six-row screen: the first four scroll into history.
    let lines: String = (0..10).map(|n| format!("line {n}\r\n")).collect();
    authority.feed(0, lines.as_bytes()).expect("fed");
    let text = authority.screen_text().expect("text");
    assert!(text.contains("line 9"), "{text}");
    assert!(!text.contains("line 3"), "{text}");

    // A program that clears and redraws leaves only the new drawing.
    let end = authority.end();
    authority.feed(end, b"\x1b[2J\x1b[Hasked: trust?").expect("fed");
    let text = authority.screen_text().expect("text");
    assert!(text.contains("asked: trust?"), "{text}");
    assert!(!text.contains("line 9"), "{text}");
}
