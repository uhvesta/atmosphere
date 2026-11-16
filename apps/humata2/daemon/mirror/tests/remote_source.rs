//! Another machine's conversation reaching the host's mirror (D189): the
//! machine reads and scrubs its file from the offset the host saved
//! (`read_lines`), and the host adds what came (`Mirror::take`).

mod support;

use std::fs;
use std::path::Path;

use humata2_mirror::{Chunk, Lines, MAX_CHUNK, Mirror, Scrubber, read_lines};
use serde_json::json;
use support::*;

const MACHINE: &str = "ma_box";

/// What the host does for one trigger: asks the machine from the saved
/// offset and takes every answer until none says `more`. Returns the
/// answers.
fn sync(mirror: &Mirror, session: &str, file: &Path, scrubber: &Scrubber) -> Vec<Lines> {
    let mut answers = Vec::new();
    loop {
        let (offset, source) = mirror.source_offset(session).unwrap();
        let lines = read_lines(file, MACHINE, offset, 0, &source, scrubber).unwrap();
        if lines.rewritten {
            let taken = mirror.take(session, &chunk(0, &lines)).unwrap();
            assert!(taken.is_some(), "a rewritten file's first chunk is never stale");
        } else {
            let taken = mirror.take(session, &chunk(offset, &lines)).unwrap();
            assert!(taken.is_some(), "a chunk from the saved offset is never stale");
        }
        let more = lines.more;
        if more {
            assert!(
                lines.next_offset > if lines.rewritten { 0 } else { offset },
                "an answer with more must have moved"
            );
        }
        answers.push(lines);
        if !more {
            return answers;
        }
    }
}

fn chunk(from: u64, lines: &Lines) -> Chunk<'_> {
    Chunk {
        from,
        data: &lines.data,
        next_offset: lines.next_offset,
        source: &lines.source,
        rewritten: lines.rewritten,
        machine: MACHINE,
    }
}

fn open() -> (tempfile::TempDir, Mirror) {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    mirror
        .describe(&meta("se_1", "ch_main", MACHINE, "claude"))
        .unwrap();
    (dir, mirror)
}

