//! Reading a session's mirrored entries by turn range, a page at a time
//! (LD-25).

mod support;

use std::path::Path;

use humata2_mirror::{Mirror, MirrorEntry, MirrorError, PageAsk, Role};
use serde_json::json;
use support::*;

/// One session, `se_1` (seq: turn): 0: 0; 1 to 4: 1; 5 and 6: 2; 7 and 8: 3.
fn session(dir: &Path) -> Mirror {
    let mirror = Mirror::open(&dir.join("state")).unwrap();
    mirror_session(
        &mirror,
        dir,
        "se_1",
        &[
            says("Resuming where we stopped.", &at(0)),
            user("first prompt", &at(1)),
            assistant(
                vec![tool_use("toolu_1", "Read", json!({"file_path": "/a"}))],
                &at(2),
            ),
            user_blocks(vec![tool_result("toolu_1", "contents of a")], &at(3)),
            says("first answer", &at(4)),
            user("second prompt", &at(5)),
            says("second answer", &at(6)),
            user("third prompt", &at(7)),
            says("third answer", &at(8)),
        ],
    );
    mirror
}

fn seqs(entries: &[MirrorEntry]) -> Vec<u64> {
    entries.iter().map(|entry| entry.seq).collect()
}

#[test]
fn a_session_reads_back_whole_and_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = session(dir.path());

    let page = mirror
        .read("se_1", None, None, &PageAsk::default())
        .unwrap();

    assert_eq!(page.next_token, "");
    assert_eq!(seqs(&page.items), (0..9).collect::<Vec<u64>>());
    assert_eq!(
        page.items[0],
        MirrorEntry {
            seq: 0,
            turn: 0,
            role: Role::Agent,
            text: "Resuming where we stopped.".to_owned(),
            at: Some(at(0)),
        }
    );
    assert_eq!(
        page.items[3],
        MirrorEntry {
            seq: 3,
            turn: 1,
            role: Role::Tool,
            text: "contents of a".to_owned(),
            at: Some(at(3)),
        }
    );
    assert_eq!(
        texts(&page.items),
        vec![
            "Resuming where we stopped.",
            "first prompt",
            r#"Read {"file_path":"/a"}"#,
            "contents of a",
            "first answer",
            "second prompt",
            "second answer",
            "third prompt",
            "third answer",
        ]
    );
}

#[test]
fn a_turn_range_returns_just_those_turns() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = session(dir.path());
    let range = |first, last| seqs(&entries_in(&mirror, "se_1", first, last));

    assert_eq!(range(Some(2), Some(2)), vec![5, 6]);
    assert_eq!(range(Some(1), Some(2)), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(range(Some(2), None), vec![5, 6, 7, 8]);
    assert_eq!(range(None, Some(1)), vec![0, 1, 2, 3, 4]);
    assert_eq!(range(Some(0), Some(0)), vec![0]);
    assert_eq!(range(None, Some(0)), vec![0]);
    assert_eq!(range(Some(3), Some(9)), vec![7, 8]);
    assert_eq!(range(Some(4), None), Vec::<u64>::new());
    assert_eq!(range(Some(3), Some(2)), Vec::<u64>::new());
}

#[test]
fn a_hits_turn_reads_back_with_the_matching_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = session(dir.path());

    let hits = find(&mirror, "contents");
    assert_eq!(hits.len(), 1);
    let turn = entries_in(&mirror, "se_1", Some(hits[0].turn), Some(hits[0].turn));

    let matching = turn.iter().find(|entry| entry.seq == hits[0].seq).unwrap();
    assert_eq!(matching.text, "contents of a");
    assert_eq!(seqs(&turn), vec![1, 2, 3, 4]);
}

#[test]
fn pages_cover_every_entry_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = session(dir.path());

    for (size, expected_pages) in [
        (1, vec![1; 9]),
        (2, vec![2, 2, 2, 2, 1]),
        (3, vec![3, 3, 3]),
        (4, vec![4, 4, 1]),
        (9, vec![9]),
        (20, vec![9]),
    ] {
        let mut ask = PageAsk {
            size,
            token: String::new(),
        };
        let mut pages = Vec::new();
        let mut read = Vec::new();
        loop {
            let page = mirror.read("se_1", None, None, &ask).unwrap();
            pages.push(page.items.len());
            read.extend(page.items);
            if page.next_token.is_empty() || pages.len() > 9 {
                break;
            }
            ask.token = page.next_token;
        }

        // The page that holds the last entry says so: no empty page follows.
        assert_eq!(pages, expected_pages, "pages of {size}");
        assert_eq!(seqs(&read), (0..9).collect::<Vec<u64>>(), "pages of {size}");
    }
}

#[test]
fn pages_keep_to_the_turn_range() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = session(dir.path());

    let first = mirror
        .read(
            "se_1",
            Some(1),
            Some(2),
            &PageAsk {
                size: 4,
                token: String::new(),
            },
        )
        .unwrap();
    assert_eq!(seqs(&first.items), vec![1, 2, 3, 4]);
    assert_ne!(first.next_token, "");

    let second = mirror
        .read(
            "se_1",
            Some(1),
            Some(2),
            &PageAsk {
                size: 4,
                token: first.next_token,
            },
        )
        .unwrap();
    assert_eq!(seqs(&second.items), vec![5, 6]);
    assert_eq!(second.next_token, "");
}

#[test]
fn a_page_size_of_zero_means_fifty() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    let lines: Vec<String> = (0..60)
        .map(|n| says(&format!("note {n}"), &at(n)))
        .collect();
    mirror_session(&mirror, dir.path(), "se_long", &lines);

    let first = mirror
        .read("se_long", None, None, &PageAsk::default())
        .unwrap();
    assert_eq!(seqs(&first.items), (0..50).collect::<Vec<u64>>());
    assert_ne!(first.next_token, "");

    let rest = mirror
        .read(
            "se_long",
            None,
            None,
            &PageAsk {
                size: 0,
                token: first.next_token,
            },
        )
        .unwrap();
    assert_eq!(seqs(&rest.items), (50..60).collect::<Vec<u64>>());
    assert_eq!(rest.next_token, "");
}

#[test]
fn a_page_token_the_read_did_not_hand_out_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = session(dir.path());

    for token in ["not a token", "\u{0}", "999999999999999999999999999999"] {
        let result = mirror.read(
            "se_1",
            None,
            None,
            &PageAsk {
                size: 2,
                token: token.to_owned(),
            },
        );
        assert!(
            matches!(result, Err(MirrorError::BadPageToken)),
            "{token:?} gave {result:?}"
        );
    }
}

#[test]
fn a_session_the_mirror_does_not_know_has_no_entries() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = session(dir.path());

    let page = mirror
        .read("se_unknown", None, None, &PageAsk::default())
        .unwrap();

    assert_eq!(page.items, vec![]);
    assert_eq!(page.next_token, "");
}
