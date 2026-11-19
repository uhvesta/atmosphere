//! The conversations the host keeps (LD-8, LD-23 to LD-25; D62, D63)
//! through a real `humatad2`: HistoryService's two calls, what is found of
//! a conversation written before the daemon started, while it runs and
//! while it was down, what a hit names, how both calls page, and what an
//! agent may search and read. Needs FIND T4b's seam (the service
//! registered); moves up from tests/pending with it.
//!
//! LD-21's scrubbing is the last test: a host variable the user set is
//! replaced by its name before its text is kept.

use humatad_test_common as common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    AGENT_NAME, Attach, Daemon, WAIT, agent_tab, create_channel, folder, get_session,
    invalid_field, machine_id, open, shell_token, start_session, start_world, state_folder,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{AgentLaunch, AgentRecord, AgentState, ChannelRecord, SessionRecord, Store};
use pb::history_service_client::HistoryServiceClient;
use pb::mirror_entry::Role;
use serde_json::{Value, json};
use tempfile::TempDir;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request, Status};

/// The channel most sessions here are in, and its name.
const HOME: &str = "ch_home";
const HOME_NAME: &str = "Payments";
/// Another channel.
const AWAY: &str = "ch_away";
const AWAY_NAME: &str = "Research";

/// A session of `HOME` with three turns (see [`ledger`]).
const LEDGER: &str = "se_ledger";
const LEDGER_TITLE: &str = "Ledger export";
/// Another session of `HOME`.
const SIBLING: &str = "se_sibling";
/// A session of `AWAY`.
const FAR: &str = "se_far";

/// The agent kind the seeded sessions run.
const AGENT: &str = "claude";
const TOKEN_HEADER: &str = "humata-session-token";
/// 2026-11-10T09:00:00Z, the minute the seeded conversations start at.
const MORNING_UNIX: i64 = 1_794_301_200;

// ---------------------------------------------------------------------------
// The lines an agent writes to its conversation file (Claude's JSONL).

fn line(kind: &str, content: Value, minute: u32) -> String {
    json!({
        "type": kind,
        "isSidechain": false,
        "message": {"role": kind, "content": content},
        "timestamp": format!("2026-11-10T09:{minute:02}:00.000Z"),
    })
    .to_string()
}

/// A prompt the user typed, `minute` minutes into the morning.
fn user(text: &str, minute: u32) -> String {
    line("user", json!(text), minute)
}

/// What the agent said.
fn says(text: &str, minute: u32) -> String {
    line("assistant", json!([{"type": "text", "text": text}]), minute)
}

/// A tool call the agent made.
fn calls(call: &str, tool: &str, input: Value, minute: u32) -> String {
    line(
        "assistant",
        json!([{"type": "tool_use", "id": call, "name": tool, "input": input}]),
        minute,
    )
}

/// What the tool answered to `call`.
fn answers(call: &str, answer: &str, minute: u32) -> String {
    line(
        "user",
        json!([{"type": "tool_result", "tool_use_id": call, "content": answer}]),
        minute,
    )
}

/// `LEDGER`'s conversation: eight entries in three turns. "ledger" and
/// "csv" are only in the first prompt, "quokka" only in the second and
/// "regression" only in the third turn, which is the last line too.
fn ledger() -> Vec<String> {
    vec![
        user("export the ledger as csv", 0),
        says("The exporter lives in billing/export.rs", 1),
        user("add a column for quokka refunds", 2),
        calls("toolu_read", "Read", json!({"file_path": "billing/export.rs"}), 3),
        answers("toolu_read", "fn export_rows() {}", 4),
        says("Added the refunds column", 5),
        user("now write a regression test", 6),
        says("Wrote the regression test under billing/tests", 7),
    ]
}

/// A one-turn conversation that mentions `word`.
fn mentions(word: &str) -> Vec<String> {
    vec![
        user(&format!("where is the {word} handled"), 0),
        says(&format!("In the {word} module"), 1),
    ]
}

fn conversation_in(dir: &Path, session: &str) -> PathBuf {
    dir.join("conv").join(format!("{session}.jsonl"))
}

/// The conversation file of the seeded `session`.
fn conversation(daemon: &Daemon, session: &str) -> PathBuf {
    conversation_in(daemon.dir.path(), session)
}

