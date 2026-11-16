//! Searching every mirrored session (LD-23, LD-24).

mod support;

use std::collections::HashSet;
use std::path::Path;

use humata2_mirror::{Hit, Mirror, MirrorError, PageAsk, Query, SessionMeta};
use serde_json::json;
use support::*;

fn place(session: &str, turn: u32) -> (String, u32) {
    (session.to_owned(), turn)
}

fn books() -> SessionMeta {
    SessionMeta {
        session: "se_books".to_owned(),
        channel: "ch_books".to_owned(),
        channel_name: "Bookkeeping".to_owned(),
        title: "Fix the ledger import".to_owned(),
        machine: "ma_mac".to_owned(),
        machine_name: "MacBook".to_owned(),
        agent: "claude".to_owned(),
        archived: false,
    }
}

fn ops() -> SessionMeta {
    SessionMeta {
        session: "se_ops".to_owned(),
        channel: "ch_ops".to_owned(),
        channel_name: "Operations".to_owned(),
        title: "Deploy the importer".to_owned(),
        machine: "ma_box".to_owned(),
        machine_name: "Build box".to_owned(),
        agent: "second-agent".to_owned(),
        archived: false,
    }
}

fn old() -> SessionMeta {
    SessionMeta {
        session: "se_old".to_owned(),
        channel: "ch_books".to_owned(),
        channel_name: "Bookkeeping".to_owned(),
        title: "Old migration".to_owned(),
        machine: "ma_box".to_owned(),
        machine_name: "Build box".to_owned(),
        agent: "claude".to_owned(),
        archived: true,
    }
}

/// Three sessions in two channels on two machines, one of them archived.
///
/// `se_books` (seq: text): turn 1 = 0 "Why does the ledger import fail?",
/// 1 "Looking at the ledger parser.", 2 the Read call, 3 its answer,
/// 4 "The ledger header is missing."; turn 2 = 5, 6 "Added a ledger header
/// test."; turn 3 = 7, 8 the Bash call, 9 its answer, 10.
fn library(dir: &Path) -> Mirror {
    let mirror = Mirror::open(&dir.join("state")).unwrap();
    mirror_session_as(
        &mirror,
        dir,
        &books(),
        &[
            user("Why does the ledger import fail?", &at(0)),
            assistant(
                vec![
                    text("Looking at the ledger parser."),
                    tool_use(
                        "toolu_1",
                        "Read",
                        json!({"file_path": "/work/books/parser.rs"}),
                    ),
                ],
                &at(1),
            ),
            user_blocks(
                vec![tool_result(
                    "toolu_1",
                    "the parser reads ledger rows one by one",
                )],
                &at(2),
            ),
            says("The ledger header is missing.", &at(3)),
            user("Add a test for the header", &at(4)),
            says("Added a ledger header test.", &at(5)),
            user("Now run the linter", &at(6)),
            assistant(
                vec![tool_use("toolu_2", "Bash", json!({"command": "make lint"}))],
                &at(7),
            ),
            user_blocks(
                vec![tool_result(
                    "toolu_2",
                    "lint finished: 0 warnings, quokka check passed",
                )],
                &at(8),
            ),
            says("Lint is clean.", &at(9)),
        ],
    );
    mirror_session_as(
        &mirror,
        dir,
        &ops(),
        &[
            user("Deploy the importer to staging", &at(20)),
            says("Deploying. Salt and pepper the config first.", &at(21)),
            user("Did the ledger service restart?", &at(22)),
            says("Yes, the LEDGER service restarted cleanly.", &at(23)),
        ],
    );
    mirror_session_as(
        &mirror,
        dir,
        &old(),
        &[
            user("Migrate the invoices table", &at(40)),
            says("Migrated the invoices; the ledger is untouched.", &at(41)),
        ],
    );
    mirror
}

/// Every place "ledger" is found in [`library`].
fn ledger_places() -> Vec<(String, u32)> {
    vec![
        place("se_books", 1),
        place("se_books", 2),
        place("se_old", 1),
        place("se_ops", 2),
    ]
}

/// `turns` turns in one session, each prompt asking about the invoice.
fn invoice_turns(turns: u32, first_minute: u32) -> Vec<String> {
    (0..turns)
        .flat_map(|turn| {
            [
                user(
                    &format!("question {turn} about the invoice"),
                    &at(first_minute + turn),
                ),
                says(&format!("answer {turn}"), &at(first_minute + turn)),
            ]
        })
        .collect()
}

