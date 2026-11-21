//! The history tools of `humatad2 mcp` (LD-21 to LD-25; MCP.md §6.2):
//! `sessions_search` and `session_read`, which an agent calls to search and
//! read the conversations the host keeps. They are HistoryService's
//! SearchSessions and ReadTurns, so LD-25's "the CLI and the tool give the
//! same results" is checked against the gRPC call for the same query.
//!
//! Waits on HistoryService in humatad (tests/history.rs) and the two tools
//! declared in the MCP server and dispatched by humatad; moves up from
//! tests/pending with them.
//!
//! Not here: LD-21's scrubbing. The daemon's scrubber has no host
//! variables to scrub yet (MachineService.SetHostVariable is not built, a
//! SEAM), so there is nothing a test can observe; the mirror crate's own
//! tests cover it until it does.
//!
//! Shapes assumed (MCP.md §6.2, D25: ids are their plain strings, enum
//! values lowercase without their prefix):
//! - `sessions_search {query, channel?, machine?, agent?, since?, until?,
//!   page_token?}` → `result: {hits: [{session, channel, machine, agent,
//!   turn, snippet, as_of, ...}], next_page_token?}`;
//! - `session_read {session, first_turn?, last_turn?, page_token?}` →
//!   `result: {turns: [{seq, role, text, ...}], next_page_token?}`, `role`
//!   one of `user`, `agent`, `tool`;
//! - a refusal is a tool result with `isError` and `error.kind` `denied`;
//! - a bad argument is `error.kind` `invalid_field` naming the field in
//!   `error.details.field`, or the server's own invalid-params error naming
//!   it in `error.data.field` when the schema already refuses it.

use humatad_test_common as common;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use common::{Attach, Daemon, WAIT, binary, folder, shell_token, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{AgentLaunch, AgentRecord, AgentState, ChannelRecord, SessionRecord, Store};
use pb::history_service_client::HistoryServiceClient;
use pb::mirror_entry::Role;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Request, Status};

/// The channel the agent's session is in.
const HOME: &str = "ch_home";
/// Another channel.
const AWAY: &str = "ch_away";

/// The agent's own session: three turns (see [`ledger`]).
const LEDGER: &str = "se_ledger";
/// Another session of `HOME`, that mentions "wombat".
const SIBLING: &str = "se_sibling";
/// A session of `HOME` with more entries than one page holds (see
/// [`many`]).
const MANY: &str = "se_many";
/// A session of `AWAY`, that mentions "wombat" too.
const FAR: &str = "se_far";

/// How many entries `MANY` has, each mentioning "kiwi": a prompt and an
/// answer per turn.
const MANY_ENTRIES: usize = 120;
/// Its turns: a search has one hit per turn (the best entry of each), and
/// these are more than a page holds.
const MANY_TURNS: usize = MANY_ENTRIES / 2;

const AGENT: &str = "claude";
const TOKEN_HEADER: &str = "humata-session-token";

// ---------------------------------------------------------------------------
// The lines an agent writes to its conversation file (Claude's JSONL).

fn line(kind: &str, content: Value, second: usize) -> String {
    json!({
        "type": kind,
        "isSidechain": false,
        "message": {"role": kind, "content": content},
        "timestamp": format!("2026-11-10T09:{:02}:{:02}.000Z", second / 60, second % 60),
    })
    .to_string()
}

fn user(text: &str, second: usize) -> String {
    line("user", json!(text), second)
}

fn says(text: &str, second: usize) -> String {
    line("assistant", json!([{"type": "text", "text": text}]), second)
}

fn calls(call: &str, tool: &str, input: Value, second: usize) -> String {
    line(
        "assistant",
        json!([{"type": "tool_use", "id": call, "name": tool, "input": input}]),
        second,
    )
}

fn answers(call: &str, answer: &str, second: usize) -> String {
    line(
        "user",
        json!([{"type": "tool_result", "tool_use_id": call, "content": answer}]),
        second,
    )
}