#[test]
fn what_the_machine_wrote_is_added_once_and_only_what_is_new_follows() {
    let (dir, mirror) = open();
    let file = dir.path().join("talk.jsonl");
    write_lines(&file, &[user("first prompt", &at(0)), says("first answer", &at(1))]);

    sync(&mirror, "se_1", &file, &no_vars());
    sync(&mirror, "se_1", &file, &no_vars());
    append_lines(&file, &[user("second prompt", &at(2))]);
    sync(&mirror, "se_1", &file, &no_vars());

    let all = entries(&mirror, "se_1");
    assert_eq!(texts(&all), ["first prompt", "first answer", "second prompt"]);
    assert_eq!(all.iter().map(|entry| entry.seq).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(all.iter().map(|entry| entry.turn).collect::<Vec<_>>(), [1, 1, 2]);
}

#[test]
fn a_chunk_that_does_not_start_where_the_mirror_stopped_is_dropped() {
    let (dir, mirror) = open();
    let file = dir.path().join("talk.jsonl");
    write_lines(&file, &[user("only prompt", &at(0))]);
    let lines = read_lines(&file, MACHINE, 0, 0, "", &no_vars()).unwrap();

    assert_eq!(mirror.take("se_1", &chunk(0, &lines)).unwrap(), Some(1));
    assert_eq!(mirror.take("se_1", &chunk(0, &lines)).unwrap(), None);

    assert_eq!(texts(&entries(&mirror, "se_1")), ["only prompt"]);
}

#[test]
fn a_half_written_last_line_waits_until_it_is_finished() {
    let (dir, mirror) = open();
    let file = dir.path().join("talk.jsonl");
    let finished = says("whole answer", &at(1));
    write_lines(&file, &[user("a prompt", &at(0))]);
    append_bytes(&file, &finished.as_bytes()[..20]);

    sync(&mirror, "se_1", &file, &no_vars());
    assert_eq!(texts(&entries(&mirror, "se_1")), ["a prompt"]);

    append_bytes(&file, format!("{}\n", &finished[20..]).as_bytes());
    sync(&mirror, "se_1", &file, &no_vars());
    assert_eq!(texts(&entries(&mirror, "se_1")), ["a prompt", "whole answer"]);
}

#[test]
fn a_truncated_file_keeps_the_old_entries_and_adds_the_new_ones() {
    let (dir, mirror) = open();
    let file = dir.path().join("talk.jsonl");
    write_lines(
        &file,
        &[
            user("an old question that is long enough to matter", &at(0)),
            says("an old answer that is long enough to matter", &at(1)),
        ],
    );
    sync(&mirror, "se_1", &file, &no_vars());

    // The same file, cut and started again with something shorter.
    write_lines(&file, &[user("new", &at(5))]);
    let answers = sync(&mirror, "se_1", &file, &no_vars());

    assert!(answers[0].rewritten);
    let all = entries(&mirror, "se_1");
    assert_eq!(
        texts(&all),
        [
            "an old question that is long enough to matter",
            "an old answer that is long enough to matter",
            "new"
        ]
    );
    assert_eq!(all.iter().map(|entry| entry.seq).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(all.last().map(|entry| entry.turn), Some(2));

    // And from there on only what is appended.
    append_lines(&file, &[says("after", &at(6))]);
    sync(&mirror, "se_1", &file, &no_vars());
    assert_eq!(entries(&mirror, "se_1").len(), 4);
}

#[test]
fn a_replaced_file_keeps_the_old_entries_even_when_it_is_longer() {
    let (dir, mirror) = open();
    let file = dir.path().join("talk.jsonl");
    write_lines(&file, &[user("old prompt", &at(0))]);
    sync(&mirror, "se_1", &file, &no_vars());

    // Written beside it and moved over it, so it is another file; it is
    // longer than the offset the host saved, so only its identity says so.
    let other = dir.path().join("other.jsonl");
    write_lines(
        &other,
        &[
            user("replacement prompt", &at(3)),
            says("replacement answer with plenty of words", &at(4)),
        ],
    );
    fs::rename(&other, &file).unwrap();
    let answers = sync(&mirror, "se_1", &file, &no_vars());

    assert!(answers[0].rewritten);
    assert_eq!(
        texts(&entries(&mirror, "se_1")),
        [
            "old prompt",
            "replacement prompt",
            "replacement answer with plenty of words"
        ]
    );
}

#[test]
fn a_line_over_a_mebibyte_does_not_stop_the_lines_after_it() {
    let (dir, mirror) = open();
    let file = dir.path().join("talk.jsonl");
    // One huge string: it is cut, and the line is kept.
    let huge = "x".repeat(2 * MAX_CHUNK);
    // Many strings each under the cut: the line stays too big and is left out.
    let spread: Vec<String> = (0..40).map(|n| format!("{n}{}", "y".repeat(30_000))).collect();
    write_lines(
        &file,
        &[
            user("before", &at(0)),
            says(&huge, &at(1)),
            assistant(spread.iter().map(|part| text(part)).collect(), &at(2)),
            user("after", &at(3)),
        ],
    );

    let answers = sync(&mirror, "se_1", &file, &no_vars());

    let all = entries(&mirror, "se_1");
    assert_eq!(all.first().map(|entry| entry.text.as_str()), Some("before"));
    assert_eq!(all.last().map(|entry| entry.text.as_str()), Some("after"));
    let cut = all.iter().find(|entry| entry.text.starts_with("xxxx")).expect("the cut line is kept");
    assert!(cut.text.len() < 70 * 1024, "{} bytes", cut.text.len());
    assert!(all.iter().all(|entry| !entry.text.starts_with("0yyy")), "the line that stayed too big is left out");
    assert_eq!(answers.iter().map(|answer| answer.left_out.len()).sum::<usize>(), 1);
    for answer in &answers {
        assert!(answer.data.len() <= MAX_CHUNK);
    }
}

#[test]
fn a_long_file_comes_in_answers_of_at_most_a_mebibyte_that_each_move_on() {
    let (dir, mirror) = open();
    let file = dir.path().join("talk.jsonl");
    let lines: Vec<String> = (0..60)
        .map(|n| user(&format!("prompt {n} {}", "z".repeat(40_000)), &at(n)))
        .collect();
    write_lines(&file, &lines);

    let answers = sync(&mirror, "se_1", &file, &no_vars());

    assert!(answers.len() > 1, "{} answers", answers.len());
    for answer in &answers {
        assert!(answer.data.len() <= MAX_CHUNK);
    }
    let all = entries(&mirror, "se_1");
    assert_eq!(all.len(), 60);
    assert_eq!(all.iter().map(|entry| entry.seq).collect::<Vec<_>>(), (0..60).collect::<Vec<u64>>());
}

#[test]
fn a_value_is_replaced_in_a_string_that_json_escapes_in_a_key_and_in_a_number() {
    let scrubber = Scrubber::new([
        ("QUOTED".to_owned(), "pa\"ss\\word".to_owned()),
        ("KEY".to_owned(), "sk-live-key".to_owned()),
        ("PIN".to_owned(), "424242".to_owned()),
    ]);
    let (dir, mirror) = open();
    let file = dir.path().join("talk.jsonl");
    write_lines(
        &file,
        &[
            user("the password is pa\"ss\\word, keep it", &at(0)),
            assistant(
                vec![
                    tool_use("t1", "Config", json!({"sk-live-key": true})),
                    tool_use("t2", "Config", json!({"pin": 424242})),
                ],
                &at(1),
            ),
            "not json but mentions sk-live-key".to_owned(),
        ],
    );

    let answers = sync(&mirror, "se_1", &file, &scrubber);

    let sent: Vec<u8> = answers.iter().flat_map(|answer| answer.data.clone()).collect();
    for secret in ["pa\\\"ss\\\\word", "pa\"ss\\word", "sk-live-key", "424242"] {
        assert!(
            !String::from_utf8_lossy(&sent).contains(secret),
            "{secret} crossed the link"
        );
    }
    let mut names: Vec<String> = answers.iter().flat_map(|answer| answer.scrubbed_names.clone()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names, ["KEY", "PIN", "QUOTED"]);
    let all = entries(&mirror, "se_1");
    assert_eq!(all[0].text, "the password is ‹QUOTED›, keep it");
    assert_eq!(all[1].text, "Config {\"‹KEY›\":true}");
    assert_eq!(all[2].text, "Config {\"pin\":\"‹PIN›\"}");
    assert_eq!(all.len(), 3, "the line that isn't JSON is left out");
}

#[test]
fn a_missing_file_gives_nothing_and_stays_where_it_was() {
    let dir = tempfile::tempdir().unwrap();

    let lines = read_lines(&dir.path().join("none.jsonl"), MACHINE, 77, 0, "src", &no_vars()).unwrap();

    assert!(lines.data.is_empty() && !lines.more && !lines.rewritten);
    assert_eq!(lines.next_offset, 77);
}