fn keys(hits: &[Hit]) -> Vec<(String, u32, u64)> {
    hits.iter()
        .map(|hit| (hit.session.clone(), hit.turn, hit.seq))
        .collect()
}

#[test]
fn every_word_must_be_in_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());

    assert_eq!(places(&find(&mirror, "ledger")), ledger_places());
    assert_eq!(
        places(&find(&mirror, "ledger header")),
        vec![place("se_books", 1), place("se_books", 2)]
    );
    assert_eq!(
        places(&find(&mirror, "header   ledger\tmissing")),
        vec![place("se_books", 1)]
    );
    assert_eq!(find(&mirror, "ledger zeppelin"), vec![]);
}

#[test]
fn a_search_without_hits_is_an_empty_last_page() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());

    let page = mirror.search(&words("zeppelin")).unwrap();

    assert_eq!(page.items, vec![]);
    assert_eq!(page.next_token, "");
}

#[test]
fn case_does_not_matter() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());

    assert_eq!(places(&find(&mirror, "LEDGER")), ledger_places());
    assert_eq!(
        places(&find(&mirror, "Ledger Service")),
        vec![place("se_ops", 2)]
    );
    assert_eq!(
        places(&find(&mirror, "salt PEPPER")),
        vec![place("se_ops", 1)]
    );
}

#[test]
fn the_last_word_also_matches_the_start_of_a_word() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());

    assert_eq!(places(&find(&mirror, "ledg")), ledger_places());
    assert_eq!(
        places(&find(&mirror, "ledger head")),
        vec![place("se_books", 1), place("se_books", 2)]
    );
    assert_eq!(places(&find(&mirror, "quok")), vec![place("se_books", 3)]);
}

#[test]
fn words_before_the_last_must_be_whole() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());

    assert_eq!(find(&mirror, "head ledger"), vec![]);
    assert_eq!(find(&mirror, "ledg header"), vec![]);
}

#[test]
fn query_syntax_is_plain_text_and_never_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());

    for odd in [
        "\"",
        "AND(",
        "ledger\"",
        "\"ledger",
        "(ledger",
        "ledger)",
        "*",
        "ledger*",
        "^ledger",
        "-",
        "ledger -header",
        "ledger:header",
        "NEAR(ledger header)",
        "NEAR(ledger header, 3)",
        "ledger AND",
        "OR",
        "NOT ledger",
        "{ledger}",
        "ledger + header",
        "'ledger'",
        "ledger \\",
        "%",
        "_",
        "ledger;",
        "[ledger]",
        "led\0ger",
        "\"\"",
        "a\" OR \"b",
    ] {
        let page = mirror.search(&words(odd));
        assert!(page.is_ok(), "{odd:?} failed: {page:?}");
    }

    // With their query-language meaning, each of these would find the
    // salt-and-pepper entry or every ledger entry.
    assert_eq!(find(&mirror, "pepper OR zeppelin"), vec![]);
    assert_eq!(find(&mirror, "pepper NOT cumin"), vec![]);
    assert_eq!(find(&mirror, "ledger OR zeppelin"), vec![]);
    assert_eq!(find(&mirror, "text:ledger"), vec![]);
    // And each of these would find less, or fail.
    assert_eq!(
        places(&find(&mirror, "\"salt pepper\"")),
        vec![place("se_ops", 1)]
    );
    assert_eq!(places(&find(&mirror, "AND(")), vec![place("se_ops", 1)]);
    assert_eq!(places(&find(&mirror, "(ledger)")), ledger_places());
    assert_eq!(places(&find(&mirror, "ledger*")), ledger_places());
    assert_eq!(
        places(&find(&mirror, "ledger -header")),
        vec![place("se_books", 1), place("se_books", 2)]
    );
}

#[test]
fn a_search_for_nothing_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());

    for nothing in ["", "   ", " \t\n "] {
        let result = mirror.search(&words(nothing));
        assert!(
            matches!(result, Err(MirrorError::EmptyQuery)),
            "{nothing:?} gave {result:?}"
        );
    }
}

#[test]
fn a_turn_is_one_hit_however_many_of_its_entries_match() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());

    let hits = find(&mirror, "ledger");

    // Turn 1 of se_books says "ledger" in four entries, turn 2 of se_ops in
    // two.
    assert_eq!(hits.len(), 4);
    assert_eq!(places(&hits), ledger_places());
    let seq_of = |session: &str, turn: u32| {
        hits.iter()
            .find(|hit| hit.session == session && hit.turn == turn)
            .unwrap()
            .seq
    };
    assert!([0, 1, 3, 4].contains(&seq_of("se_books", 1)));
    assert_eq!(seq_of("se_books", 2), 6);
    assert!([2, 3].contains(&seq_of("se_ops", 2)));
    assert_eq!(seq_of("se_old", 1), 1);
}