/// Eight entries in three turns; "regression" is only in the last turn.
fn ledger() -> Vec<String> {
    vec![
        user("export the ledger as csv", 0),
        says("The exporter lives in billing/export.rs", 60),
        user("add a column for quokka refunds", 120),
        calls("toolu_read", "Read", json!({"file_path": "billing/export.rs"}), 180),
        answers("toolu_read", "fn export_rows() {}", 240),
        says("Added the refunds column", 300),
        user("now write a regression test", 360),
        says("Wrote the regression test under billing/tests", 420),
    ]
}

/// A one-turn conversation with two entries that mention `word`.
fn mentions(word: &str) -> Vec<String> {
    vec![
        user(&format!("where is the {word} handled"), 0),
        says(&format!("In the {word} module"), 60),
    ]
}

/// `MANY_ENTRIES` entries, each mentioning "kiwi" with a number of its own.
fn many() -> Vec<String> {
    (0..MANY_ENTRIES)
        .map(|n| {
            if n % 2 == 0 {
                user(&format!("check kiwi batch number {n}"), n)
            } else {
                says(&format!("Checked kiwi batch number {n}"), n)
            }
        })
        .collect()
}

fn joined(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

// ---------------------------------------------------------------------------
// The daemon, seeded before it starts, and the agent of `LEDGER`.

struct Fixture {
    daemon: Daemon,
    /// The token of a shell in `LEDGER`: an agent of `HOME`.
    token: String,
    _attach: Attach,
}

impl Fixture {
    async fn start() -> Self {
        let dir = folder();
        let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
        for (id, name, at) in [(HOME, "Payments", "home"), (AWAY, "Research", "away")] {
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
        for (id, channel, title, lines) in [
            (LEDGER, HOME, "Ledger export", ledger()),
            (SIBLING, HOME, "Rate limiter", mentions("wombat")),
            (MANY, HOME, "Kiwi batches", many()),
            (FAR, AWAY, "Tide tables", mentions("wombat")),
        ] {
            seed_session(&store, dir.path(), id, channel, title, &lines);
        }
        drop(store);
        let daemon = Daemon::start_in(dir).await;

        // Everything is mirrored once each conversation's last line is found.
        user_hits_when(&daemon, "regression", |hits| hits.len() == 1).await;
        user_hits_when(&daemon, "wombat", |hits| hits.len() == 2).await;
        user_hits_when(&daemon, "kiwi", |hits| hits.len() == MANY_TURNS).await;

        let (token, attach) = shell_token(&daemon, &id(LEDGER)).await;
        Self {
            daemon,
            token,
            _attach: attach,
        }
    }

    async fn mcp(&self) -> Mcp {
        Mcp::start(&self.daemon.socket, &self.token).await
    }

    /// Every hit of SearchSessions for `text`, asked over gRPC by the same
    /// agent the MCP server works for.
    async fn agent_grpc_hits(&self, text: &str) -> Vec<pb::SearchHit> {
        grpc_hits(&self.daemon, text, Some(&self.token)).await.unwrap()
    }

    /// Every entry of ReadTurns for `session` in `first..=last`, asked over
    /// gRPC by the agent.
    async fn agent_grpc_entries(
        &self,
        session: &str,
        first: Option<u32>,
        last: Option<u32>,
    ) -> Vec<pb::MirrorEntry> {
        let mut client = HistoryServiceClient::new(self.daemon.channel().await);
        let mut request = pb::ReadTurnsRequest {
            session: Some(id(session)),
            first_turn: first,
            last_turn: last,
            page: None,
        };
        let mut entries = Vec::new();
        for _ in 0..100 {
            let response = client
                .read_turns(asked(request.clone(), Some(&self.token)))
                .await
                .unwrap()
                .into_inner();
            entries.extend(response.entries);
            let next = response.page.map(|page| page.next_page_token).unwrap_or_default();
            if next.is_empty() {
                return entries;
            }
            request.page = Some(pb::PageRequest {
                page_size: 0,
                page_token: next,
            });
        }
        panic!("the read never reached a last page");
    }
}

fn seed_session(store: &Store, dir: &Path, id: &str, channel: &str, title: &str, lines: &[String]) {
    store
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
    let file: PathBuf = dir.join("conv").join(format!("{id}.jsonl"));
    std::fs::write(&file, joined(lines)).unwrap();
    store
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

fn id(session: &str) -> pb::SessionId {
    pb::SessionId {
        value: session.to_owned(),
    }
}

fn asked<T>(message: T, token: Option<&str>) -> Request<T> {
    let mut request = Request::new(message);
    if let Some(token) = token {
        request
            .metadata_mut()
            .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    }
    request
}

/// Every hit of SearchSessions for `text` naming no channel, following page
/// tokens; by the user, or by the agent holding `token`.
async fn grpc_hits(daemon: &Daemon, text: &str, token: Option<&str>) -> Result<Vec<pb::SearchHit>, Status> {
    let mut client: HistoryServiceClient<Channel> = HistoryServiceClient::new(daemon.channel().await);
    let mut request = pb::SearchSessionsRequest {
        query: text.to_owned(),
        ..Default::default()
    };
    let mut hits = Vec::new();
    for _ in 0..100 {
        let response = client
            .search_sessions(asked(request.clone(), token))
            .await?
            .into_inner();
        hits.extend(response.hits);
        let next = response.page.map(|page| page.next_page_token).unwrap_or_default();
        if next.is_empty() {
            return Ok(hits);
        }
        request.page = Some(pb::PageRequest {
            page_size: 0,
            page_token: next,
        });
    }
    panic!("the search never reached a last page");
}

/// The user's search for `text`, asked again until its hits are `enough`:
/// the mirror follows the conversations, it isn't asked to.
async fn user_hits_when(daemon: &Daemon, text: &str, enough: impl Fn(&[pb::SearchHit]) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        let seen = grpc_hits(daemon, text, None).await.unwrap();
        if enough(&seen) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the search for {text:?} never gave what was waited for; it last gave {} hits",
            seen.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ---------------------------------------------------------------------------
// One `humatad2 mcp` process, spoken to as an agent's client would.

struct Mcp {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl Mcp {
    async fn start(socket: &Path, token: &str) -> Self {
        let mut child = Command::new(binary("HUMATAD2"))
            .arg("mcp")
            .arg("--socket")
            .arg(socket)
            .env("HUMATA_SESSION_TOKEN", token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut mcp = Self {
            _child: child,
            stdin,
            stdout,
            next_id: 0,
        };
        let initialize = json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0" },
        });
        let reply = mcp.exchange("initialize", initialize).await;
        assert!(reply.get("error").is_none(), "{reply}");
        mcp.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await;
        mcp
    }

    async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    /// The whole JSON-RPC response to one request, skipping notifications.
    async fn exchange(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        loop {
            let line = tokio::time::timeout(WAIT, self.stdout.next_line())
                .await
                .expect("humatad2 mcp went quiet")
                .unwrap()
                .expect("humatad2 mcp closed its output");
            let message: Value = serde_json::from_str(&line).unwrap();
            if message.get("method").is_none() && message["id"] == json!(id) {
                return message;
            }
        }
    }

    /// The result of a request that the server took.
    async fn request(&mut self, method: &str, params: Value) -> Value {
        let reply = self.exchange(method, params).await;
        assert!(reply.get("error").is_none(), "{reply}");
        reply["result"].clone()
    }

    /// The tool result of a call the server took.
    async fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({ "name": tool, "arguments": arguments }))
            .await
    }

    /// What `tool` gave for `arguments` and each page after, following
    /// `next_page_token` until there is none.
    async fn pages(&mut self, tool: &str, arguments: Value) -> Vec<Value> {
        let mut arguments = arguments;
        let mut pages = Vec::new();
        for _ in 0..100 {
            let result = ok(&self.call(tool, arguments.clone()).await).clone();
            let next = next_token(&result);
            pages.push(result);
            match next {
                Some(token) => arguments["page_token"] = json!(token),
                None => return pages,
            }
        }
        panic!("{tool} never reached a last page");
    }
}

/// What a successful tool call returned.
#[track_caller]
fn ok(result: &Value) -> &Value {
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["outcome"], "applied", "{result}");
    &result["structuredContent"]["result"]
}

/// The kind of a refused tool call.
#[track_caller]
fn error_kind(result: &Value) -> &str {
    assert_eq!(result["isError"], true, "{result}");
    result["structuredContent"]["error"]["kind"]
        .as_str()
        .unwrap_or_default()
}

/// The field a call was refused over: the daemon's `invalid_field`, or the
/// server's invalid params when the schema already refuses it. `None` when
/// the call wasn't refused over a field.
fn refused_field(reply: &Value) -> Option<String> {
    if let Some(error) = reply.get("error") {
        return error["data"]["field"].as_str().map(str::to_owned);
    }
    let result = &reply["result"];
    if result["isError"] != true {
        return None;
    }
    let error = &result["structuredContent"]["error"];
    if error["kind"] != "invalid_field" {
        return None;
    }
    error["details"]["field"].as_str().map(str::to_owned)
}

/// A page's `next_page_token`; absent, null and empty are all "no more".
fn next_token(result: &Value) -> Option<String> {
    result["next_page_token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
}

fn listed<'a>(result: &'a Value, field: &str) -> &'a [Value] {
    result[field]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| panic!("no {field} list in {result}"))
}

