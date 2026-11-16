//! `humata sessions search` and `humata session read` (LD-23–LD-25): the
//! filters are sent as given, every page is gathered into one reply, and a
//! refusal exits with its kind's code.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use pb::error::Kind;
use serde_json::json;
use support::{FakeConsole, FakeDaemon, run};

fn hit(session: &str, turn: u32, snippet: &str) -> pb::SearchHit {
    pb::SearchHit {
        session: support::session(session),
        session_title: "Fix login".into(),
        channel: support::channel("ch_pay"),
        channel_name: "payments".into(),
        machine: support::machine("ma_laptop"),
        machine_name: "laptop".into(),
        agent: "claude".into(),
        turn,
        snippet: snippet.into(),
        ..Default::default()
    }
}

fn hits_page(hits: Vec<pb::SearchHit>, next: &str) -> pb::SearchSessionsResponse {
    pb::SearchSessionsResponse {
        hits,
        page: Some(pb::PageResponse {
            next_page_token: next.into(),
        }),
    }
}

fn entry(seq: u64, turn: u32, text: &str) -> pb::MirrorEntry {
    pb::MirrorEntry {
        seq,
        turn,
        role: pb::mirror_entry::Role::Agent as i32,
        text: text.into(),
        ..Default::default()
    }
}

fn entries_page(entries: Vec<pb::MirrorEntry>, next: &str) -> pb::ReadTurnsResponse {
    pb::ReadTurnsResponse {
        entries,
        page: Some(pb::PageResponse {
            next_page_token: next.into(),
        }),
    }
}

fn tokens<R>(sent: &[R], page: impl Fn(&R) -> Option<pb::PageRequest>) -> Vec<String> {
    sent.iter()
        .map(|request| page(request).unwrap_or_default().page_token)
        .collect()
}

#[tokio::test]
async fn search_sends_the_words_joined_and_every_filter() {
    let daemon = FakeDaemon::new();
    daemon
        .reply(
            rpc::channel::LIST_CHANNELS,
            &pb::ListChannelsResponse {
                channels: vec![pb::Channel {
                    id: support::channel("ch_pay"),
                    name: "payments".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .reply(rpc::history::SEARCH_SESSIONS, &hits_page(vec![], ""));
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &[
            "--json",
            "sessions",
            "search",
            "retry",
            "backoff",
            "--channel",
            "payments",
            "--machine",
            "ma_laptop",
            "--agent",
            "claude",
            "--since",
            "2026-11-01T00:00:00Z",
            "--until",
            "2026-11-15T12:30:00Z",
        ],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::SearchSessionsRequest = daemon.only(rpc::history::SEARCH_SESSIONS);
    assert_eq!(sent.query, "retry backoff");
    assert_eq!(sent.channel, support::channel("ch_pay"));
    assert_eq!(sent.machine, support::machine("ma_laptop"));
    assert_eq!(sent.agent, "claude");
    let since = sent.since.unwrap();
    assert_eq!((since.seconds, since.nanos), (1_793_491_200, 0));
    let until = sent.until.unwrap();
    assert_eq!((until.seconds, until.nanos), (1_794_745_800, 0));
}

#[tokio::test]
async fn search_without_filters_leaves_them_unset() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::history::SEARCH_SESSIONS, &hits_page(vec![], ""));
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "sessions", "search", "deadlock"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::SearchSessionsRequest = daemon.only(rpc::history::SEARCH_SESSIONS);
    assert_eq!(sent.query, "deadlock");
    assert!(sent.channel.is_none());
    assert!(sent.machine.is_none());
    assert_eq!(sent.agent, "");
    assert!(sent.since.is_none());
    assert!(sent.until.is_none());
}

#[tokio::test]
async fn search_follows_every_page_and_keeps_the_hits_in_order() {
    let daemon = FakeDaemon::new();
    daemon
        .reply(
            rpc::history::SEARCH_SESSIONS,
            &hits_page(vec![hit("se_1", 1, "first"), hit("se_1", 2, "second")], "p2"),
        )
        .reply(
            rpc::history::SEARCH_SESSIONS,
            &hits_page(vec![hit("se_2", 0, "third")], "p3"),
        )
        .reply(
            rpc::history::SEARCH_SESSIONS,
            &hits_page(vec![hit("se_3", 4, "fourth")], ""),
        );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "sessions", "search", "retry"]).await;
    assert_eq!(exit, exit::OK);

    let sent: Vec<pb::SearchSessionsRequest> = daemon.sent(rpc::history::SEARCH_SESSIONS);
    assert_eq!(
        tokens(&sent, |request| request.page.clone()),
        vec![String::new(), "p2".to_owned(), "p3".to_owned()]
    );
    assert!(sent.iter().all(|request| request.query == "retry"));

    let result = &console.json()["result"];
    let hits = result["hits"].as_array().unwrap();
    let snippets: Vec<_> = hits.iter().map(|hit| hit["snippet"].clone()).collect();
    assert_eq!(
        snippets,
        vec![json!("first"), json!("second"), json!("third"), json!("fourth")]
    );
    let sessions: Vec<_> = hits.iter().map(|hit| hit["session"].clone()).collect();
    assert_eq!(
        sessions,
        vec![json!("se_1"), json!("se_1"), json!("se_2"), json!("se_3")]
    );
    assert_eq!(hits[0]["channel_name"], "payments");
    assert_eq!(hits[0]["machine_name"], "laptop");
    assert_eq!(hits[0]["agent"], "claude");
}