#[test]
fn a_hit_names_its_session_channel_machine_agent_and_turn() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());

    let hits = find(&mirror, "quokka");

    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert!(!hit.as_of.is_empty());
    assert_eq!(
        *hit,
        Hit {
            session: "se_books".to_owned(),
            channel: "ch_books".to_owned(),
            channel_name: "Bookkeeping".to_owned(),
            title: "Fix the ledger import".to_owned(),
            machine: "ma_mac".to_owned(),
            machine_name: "MacBook".to_owned(),
            agent: "claude".to_owned(),
            turn: 3,
            seq: 9,
            snippet: "lint finished: 0 warnings, quokka check passed".to_owned(),
            at: Some(at(8)),
            as_of: hit.as_of.clone(),
        }
    );
}

#[test]
fn tool_calls_and_answers_are_searchable() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());

    assert_eq!(
        keys(&find(&mirror, "make lint")),
        vec![("se_books".to_owned(), 3, 8)]
    );
    assert_eq!(
        keys(&find(&mirror, "rows one by one")),
        vec![("se_books".to_owned(), 1, 3)]
    );
}

#[test]
fn filters_narrow_by_channel_machine_and_agent() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());
    let ledger = |channels: &[&str], machine: Option<&str>, agent: Option<&str>| {
        let query = Query {
            channels: channels
                .iter()
                .map(|channel| (*channel).to_owned())
                .collect(),
            machine: machine.map(str::to_owned),
            agent: agent.map(str::to_owned),
            ..words("ledger")
        };
        places(&all_hits(&mirror, &query))
    };

    assert_eq!(ledger(&[], None, None), ledger_places());
    assert_eq!(ledger(&["ch_ops"], None, None), vec![place("se_ops", 2)]);
    assert_eq!(
        ledger(&["ch_books"], None, None),
        vec![
            place("se_books", 1),
            place("se_books", 2),
            place("se_old", 1)
        ]
    );
    assert_eq!(ledger(&["ch_ops", "ch_books"], None, None), ledger_places());
    assert_eq!(
        ledger(&[], Some("ma_box"), None),
        vec![place("se_old", 1), place("se_ops", 2)]
    );
    assert_eq!(
        ledger(&[], None, Some("second-agent")),
        vec![place("se_ops", 2)]
    );
    assert_eq!(
        ledger(&[], Some("ma_box"), Some("claude")),
        vec![place("se_old", 1)]
    );
    assert_eq!(
        ledger(&["ch_books"], Some("ma_mac"), Some("claude")),
        vec![place("se_books", 1), place("se_books", 2)]
    );
    assert_eq!(ledger(&["ch_ops"], Some("ma_mac"), None), vec![]);
    assert_eq!(ledger(&["ch_elsewhere"], None, None), vec![]);
    // Filters take ids, not the names shown.
    assert_eq!(ledger(&["Bookkeeping"], None, None), vec![]);
    assert_eq!(ledger(&[], Some("MacBook"), None), vec![]);
}

#[test]
fn a_time_range_narrows_by_when_an_entry_was_written() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());
    let ledger = |since: Option<&str>, until: Option<&str>| {
        let query = Query {
            since: since.map(str::to_owned),
            until: until.map(str::to_owned),
            ..words("ledger")
        };
        places(&all_hits(&mirror, &query))
    };
    // se_books was written 09:00 to 09:09, se_ops 09:20 to 09:23 and
    // se_old 09:40 to 09:41.
    let (quarter_past, half_past) = ("2026-11-10T09:15:00.000Z", "2026-11-10T09:30:00.000Z");

    assert_eq!(
        ledger(Some(quarter_past), None),
        vec![place("se_old", 1), place("se_ops", 2)]
    );
    assert_eq!(
        ledger(None, Some(quarter_past)),
        vec![place("se_books", 1), place("se_books", 2)]
    );
    assert_eq!(
        ledger(Some(quarter_past), Some(half_past)),
        vec![place("se_ops", 2)]
    );
    assert_eq!(ledger(Some("2026-11-11T00:00:00.000Z"), None), vec![]);
    assert_eq!(ledger(None, Some("2026-11-09T00:00:00.000Z")), vec![]);
}

