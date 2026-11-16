//! A session's conversation carried to a new file (a move, a rebuild;
//! MV-13, D209): the mirror reads the new file from its start, skips the
//! entries it already holds, adds only what follows, and takes nothing
//! more from the machine the session left.

mod support;

use std::fs;
use std::path::Path;

use humata2_mirror::{Chunk, Lines, Mirror, Scrubber, entry_count, read_lines};
use support::*;

const OLD: &str = "ma_old";
const NEW: &str = "ma_new";

fn chunk<'a>(machine: &'a str, from: u64, lines: &'a Lines) -> Chunk<'a> {
    Chunk {
        from,
        data: &lines.data,
        next_offset: lines.next_offset,
        source: &lines.source,
        rewritten: lines.rewritten,
        machine,
    }
}

/// The host's reads of `machine`'s `file` until no answer says `more`.
fn sync(mirror: &Mirror, machine: &str, file: &Path) {
    loop {
        let (offset, source) = mirror.source_offset("se_1").unwrap();
        let lines = read_lines(file, machine, offset, 0, &source, &no_vars()).unwrap();
        let from = if lines.rewritten { 0 } else { offset };
        let taken = mirror.take("se_1", &chunk(machine, from, &lines)).unwrap();
        if taken.is_none() || !lines.more {
            return;
        }
    }
}

fn open() -> (tempfile::TempDir, Mirror) {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    mirror.describe(&meta("se_1", "ch_main", OLD, "claude")).unwrap();
    (dir, mirror)
}

/// The old machine's file, mirrored through its second answer.
fn mirrored_on_old(dir: &Path, mirror: &Mirror) -> std::path::PathBuf {
    let file = dir.join("old.jsonl");
    write_lines(&file, &[user("fix /old/place/api", &at(0)), says("done in /old/place/api", &at(1))]);
    sync(mirror, OLD, &file);
    file
}

/// `old`'s lines with `/old/place` made `/new/place`, as a move writes them.
fn carried(old: &Path, new: &Path) {
    let text = fs::read_to_string(old).unwrap().replace("/old/place", "/new/place/longer");
    fs::write(new, text).unwrap();
}

