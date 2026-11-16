//! The host keeps the scrubbed lines of another machine's conversation as
//! they came, so a session whose machine is gone can resume from them on
//! another machine (SL-4, flow 26.23; D195). Only sessions of other
//! machines have them: this Mac's own sessions resume from their own file.

mod support;

use std::path::Path;

use humata2_mirror::{Chunk, Lines, Mirror, Scrubber, read_lines};
use support::*;

const MACHINE: &str = "ma_box";

/// What the host does for one trigger (see remote_source.rs).
fn sync(mirror: &Mirror, session: &str, file: &Path, scrubber: &Scrubber) -> Vec<u8> {
    let mut sent = Vec::new();
    loop {
        let (offset, source) = mirror.source_offset(session).unwrap();
        let lines = read_lines(file, MACHINE, offset, 0, &source, scrubber).unwrap();
        let from = if lines.rewritten { 0 } else { offset };
        mirror.take(session, &chunk(from, &lines)).unwrap();
        sent.extend_from_slice(&lines.data);
        if !lines.more {
            return sent;
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

fn open(root: &Path) -> Mirror {
    let mirror = Mirror::open(&root.join("state")).unwrap();
    mirror.describe(&meta("se_1", "ch_main", MACHINE, "claude")).unwrap();
    mirror
}

fn raw(mirror: &Mirror, session: &str) -> Vec<u8> {
    let kept = mirror
        .raw_conversation(session)
        .unwrap()
        .unwrap_or_else(|| panic!("{session} has no raw lines kept"));
    assert_eq!(kept.agent, "claude", "the lines are named by the agent that reads them");
    kept.data
}

fn vars() -> Scrubber {
    Scrubber::new([("API_TOKEN".to_owned(), "s3cr3t-value".to_owned())])
}

#[test]
fn the_lines_from_a_machine_are_kept_byte_for_byte_as_scrubbed() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = open(dir.path());
    let file = dir.path().join("talk.jsonl");
    write_lines(&file, &[user("use s3cr3t-value please", &at(0)), says("done", &at(1))]);
    let mut sent = sync(&mirror, "se_1", &file, &vars());
    append_lines(&file, &[user("and again", &at(2))]);
    sent.extend(sync(&mirror, "se_1", &file, &vars()));
    // A repeated trigger sends nothing new and keeps nothing twice.
    sync(&mirror, "se_1", &file, &vars());

    let kept = raw(&mirror, "se_1");
    assert_eq!(kept, sent);
    let text = String::from_utf8(kept).unwrap();
    assert!(!text.contains("s3cr3t-value"), "a value never reaches the host");
    assert!(text.contains("API_TOKEN"), "its name stands in its place");
    assert_eq!(text.lines().count(), 3);
}

#[test]
fn the_kept_lines_survive_reopening_the_mirror() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("talk.jsonl");
    write_lines(&file, &[user("first", &at(0)), says("second", &at(1))]);
    let sent = {
        let mirror = open(dir.path());
        sync(&mirror, "se_1", &file, &no_vars())
    };
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    assert_eq!(raw(&mirror, "se_1"), sent);
}

#[test]
fn a_rewritten_file_keeps_only_its_new_lines_while_the_entries_keep_all() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = open(dir.path());
    let file = dir.path().join("talk.jsonl");
    write_lines(
        &file,
        &[
            user("an old question that is long enough to matter", &at(0)),
            says("an old answer that is long enough to matter", &at(1)),
        ],
    );
    sync(&mirror, "se_1", &file, &no_vars());
    write_lines(&file, &[user("compacted", &at(5))]);
    let sent = sync(&mirror, "se_1", &file, &no_vars());

    // The agent resumes the file it has now, not the one it threw away.
    assert_eq!(raw(&mirror, "se_1"), sent);
    assert_eq!(entries(&mirror, "se_1").len(), 3, "the mirror's entries lose nothing (LD-9)");
}

#[test]
fn a_session_of_this_mac_keeps_no_raw_lines() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    mirror.describe(&meta("se_local", "ch_main", "ma_host", "claude")).unwrap();
    let file = dir.path().join("local.jsonl");
    write_lines(&file, &[user("on this mac", &at(0))]);
    mirror.catch_up("se_local", &file, &no_vars()).unwrap();

    assert_eq!(entries(&mirror, "se_local").len(), 1);
    assert!(mirror.raw_conversation("se_local").unwrap().is_none());
}

/// A session mirrored before the host kept raw lines: a backfill reads the
/// machine's file again from the start for the raw lines alone, so no entry
/// is added twice, and what follows is added to both.
#[test]
fn a_backfill_fills_the_raw_lines_without_adding_entries_twice() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = open(dir.path());
    let file = dir.path().join("talk.jsonl");
    write_lines(&file, &[user("one", &at(0)), says("two", &at(1))]);
    sync(&mirror, "se_1", &file, &no_vars());

    mirror.begin_raw_backfill("se_1").unwrap();
    assert_eq!(mirror.source_offset("se_1").unwrap().0, 0, "a backfill reads from the start");
    let mut sent = sync(&mirror, "se_1", &file, &no_vars());
    assert_eq!(texts(&entries(&mirror, "se_1")), ["one", "two"]);
    append_lines(&file, &[user("three", &at(2))]);
    sent.extend(sync(&mirror, "se_1", &file, &no_vars()));

    assert_eq!(raw(&mirror, "se_1"), sent);
    assert_eq!(texts(&entries(&mirror, "se_1")), ["one", "two", "three"]);
}