fn joined(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// Adds `lines` to the end of `file` in one write, as an agent that went
/// on with its conversation.
fn append(file: &Path, lines: &[String]) {
    let mut open = std::fs::OpenOptions::new().append(true).open(file).unwrap();
    open.write_all(joined(lines).as_bytes()).unwrap();
}

// ---------------------------------------------------------------------------
// A store seeded before the daemon starts.

/// The state a daemon starts on: the two channels, and the sessions a
/// test adds, each with an agent that was stopped (so nothing is resumed)
/// and the conversation file that agent wrote.
struct Seed {
    dir: TempDir,
    store: Store,
}

impl Seed {
    fn new() -> Self {
        let dir = folder();
        let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
        for (id, name, at) in [(HOME, HOME_NAME, "home"), (AWAY, AWAY_NAME, "away")] {
            store
                .put_channel(&ChannelRecord {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    folder: at.to_owned(),
                    branch_prefix: None,
                    default_agent: AGENT.to_owned(),
                    repos: Vec::new(),
                    machines: Vec::new(),
                    revision: 1,
                    created_unix_ms: 0,
                })
                .unwrap();
        }
        std::fs::create_dir_all(dir.path().join("conv")).unwrap();
        Self { dir, store }
    }

    /// A session of `channel` whose agent wrote `lines`. Its machine is
    /// left unset, as for a session of the host itself.
    fn session(&self, id: &str, channel: &str, title: &str, lines: &[String]) {
        let dir = self.dir.path();
        self.store
            .put_session(&SessionRecord {
                id: id.to_owned(),
                channel: channel.to_owned(),
                machine: String::new(),
                title: title.to_owned(),
                agent: AGENT.to_owned(),
                folder: dir.to_owned(),
                created_unix_ms: 0,
            })
            .unwrap();
        let file = conversation_in(dir, id);
        std::fs::write(&file, joined(lines)).unwrap();
        self.store
            .put_agent(&AgentRecord {
                session: id.to_owned(),
                tab: format!("tb_{id}"),
                launch: AgentLaunch {
                    agent: AGENT.to_owned(),
                    folder: dir.to_owned(),
                    conversation_id: format!("conv-{id}"),
                    conversation_file: file,
                    holder_socket: dir.join(format!("{id}.sock")),
                    cols: 80,
                    rows: 24,
                    ..AgentLaunch::default()
                },
                state: AgentState::Stopped,
                generation: 1,
                interruptions: Vec::new(),
                updated_unix_ms: 0,
            })
            .unwrap();
    }

    async fn start(self) -> Daemon {
        let Self { dir, store } = self;
        drop(store);
        Daemon::start_in(dir).await
    }
}

/// A daemon that starts knowing `LEDGER`.
async fn start_ledger() -> Daemon {
    let seed = Seed::new();
    seed.session(LEDGER, HOME, LEDGER_TITLE, &ledger());
    seed.start().await
}

/// [`start_ledger`], once all of `LEDGER`'s conversation is mirrored: its
/// last line is found.
async fn ledger_mirrored() -> Daemon {
    let daemon = start_ledger().await;
    found(&daemon, "regression").await;
    daemon
}

/// A daemon that starts knowing three sessions that each mention
/// "wombat" in one turn: `LEDGER` and `SIBLING` in `HOME`, `FAR` in `AWAY`.
async fn start_three() -> Daemon {
    let seed = Seed::new();
    seed.session(LEDGER, HOME, LEDGER_TITLE, &mentions("wombat"));
    seed.session(SIBLING, HOME, "Rate limiter", &mentions("wombat"));
    seed.session(FAR, AWAY, "Tide tables", &mentions("wombat"));
    seed.start().await
}

/// [`start_three`], once all three conversations are mirrored.
async fn three_mirrored() -> Daemon {
    let daemon = start_three().await;
    hits_when(&daemon, &query("wombat"), |hits| hits.len() == 3).await;
    daemon
}

// ---------------------------------------------------------------------------
// Asking the daemon.

async fn history(daemon: &Daemon) -> HistoryServiceClient<Channel> {
    HistoryServiceClient::new(daemon.channel().await)
}

fn id(session: &str) -> pb::SessionId {
    pb::SessionId {
        value: session.to_owned(),
    }
}

fn channel_id(channel: &str) -> pb::ChannelId {
    pb::ChannelId {
        value: channel.to_owned(),
    }
}

/// `message` as the user sends it, or as the agent holding `token` does.
fn asked<T>(message: T, token: Option<&str>) -> Request<T> {
    let mut request = Request::new(message);
    if let Some(token) = token {
        request
            .metadata_mut()
            .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    }
    request
}

/// A search for `text` that names no channel.
fn query(text: &str) -> pb::SearchSessionsRequest {
    pb::SearchSessionsRequest {
        query: text.to_owned(),
        ..Default::default()
    }
}

/// A search for `text` in `channel`.
fn query_in(text: &str, channel: &str) -> pb::SearchSessionsRequest {
    pb::SearchSessionsRequest {
        channel: Some(channel_id(channel)),
        ..query(text)
    }
}

/// `session`'s entries in turns `first..=last`; unset is from the first
/// turn, to the last.
fn turns(session: &str, first: Option<u32>, last: Option<u32>) -> pb::ReadTurnsRequest {
    pb::ReadTurnsRequest {
        session: Some(id(session)),
        first_turn: first,
        last_turn: last,
        page: None,
    }
}

/// The request's page size with the token of the page before.
fn next_page(page: Option<&pb::PageRequest>, token: String) -> Option<pb::PageRequest> {
    Some(pb::PageRequest {
        page_size: page.map_or(0, |page| page.page_size),
        page_token: token,
    })
}

/// Every page of a search, following page tokens to the last one; by the
/// user, or by the agent holding `token`.
async fn hit_pages(
    daemon: &Daemon,
    request: &pb::SearchSessionsRequest,
    token: Option<&str>,
) -> Result<Vec<Vec<pb::SearchHit>>, Status> {
    let mut client = history(daemon).await;
    let mut request = request.clone();
    let mut pages = Vec::new();
    for _ in 0..100 {
        let response = client
            .search_sessions(asked(request.clone(), token))
            .await?
            .into_inner();
        pages.push(response.hits);
        let next = response
            .page
            .map(|page| page.next_page_token)
            .unwrap_or_default();
        if next.is_empty() {
            return Ok(pages);
        }
        request.page = next_page(request.page.as_ref(), next);
    }
    panic!("the search never reached a last page");
}

/// Every hit of the user's search.
async fn hits(daemon: &Daemon, request: &pb::SearchSessionsRequest) -> Vec<pb::SearchHit> {
    hit_pages(daemon, request, None)
        .await
        .unwrap()
        .into_iter()
        .flatten()
        .collect()
}

/// Every hit of the search as made by the agent holding `token`, or why
/// it was refused.
async fn hits_as(
    daemon: &Daemon,
    request: &pb::SearchSessionsRequest,
    token: &str,
) -> Result<Vec<pb::SearchHit>, Status> {
    hit_pages(daemon, request, Some(token))
        .await
        .map(|pages| pages.into_iter().flatten().collect())
}

/// The user's search asked again until its hits are `enough`, within
/// `WAIT`: the mirror follows a conversation, it isn't asked to. A search
/// that fails is not asked again.
async fn hits_when(
    daemon: &Daemon,
    request: &pb::SearchSessionsRequest,
    enough: impl Fn(&[pb::SearchHit]) -> bool,
) -> Vec<pb::SearchHit> {
    let deadline = Instant::now() + WAIT;
    loop {
        let seen = hits(daemon, request).await;
        if enough(&seen) {
            return seen;
        }
        assert!(
            Instant::now() < deadline,
            "the search for {:?} never gave what was waited for; it last gave {seen:?}",
            request.query
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The hits for `text` in any channel, once there is one.
async fn found(daemon: &Daemon, text: &str) -> Vec<pb::SearchHit> {
    hits_when(daemon, &query(text), |hits| !hits.is_empty()).await
}

/// Every page of a read, following page tokens to the last one; by the
/// user, or by the agent holding `token`.
async fn entry_pages(
    daemon: &Daemon,
    request: &pb::ReadTurnsRequest,
    token: Option<&str>,
) -> Result<Vec<Vec<pb::MirrorEntry>>, Status> {
    let mut client = history(daemon).await;
    let mut request = request.clone();
    let mut pages = Vec::new();
    for _ in 0..100 {
        let response = client
            .read_turns(asked(request.clone(), token))
            .await?
            .into_inner();
        pages.push(response.entries);
        let next = response
            .page
            .map(|page| page.next_page_token)
            .unwrap_or_default();
        if next.is_empty() {
            return Ok(pages);
        }
        request.page = next_page(request.page.as_ref(), next);
    }
    panic!("the read never reached a last page");
}

/// Every entry the user's read returns.
async fn entries(daemon: &Daemon, request: &pb::ReadTurnsRequest) -> Vec<pb::MirrorEntry> {
    entry_pages(daemon, request, None)
        .await
        .unwrap()
        .into_iter()
        .flatten()
        .collect()
}

/// The session a hit is in.
fn session_of(hit: &pb::SearchHit) -> &str {
    hit.session.as_ref().map_or("", |session| session.value.as_str())
}

/// The sessions of `hits`, sorted.
fn sessions_of(hits: &[pb::SearchHit]) -> Vec<&str> {
    let mut sessions: Vec<&str> = hits.iter().map(session_of).collect();
    sessions.sort_unstable();
    sessions
}

/// Where each hit points and what it shows, sorted: what must stay the
/// same when nothing new was written.
fn places(hits: &[pb::SearchHit]) -> Vec<(&str, u32, u64, &str)> {
    let mut places: Vec<_> = hits
        .iter()
        .map(|hit| (session_of(hit), hit.turn, hit.seq, hit.snippet.as_str()))
        .collect();
    places.sort_unstable();
    places
}

/// The user gives `session` another title, which the feed tells as a
/// Session event.
async fn rename(daemon: &Daemon, session: &str, title: &str) {
    daemon
        .session_service()
        .await
        .rename_session(pb::RenameSessionRequest {
            id: Some(id(session)),
            title: title.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
}

/// The token of an agent of `session` (its shell's), and the attach that
/// keeps its tab in use.
async fn agent_of(daemon: &Daemon, session: &str) -> (String, Attach) {
    shell_token(daemon, &id(session)).await
}

/// Waits until `file` holds `text` in a finished line.
async fn written(file: &Path, text: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        let now = std::fs::read_to_string(file).unwrap_or_default();
        if now.contains(text) && now.ends_with('\n') {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} never held {text:?}; it holds {now:?}",
            file.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// `refused` says the request's `field` is wrong.
#[track_caller]
fn assert_bad_field(refused: &Status, field: &str) {
    let (named, _) = invalid_field(refused);
    assert!(
        named.starts_with(field),
        "the failure names {named:?}, not {field:?}"
    );
}

// ---------------------------------------------------------------------------
// Searching.

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_written_before_the_daemon_started_is_found() {
    let daemon = start_ledger().await;

    let hits = found(&daemon, "ledger").await;

    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(session_of(&hits[0]), LEDGER);
    assert!(hits[0].snippet.contains("ledger"), "{:?}", hits[0].snippet);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_word_must_match_and_only_the_last_may_be_the_start_of_one() {
    let daemon = ledger_mirrored().await;

    // Both words are in the first prompt, in the other order.
    let both = hits(&daemon, &query("csv ledger")).await;
    assert_eq!(both.len(), 1, "{both:?}");
    // The last word as typed so far.
    let prefixed = hits(&daemon, &query("csv ledg")).await;
    assert_eq!(prefixed.len(), 1, "{prefixed:?}");
    assert_eq!(prefixed[0].turn, 1);
    // A word that isn't the last is a whole word.
    let cut = hits(&daemon, &query("ledg csv")).await;
    assert!(cut.is_empty(), "{cut:?}");
    // One word matches and one is nowhere.
    let half = hits(&daemon, &query("ledger zebra")).await;
    assert!(half.is_empty(), "{half:?}");
}

/// LD-24, on a session seeded with no machine: it runs on the host, as
/// GetSession shows it.
#[tokio::test(flavor = "multi_thread")]
async fn a_hit_names_its_session_channel_machine_agent_and_turn() {
    let daemon = start_ledger().await;

    let hits = found(&daemon, "quokka").await;

    assert_eq!(hits.len(), 1, "{hits:?}");
    let hit = &hits[0];
    assert_eq!(hit.session, Some(id(LEDGER)));
    assert_eq!(hit.session_title, LEDGER_TITLE);
    assert_eq!(hit.channel, Some(channel_id(HOME)));
    assert_eq!(hit.channel_name, HOME_NAME);
    assert_eq!(hit.machine, Some(machine_id(&daemon).await));
    assert_eq!(hit.machine, get_session(&daemon, &id(LEDGER)).await.machine);
    assert_eq!(hit.agent, AGENT);
    // The second prompt and its turn, written two minutes in.
    assert_eq!(hit.turn, 2);
    assert!(hit.snippet.contains("quokka"), "{:?}", hit.snippet);
    assert_eq!(
        hit.at.as_ref().map(|at| at.seconds),
        Some(MORNING_UNIX + 2 * 60)
    );
    assert!(hit.as_of.is_some(), "{hit:?}");
}

/// `session#seq`: a hit's seq is the number ReadTurns gives the entry.
#[tokio::test(flavor = "multi_thread")]
async fn a_hits_seq_is_its_entrys_place_in_the_session() {
    let daemon = ledger_mirrored().await;

    let hits = hits(&daemon, &query("quokka")).await;
    assert_eq!(hits.len(), 1, "{hits:?}");

    let read = entries(&daemon, &turns(LEDGER, None, None)).await;
    let matching: Vec<u64> = read
        .iter()
        .filter(|entry| entry.text.contains("quokka"))
        .map(|entry| entry.seq)
        .collect();
    assert_eq!(matching, [hits[0].seq]);
}

/// LD-24: what an agent asked Humata and what Humata answered is not
/// history to search.
#[tokio::test(flavor = "multi_thread")]
async fn calls_to_humatas_own_tools_are_not_found() {
    let seed = Seed::new();
    seed.session(
        LEDGER,
        HOME,
        LEDGER_TITLE,
        &[
            user("find where we talked about limits", 0),
            // The MCP tool, and what it answered.
            calls(
                "toolu_mcp",
                "mcp__humata__sessions_search",
                json!({"query": "axolotl"}),
                1,
            ),
            answers("toolu_mcp", "se_old, turn 3: the axolotl limiter", 2),
            // The CLI in a shell, and what it printed.
            calls(
                "toolu_cli",
                "Bash",
                json!({"command": "humata sessions search narwhal"}),
                3,
            ),
            answers("toolu_cli", "se_old, turn 5: narwhal quotas", 4),
            // Any other tool is history.
            calls("toolu_read", "Read", json!({"file_path": "docs/capybara.md"}), 5),
            answers("toolu_read", "capybara notes", 6),
        ],
    );
    let daemon = seed.start().await;

    // The last lines are mirrored, so the ones before were read.
    let kept = found(&daemon, "capybara").await;
    assert_eq!(sessions_of(&kept), [LEDGER]);

    for own in ["axolotl", "narwhal"] {
        let hits = hits(&daemon, &query(own)).await;
        assert!(hits.is_empty(), "{own}: {hits:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_user_searches_every_channel_or_the_one_named() {
    let daemon = three_mirrored().await;

    let everywhere = hits(&daemon, &query("wombat")).await;
    assert_eq!(sessions_of(&everywhere), [FAR, LEDGER, SIBLING]);

    let home = hits(&daemon, &query_in("wombat", HOME)).await;
    assert_eq!(sessions_of(&home), [LEDGER, SIBLING]);
    let away = hits(&daemon, &query_in("wombat", AWAY)).await;
    assert_eq!(sessions_of(&away), [FAR]);
    assert_eq!(away[0].channel_name, AWAY_NAME);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_search_pages_through_its_hits() {
    let daemon = three_mirrored().await;
    let whole = hits(&daemon, &query("wombat")).await;

    let request = pb::SearchSessionsRequest {
        page: Some(pb::PageRequest {
            page_size: 1,
            page_token: String::new(),
        }),
        ..query("wombat")
    };
    let pages = hit_pages(&daemon, &request, None).await.unwrap();

    // No page is over the size asked or empty, and together they are the
    // hits of one search, each once.
    assert!(pages.iter().all(|page| page.len() == 1), "{pages:?}");
    let paged: Vec<pb::SearchHit> = pages.into_iter().flatten().collect();
    assert_eq!(sessions_of(&paged), [FAR, LEDGER, SIBLING]);
    assert_eq!(places(&paged), places(&whole));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_search_for_nothing_is_refused() {
    let daemon = start_ledger().await;
    let mut client = history(&daemon).await;

    for nothing in ["", "  \t "] {
        let refused = client
            .search_sessions(query(nothing))
            .await
            .unwrap_err();
        assert_bad_field(&refused, "query");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_token_the_daemon_never_gave_is_refused() {
    let daemon = start_ledger().await;
    let mut client = history(&daemon).await;
    let made_up = Some(pb::PageRequest {
        page_size: 0,
        page_token: "never-given".to_owned(),
    });

    let refused = client
        .search_sessions(pb::SearchSessionsRequest {
            page: made_up.clone(),
            ..query("ledger")
        })
        .await
        .unwrap_err();
    assert_bad_field(&refused, "page");

    let refused = client
        .read_turns(pb::ReadTurnsRequest {
            page: made_up,
            ..turns(LEDGER, None, None)
        })
        .await
        .unwrap_err();
    assert_bad_field(&refused, "page");
}

// ---------------------------------------------------------------------------
// Following a conversation while the daemon runs.

/// The mirror reads a conversation again when the feed tells of its
/// session; here the user's rename is what the feed tells.
#[tokio::test(flavor = "multi_thread")]
async fn what_a_conversation_gained_is_found_after_its_sessions_next_event() {
    let daemon = ledger_mirrored().await;
    append(
        &conversation(&daemon, LEDGER),
        &[
            user("ship the numbat release", 8),
            says("Tagged the numbat release", 9),
        ],
    );

    rename(&daemon, LEDGER, "Ledger export, shipped").await;

    let hits = found(&daemon, "numbat").await;
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(session_of(&hits[0]), LEDGER);
    // The fourth prompt of the conversation.
    assert_eq!(hits[0].turn, 4);
}

/// D438: the index tells, on the session's feed, when it has caught a
/// session up, so whoever waits for a search to find something can search
/// again on that instead of asking until it does. One told before the
/// test listened (the start's catch-up) may come first; each is searched
/// on, and the one after the new entries finds them.
#[tokio::test(flavor = "multi_thread")]
async fn catching_a_conversation_up_is_told_on_its_sessions_feed() {
    let daemon = ledger_mirrored().await;
    let mut events = pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            sessions: vec![id(LEDGER)],
            topics: vec![pb::Topic::Sessions as i32],
            ..pb::SubscribeRequest::default()
        })
        .await
        .unwrap()
        .into_inner();
    append(
        &conversation(&daemon, LEDGER),
        &[
            user("ship the numbat release", 8),
            says("Tagged the numbat release", 9),
        ],
    );

    rename(&daemon, LEDGER, "Ledger export, shipped").await;

    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let event = tokio::time::timeout_at(deadline, events.message())
            .await
            .expect("no HistoryIndexed told of the new entries")
            .unwrap()
            .expect("the feed ended");
        let Some(pb::event::Payload::HistoryIndexed(indexed)) = event.payload else {
            continue;
        };
        assert_eq!(indexed.session, Some(id(LEDGER)));
        assert!(indexed.added > 0, "an index event adds something: {indexed:?}");
        let seen = hits(&daemon, &query("numbat")).await;
        if !seen.is_empty() {
            assert_eq!(sessions_of(&seen), vec![LEDGER]);
            return;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_renamed_sessions_hits_carry_its_new_title() {
    let daemon = ledger_mirrored().await;

    rename(&daemon, LEDGER, "Refunds column").await;

    let hits = hits_when(&daemon, &query("quokka"), |hits| {
        hits.iter().any(|hit| hit.session_title == "Refunds column")
    })
    .await;
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].turn, 2);
}

/// A session started while the daemon runs, with a real agent in a real
/// terminal: the test agent keeps every line typed to it in its
/// conversation file, so a line of Claude's JSONL typed to it is a prompt
/// in its conversation. Nothing else is done to the session: the feed
/// tells of it by itself when its agent's output goes quiet (SE-21).
#[tokio::test(flavor = "multi_thread")]
async fn what_a_running_agent_wrote_is_found_once_its_output_goes_quiet() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    assert_eq!(
        setup.state,
        pb::OperationState::Succeeded as i32,
        "{:?}",
        setup.error
    );
    let (session, operation) = start_session(&daemon, &channel, "Fix login").await;
    assert_eq!(
        operation.state,
        pb::OperationState::Succeeded as i32,
        "{:?}",
        operation.error
    );
    let shown = get_session(&daemon, &session).await;
    assert!(!shown.conversation_id.is_empty(), "{shown:?}");
    let file = world.conversation(&shown.conversation_id);

    let tab = agent_tab(&daemon, &session).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach
        .type_text(&format!("{}\r", user("reconcile the pangolin invoices", 0)))
        .await;
    written(&file, "pangolin").await;

    let hits = found(&daemon, "pangolin").await;

    assert_eq!(hits.len(), 1, "{hits:?}");
    let hit = &hits[0];
    assert_eq!(hit.session, Some(session));
    assert_eq!(hit.session_title, "Fix login");
    assert_eq!(hit.channel, channel.id);
    assert_eq!(hit.channel_name, "Payments");
    assert_eq!(hit.machine, Some(machine_id(&daemon).await));
    assert_eq!(hit.agent, AGENT_NAME);
    assert_eq!(hit.turn, 1);
}

/// N1: an agent may write the end of a turn to its conversation file after
/// the last event of its session (Copilot 1.0.113 writes its reply after its
/// turn-ended hook has run). What the running agent's file gains is still
/// mirrored, though nothing more happens to the session.
#[tokio::test(flavor = "multi_thread")]
async fn what_a_running_agent_writes_after_its_sessions_last_event_is_found() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (session, operation) = start_session(&daemon, &channel, "Fix login").await;
    assert_eq!(
        operation.state,
        pb::OperationState::Succeeded as i32,
        "{:?}",
        operation.error
    );
    let shown = get_session(&daemon, &session).await;
    let file = world.conversation(&shown.conversation_id);
    let tab = agent_tab(&daemon, &session).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach
        .type_text(&format!("{}\r", user("reconcile the pangolin invoices", 0)))
        .await;
    written(&file, "pangolin").await;
    found(&daemon, "pangolin").await;
    // The output went quiet: the session's last event, and the catch-up it
    // started is done.
    let deadline = Instant::now() + WAIT;
    while get_session(&daemon, &session).await.status != pb::SessionStatus::ProbablyIdle as i32 {
        assert!(Instant::now() < deadline, "the session's agent never went quiet");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;

    // The reply lands in the file after that, with no output and no event.
    append(&file, &[says("Reconciled the axolotl ledger", 1)]);

    let hits = found(&daemon, "axolotl").await;
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].session, Some(session));
    assert_eq!(hits[0].turn, 1);
}

// ---------------------------------------------------------------------------
// Reading turns.

#[tokio::test(flavor = "multi_thread")]
async fn a_read_returns_the_turns_asked_for_in_order() {
    let daemon = ledger_mirrored().await;

    // Everything: numbered from 0 without gaps, each entry in its turn.
    let all = entries(&daemon, &turns(LEDGER, None, None)).await;
    let seqs: Vec<u64> = all.iter().map(|entry| entry.seq).collect();
    assert_eq!(seqs, [0, 1, 2, 3, 4, 5, 6, 7]);
    let in_turn: Vec<u32> = all.iter().map(|entry| entry.turn).collect();
    assert_eq!(in_turn, [1, 1, 2, 2, 2, 2, 3, 3]);

    // One turn: the prompt, the tool call, its answer and what the agent
    // said, as they were written.
    let second = entries(&daemon, &turns(LEDGER, Some(2), Some(2))).await;
    assert_eq!(second, all[2..6]);
    let roles: Vec<Role> = second.iter().map(|entry| entry.role()).collect();
    assert_eq!(roles, [Role::User, Role::Tool, Role::Tool, Role::Agent]);
    assert_eq!(second[0].text, "add a column for quokka refunds");
    assert!(
        second[1].text.contains("billing/export.rs"),
        "{:?}",
        second[1].text
    );
    assert_eq!(second[2].text, "fn export_rows() {}");
    assert_eq!(second[3].text, "Added the refunds column");
    assert_eq!(
        second[0].at.as_ref().map(|at| at.seconds),
        Some(MORNING_UNIX + 2 * 60)
    );

    // A range open at one end.
    let from_second = entries(&daemon, &turns(LEDGER, Some(2), None)).await;
    assert_eq!(from_second, all[2..]);
    let to_first = entries(&daemon, &turns(LEDGER, None, Some(1))).await;
    assert_eq!(to_first, all[..2]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_pages_through_a_session() {
    let daemon = ledger_mirrored().await;
    let whole = entries(&daemon, &turns(LEDGER, None, None)).await;
    assert_eq!(whole.len(), 8, "{whole:?}");

    let request = pb::ReadTurnsRequest {
        page: Some(pb::PageRequest {
            page_size: 3,
            page_token: String::new(),
        }),
        ..turns(LEDGER, None, None)
    };
    let pages = entry_pages(&daemon, &request, None).await.unwrap();

    // Eight entries don't fit one page of three; no page is over the size
    // asked or empty, and together they are the session in order.
    assert!(pages.len() > 1, "{pages:?}");
    assert!(
        pages.iter().all(|page| (1..=3).contains(&page.len())),
        "{pages:?}"
    );
    let paged: Vec<pb::MirrorEntry> = pages.into_iter().flatten().collect();
    assert_eq!(paged, whole);
}

// ---------------------------------------------------------------------------
// What an agent may search and read (LD-25): its own channel.

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_that_names_no_channel_searches_its_own() {
    let daemon = three_mirrored().await;
    let (token, _attach) = agent_of(&daemon, LEDGER).await;

    // Its own session and the other one of its channel, not the session
    // of the channel it isn't in.
    let unnamed = hits_as(&daemon, &query("wombat"), &token).await.unwrap();
    assert_eq!(sessions_of(&unnamed), [LEDGER, SIBLING]);

    // Naming its own channel is the same search.
    let named = hits_as(&daemon, &query_in("wombat", HOME), &token)
        .await
        .unwrap();
    assert_eq!(places(&named), places(&unnamed));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_is_refused_a_search_of_another_channel() {
    let daemon = start_three().await;
    let (token, _attach) = agent_of(&daemon, LEDGER).await;

    let refused = hits_as(&daemon, &query_in("wombat", AWAY), &token)
        .await
        .unwrap_err();

    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_reads_the_sessions_of_its_channel_and_is_refused_another_channels() {
    let daemon = three_mirrored().await;
    let (token, _attach) = agent_of(&daemon, LEDGER).await;

    // Another session of its channel reads as it does for the user.
    let sibling = entry_pages(&daemon, &turns(SIBLING, None, None), Some(&token))
        .await
        .unwrap()
        .concat();
    assert_eq!(sibling.len(), 2, "{sibling:?}");
    assert_eq!(
        sibling,
        entries(&daemon, &turns(SIBLING, None, None)).await
    );

    // A session of another channel doesn't, whatever turns are asked for.
    for request in [turns(FAR, None, None), turns(FAR, Some(1), Some(1))] {
        let refused = entry_pages(&daemon, &request, Some(&token))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    }
    // The user reads it.
    let far = entries(&daemon, &turns(FAR, None, None)).await;
    assert_eq!(far.len(), 2, "{far:?}");
}

// ---------------------------------------------------------------------------
// Restarts.

/// The mirror is the host's own copy: it answers after a restart even
/// when the file it was read from is gone, as a session's is while its
/// machine is offline (LD-8, LD-23).
#[tokio::test(flavor = "multi_thread")]
async fn a_restart_keeps_what_was_mirrored() {
    let mut daemon = ledger_mirrored().await;
    let before = hits(&daemon, &query("quokka")).await;
    assert_eq!(before.len(), 1, "{before:?}");
    let read_before = entries(&daemon, &turns(LEDGER, None, None)).await;

    // Killed, as a crash would; the agent's file goes while it is down.
    daemon.kill();
    std::fs::remove_file(conversation(&daemon, LEDGER)).unwrap();
    daemon.restart().await;

    // Asked once: nothing has to be read again for it to answer.
    let after = hits(&daemon, &query("quokka")).await;
    assert_eq!(places(&after), places(&before));
    assert_eq!(after[0].session_title, LEDGER_TITLE);
    assert_eq!(after[0].channel_name, HOME_NAME);
    assert_eq!(
        entries(&daemon, &turns(LEDGER, None, None)).await,
        read_before
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_conversation_gained_while_the_daemon_was_down_is_found_after_it_starts() {
    let mut daemon = ledger_mirrored().await;
    let before = hits(&daemon, &query("quokka")).await;
    let read_before = entries(&daemon, &turns(LEDGER, None, None)).await;

    daemon.kill();
    append(
        &conversation(&daemon, LEDGER),
        &[
            user("ship the numbat release", 8),
            says("Tagged the numbat release", 9),
        ],
    );
    daemon.restart().await;

    let gained = found(&daemon, "numbat").await;
    assert_eq!(gained.len(), 1, "{gained:?}");
    assert_eq!(session_of(&gained[0]), LEDGER);
    assert_eq!(gained[0].turn, 4);

    // What was mirrored before is there once, where it was, and the new
    // entries follow it.
    assert_eq!(
        places(&hits(&daemon, &query("quokka")).await),
        places(&before)
    );
    let read = entries(&daemon, &turns(LEDGER, None, None)).await;
    assert_eq!(read[..read_before.len()], read_before[..]);
    let gained_texts: Vec<&str> = read[read_before.len()..]
        .iter()
        .map(|entry| entry.text.as_str())
        .collect();
    assert_eq!(
        gained_texts,
        ["ship the numbat release", "Tagged the numbat release"]
    );
}

/// LD-21: a host variable's value is replaced by the variable's name before
/// any of its text is kept in the mirror.
#[tokio::test(flavor = "multi_thread")]
async fn a_host_variable_value_in_a_conversation_is_replaced_by_its_name_before_it_is_kept() {
    let daemon = ledger_mirrored().await;
    let secret = "tok-7c1f9d2e-secret";
    pb::machine_service_client::MachineServiceClient::new(daemon.channel().await)
        .set_host_variable(pb::SetHostVariableRequest {
            idempotency_key: None,
            id: None,
            name: "BILLING_TOKEN".to_owned(),
            value: secret.to_owned(),
            presence: None,
        })
        .await
        .unwrap();

    append(
        &conversation(&daemon, LEDGER),
        &[user(&format!("call the marmot service with {secret}"), 8)],
    );
    // The mirror reads the conversation again on its session's next event.
    rename(&daemon, LEDGER, "Ledger export, scrubbed").await;
    found(&daemon, "marmot").await;

    let read = entries(&daemon, &turns(LEDGER, None, None)).await;
    let kept = read.iter().find(|entry| entry.text.contains("marmot")).expect("the new turn is kept");
    assert!(kept.text.contains("BILLING_TOKEN"), "{}", kept.text);
    assert!(read.iter().all(|entry| !entry.text.contains(secret)));
}