#[test]
fn a_moved_conversation_is_mirrored_once_and_what_follows_is_added() {
    let (dir, mirror) = open();
    let old = mirrored_on_old(dir.path(), &mirror);
    // The mirror lagged: the old machine wrote one more line before the move.
    append_lines(&old, &[user("one more", &at(2))]);
    let new = dir.path().join("new.jsonl");
    carried(&old, &new);

    mirror.rebase_source("se_1", NEW, None).unwrap();
    sync(&mirror, NEW, &new);
    append_lines(&new, &[says("after the move", &at(3))]);
    sync(&mirror, NEW, &new);

    let all = entries(&mirror, "se_1");
    assert_eq!(
        texts(&all),
        ["fix /old/place/api", "done in /old/place/api", "one more", "after the move"]
    );
    assert_eq!(all.iter().map(|entry| entry.seq).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert_eq!(all.iter().map(|entry| entry.turn).collect::<Vec<_>>(), [1, 1, 2, 2]);
}

#[test]
fn the_machine_a_session_left_is_no_longer_read() {
    let (dir, mirror) = open();
    let old = mirrored_on_old(dir.path(), &mirror);

    mirror.rebase_source("se_1", NEW, None).unwrap();
    append_lines(&old, &[user("written on the old machine after it left", &at(2))]);
    let (offset, source) = mirror.source_offset("se_1").unwrap();
    let stale = read_lines(&old, OLD, offset, 0, &source, &no_vars()).unwrap();

    assert_eq!(mirror.take("se_1", &chunk(OLD, 0, &stale)).unwrap(), None);
    assert_eq!(texts(&entries(&mirror, "se_1")), ["fix /old/place/api", "done in /old/place/api"]);
}

#[test]
fn a_rebuilt_conversation_skips_the_entries_it_was_written_from() {
    let (dir, mirror) = open();
    let old = mirrored_on_old(dir.path(), &mirror);
    // What the host sends a rebuild: the raw lines it kept.
    let sent = mirror.raw_conversation("se_1").unwrap().unwrap().data;
    let new = dir.path().join("rebuilt.jsonl");
    fs::write(&new, &sent).unwrap();
    append_lines(&new, &[user("what was lost?", &at(2))]);
    drop(old);

    mirror.rebase_source("se_1", NEW, Some(entry_count(&sent))).unwrap();
    sync(&mirror, NEW, &new);

    assert_eq!(
        texts(&entries(&mirror, "se_1")),
        ["fix /old/place/api", "done in /old/place/api", "what was lost?"]
    );
    let raw = mirror.raw_conversation("se_1").unwrap().unwrap().data;
    // The kept raw lines are the new file's, for a later rebuild.
    assert_eq!(entry_count(&raw), 3);
}

#[test]
fn a_rebase_done_twice_or_taken_back_mirrors_nothing_twice() {
    let (dir, mirror) = open();
    let old = mirrored_on_old(dir.path(), &mirror);

    // A host that died between the rebase and the switch rebases again.
    mirror.rebase_source("se_1", NEW, None).unwrap();
    mirror.rebase_source("se_1", NEW, None).unwrap();
    // The move failed: the session stays on the old machine.
    mirror.rebase_source("se_1", OLD, None).unwrap();
    append_lines(&old, &[user("still here", &at(2))]);
    sync(&mirror, OLD, &old);

    assert_eq!(
        texts(&entries(&mirror, "se_1")),
        ["fix /old/place/api", "done in /old/place/api", "still here"]
    );
}

#[test]
fn a_conversation_moved_onto_this_host_is_caught_up_from_its_new_file_once() {
    let (dir, mirror) = open();
    let old = mirrored_on_old(dir.path(), &mirror);
    let new = dir.path().join("here.jsonl");
    carried(&old, &new);
    append_lines(&new, &[says("resumed here", &at(2))]);

    // "" is this host: its own sessions are caught up from their files.
    mirror.rebase_source("se_1", "", None).unwrap();
    let late = read_lines(&old, OLD, 0, 0, "", &no_vars()).unwrap();
    assert_eq!(mirror.take("se_1", &chunk(OLD, 0, &late)).unwrap(), None);
    mirror.catch_up("se_1", &new, &Scrubber::default()).unwrap();
    mirror.catch_up("se_1", &new, &Scrubber::default()).unwrap();

    assert_eq!(
        texts(&entries(&mirror, "se_1")),
        ["fix /old/place/api", "done in /old/place/api", "resumed here"]
    );
}

#[test]
fn a_session_moved_off_this_host_is_no_longer_caught_up_here() {
    let (dir, mirror) = open();
    let here = dir.path().join("here.jsonl");
    write_lines(&here, &[user("on the host", &at(0))]);
    mirror.catch_up("se_1", &here, &Scrubber::default()).unwrap();
    let there = dir.path().join("there.jsonl");
    fs::copy(&here, &there).unwrap();

    mirror.rebase_source("se_1", NEW, None).unwrap();
    append_lines(&here, &[says("late on the host", &at(1))]);
    mirror.catch_up("se_1", &here, &Scrubber::default()).unwrap();
    append_lines(&there, &[says("on the box", &at(2))]);
    sync(&mirror, NEW, &there);

    assert_eq!(texts(&entries(&mirror, "se_1")), ["on the host", "on the box"]);
}

#[test]
fn entry_count_counts_what_the_mirror_would_add() {
    let (dir, mirror) = open();
    mirrored_on_old(dir.path(), &mirror);
    let raw = mirror.raw_conversation("se_1").unwrap().unwrap().data;
    assert_eq!(entry_count(&raw), entries(&mirror, "se_1").len() as u64);
    assert_eq!(entry_count(b""), 0);
}
