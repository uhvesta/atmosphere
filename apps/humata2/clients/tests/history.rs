//! The scripted history searches the conversations a test gives it the way
//! the host's index answers (LD-23, LD-24): every word, one hit a turn,
//! newest first, in pages.

use humata2_clients::DaemonError;
use humata2_clients::history::{FakeHistory, History, IndexedSession, SearchHit, Turn, TurnRole};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn about(session: &str) -> IndexedSession {
    IndexedSession {
        session: SessionId::new(session),
        channel: ChannelId::new("billing"),
        channel_name: "Billing".to_owned(),
        title: format!("Session {session}"),
        machine: MachineId::new("mac-studio"),
        machine_name: "Mac Studio".to_owned(),
        agent: "claude".to_owned(),
        as_of: "2026-11-15T09:00:00Z".to_owned(),
    }
}

/// Two sessions, pushed oldest first: `s-1` turns 1 and 2, then `s-2`
/// turn 1.
fn history() -> FakeHistory {
    FakeHistory::new()
        .with_turns(
            &about("s-1"),
            &[
                (1, TurnRole::User, "Why is the Ledger rollup slow?"),
                (1, TurnRole::Agent, "The ledger query scans every row."),
                (2, TurnRole::User, "Add an index for the rollup."),
            ],
        )
        .with_turns(
            &about("s-2"),
            &[(1, TurnRole::User, "Export the ledger as CSV.")],
        )
}

fn search(history: &FakeHistory, query: &str) -> Vec<SearchHit> {
    match block_on(history.search(query, "")) {
        Ok((hits, _)) => hits,
        Err(error) => panic!("searching {query:?} works: {error}"),
    }
}

/// The hits as (session, turn), in the order they came.
fn places(hits: &[SearchHit]) -> Vec<(&str, u32)> {
    hits.iter()
        .map(|hit| (hit.session.as_str(), hit.turn))
        .collect()
}

fn refused() -> DaemonError {
    DaemonError::Refused {
        what: "This search".to_owned(),
        reason: "the test said so".to_owned(),
    }
}

#[test]
fn a_search_finds_text_whatever_its_case() {
    let history = history();
    assert_eq!(places(&search(&history, "CSV")), [("s-2", 1)]);
    assert_eq!(places(&search(&history, "csv")), [("s-2", 1)]);
    assert_eq!(places(&search(&history, "INDEX")), [("s-1", 2)]);
}

#[test]
fn a_word_is_found_inside_a_longer_one() {
    let history = history();
    assert_eq!(places(&search(&history, "expor")), [("s-2", 1)]);
}

#[test]
fn every_word_of_the_query_must_be_there_in_any_order() {
    let history = history();
    // "ledger" alone is in three turns; with "csv" only one has both.
    assert_eq!(places(&search(&history, "ledger csv")), [("s-2", 1)]);
    assert_eq!(places(&search(&history, "csv   ledger")), [("s-2", 1)]);
    assert!(search(&history, "ledger kubernetes").is_empty());
}

#[test]
fn a_turn_is_one_hit_however_many_of_its_pieces_match() {
    let history = history();
    // Turn 1 of s-1 says "ledger" in the prompt and in the answer.
    let hits = search(&history, "ledger");
    let in_first_turn = hits
        .iter()
        .filter(|hit| hit.session.as_str() == "s-1" && hit.turn == 1)
        .count();
    assert_eq!(in_first_turn, 1);
    assert_eq!(hits.len(), 2);
}

#[test]
fn hits_come_newest_first() {
    let history = history();
    assert_eq!(places(&search(&history, "ledger")), [("s-2", 1), ("s-1", 1)]);
    assert_eq!(places(&search(&history, "rollup")), [("s-1", 2), ("s-1", 1)]);

    // What is said now is the newest, and clones share it.
    let other = history.clone();
    other.push_turn(&SessionId::new("s-1"), 3, TurnRole::Agent, "The ledger index is in.");
    assert_eq!(
        places(&search(&history, "ledger")),
        [("s-1", 3), ("s-2", 1), ("s-1", 1)]
    );
}