fn text_of<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("no {field} text in {value}"))
}

fn number_of(value: &Value, field: &str) -> u64 {
    value[field]
        .as_u64()
        .unwrap_or_else(|| panic!("no {field} number in {value}"))
}

/// What a hit names and shows: (session, channel, machine, agent, turn,
/// snippet).
type Place = (String, String, String, String, u64, String);

fn mcp_places(hits: &[Value]) -> Vec<Place> {
    let mut places: Vec<Place> = hits
        .iter()
        .map(|hit| {
            (
                text_of(hit, "session").to_owned(),
                text_of(hit, "channel").to_owned(),
                text_of(hit, "machine").to_owned(),
                text_of(hit, "agent").to_owned(),
                number_of(hit, "turn"),
                text_of(hit, "snippet").to_owned(),
            )
        })
        .collect();
    places.sort();
    places
}

fn grpc_places(hits: &[pb::SearchHit]) -> Vec<Place> {
    let value = |id: Option<String>| id.unwrap_or_default();
    let mut places: Vec<Place> = hits
        .iter()
        .map(|hit| {
            (
                value(hit.session.as_ref().map(|id| id.value.clone())),
                value(hit.channel.as_ref().map(|id| id.value.clone())),
                value(hit.machine.as_ref().map(|id| id.value.clone())),
                hit.agent.clone(),
                u64::from(hit.turn),
                hit.snippet.clone(),
            )
        })
        .collect();
    places.sort();
    places
}