#[test]
fn archived_sessions_are_still_found() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());

    // se_old was archived before it was mirrored; se_ops is archived now.
    assert_eq!(places(&find(&mirror, "invoices")), vec![place("se_old", 1)]);
    mirror
        .describe(&SessionMeta {
            archived: true,
            ..ops()
        })
        .unwrap();

    assert_eq!(places(&find(&mirror, "staging")), vec![place("se_ops", 1)]);
    assert_eq!(places(&find(&mirror, "ledger")), ledger_places());
}

#[test]
fn describing_a_session_again_changes_what_its_hits_say() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());
    let in_channel = |channel: &str| {
        let query = Query {
            channels: vec![channel.to_owned()],
            ..words("staging")
        };
        places(&all_hits(&mirror, &query))
    };
    assert_eq!(in_channel("ch_ops"), vec![place("se_ops", 1)]);

    // The session is renamed and moved to another channel and machine.
    mirror
        .describe(&SessionMeta {
            channel: "ch_books".to_owned(),
            channel_name: "Books and taxes".to_owned(),
            title: "Ship the importer".to_owned(),
            machine: "ma_mac".to_owned(),
            machine_name: "MacBook".to_owned(),
            ..ops()
        })
        .unwrap();

    let hits = find(&mirror, "staging");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].channel, "ch_books");
    assert_eq!(hits[0].channel_name, "Books and taxes");
    assert_eq!(hits[0].title, "Ship the importer");
    assert_eq!(hits[0].machine, "ma_mac");
    assert_eq!(hits[0].machine_name, "MacBook");
    assert_eq!(hits[0].agent, "second-agent");
    assert_eq!(in_channel("ch_ops"), vec![]);
    assert_eq!(in_channel("ch_books"), vec![place("se_ops", 1)]);
}

#[test]
fn a_session_described_after_it_was_mirrored_is_named_in_its_hits() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    let file = dir.path().join("talk.jsonl");
    write_lines(&file, &[user("where is the lighthouse?", &at(0))]);

    mirror.catch_up("se_books", &file, &no_vars()).unwrap();
    mirror.describe(&books()).unwrap();

    let hits = find(&mirror, "lighthouse");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].session, "se_books");
    assert_eq!(hits[0].channel, "ch_books");
    assert_eq!(hits[0].channel_name, "Bookkeeping");
    assert_eq!(hits[0].title, "Fix the ledger import");
    assert_eq!(hits[0].machine_name, "MacBook");
    assert_eq!(hits[0].agent, "claude");
}