#[test]
fn a_hit_names_its_channel_session_machine_agent_and_turn() {
    let history = history();
    let hits = search(&history, "csv");
    let [hit] = hits.as_slice() else {
        panic!("one hit, not {hits:?}");
    };
    let about = about("s-2");
    assert_eq!(hit.session, about.session);
    assert_eq!(hit.channel, about.channel);
    assert_eq!(hit.channel_name, about.channel_name);
    assert_eq!(hit.title, about.title);
    assert_eq!(hit.machine, about.machine);
    assert_eq!(hit.machine_name, about.machine_name);
    assert_eq!(hit.agent, about.agent);
    assert_eq!(hit.as_of, about.as_of);
    assert_eq!(hit.turn, 1);
    assert!(hit.snippet.to_lowercase().contains("csv"));
}

#[test]
fn pages_follow_each_other_to_the_end() {
    let mut history = FakeHistory::new().with_page_size(2);
    for turn in 1..=5 {
        history = history.with_turns(
            &about("s-1"),
            &[(turn, TurnRole::User, "Check the ledger again.")],
        );
    }

    let mut seen = Vec::new();
    let mut sizes = Vec::new();
    let mut token = String::new();
    loop {
        let (hits, next) = match block_on(history.search("ledger", &token)) {
            Ok(page) => page,
            Err(error) => panic!("page {token:?} works: {error}"),
        };
        sizes.push(hits.len());
        seen.extend(hits.iter().map(|hit| hit.turn));
        if next.is_empty() {
            break;
        }
        assert_ne!(next, token, "a page's token leads on, not back to itself");
        token = next;
        assert!(sizes.len() < 10, "the pages end");
    }
    assert_eq!(sizes, [2, 2, 1]);
    assert_eq!(seen, [5, 4, 3, 2, 1]);
}

#[test]
fn a_page_token_the_fake_never_gave_is_an_error() {
    let history = history();
    assert!(block_on(history.search("ledger", "not-a-token")).is_err());
}

#[test]
fn a_query_with_no_words_is_an_error() {
    let history = history();
    assert!(block_on(history.search("", "")).is_err());
    assert!(block_on(history.search("   ", "")).is_err());
}

#[test]
fn fail_next_fails_one_call_with_the_error_given() {
    let history = history();
    history.fail_next(refused());
    assert_eq!(block_on(history.search("ledger", "")), Err(refused()));
    assert_eq!(search(&history, "ledger").len(), 2);
}

#[test]
fn a_held_search_waits_for_release_and_later_ones_answer_at_once() {
    let history = history();
    block_on(async {
        history.hold();
        let held = tokio::spawn(history.search("ledger", ""));
        // The next search isn't held: it answers while the first waits.
        let later = history.search("csv", "").await;
        assert!(matches!(&later, Ok((hits, _)) if hits.len() == 1));
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
        assert!(!held.is_finished(), "the held search waits for release");

        history.release();
        match held.await {
            Ok(Ok((hits, _))) => assert_eq!(hits.len(), 2),
            other => panic!("the held search answers once released: {other:?}"),
        }
    });
}

#[test]
fn read_gives_the_turns_asked_for_in_order() {
    let history = history();
    let session = SessionId::new("s-1");
    let texts = |turns: &[Turn]| -> Vec<(u64, u32, TurnRole)> {
        turns.iter().map(|t| (t.seq, t.turn, t.role)).collect()
    };

    let all = match block_on(history.read(&session, 0, u32::MAX)) {
        Ok(turns) => turns,
        Err(error) => panic!("reading works: {error}"),
    };
    assert_eq!(
        texts(&all),
        [
            (0, 1, TurnRole::User),
            (1, 1, TurnRole::Agent),
            (2, 2, TurnRole::User),
        ]
    );
    assert_eq!(all[2].text, "Add an index for the rollup.");

    let first = match block_on(history.read(&session, 1, 1)) {
        Ok(turns) => turns,
        Err(error) => panic!("reading works: {error}"),
    };
    assert_eq!(texts(&first), [(0, 1, TurnRole::User), (1, 1, TurnRole::Agent)]);
}

#[test]
fn reading_a_session_the_host_has_nothing_of_is_not_found() {
    let history = history();
    let result = block_on(history.read(&SessionId::new("nowhere"), 0, u32::MAX));
    assert!(matches!(result, Err(DaemonError::NotFound { .. })));
}
