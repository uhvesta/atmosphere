//! `humatad mcp`: the MCP server an agent's CLI starts for one session, on
//! its standard input and output, so the agent can read and change the
//! board and answer review threads. It works through the daemon's socket,
//! as the app and the CLI do; the session is `HUMATA_SESSION`.
//!
//! Answers to review threads aren't stored here: the call is in the
//! session's history, and the daemon reads it from there as it reads
//! `[thread N]` tags, so it gets the turn whose changes are the fix.

use std::path::{Path, PathBuf};

use humata_wire::{Client, WireError, pb};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// The MCP versions this server speaks, newest first.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error(
        "HUMATA_SESSION isn't set, so there is no session to serve; Humata sets it when it starts the agent, so run this only from there"
    )]
    NoSession,

    #[error(
        "couldn't reach humatad at {path}; check that the Humata app or `humatad daemon` is running"
    )]
    Connect {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(transparent)]
    Wire(#[from] WireError),

    #[error("lost the agent's connection; it starts this server again when it needs it")]
    Pipe {
        #[source]
        source: std::io::Error,
    },
}

/// Serves `session`'s agent on stdin and stdout until it hangs up.
pub async fn mcp_stdio(socket: &Path, session: Option<String>) -> Result<(), McpError> {
    let session = session
        .filter(|session| !session.is_empty())
        .ok_or(McpError::NoSession)?;
    let stream = UnixStream::connect(socket)
        .await
        .map_err(|source| McpError::Connect {
            path: socket.to_owned(),
            source,
        })?;
    let (reader, writer) = stream.into_split();
    let client = Client::connect(reader, writer, pb::Role::Client, crate::VERSION).await?;
    serve(
        &Server { client, session },
        tokio::io::stdin(),
        tokio::io::stdout(),
    )
    .await
}

/// Answers each JSON-RPC message, one per line, until the input ends.
async fn serve(
    server: &Server,
    input: impl AsyncRead + Unpin,
    mut output: impl AsyncWrite + Unpin,
) -> Result<(), McpError> {
    let mut lines = BufReader::new(input).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|source| McpError::Pipe { source })?
    {
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(message) => server.handle(&message).await,
            Err(_) => Some(error(&Value::Null, -32700, "that isn't JSON")),
        };
        if let Some(answer) = answer {
            let mut text = answer.to_string();
            text.push('\n');
            output
                .write_all(text.as_bytes())
                .await
                .map_err(|source| McpError::Pipe { source })?;
            output
                .flush()
                .await
                .map_err(|source| McpError::Pipe { source })?;
        }
    }
    Ok(())
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

struct Server {
    client: Client,
    session: String,
}