#[tokio::test]
async fn search_without_words_is_a_usage_error_before_any_call() {
    for words in [
        &["sessions", "search"][..],
        &["sessions", "search", ""][..],
        &["sessions", "search", "  "][..],
    ] {
        let daemon = FakeDaemon::new();
        let console = FakeConsole::pipe();
        assert_eq!(run(&daemon, &console, words).await, exit::USAGE, "{words:?}");
        assert!(!daemon.called(rpc::history::SEARCH_SESSIONS), "{words:?}");
    }
}

#[tokio::test]
async fn a_bad_time_is_a_usage_error_before_any_call() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["sessions", "search", "retry", "--since", "last tuesday"],
    )
    .await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::history::SEARCH_SESSIONS));
}

#[tokio::test]
async fn a_search_the_daemon_refuses_exits_with_that_kinds_code() {
    for (kind, code) in [
        (Kind::Denied(Default::default()), exit::DENIED),
        (Kind::InvalidField(Default::default()), exit::USAGE),
    ] {
        let daemon = FakeDaemon::new();
        daemon.refuse(rpc::history::SEARCH_SESSIONS, kind);
        let console = FakeConsole::pipe();
        let exit = run(
            &daemon,
            &console,
            &["--json", "sessions", "search", "retry", "--channel", "ch_other"],
        )
        .await;
        assert_eq!(exit, code);
        assert_eq!(console.json()["ok"], false);
    }
}

#[tokio::test]
async fn a_bare_permission_denied_status_exits_as_denied() {
    let daemon = FakeDaemon::new();
    daemon.status(rpc::history::SEARCH_SESSIONS, 7);
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["sessions", "search", "retry"]).await;
    assert_eq!(exit, exit::DENIED);
}

#[tokio::test]
async fn read_sends_the_session_and_turn_range() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::history::READ_TURNS, &entries_page(vec![], ""));
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "session", "read", "se_1", "--from", "3", "--to", "5"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ReadTurnsRequest = daemon.only(rpc::history::READ_TURNS);
    assert_eq!(sent.session, support::session("se_1"));
    assert_eq!(sent.first_turn, Some(3));
    assert_eq!(sent.last_turn, Some(5));
}

#[tokio::test]
async fn read_without_a_range_leaves_both_ends_unset() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::history::READ_TURNS, &entries_page(vec![], ""));
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "session", "read", "se_1"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ReadTurnsRequest = daemon.only(rpc::history::READ_TURNS);
    assert_eq!(sent.first_turn, None);
    assert_eq!(sent.last_turn, None);
}

#[tokio::test]
async fn read_resolves_a_session_by_title() {
    let daemon = FakeDaemon::new();
    daemon
        .reply(
            rpc::session::LIST_SESSIONS,
            &pb::ListSessionsResponse {
                sessions: vec![pb::Session {
                    id: support::session("se_7"),
                    title: "Fix login".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .reply(rpc::history::READ_TURNS, &entries_page(vec![], ""));
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "session", "read", "fix login"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ReadTurnsRequest = daemon.only(rpc::history::READ_TURNS);
    assert_eq!(sent.session, support::session("se_7"));
}

#[tokio::test]
async fn read_gathers_every_page_in_order() {
    let daemon = FakeDaemon::new();
    daemon
        .reply(
            rpc::history::READ_TURNS,
            &entries_page(vec![entry(1, 1, "one"), entry(2, 1, "two")], "next"),
        )
        .reply(
            rpc::history::READ_TURNS,
            &entries_page(vec![entry(3, 2, "three")], ""),
        );
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "session", "read", "se_1", "--from", "1", "--to", "2"],
    )
    .await;
    assert_eq!(exit, exit::OK);

    let sent: Vec<pb::ReadTurnsRequest> = daemon.sent(rpc::history::READ_TURNS);
    assert_eq!(
        tokens(&sent, |request| request.page.clone()),
        vec![String::new(), "next".to_owned()]
    );
    assert!(
        sent.iter()
            .all(|request| request.first_turn == Some(1) && request.last_turn == Some(2))
    );

    let result = &console.json()["result"];
    let texts: Vec<_> = result["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["text"].clone())
        .collect();
    assert_eq!(texts, vec![json!("one"), json!("two"), json!("three")]);
}

#[tokio::test]
async fn a_read_the_daemon_refuses_exits_with_that_kinds_code() {
    let daemon = FakeDaemon::new();
    daemon.refuse(rpc::history::READ_TURNS, Kind::NotFound(Default::default()));
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["session", "read", "se_gone"]).await;
    assert_eq!(exit, exit::NOT_FOUND);
}

#[tokio::test]
async fn a_person_gets_the_hits_as_text() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::history::SEARCH_SESSIONS,
        &hits_page(vec![hit("se_1", 2, "retry with backoff")], ""),
    );
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["sessions", "search", "retry"]).await, exit::OK);
    assert!(console.stdout().contains("retry with backoff"));
    assert!(serde_json::from_str::<serde_json::Value>(&console.stdout()).is_err());
}