/// An entry as `session_read` gives it: (seq, role, text).
type Turn = (u64, String, String);

fn mcp_turns(turns: &[Value]) -> Vec<Turn> {
    turns
        .iter()
        .map(|turn| {
            (
                number_of(turn, "seq"),
                text_of(turn, "role").to_owned(),
                text_of(turn, "text").to_owned(),
            )
        })
        .collect()
}

fn grpc_turns(entries: &[pb::MirrorEntry]) -> Vec<Turn> {
    entries
        .iter()
        .map(|entry| {
            let role = match entry.role() {
                Role::User => "user",
                Role::Agent => "agent",
                Role::Tool => "tool",
                Role::Unspecified => "unspecified",
            };
            (entry.seq, role.to_owned(), entry.text.clone())
        })
        .collect()
}

fn seqs(turns: &[Turn]) -> Vec<u64> {
    turns.iter().map(|turn| turn.0).collect()
}

// ---------------------------------------------------------------------------
// The tools are there.

#[tokio::test(flavor = "multi_thread")]
async fn both_tools_are_listed_and_need_what_they_search_or_read() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let described = mcp
        .call(
            "humata_describe",
            json!({ "names": ["sessions_search", "session_read"] }),
        )
        .await;
    let tools = listed(&described["structuredContent"], "tools");

    for (name, needed) in [("sessions_search", "query"), ("session_read", "session")] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} isn't listed"));
        let required = tool["inputSchema"]["required"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(required.contains(&json!(needed)), "{name}: {tool}");
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{name}: {tool}");
    }
}