impl Server {
    /// The answer to one message; none for a notification.
    async fn handle(&self, message: &Value) -> Option<Value> {
        let id = message.get("id")?;
        let params = &message["params"];
        let result = match message["method"].as_str().unwrap_or_default() {
            "initialize" => {
                let asked = params["protocolVersion"].as_str().unwrap_or_default();
                let version = PROTOCOL_VERSIONS
                    .into_iter()
                    .find(|version| *version == asked)
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "humata", "version": crate::VERSION},
                    "instructions": instructions(self.place().await),
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({"tools": tools(self.place().await)}),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                let empty = json!({});
                let arguments = params.get("arguments").unwrap_or(&empty);
                let (text, failed) = match self.call(name, arguments).await {
                    Ok(text) => (text, false),
                    Err(text) => (text, true),
                };
                json!({"content": [{"type": "text", "text": text}], "isError": failed})
            }
            method => {
                return Some(error(
                    id,
                    -32601,
                    &format!("humata's MCP server has no method {method:?}"),
                ));
            }
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    /// What a tool answers, or why it failed, for the model to read.
    async fn call(&self, name: &str, arguments: &Value) -> Result<String, String> {
        let text = |field: &str| arguments[field].as_str().map(str::trim).map(str::to_owned);
        match name {
            "board" => self.board(arguments["all"] == json!(true)).await,
            "search_sessions" => {
                let query = required(text("query"), "query")?;
                // This session's project, unless asked for another or for
                // every project.
                let project_id = match text("project_id").filter(|id| !id.is_empty()) {
                    Some(id) => id,
                    None if arguments["everywhere"] == json!(true) => String::new(),
                    None => self.session().await?.project_id,
                };
                let search = pb::Search {
                    query,
                    project_id,
                    // One more than shown, to know if there are more.
                    limit: SEARCH_HITS + 1,
                    ..pb::Search::default()
                };
                match self.ask(pb::request::Kind::Search(search)).await? {
                    pb::response::Kind::SearchResults(mut results) => {
                        let more = results.hits.len() > SEARCH_HITS as usize;
                        results.hits.truncate(SEARCH_HITS as usize);
                        let mut text = found(&results);
                        if more {
                            text.push_str(&format!(
                                "There are more than {SEARCH_HITS} hits; add words or a time to narrow them.\n"
                            ));
                        }
                        Ok(text)
                    }
                    other => Err(unexpected(&other)),
                }
            }
            "read_turn" => {
                let session_id = required(text("session_id"), "session_id")?;
                let turn = arguments["turn"]
                    .as_u64()
                    .filter(|turn| *turn > 0)
                    .ok_or_else(|| "give turn, a number from 1, from a search hit".to_owned())?;
                let read = pb::ReadHistory {
                    session_id: session_id.clone(),
                };
                match self.ask(pb::request::Kind::ReadHistory(read)).await? {
                    pb::response::Kind::History(history) => {
                        let event = text("event_id").unwrap_or_default();
                        turn_text(&history.events, turn as u32, &event).ok_or_else(|| {
                            format!(
                                "session {session_id} has no turn {turn}; take it from a search hit"
                            )
                        })
                    }
                    other => Err(unexpected(&other)),
                }
            }
            "read_skill" => {
                let read = pb::ReadSkill {
                    session_id: self.session.clone(),
                    name: required(text("name"), "name")?,
                };
                match self.ask(pb::request::Kind::ReadSkill(read)).await? {
                    pb::response::Kind::Skill(skill) => {
                        Ok(format!("# Skill: {}\n\n{}", skill.name, skill.text.trim()))
                    }
                    other => Err(unexpected(&other)),
                }
            }
            "read_item" => {
                let item_id = required(text("item_id"), "item_id")?;
                // The host's copy of this session's specs answers while the
                // Mac is away.
                let read = pb::ReadSpec {
                    session_id: self.session.clone(),
                    item_id,
                };
                let spec = match self.ask(pb::request::Kind::ReadSpec(read)).await? {
                    pb::response::Kind::Spec(spec) => spec,
                    other => return Err(unexpected(&other)),
                };
                Ok(format!(
                    "{} ({})\n\n{}",
                    spec.title,
                    spec.item_id,
                    if spec.spec.trim().is_empty() {
                        "It has no spec."
                    } else {
                        &spec.spec
                    }
                ))
            }
            // Proposals are answered here: the call itself, in the session's
            // history, is the proposal, which the person applies or declines
            // in Humata. So proposing works while the Mac is away.
            "propose_task" => {
                let title = required(text("title"), "title")?;
                if let Some(parent) = text("parent_id").filter(|id| !id.is_empty()) {
                    self.check_item(&parent).await?;
                }
                Ok(format!(
                    "Proposed {title:?}. It reaches the board only if the person applies it, perhaps edited; don't work on it until then."
                ))
            }
            "propose_spec_edit" => {
                let item_id = required(text("item_id"), "item_id")?;
                let spec = arguments["spec"]
                    .as_str()
                    .is_some_and(|spec| !spec.trim().is_empty());
                let replacements = replacements(arguments)?;
                if spec && !replacements.is_empty() {
                    return Err("give spec or replacements, not both".to_owned());
                }
                if !spec
                    && replacements.is_empty()
                    && text("title").is_none_or(|title| title.is_empty())
                {
                    return Err(
                        "give spec, the whole new spec in Markdown, or replacements, or title"
                            .to_owned(),
                    );
                }
                if let Some(current) = self.check_item(&item_id).await?
                    && let Err(why) = replaced(&current, &replacements)
                {
                    return Err(why);
                }
                Ok(format!(
                    "Proposed a change to {item_id}. It's made only if the person applies it; until then read_item shows it as it was."
                ))
            }
            "ask_person" => {
                let ask = pb::AskPerson {
                    session_id: self.session.clone(),
                    text: required(text("question"), "question")?,
                    client_id: String::new(),
                };
                match self.ask(pb::request::Kind::AskPerson(ask)).await? {
                    pb::response::Kind::RequestMade(made) if made.request.is_none() => Ok(
                        "Asked; Humata on the person's Mac isn't connected, so it waits on this host and reaches their inbox when the Mac reconnects. Their answer comes as your next message. End your turn unless you can carry on without it."
                            .to_owned(),
                    ),
                    pb::response::Kind::RequestMade(_) => Ok(
                        "Asked; it waits in the person's inbox, and their answer comes as your next message. End your turn unless you can carry on without it."
                            .to_owned(),
                    ),
                    other => Err(unexpected(&other)),
                }
            }
            "request_move" => {
                let status = required(text("status"), "status")?;
                let status = status_named(&status)
                    .filter(|status| asked_for(*status))
                    .ok_or_else(|| {
                        format!(
                            "{status:?} isn't a move to ask for; give in_review, done or wont_do"
                        )
                    })?;
                let summary = required(text("summary"), "summary")?;
                // Left empty, the daemon takes this session's task, so the
                // request needs nothing from it while the Mac is away.
                let item_id = text("item_id").unwrap_or_default();
                let asked = pb::RequestMove {
                    session_id: self.session.clone(),
                    item_id,
                    status: status.into(),
                    summary,
                    client_id: String::new(),
                };
                match self.ask(pb::request::Kind::RequestMove(asked)).await? {
                    pb::response::Kind::RequestMade(made) if made.request.is_none() => Ok(format!(
                        "Asked the person to move it to {}; Humata on their Mac isn't connected, so it waits on this host and reaches their inbox when the Mac reconnects. They look over your changes, then approve, move it themselves, or send feedback as your next message.",
                        status_name(status)
                    )),
                    pb::response::Kind::RequestMade(_) => Ok(format!(
                        "Asked the person to move it to {}. They look over your changes, then approve, move it themselves, or send feedback as your next message.",
                        status_name(status)
                    )),
                    other => Err(unexpected(&other)),
                }
            }
            "review_threads" => {
                let threads = self.threads().await?;
                let open: Vec<_> = threads.iter().filter(|thread| !thread.resolved).collect();
                if open.is_empty() {
                    return Ok("No review threads are open on this session.".to_owned());
                }
                Ok(open
                    .into_iter()
                    .map(describe_thread)
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "reply_to_thread" => {
                let thread_id = match &arguments["thread_id"] {
                    Value::Number(id) => id.to_string(),
                    _ => required(text("thread_id"), "thread_id")?,
                };
                required(text("text"), "text")?;
                let threads = self.threads().await?;
                match threads.iter().find(|thread| thread.id == thread_id) {
                    None => Err(format!(
                        "this session has no review thread {thread_id}; see review_threads for the open ones"
                    )),
                    Some(thread) if thread.resolved => Err(format!(
                        "thread {thread_id} is resolved; answer only the open ones"
                    )),
                    Some(_) => Ok(format!(
                        "Your answer shows under thread {thread_id} in Humata, with this turn's changes as the fix."
                    )),
                }
            }
            _ => Err(format!("humata has no tool {name:?}; see tools/list")),
        }
    }

    /// Fails when the board has no item `id`, and gives its spec. While it
    /// can't be read, as when the Mac is away, it goes unchecked.
    async fn check_item(&self, id: &str) -> Result<Option<String>, String> {
        let read = pb::ReadSpec {
            session_id: self.session.clone(),
            item_id: id.to_owned(),
        };
        match self.client.call(pb::request::Kind::ReadSpec(read)).await {
            Ok(pb::response::Kind::Spec(spec)) => Ok(Some(spec.spec)),
            Err(humata_wire::ClientError::Refused {
                code: pb::ErrorCode::UnknownItem,
                ..
            }) => Err(format!(
                "the board has no task or subtask {id}; take the id from board"
            )),
            _ => Ok(None),
        }
    }

    async fn ask(&self, request: pb::request::Kind) -> Result<pb::response::Kind, String> {
        self.client
            .call(request)
            .await
            .map_err(|error| humata_wire::describe(&error))
    }

    async fn session(&self) -> Result<pb::SessionInfo, String> {
        let listed = self
            .ask(pb::request::Kind::ListSessions(pb::ListSessions::default()))
            .await?;
        let pb::response::Kind::Sessions(sessions) = listed else {
            return Err(unexpected(&listed));
        };
        sessions
            .sessions
            .into_iter()
            .find(|session| session.id == self.session)
            .ok_or_else(|| {
                format!(
                    "humatad doesn't know the session {}; it may have been removed",
                    self.session
                )
            })
    }

    async fn items(&self, project: &str) -> Result<Vec<pb::Item>, String> {
        let listed = self
            .ask(pb::request::Kind::ListItems(pb::ListItems {
                project_id: project.to_owned(),
            }))
            .await?;
        match listed {
            pb::response::Kind::Items(items) => Ok(items.items),
            other => Err(unexpected(&other)),
        }
    }

    /// Where this session is, to offer only the tools that work there. When
    /// humatad can't say, every tool is offered.
    async fn place(&self) -> Place {
        match self.session().await {
            Err(_) => Place::Unknown,
            Ok(session) if session.project_id.is_empty() => Place::Alone,
            Ok(session) if session.item_id.is_empty() => Place::Project,
            Ok(_) => Place::Task,
        }
    }

    async fn board(&self, all: bool) -> Result<String, String> {
        let session = self.session().await?;
        if session.project_id.is_empty() {
            return Ok("This session isn't in a project, so it has no board.".to_owned());
        }
        let items = self.items(&session.project_id).await?;
        Ok(board_text(&items, &session.item_id, all))
    }

    async fn threads(&self) -> Result<Vec<pb::ReviewThread>, String> {
        let listed = self
            .ask(pb::request::Kind::ListThreads(pb::ListThreads {
                session_id: self.session.clone(),
            }))
            .await?;
        match listed {
            pb::response::Kind::Threads(threads) => Ok(threads.threads),
            other => Err(unexpected(&other)),
        }
    }
}

/// Where a session is, which decides the tools it gets: one outside a
/// project has no board to read or propose to, and one on no task has none
/// to ask to move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// humatad couldn't say, as when it's restarting.
    Unknown,
    /// A free chat, in no project.
    Alone,
    /// In a project, on no task.
    Project,
    /// On a task or subtask.
    Task,
}

impl Place {
    fn has_board(self) -> bool {
        self != Place::Alone
    }

    fn has_task(self) -> bool {
        matches!(self, Place::Unknown | Place::Task)
    }
}

/// The board as an agent reads it: the open items and the session's own,
/// each with its task, unless `all`; and how many are left out.
fn board_text(items: &[pb::Item], own: &str, all: bool) -> String {
    if items.is_empty() {
        return "The board is empty; propose_task to suggest one.".to_owned();
    }
    let wanted = |item: &pb::Item| {
        all || item.id == own
            || !matches!(item.status(), pb::ItemStatus::Done | pb::ItemStatus::WontDo)
    };
    let shown: Vec<bool> = items
        .iter()
        .map(|item| {
            wanted(item)
                || items
                    .iter()
                    .any(|child| child.parent_id == item.id && wanted(child))
        })
        .collect();
    let mut text = String::from("Tasks, with their subtasks under them:\n");
    for (item, _) in items.iter().zip(&shown).filter(|(_, shown)| **shown) {
        let indent = if item.parent_id.is_empty() { "" } else { "  " };
        let yours = if item.id == own {
            "  <- this session works on it"
        } else {
            ""
        };
        text.push_str(&format!(
            "{indent}- [{}] {} (id {}){yours}\n",
            status_name(item.status()),
            item.title,
            item.id
        ));
    }
    let left_out = shown.iter().filter(|shown| !**shown).count();
    if left_out > 0 {
        text.push_str(&format!(
            "{left_out} done or won't-do items are left out; call board with all to list them too.\n"
        ));
    }
    text
}

fn describe_thread(thread: &pb::ReviewThread) -> String {
    let file = thread
        .file
        .as_ref()
        .map(|file| format!("{}/{}", file.repository, file.path))
        .unwrap_or_default();
    let mut text = format!(
        "Thread {} on {file}, lines {}-{}{}:\n",
        thread.id,
        thread.start_line,
        thread.end_line,
        if thread.outdated {
            " (the lines have changed since)"
        } else {
            ""
        }
    );
    for line in &thread.code {
        text.push_str(&format!("    {line}\n"));
    }
    for comment in &thread.comments {
        let who = match comment.author() {
            pb::CommentAuthor::You => "The person".to_owned(),
            pb::CommentAuthor::Agent => "You".to_owned(),
            pb::CommentAuthor::Github => format!("@{} on GitHub", comment.login),
        };
        text.push_str(&format!("{who}: {}\n", comment.text.trim()));
    }
    text
}

/// The most hits search_sessions gives an agent, so they fit its context.
const SEARCH_HITS: u32 = 20;

/// Search results as an agent reads them: grouped by session, best first,
/// so each session's id is said once; under it where each hit is and the
/// text around it, the words found in bold.
fn found(results: &pb::SearchResults) -> String {
    if results.hits.is_empty() {
        return format!(
            "Nothing found for {:?}. Try fewer words, or others that mean the same.",
            results.words
        );
    }
    // Specs belong to no session; they're grouped under "".
    let mut sessions: Vec<&str> = Vec::new();
    for hit in &results.hits {
        if !sessions.contains(&hit.session_id.as_str()) {
            sessions.push(&hit.session_id);
        }
    }
    let mut text = String::new();
    for session in sessions {
        let hits: Vec<&pb::SearchHit> = results
            .hits
            .iter()
            .filter(|hit| hit.session_id == session)
            .collect();
        match hits.first() {
            Some(first) if !session.is_empty() => {
                text.push_str(&format!("Session {} ({session}):\n", first.session_name))
            }
            _ => text.push_str("Specs:\n"),
        }
        for hit in hits {
            let snippet = hit
                .snippet
                .replace([crate::store::MARK_START, crate::store::MARK_END], "**")
                .replace('\n', " ");
            let related = if hit.related { " (a related word)" } else { "" };
            let place = match hit.kind() {
                pb::SearchHitKind::Spec => format!("item {}", hit.item_id),
                pb::SearchHitKind::Comment => format!("review thread {}", hit.thread_id),
                kind => format!(
                    "turn {}, {}, event {}",
                    hit.turn,
                    match kind {
                        pb::SearchHitKind::Message => "the person",
                        pb::SearchHitKind::Reply => "the agent",
                        pb::SearchHitKind::ToolCall => "a tool call",
                        _ => "a tool's output",
                    },
                    hit.event_id
                ),
            };
            text.push_str(&format!("  - {place}{related}: {snippet}\n"));
        }
    }
    text
}

/// The most of a turn read_turn gives, so it fits the agent's context.
const TURN_TEXT: usize = 16 * 1024;
/// The most of each tool's arguments or output in it.
const TURN_PART: usize = 1500;

/// Turn `turn` of `events` (counted from 1, as search hits count them) as
/// an agent reads it. When it's too long, it starts at `event`, the hit.
fn turn_text(events: &[pb::SessionEvent], turn: u32, event: &str) -> Option<String> {
    use pb::session_event::Kind;
    let mut at = 1;
    let mut parts: Vec<(String, String)> = Vec::new();
    for found in events {
        if at == turn {
            let part = match &found.kind {
                Some(Kind::UserMessage(message)) => Some(format!("The person: {}", message.text)),
                Some(Kind::AssistantMessage(message)) => Some(format!("Agent: {}", message.text)),
                Some(Kind::ToolStarted(call)) => Some(format!(
                    "Tool {}: {}",
                    call.tool,
                    cut(&call.arguments_json, TURN_PART)
                )),
                Some(Kind::ToolCompleted(done)) => Some(format!(
                    "{}: {}",
                    if done.success { "Output" } else { "Failed" },
                    cut(&done.output, TURN_PART)
                )),
                _ => None,
            };
            if let Some(part) = part {
                parts.push((found.id.clone(), part));
            }
        }
        if matches!(found.kind, Some(Kind::TurnEnded(_))) {
            at += 1;
            if at > turn {
                break;
            }
        }
    }
    if at < turn || (parts.is_empty() && at == turn && events.is_empty()) {
        return None;
    }
    let total: usize = parts.iter().map(|(_, part)| part.len() + 1).sum();
    let from = if total > TURN_TEXT {
        parts.iter().position(|(id, _)| id == event).unwrap_or(0)
    } else {
        0
    };
    let mut text = String::new();
    if from > 0 {
        text.push_str("(Earlier in the turn left out.)\n");
    }
    for (_, part) in &parts[from..] {
        if text.len() + part.len() > TURN_TEXT {
            text.push_str("(The rest of the turn left out.)\n");
            break;
        }
        text.push_str(part);
        text.push('\n');
    }
    if text.is_empty() {
        text.push_str("Nothing was said or done in this turn.");
    }
    Some(text)
}

fn cut(text: &str, most: usize) -> String {
    if text.len() <= most {
        return text.to_owned();
    }
    let mut end = most;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… ({} bytes more)", &text[..end], text.len() - end)
}

/// The `replacements` of a spec edit: each `old_text` to `new_text`.
pub(crate) fn replacements(arguments: &Value) -> Result<Vec<(String, String)>, String> {
    let Some(listed) = arguments.get("replacements") else {
        return Ok(Vec::new());
    };
    let bad = || "replacements must be a list of {old_text, new_text}".to_owned();
    listed
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .map(
            |one| match (one["old_text"].as_str(), one["new_text"].as_str()) {
                (Some(old), Some(new)) if !old.is_empty() => Ok((old.to_owned(), new.to_owned())),
                _ => Err(bad()),
            },
        )
        .collect()
}

/// `spec` with each replacement made, or why one can't be: its text must be
/// there exactly once.
pub(crate) fn replaced(spec: &str, replacements: &[(String, String)]) -> Result<String, String> {
    let mut spec = spec.to_owned();
    for (old, new) in replacements {
        match spec.matches(old.as_str()).count() {
            1 => spec = spec.replacen(old.as_str(), new, 1),
            0 => {
                return Err(format!(
                    "the spec doesn't hold {old:?}; read_item it again and copy the text exactly"
                ));
            }
            count => {
                return Err(format!(
                    "the spec holds {old:?} {count} times; give more of the text around it"
                ));
            }
        }
    }
    Ok(spec)
}

fn required(value: Option<String>, field: &str) -> Result<String, String> {
    value
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("give {field}"))
}

fn unexpected(response: &pb::response::Kind) -> String {
    format!("humatad answered with {response:?}; update Humata so the app and humatad match")
}

/// The moves an agent asks the person for rather than makes.
fn asked_for(status: pb::ItemStatus) -> bool {
    matches!(
        status,
        pb::ItemStatus::InReview | pb::ItemStatus::Done | pb::ItemStatus::WontDo
    )
}

fn status_name(status: pb::ItemStatus) -> &'static str {
    use pb::ItemStatus::*;
    match status {
        Unspecified | Todo => "todo",
        InProgress => "in_progress",
        WaitingOnYou => "waiting_on_you",
        InReview => "in_review",
        Done => "done",
        WontDo => "wont_do",
    }
}