#[test]
fn the_better_match_comes_first() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    let filler = |from: u32| {
        (from..from + 150)
            .map(|n| format!("w{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let buried = format!("{} ledger {}", filler(0), filler(1000));
    // The entry that is all about the word is the older one, so it isn't
    // first for being newer.
    mirror_session(&mirror, dir.path(), "se_exact", &[user("ledger", &at(0))]);
    mirror_session(
        &mirror,
        dir.path(),
        "se_unrelated",
        &[
            user("water the plants", &at(40)),
            says("watered the ferns", &at(41)),
            user("feed the cat", &at(42)),
            says("the cat is fed", &at(43)),
        ],
    );
    mirror_session(&mirror, dir.path(), "se_buried", &[user(&buried, &at(30))]);

    let hits = find(&mirror, "ledger");

    let sessions: Vec<&str> = hits.iter().map(|hit| hit.session.as_str()).collect();
    assert_eq!(sessions, vec!["se_exact", "se_buried"]);
}

#[test]
fn equal_matches_come_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    // Mirrored in an order that is neither oldest nor newest first.
    mirror_session(
        &mirror,
        dir.path(),
        "se_middle",
        &[user("the ledger is balanced", &at(20))],
    );
    mirror_session(
        &mirror,
        dir.path(),
        "se_newest",
        &[user("the ledger is balanced", &at(30))],
    );
    mirror_session(
        &mirror,
        dir.path(),
        "se_oldest",
        &[user("the ledger is balanced", &at(10))],
    );

    let hits = find(&mirror, "ledger");

    let sessions: Vec<&str> = hits.iter().map(|hit| hit.session.as_str()).collect();
    assert_eq!(sessions, vec!["se_newest", "se_middle", "se_oldest"]);
}

#[test]
fn a_long_entry_is_cut_to_a_snippet_around_the_match() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    let filler = |from: u32| {
        (from..from + 300)
            .map(|n| format!("w{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let middle = format!("{} the quokka sat here {}", filler(0), filler(1000));
    let start = format!("narwhal first of all {}", filler(2000));
    mirror_session(&mirror, dir.path(), "se_middle", &[user(&middle, &at(0))]);
    mirror_session(&mirror, dir.path(), "se_start", &[user(&start, &at(1))]);

    let in_middle = &find(&mirror, "quokka")[0].snippet;
    let at_start = &find(&mirror, "narwhal")[0].snippet;

    assert!(in_middle.contains("quokka"), "{in_middle:?}");
    assert!(in_middle.chars().count() <= 200, "{in_middle:?}");
    assert!(
        in_middle.starts_with('…') && in_middle.ends_with('…'),
        "{in_middle:?}"
    );
    assert!(at_start.starts_with("narwhal"), "{at_start:?}");
    assert!(at_start.chars().count() <= 200, "{at_start:?}");
    assert!(at_start.ends_with('…'), "{at_start:?}");
}

#[test]
fn pages_cover_every_hit_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    mirror_session(&mirror, dir.path(), "se_1", &invoice_turns(5, 0));
    mirror_session(&mirror, dir.path(), "se_2", &invoice_turns(2, 30));
    let whole = mirror.search(&words("invoice")).unwrap();
    assert_eq!(whole.items.len(), 7);
    assert_eq!(whole.next_token, "");

    for (size, expected_pages) in [
        (1, vec![1; 7]),
        (2, vec![2, 2, 2, 1]),
        (3, vec![3, 3, 1]),
        (7, vec![7]),
        (8, vec![7]),
    ] {
        let mut query = Query {
            page: PageAsk {
                size,
                token: String::new(),
            },
            ..words("invoice")
        };
        let mut pages = Vec::new();
        let mut hits = Vec::new();
        loop {
            let page = mirror.search(&query).unwrap();
            pages.push(page.items.len());
            hits.extend(page.items);
            if page.next_token.is_empty() || pages.len() > 7 {
                break;
            }
            query.page.token = page.next_token;
        }

        // The page that holds the last hit says so: no empty page follows.
        assert_eq!(pages, expected_pages, "pages of {size}");
        assert_eq!(keys(&hits), keys(&whole.items), "pages of {size}");
        let distinct: HashSet<(String, u32)> = places(&hits).into_iter().collect();
        assert_eq!(distinct.len(), 7, "pages of {size}");
    }
}

#[test]
fn pages_keep_to_the_querys_filters() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    let here = meta("se_here", "ch_here", "ma_mac", "claude");
    let there = meta("se_there", "ch_there", "ma_mac", "claude");
    mirror_session_as(&mirror, dir.path(), &here, &invoice_turns(5, 0));
    mirror_session_as(&mirror, dir.path(), &there, &invoice_turns(4, 30));
    let query = Query {
        channels: vec!["ch_here".to_owned()],
        page: PageAsk {
            size: 2,
            token: String::new(),
        },
        ..words("invoice")
    };

    let hits = all_hits(&mirror, &query);

    assert_eq!(
        places(&hits),
        (1..=5)
            .map(|turn| place("se_here", turn))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_page_size_of_zero_means_fifty() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = Mirror::open(&dir.path().join("state")).unwrap();
    mirror_session(&mirror, dir.path(), "se_1", &invoice_turns(60, 0));

    let first = mirror.search(&words("invoice")).unwrap();
    assert_eq!(first.items.len(), 50);
    assert_ne!(first.next_token, "");

    let rest = mirror
        .search(&Query {
            page: PageAsk {
                size: 0,
                token: first.next_token,
            },
            ..words("invoice")
        })
        .unwrap();
    assert_eq!(rest.items.len(), 10);
    assert_eq!(rest.next_token, "");
    let mut all = places(&first.items);
    all.extend(places(&rest.items));
    all.sort();
    assert_eq!(
        all,
        (1..=60).map(|turn| place("se_1", turn)).collect::<Vec<_>>()
    );
}

#[test]
fn a_page_token_the_search_did_not_hand_out_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let mirror = library(dir.path());

    for token in ["not a token", "\u{0}", "999999999999999999999999999999"] {
        let query = Query {
            page: PageAsk {
                size: 2,
                token: token.to_owned(),
            },
            ..words("ledger")
        };
        let result = mirror.search(&query);
        assert!(
            matches!(result, Err(MirrorError::BadPageToken)),
            "{token:?} gave {result:?}"
        );
    }
}