// ---------------------------------------------------------------------------
// Searching.

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_finds_a_turn_of_its_own_channel() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let found = mcp.call("sessions_search", json!({ "query": "quokka" })).await;
    let hits = listed(ok(&found), "hits");

    assert_eq!(hits.len(), 1, "{found}");
    let hit = &hits[0];
    assert_eq!(text_of(hit, "session"), LEDGER);
    assert_eq!(text_of(hit, "channel"), HOME);
    assert_eq!(text_of(hit, "agent"), AGENT);
    // The second prompt.
    assert_eq!(number_of(hit, "turn"), 2);
    assert!(text_of(hit, "snippet").contains("quokka"), "{hit}");
    assert!(hit["as_of"].is_string(), "{hit}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_that_names_no_channel_gets_only_its_own() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let unnamed = mcp.pages("sessions_search", json!({ "query": "wombat" })).await;
    let unnamed: Vec<Value> = unnamed.iter().flat_map(|page| listed(page, "hits").to_vec()).collect();

    // SIBLING's turn; FAR's, in the other channel, isn't searched.
    let mut sessions: Vec<&str> = unnamed.iter().map(|hit| text_of(hit, "session")).collect();
    sessions.dedup();
    assert_eq!(sessions, [SIBLING], "{unnamed:?}");

    // Naming its own channel is the same search.
    let named = mcp
        .pages("sessions_search", json!({ "query": "wombat", "channel": HOME }))
        .await;
    let named: Vec<Value> = named.iter().flat_map(|page| listed(page, "hits").to_vec()).collect();
    assert_eq!(mcp_places(&named), mcp_places(&unnamed));
}