fn status_named(name: &str) -> Option<pb::ItemStatus> {
    use pb::ItemStatus::*;
    [Todo, InProgress, WaitingOnYou, InReview, Done, WontDo]
        .into_iter()
        .find(|status| status_name(*status) == name)
}

/// What the server tells the agent at the start, naming only the tools it
/// has at `place`.
fn instructions(place: Place) -> String {
    let mut text = String::from("Humata is directing this session.");
    if place.has_board() {
        text.push_str(
            " board and read_item show the project's tasks, subtasks and specs; you can't \
             change them, but propose_task and propose_spec_edit suggest changes the person \
             applies.",
        );
    }
    text.push_str(" ask_person asks the person something.");
    if place.has_task() {
        text.push_str(" request_move asks them to move your task once it's done.");
    }
    if place.has_board() {
        text.push_str(" Answer each review thread you're sent with reply_to_thread.");
    }
    text.push_str(" search_sessions finds earlier work and read_turn opens a hit.");
    if place.has_board() {
        text.push_str(" read_skill reads a project skill your instructions list.");
    }
    text
}

/// The tools a session at `place` has: outside a project, none for a board,
/// proposals, review or project skills; on no task, none to ask to move it.
/// Each listed tool costs tokens on every request, so only those that work
/// there are offered.
fn tools(place: Place) -> Value {
    let wanted = |tool: &Value| match tool["name"].as_str().unwrap_or_default() {
        "ask_person" | "search_sessions" | "read_turn" => true,
        "request_move" => place.has_task(),
        _ => place.has_board(),
    };
    let Value::Array(every) = every_tool() else {
        return json!([]);
    };
    Value::Array(every.into_iter().filter(|tool| wanted(tool)).collect())
}

fn every_tool() -> Value {
    let object = |properties: Value, required: &[&str]| json!({"type": "object", "properties": properties, "required": required});
    let string = |description: &str| json!({"type": "string", "description": description});
    json!([
        {
            "name": "board",
            "description": "The board of the project this session is in: its open tasks and subtasks and this session's own, their ids and statuses, and which one this session works on.",
            "inputSchema": object(json!({
                "all": {"type": "boolean", "description": "Done and won't-do items too."},
            }), &[]),
        },
        {
            "name": "read_item",
            "description": "A task's or subtask's title and spec, as it is now.",
            "inputSchema": object(json!({"item_id": string("From board.")}), &["item_id"]),
        },
        {
            "name": "propose_task",
            "description": "Propose a task, or a subtask under parent_id, with its spec. You can't change the board yourself: nothing is added until the person applies it from your proposal in the chat, perhaps after editing it.",
            "inputSchema": object(json!({
                "title": string("Short, like a commit subject."),
                "spec": string("What it is, why, and when it's done, in Markdown."),
                "parent_id": string("A task from board, to propose a subtask under it."),
                "note": string("Why you propose it, if it isn't plain from the spec."),
            }), &["title", "spec"]),
        },
        {
            "name": "propose_spec_edit",
            "description": "Propose a new spec or title for a task or subtask. You can't change the board yourself: it changes only when the person applies your proposal; sessions working on it are told to read it again.",
            "inputSchema": object(json!({
                "item_id": string("From board."),
                "spec": string("The whole new spec, in Markdown; leave out to keep it, or to give replacements."),
                "replacements": {
                    "type": "array",
                    "description": "Changes to make to the spec as it is, instead of giving it whole: each old_text, there exactly once, becomes new_text.",
                    "items": object(json!({
                        "old_text": string("Copied exactly from the spec."),
                        "new_text": string("What goes in its place."),
                    }), &["old_text", "new_text"]),
                },
                "title": string("A new title; leave out to keep it."),
                "note": string("What changed and why."),
            }), &["item_id"]),
        },
        {
            "name": "ask_person",
            "description": "Ask the person a question, or for their opinion, when you can't decide well without them. It waits in their inbox; their answer comes as your next message.",
            "inputSchema": object(json!({
                "question": string("What you need to know, with the options and what you'd pick, in Markdown."),
            }), &["question"]),
        },
        {
            "name": "request_move",
            "description": "When your work on a task is finished, ask the person to move it to in_review, done or wont_do. They look over your changes and approve it, or send feedback to carry on from.",
            "inputSchema": object(json!({
                "status": {"type": "string", "enum": ["in_review", "done", "wont_do"]},
                "summary": string("What you did and anything they should check, in Markdown."),
                "item_id": string("From board; defaults to this session's task."),
            }), &["status", "summary"]),
        },
        {
            "name": "review_threads",
            "description": "The person's open review threads on this session's changes: where each is, the code it's on, and what was said.",
            "inputSchema": object(json!({}), &[]),
        },
        {
            "name": "reply_to_thread",
            "description": "Answer one review thread you were sent: what you changed for it, or why not. One call per thread.",
            "inputSchema": object(json!({
                "thread_id": string("The thread's number, from the review or review_threads."),
                "text": string("Your answer, in Markdown."),
            }), &["thread_id", "text"]),
        },
        {
            "name": "search_sessions",
            "description": "Search what was said and done in this project's sessions, and their review comments and specs: to find how something was done or decided before. Each hit names the session, turn and event; read_turn opens it.",
            "inputSchema": object(json!({
                "query": string("Words to find, all of them. A time in it, such as \"last week\" or \"past 3 days\", narrows to then."),
                "project_id": string("Another project's sessions instead of this one's."),
                "everywhere": {"type": "boolean", "description": "Every project's sessions instead of this one's."},
            }), &["query"]),
        },
        {
            "name": "read_turn",
            "description": "What was said and done in one turn of a session, from a search_sessions hit.",
            "inputSchema": object(json!({
                "session_id": string("From the hit."),
                "turn": {"type": "integer", "minimum": 1, "description": "From the hit."},
                "event_id": string("From the hit; where to start when the turn is long."),
            }), &["session_id", "turn"]),
        },
        {
            "name": "read_skill",
            "description": "A project skill's text, by the name your instructions list it under. Read it before work it covers.",
            "inputSchema": object(json!({"name": string("The skill's name.")}), &["name"]),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The server's answers to `messages`, one per line in and out.
    async fn exchange(server: &Server, messages: &[Value]) -> Vec<Value> {
        let input: String = messages
            .iter()
            .map(|message| format!("{message}\n"))
            .collect();
        let mut output = Vec::new();
        serve(server, input.as_bytes(), &mut output)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        String::from_utf8_lossy(&output)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}")))
            .collect()
    }

    pub(crate) async fn call(server: &Server, tool: &str, arguments: Value) -> (String, bool) {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                             "params": {"name": tool, "arguments": arguments}});
        let answers = exchange(server, &[message]).await;
        let result = &answers[0]["result"];
        (
            result["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            result["isError"] == json!(true),
        )
    }

    pub(crate) fn server(client: Client, session: &str) -> Server {
        Server {
            client,
            session: session.to_owned(),
        }
    }

    fn said(id: &str, kind: pb::session_event::Kind) -> pb::SessionEvent {
        pb::SessionEvent {
            id: id.into(),
            kind: Some(kind),
            ..Default::default()
        }
    }

    #[test]
    fn a_turn_is_read_by_its_number_and_one_past_the_end_is_none() {
        use pb::session_event::Kind;
        let message = |id: &str, text: &str| {
            said(
                id,
                Kind::UserMessage(pb::UserMessage {
                    text: text.into(),
                    ..Default::default()
                }),
            )
        };
        let ended = |id: &str| said(id, Kind::TurnEnded(pb::TurnEnded::default()));
        let events = [
            message("a", "first ask"),
            ended("b"),
            message("c", "second ask"),
            ended("d"),
        ];
        let second = turn_text(&events, 2, "c").unwrap_or_default();
        assert!(
            second.contains("second ask") && !second.contains("first ask"),
            "{second}"
        );
        assert_eq!(turn_text(&events, 4, ""), None);
    }

    #[test]
    fn a_replacement_is_made_only_where_its_text_is_there_once() {
        let edit = |old: &str| vec![(old.to_owned(), "Y".to_owned())];
        assert_eq!(replaced("a X b", &edit("X")), Ok("a Y b".to_owned()));
        assert!(replaced("a b", &edit("X")).is_err());
        assert!(replaced("X X", &edit("X")).is_err());
        let listed = json!({"replacements": [{"old_text": "", "new_text": "Y"}]});
        assert!(replacements(&listed).is_err());
        assert_eq!(replacements(&json!({})), Ok(Vec::new()));
    }

    fn names(place: Place) -> Vec<String> {
        tools(place)
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn a_session_gets_only_the_tools_that_work_where_it_is() {
        let alone = names(Place::Alone);
        assert_eq!(alone, ["ask_person", "search_sessions", "read_turn"]);

        let project = names(Place::Project);
        assert!(project.iter().any(|name| name == "board"), "{project:?}");
        assert!(project.iter().any(|name| name == "propose_task"));
        assert!(!project.iter().any(|name| name == "request_move"));

        let task = names(Place::Task);
        assert!(task.iter().any(|name| name == "request_move"), "{task:?}");
        assert_eq!(task, names(Place::Unknown));
        assert_eq!(task.len(), project.len() + 1);
    }

    fn board_item(id: &str, parent: &str, status: pb::ItemStatus) -> pb::Item {
        pb::Item {
            id: id.into(),
            parent_id: parent.into(),
            title: format!("title of {id}"),
            status: status.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_board_lists_open_items_and_the_sessions_own_unless_asked_for_all() {
        use pb::ItemStatus::{Done, InProgress, Todo, WontDo};
        let items = [
            board_item("old-task", "", Done),
            board_item("old-sub", "old-task", Done),
            board_item("closed-task", "", Done),
            board_item("open-sub", "closed-task", Todo),
            board_item("dropped", "", WontDo),
            board_item("open-task", "", InProgress),
        ];
        let listed = board_text(&items, "old-sub", false);
        let shown = |id: &str| listed.contains(&format!("(id {id})"));
        // Open items, the session's own, and the task of each.
        for id in [
            "old-task",
            "old-sub",
            "closed-task",
            "open-sub",
            "open-task",
        ] {
            assert!(shown(id), "{id}: {listed}");
        }
        assert!(!shown("dropped"), "{listed}");

        let all = board_text(&items, "old-sub", true);
        assert!(all.contains("(id dropped)"), "{all}");
    }

    fn hit(session: &str, turn: u32) -> pb::SearchHit {
        pb::SearchHit {
            kind: pb::SearchHitKind::Message.into(),
            session_id: session.into(),
            session_name: format!("name of {session}"),
            event_id: format!("event-{turn}"),
            turn,
            snippet: "the words".into(),
            ..Default::default()
        }
    }

    #[test]
    fn search_hits_name_each_session_once_and_keep_every_hit() {
        let results = pb::SearchResults {
            hits: vec![hit("first", 1), hit("second", 2), hit("first", 3)],
            ..Default::default()
        };
        let text = found(&results);
        assert_eq!(text.matches("(first)").count(), 1, "{text}");
        assert_eq!(text.matches("(second)").count(), 1, "{text}");
        for event in ["event-1", "event-2", "event-3"] {
            assert!(text.contains(event), "{event}: {text}");
        }
        // The best session comes first, with both its hits under it.
        let at = |part: &str| text.find(part).unwrap_or(usize::MAX);
        assert!(
            at("event-1") < at("event-3") && at("event-3") < at("(second)"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn it_starts_lists_its_tools_and_skips_notifications() {
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let client = crate::tests::daemon(state.path(), crate::hosts::Hosts::default()).await;
        let server = server(client, "none");
        let answers = exchange(
            &server,
            &[
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                       "params": {"protocolVersion": "2025-03-26", "capabilities": {}}}),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
                json!({"jsonrpc": "2.0", "id": 3, "method": "nothing/here"}),
            ],
        )
        .await;
        assert_eq!(answers.len(), 3);
        assert_eq!(answers[0]["result"]["protocolVersion"], "2025-03-26");
        let names: Vec<&str> = answers[1]["result"]["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool["name"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        assert!(names.contains(&"propose_task") && names.contains(&"reply_to_thread"));
        assert!(!names.contains(&"update_item") && !names.contains(&"add_subtask"));
        assert_eq!(answers[2]["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn the_agent_proposes_changes_but_cant_make_them() {
        use crate::tests::{board, daemon, git_repository, item, project, start_on, started};
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let repository = git_repository();
        let client = daemon(state.path(), crate::hosts::Hosts::default()).await;
        let project = project(&client, repository.path()).await;
        let task = item(&client, &project.id, "", "login").await;
        let subtask = item(&client, &project.id, &task.id, "form").await;
        let session = started(&client, start_on(&subtask.id)).await;
        let server = server(client, &session.id);

        let (listed, failed) = call(&server, "board", json!({})).await;
        assert!(!failed && listed.contains(&subtask.id), "{listed}");

        for (tool, arguments) in [
            (
                "propose_task",
                json!({"title": "copy", "spec": "Words.", "parent_id": task.id}),
            ),
            (
                "propose_spec_edit",
                json!({"item_id": subtask.id, "title": "sign-in form"}),
            ),
        ] {
            let (_, failed) = call(&server, tool, arguments.clone()).await;
            assert!(!failed, "{tool} {arguments}");
        }
        use pb::ItemStatus::Todo;
        assert_eq!(
            board(&server.client, &project.id).await,
            [("login".to_owned(), Todo), ("form".to_owned(), Todo)]
        );

        for (tool, arguments) in [
            (
                "update_item",
                json!({"item_id": subtask.id, "status": "in_progress"}),
            ),
            ("add_subtask", json!({"title": "deeper"})),
            ("propose_spec_edit", json!({"item_id": subtask.id})),
            (
                "reply_to_thread",
                json!({"thread_id": "12", "text": "Done."}),
            ),
            ("no_such_tool", json!({})),
        ] {
            let (_, failed) = call(&server, tool, arguments.clone()).await;
            assert!(failed, "{tool} {arguments}");
        }
    }

    #[tokio::test]
    async fn the_agents_questions_and_moves_wait_in_the_inbox_until_answered() {
        use crate::tests::{board, daemon, git_repository, item, project, start_on, started};
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let repository = git_repository();
        let client = daemon(state.path(), crate::hosts::Hosts::default()).await;
        let project = project(&client, repository.path()).await;
        let task = item(&client, &project.id, "", "login").await;
        let session = started(&client, start_on(&task.id)).await;
        let server = server(client, &session.id);
        let inbox = || async {
            match server
                .client
                .call(pb::request::Kind::ListInbox(pb::ListInbox {}))
                .await
            {
                Ok(pb::response::Kind::Inbox(inbox)) => inbox
                    .entries
                    .into_iter()
                    .filter_map(|entry| entry.request)
                    .collect::<Vec<_>>(),
                other => panic!("{other:?}"),
            }
        };
        let answer = |request: &pb::AgentRequest, answer| {
            server
                .client
                .call(pb::request::Kind::AnswerRequest(pb::AnswerRequest {
                    request_id: request.id.clone(),
                    answer: Some(answer),
                }))
        };
        use pb::answer_request::Answer;

        let (_, failed) = call(
            &server,
            "ask_person",
            json!({"question": "Email or phone?"}),
        )
        .await;
        assert!(!failed);
        let (_, failed) = call(
            &server,
            "request_move",
            json!({"status": "done", "summary": "Login works."}),
        )
        .await;
        assert!(!failed);
        let (_, failed) = call(
            &server,
            "request_move",
            json!({"status": "todo", "summary": "Nothing."}),
        )
        .await;
        assert!(failed);

        let asked = inbox().await;
        let [question, moving] = asked.as_slice() else {
            panic!("{asked:?}")
        };
        assert_eq!(question.kind(), pb::AgentRequestKind::Question);
        assert_eq!(
            (moving.kind(), moving.status()),
            (pb::AgentRequestKind::Move, pb::ItemStatus::Done)
        );
        assert_eq!(moving.item_id, task.id);

        // A question is replied to, not approved; the reply goes to the session.
        assert!(answer(question, Answer::Approve(true)).await.is_err());
        assert!(
            answer(question, Answer::Reply("Email.".to_owned()))
                .await
                .is_ok()
        );
        crate::tests::after_turns(&server.client, &session.id, 1).await;

        assert!(answer(moving, Answer::Approve(true)).await.is_ok());
        assert_eq!(
            board(&server.client, &project.id).await,
            [("login".to_owned(), pb::ItemStatus::Done)]
        );
        assert!(inbox().await.is_empty());
        // Answered once; again, it is no longer there.
        assert!(answer(moving, Answer::Dismiss(true)).await.is_err());
    }
}