/// LD-25: the tool and `humata sessions search` (SearchSessions) give the
/// same results for the same query by the same caller.
#[tokio::test(flavor = "multi_thread")]
async fn the_tool_finds_what_search_sessions_finds() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    for query in ["wombat", "quokka", "csv ledg", "kiwi"] {
        let pages = mcp.pages("sessions_search", json!({ "query": query })).await;
        let tool: Vec<Value> = pages.iter().flat_map(|page| listed(page, "hits").to_vec()).collect();
        let grpc = fixture.agent_grpc_hits(query).await;

        assert!(!grpc.is_empty(), "{query}: the seed has hits for it");
        assert_eq!(mcp_places(&tool), grpc_places(&grpc), "{query}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_is_refused_a_search_of_another_channel() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let refused = mcp
        .call("sessions_search", json!({ "query": "wombat", "channel": AWAY }))
        .await;

    assert_eq!(error_kind(&refused), "denied", "{refused}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_search_for_nothing_is_refused_over_its_query() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    for nothing in ["", "  \t "] {
        let reply = mcp
            .exchange(
                "tools/call",
                json!({ "name": "sessions_search", "arguments": { "query": nothing } }),
            )
            .await;
        let field = refused_field(&reply);
        assert!(
            field.as_deref().is_some_and(|field| field.contains("query")),
            "{nothing:?}: {reply}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_search_pages_through_its_hits_without_repeats() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let first = mcp.call("sessions_search", json!({ "query": "kiwi" })).await;
    let first = ok(&first);
    assert!(
        next_token(first).is_some(),
        "{MANY_TURNS} hits don't fit one page: {first}"
    );
    assert!(!listed(first, "hits").is_empty(), "{first}");

    let pages = mcp.pages("sessions_search", json!({ "query": "kiwi" })).await;
    assert!(pages.len() > 1, "{pages:?}");
    assert!(pages.iter().all(|page| !listed(page, "hits").is_empty()), "{pages:?}");
    let paged: Vec<Value> = pages.iter().flat_map(|page| listed(page, "hits").to_vec()).collect();

    // Each turn once: every snippet names its own batch number.
    let mut places = mcp_places(&paged);
    let total = places.len();
    places.dedup();
    assert_eq!(places.len(), total, "a hit came twice");
    assert_eq!(total, MANY_TURNS);
    assert_eq!(places, grpc_places(&fixture.agent_grpc_hits("kiwi").await));
}

// ---------------------------------------------------------------------------
// Reading.

#[tokio::test(flavor = "multi_thread")]
async fn a_read_gives_a_sessions_turns_in_order() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let read = mcp.call("session_read", json!({ "session": LEDGER })).await;
    let all = mcp_turns(listed(ok(&read), "turns"));

    assert_eq!(seqs(&all), [0, 1, 2, 3, 4, 5, 6, 7]);
    let roles: Vec<&str> = all.iter().map(|turn| turn.1.as_str()).collect();
    assert_eq!(roles, ["user", "agent", "user", "tool", "tool", "agent", "user", "agent"]);
    assert_eq!(all[0].2, "export the ledger as csv");
    assert_eq!(all[7].2, "Wrote the regression test under billing/tests");
    // LD-25: what ReadTurns gives the same caller.
    assert_eq!(all, grpc_turns(&fixture.agent_grpc_entries(LEDGER, None, None).await));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_keeps_to_the_turns_asked_for() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let second = mcp
        .call("session_read", json!({ "session": LEDGER, "first_turn": 2, "last_turn": 2 }))
        .await;
    let second = mcp_turns(listed(ok(&second), "turns"));
    // The prompt, the tool call, its answer and what the agent said.
    assert_eq!(seqs(&second), [2, 3, 4, 5]);
    assert_eq!(second[0].2, "add a column for quokka refunds");

    let from_second = mcp
        .call("session_read", json!({ "session": LEDGER, "first_turn": 2 }))
        .await;
    assert_eq!(seqs(&mcp_turns(listed(ok(&from_second), "turns"))), [2, 3, 4, 5, 6, 7]);

    let to_first = mcp
        .call("session_read", json!({ "session": LEDGER, "last_turn": 1 }))
        .await;
    let to_first = mcp_turns(listed(ok(&to_first), "turns"));
    assert_eq!(seqs(&to_first), [0, 1]);
    assert_eq!(
        to_first,
        grpc_turns(&fixture.agent_grpc_entries(LEDGER, None, Some(1)).await)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_pages_through_a_long_session() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let first = mcp.call("session_read", json!({ "session": MANY })).await;
    assert!(
        next_token(ok(&first)).is_some(),
        "{MANY_ENTRIES} entries don't fit one page: {first}"
    );

    let pages = mcp.pages("session_read", json!({ "session": MANY })).await;
    assert!(pages.len() > 1, "{pages:?}");
    assert!(pages.iter().all(|page| !listed(page, "turns").is_empty()), "{pages:?}");
    let paged: Vec<Turn> = pages
        .iter()
        .flat_map(|page| mcp_turns(listed(page, "turns")))
        .collect();

    // Every entry once, in order.
    let expected: Vec<u64> = (0..MANY_ENTRIES as u64).collect();
    assert_eq!(seqs(&paged), expected);
    assert_eq!(paged, grpc_turns(&fixture.agent_grpc_entries(MANY, None, None).await));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_reads_its_channels_sessions_and_is_refused_another_channels() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let sibling = mcp.call("session_read", json!({ "session": SIBLING })).await;
    assert_eq!(listed(ok(&sibling), "turns").len(), 2, "{sibling}");

    for arguments in [
        json!({ "session": FAR }),
        json!({ "session": FAR, "first_turn": 1, "last_turn": 1 }),
    ] {
        let refused = mcp.call("session_read", arguments).await;
        assert_eq!(error_kind(&refused), "denied", "{refused}");
    }
}
