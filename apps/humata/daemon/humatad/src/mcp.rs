//! `humatad mcp`: the MCP server an agent's CLI starts for one session, on
//! its standard input and output, so the agent can read and change the
//! board and answer review threads. It works through the daemon's socket,
//! as the app and the CLI do; the session is `HUMATA_SESSION`.
//!
//! Answers to review threads aren't stored here: the call is in the
//! session's history, and the daemon reads it from there as it reads
//! `[thread N]` tags, so it gets the turn whose changes are the fix.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use humata_wire::{Client, WireError, pb};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::OnceCell;
use tokio::task::{AbortHandle, JoinSet};

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
        Arc::new(Server::new(client, session)),
        tokio::io::stdin(),
        tokio::io::stdout(),
    )
    .await
}

/// Answers each JSON-RPC message, one per line, until the input ends. Each
/// request is answered as its own task, so one that waits, as read_session
/// may, holds up no other; answers go out as they're ready, each naming
/// its request. When the input ends, calls already under way still have
/// [`ENDED_GRACE`] to be answered, since a client may close its input as
/// soon as it has written; those still waiting then, such as a long
/// read_session, stop.
async fn serve(
    server: Arc<Server>,
    input: impl AsyncRead + Unpin,
    mut output: impl AsyncWrite + Unpin,
) -> Result<(), McpError> {
    let mut lines = BufReader::new(input).lines();
    let mut answering: JoinSet<Option<Value>> = JoinSet::new();
    // The requests being answered, by their ids as JSON, to cancel one.
    let mut asked: HashMap<String, AbortHandle> = HashMap::new();
    let mut ended: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
    loop {
        if ended.is_some() && answering.is_empty() {
            break;
        }
        let answer = tokio::select! {
            () = async {
                match ended.as_mut() {
                    Some(grace) => grace.as_mut().await,
                    None => std::future::pending().await,
                }
            } => break,
            line = lines.next_line(), if ended.is_none() => {
                let Some(line) = line.map_err(|source| McpError::Pipe { source })? else {
                    ended = Some(Box::pin(tokio::time::sleep(ENDED_GRACE)));
                    continue;
                };
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    write(&mut output, &error(&Value::Null, -32700, "that isn't JSON")).await?;
                    continue;
                };
                let Some(id) = message.get("id") else {
                    // A notification is answered with nothing; this one
                    // stops the request it names.
                    if message["method"] == json!("notifications/cancelled")
                        && let Some(cancelled) =
                            asked.remove(&message["params"]["requestId"].to_string())
                    {
                        cancelled.abort();
                    }
                    continue;
                };
                let id = id.to_string();
                let server = Arc::clone(&server);
                let handle = answering.spawn(async move { server.handle(&message).await });
                asked.insert(id, handle);
                continue;
            }
            Some(done) = answering.join_next_with_id() => {
                let task = match &done {
                    Ok((task, _)) => *task,
                    Err(stopped) => stopped.id(),
                };
                let id = asked
                    .iter()
                    .find(|(_, handle)| handle.id() == task)
                    .map(|(id, _)| id.clone());
                if let Some(id) = &id {
                    asked.remove(id);
                }
                match done {
                    Ok((_, answer)) => answer,
                    // Cancelled: the agent no longer waits for it.
                    Err(stopped) if stopped.is_cancelled() => None,
                    Err(_) => id
                        .and_then(|id| serde_json::from_str::<Value>(&id).ok())
                        .map(|id| {
                            error(
                                &id,
                                -32603,
                                "humata's MCP server failed on this call; call it again",
                            )
                        }),
                }
            }
        };
        if let Some(answer) = answer {
            write(&mut output, &answer).await?;
        }
    }
    Ok(())
}

/// How long calls under way when the input ends still have to be answered.
const ENDED_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Writes one answer on its line.
async fn write(output: &mut (impl AsyncWrite + Unpin), answer: &Value) -> Result<(), McpError> {
    let mut text = answer.to_string();
    text.push('\n');
    output
        .write_all(text.as_bytes())
        .await
        .map_err(|source| McpError::Pipe { source })?;
    output
        .flush()
        .await
        .map_err(|source| McpError::Pipe { source })
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

struct Server {
    client: Client,
    session: String,
    /// This session as humatad first listed it. Listing every session to
    /// find it is asked once, not for each call; what it's kept for, where
    /// the session is and whether it only reads, doesn't change.
    known: OnceCell<pb::SessionInfo>,
}

impl Server {
    fn new(client: Client, session: String) -> Self {
        Self {
            client,
            session,
            known: OnceCell::new(),
        }
    }

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
                    "instructions": instructions(self.offered().await),
                })
            }
            "ping" => json!({}),
            "tools/list" => {
                let offered = self.offered().await;
                // Asked only where a tool it could turn off is offered.
                let off = if offered.sessions {
                    self.switched_off().await
                } else {
                    Vec::new()
                };
                json!({"tools": without(tools(offered), &off)})
            }
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
        // A side chat isn't offered these; one called all the same is
        // refused, and humatad refuses the three that reach it besides.
        // They go on only for a session known not to be one: two of them
        // are answered here alone, so while humatad can't say, none is.
        if matches!(
            name,
            "propose_task"
                | "propose_spec_edit"
                | "propose_dependency"
                | "request_move"
                | "start_session"
                | "send_message"
        ) {
            match self.session().await {
                Ok(session) if !session.read_only => {}
                Ok(_) => {
                    return Err(format!(
                        "this is a side chat, which only reads and answers questions, so it has no {name}. Say what you'd propose or hand over, and the person can do it from the session this chat is beside"
                    ));
                }
                Err(why) => {
                    return Err(format!(
                        "Humata can't tell yet whether this session may use {name}, as humatad didn't say which session this is ({why}). Nothing was done; call {name} again in a moment"
                    ));
                }
            }
        }
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
                        if results.catching_up {
                            text.push_str(
                                "\nThe search index was still being brought up to date, so hits may be missing; search again in a few seconds for the rest.\n",
                            );
                        }
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
                        let from_byte = usize::try_from(whole(&arguments["from_byte"], u64::MAX))
                            .unwrap_or(usize::MAX);
                        turn_text(&history.events, turn as u32, &event, from_byte).ok_or_else(
                            || {
                                format!(
                                    "session {session_id} has no turn {turn}; take it from a search hit"
                                )
                            },
                        )
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
                item_kind(arguments)?;
                for on in depends_on(arguments)? {
                    self.check_item(&on).await?;
                }
                Ok(format!(
                    "Proposed {title:?}. It reaches the board only if the person applies it, perhaps edited; don't work on it until then."
                ))
            }
            "propose_dependency" => {
                let item_id = required(text("item_id"), "item_id")?;
                let depends_on = depends_on(arguments)?;
                if depends_on.is_empty() {
                    return Err(
                        "give depends_on, the ids from board of the items it waits on".to_owned(),
                    );
                }
                if depends_on.contains(&item_id) {
                    return Err(format!(
                        "{item_id} can't depend on itself; leave it out of depends_on"
                    ));
                }
                self.check_item(&item_id).await?;
                for on in &depends_on {
                    self.check_item(on).await?;
                }
                Ok(format!(
                    "Proposed that {item_id} waits on {}. The link is made only if the person applies it, and is refused then if it would close a loop; until then board shows it as it was.",
                    depends_on.join(", ")
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
                let question = required(text("question"), "question")?;
                // Checked here too: one kept on a host while the Mac is
                // away would be refused only once it got there.
                short_enough("question", &question)?;
                let ask = pb::AskPerson {
                    session_id: self.session.clone(),
                    text: question,
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
                short_enough("summary", &summary)?;
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
            "start_session" => {
                let start = pb::AgentStartSession {
                    session_id: self.session.clone(),
                    name: required(text("name"), "name")?,
                    text: required(text("message"), "message")?,
                };
                let name = start.name.clone();
                match self
                    .ask(pb::request::Kind::AgentStartSession(start))
                    .await?
                {
                    pb::response::Kind::AgentActed(pb::AgentActed {
                        session: Some(started),
                        ..
                    }) => Ok(format!(
                        "Started session {name:?}, id {}; it has your message and works on it. read_session with that id shows what it does, and waits for it with wait_seconds; send_message writes to it.",
                        started.id
                    )),
                    pb::response::Kind::AgentActed(_) => Ok(format!(
                        "Asked the person to allow starting session {name:?}; it waits in their inbox. Your next message says whether it started, and its id. End your turn unless you can carry on without it."
                    )),
                    other => Err(unexpected(&other)),
                }
            }
            "send_message" => {
                let send = pb::AgentSendMessage {
                    session_id: self.session.clone(),
                    to_session_id: required(text("session_id"), "session_id")?,
                    text: required(text("text"), "text")?,
                };
                let to = send.to_session_id.clone();
                match self.ask(pb::request::Kind::AgentSendMessage(send)).await? {
                    pb::response::Kind::AgentActed(pb::AgentActed {
                        session: Some(_), ..
                    }) => Ok(format!(
                        "Sent to session {to}, which reads it as from this session. read_session shows what it does with it."
                    )),
                    pb::response::Kind::AgentActed(_) => Ok(format!(
                        "Asked the person to allow your message to session {to}; it waits in their inbox. Your next message says whether it was sent. End your turn unless you can carry on without it."
                    )),
                    other => Err(unexpected(&other)),
                }
            }
            "read_session" => {
                let session_id = required(text("session_id"), "session_id")?;
                let wait = whole(&arguments["wait_seconds"], LONGEST_READ_WAIT);
                self.read_session(&session_id, wait).await
            }
            _ => Err(format!("humata has no tool {name:?}; see tools/list")),
        }
    }

    /// Session `id`'s recent conversation, once it stops working or `wait`
    /// seconds pass.
    async fn read_session(&self, id: &str, wait: u64) -> Result<String, String> {
        if self
            .switched_off()
            .await
            .iter()
            .any(|off| off == "agent_read_session")
        {
            return Err(
                "reading other sessions is turned off for sessions' own agents in Humata's Settings › Agent Access; only the person can turn it on, in the Humata app"
                    .to_owned(),
            );
        }
        // Asked anew: the person may have changed the setting mid-turn.
        if self.listed_now().await?.agent_sessions() == pb::AgentSessionsMode::Off {
            return Err(
                "this project has agents' sessions off, so read_session isn't offered; only the person can turn it on, in the project's settings under Agent Sessions"
                    .to_owned(),
            );
        }
        let until = tokio::time::Instant::now()
            + std::time::Duration::from_secs(wait.min(LONGEST_READ_WAIT));
        // One on another host isn't listed here; it's read as it is.
        let listed = loop {
            let listed = self.listed(id).await?;
            let working = listed
                .as_ref()
                .is_some_and(|session| session.state() == pb::SessionState::Working);
            if !working || tokio::time::Instant::now() >= until {
                break listed;
            }
            tokio::time::sleep(READ_POLL).await;
        };
        let read = pb::ReadHistory {
            session_id: id.to_owned(),
        };
        match self.ask(pb::request::Kind::ReadHistory(read)).await? {
            pb::response::Kind::History(history) => {
                Ok(session_text(id, listed.as_ref(), &history.events))
            }
            other => Err(unexpected(&other)),
        }
    }

    /// Session `id` as humatad lists it here; None if it doesn't.
    async fn listed(&self, id: &str) -> Result<Option<pb::SessionInfo>, String> {
        let listed = self
            .ask(pb::request::Kind::ListSessions(pb::ListSessions::default()))
            .await?;
        match listed {
            pb::response::Kind::Sessions(sessions) => Ok(sessions
                .sessions
                .into_iter()
                .find(|session| session.id == id)),
            other => Err(unexpected(&other)),
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

    /// The ids of the Agent Access switches that are off for agents; none
    /// while the Mac can't say, when what starts or writes to a session
    /// can't reach it either.
    async fn switched_off(&self) -> Vec<String> {
        let request = pb::request::Kind::GetAgentAccess(pb::GetAgentAccess {});
        match self.client.call(request).await {
            Ok(pb::response::Kind::AgentAccess(access)) => access.refused,
            _ => Vec::new(),
        }
    }

    async fn ask(&self, request: pb::request::Kind) -> Result<pb::response::Kind, String> {
        self.client
            .call(request)
            .await
            .map_err(|error| humata_wire::describe(&error))
    }

    /// This session, as first listed; asked again only until it's found.
    async fn session(&self) -> Result<pb::SessionInfo, String> {
        self.known
            .get_or_try_init(|| self.listed_now())
            .await
            .cloned()
    }

    /// This session as humatad lists it now.
    async fn listed_now(&self) -> Result<pb::SessionInfo, String> {
        self.listed(&self.session).await?.ok_or_else(|| {
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
    /// humatad can't say, it's offered what a side chat is: every tool that
    /// reads or asks the person, and none that a side chat may not use.
    async fn offered(&self) -> Offered {
        let Ok(session) = self.session().await else {
            return Offered {
                place: Place::Unknown,
                sessions: true,
                read_only: true,
            };
        };
        Offered {
            place: if session.project_id.is_empty() {
                Place::Alone
            } else if session.item_id.is_empty() {
                Place::Project
            } else {
                Place::Task
            },
            sessions: session.agent_sessions() != pb::AgentSessionsMode::Off,
            read_only: session.read_only,
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

/// What decides the tools a session gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Offered {
    place: Place,
    /// Whether its project lets agents start, write to and read other
    /// sessions, asking the person or not.
    sessions: bool,
    /// Whether it is a side chat, which only reads and answers: it starts
    /// and writes to no session, and proposes and asks to move nothing.
    read_only: bool,
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
        let waits = if item.blocked {
            format!("  <- blocked: waits on {}", item.waiting_on.join(", "))
        } else {
            String::new()
        };
        text.push_str(&format!(
            "{indent}- [{}] {} (id {}){yours}{waits}\n",
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
const EARLIER_LEFT_OUT: &str = "(Earlier in the turn left out.)\n";

/// The most of one message in a turn, which leaves room to say how the
/// rest of it is read.
const TURN_MESSAGE: usize = TURN_TEXT - 1024;

/// Turn `turn` of `events` (counted from 1, as search hits count them) as
/// an agent reads it. When it's too long, it starts at `event`, the hit;
/// with no event named, it keeps its end, where the reply is. A message
/// too long to give whole says how to read on, and the message at `event`
/// is read from `from_byte` of its text; from a byte above 0, the turn
/// starts at that message however short it is.
fn turn_text(
    events: &[pb::SessionEvent],
    turn: u32,
    event: &str,
    from_byte: usize,
) -> Option<String> {
    use pb::session_event::Kind;
    let mut at = 1;
    let mut parts: Vec<(String, String)> = Vec::new();
    // As the chat shows it, without subagents' steps.
    for found in events
        .iter()
        .filter(|found| found.parent_call_id.is_empty())
    {
        let from = if found.id == event { from_byte } else { 0 };
        if at == turn
            && let Some(part) = event_text(found, from, TURN_MESSAGE, TURN_PART, Some(turn))
        {
            parts.push((found.id.clone(), part));
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
    let named = parts.iter().position(|(id, _)| id == event);
    let from = if total > TURN_TEXT || from_byte > 0 {
        match named {
            Some(at) => at,
            // Nothing to carry on in: the turn as it is.
            None if total <= TURN_TEXT => 0,
            None => {
                // As many of its last parts as fit under the line that
                // says the start is left out, and at least the last.
                let mut room = TURN_TEXT - EARLIER_LEFT_OUT.len();
                let mut from = parts.len().saturating_sub(1);
                for (index, (_, part)) in parts.iter().enumerate().rev() {
                    if part.len() + 1 > room {
                        break;
                    }
                    room -= part.len() + 1;
                    from = index;
                }
                from
            }
        }
    } else {
        0
    };
    let mut text = String::new();
    if from > 0 {
        text.push_str(EARLIER_LEFT_OUT);
    }
    for (index, (_, part)) in parts[from..].iter().enumerate() {
        if text.len() + part.len() > TURN_TEXT {
            if index == 0 {
                // Too long on its own: as much of it as fits.
                text.push_str(&cut(part, TURN_TEXT));
                text.push('\n');
                continue;
            }
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

/// The part of `text` from byte `from`, moved back to the start of a
/// character, that is at most `most` bytes long, and where the rest starts
/// if there is any. It always takes at least one character, so reading
/// from each rest in turn reaches the end.
fn part_of(text: &str, from: usize, most: usize) -> (&str, Option<usize>) {
    let mut from = from.min(text.len());
    while !text.is_char_boundary(from) {
        from -= 1;
    }
    let mut end = from.saturating_add(most).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == from {
        end = text[from..]
            .chars()
            .next()
            .map_or(from, |first| from + first.len_utf8());
    }
    (&text[from..end], (end < text.len()).then_some(end))
}

/// What `event` said or did, as an agent reads it: a message from byte
/// `from` of its text, cut to `message` bytes, a tool's arguments or
/// output to `tool`. A cut message says how read_turn reads on from where
/// it was cut when `turn`, the turn it is in, is given. None for events
/// that say nothing.
fn event_text(
    event: &pb::SessionEvent,
    from: usize,
    message: usize,
    tool: usize,
    turn: Option<u32>,
) -> Option<String> {
    use pb::session_event::Kind;
    let said = |text: &str| {
        let (part, rest) = part_of(text, from, message);
        // Where the part ends, so what comes before it is its start.
        let end = rest.unwrap_or(text.len());
        let earlier = if end > part.len() { "…" } else { "" };
        match (rest, turn) {
            (Some(rest), Some(turn)) => format!(
                "{earlier}{part}… ({} bytes more) Read on with read_turn: turn {turn}, event_id {:?}, from_byte {rest}.",
                text.len() - rest,
                event.id
            ),
            (Some(rest), None) => format!("{earlier}{part}… ({} bytes more)", text.len() - rest),
            (None, _) => format!("{earlier}{part}"),
        }
    };
    match &event.kind {
        Some(Kind::UserMessage(sent)) => Some(match crate::sender::sender(&sent.text) {
            Some(from) => format!("Session {:?} ({}): {}", from.name, from.id, said(from.text)),
            None => match crate::sender::notice(&sent.text) {
                Some(told) => format!("Humata: {}", said(told)),
                None => format!("The person: {}", said(&sent.text)),
            },
        }),
        Some(Kind::AssistantMessage(reply)) => Some(format!("Agent: {}", said(&reply.text))),
        Some(Kind::ToolStarted(call)) => Some(format!(
            "Tool {}: {}",
            call.tool,
            cut(&call.arguments_json, tool)
        )),
        Some(Kind::ToolCompleted(done)) => Some(format!(
            "{}: {}",
            if done.success { "Output" } else { "Failed" },
            cut(&done.output, tool)
        )),
        _ => None,
    }
}

/// The longest read_session waits for a session to stop working.
const LONGEST_READ_WAIT: u64 = 600;
/// How often it looks meanwhile.
const READ_POLL: std::time::Duration = std::time::Duration::from_secs(2);
/// The most read_session gives, so it fits the agent's context.
const SESSION_TEXT: usize = 16 * 1024;
/// The most of each message in it, and of each tool's arguments or output.
const SESSION_MESSAGE: usize = 4 * 1024;
/// The most of the agent's last reply in it: all there is room for, less a
/// little for how the session stands and for saying where the rest is.
const SESSION_REPLY: usize = SESSION_TEXT - 1024;
const SESSION_TOOL: usize = 300;

/// Session `id` as another session's agent reads it: how it stands, as
/// `listed` if humatad lists it here, then the end of its conversation, as
/// much as fits.
fn session_text(id: &str, listed: Option<&pb::SessionInfo>, events: &[pb::SessionEvent]) -> String {
    let mut text = match listed {
        Some(session) => format!(
            "Session {:?} ({id}) {}, after {} turns.\n",
            session.name,
            match session.state() {
                pb::SessionState::Working => "is working",
                pb::SessionState::WaitingForApproval => "waits for the person to allow something",
                pb::SessionState::Failed => "failed its last turn",
                pb::SessionState::Idle | pb::SessionState::Unspecified => "is idle",
            },
            session.turns
        ),
        None => format!("Session {id}:\n"),
    };
    // As the chat shows them, without subagents' steps, each with its turn
    // as read_turn counts them.
    let mut turn = 1;
    let mut shown: Vec<(u32, &pb::SessionEvent)> = Vec::new();
    for event in events
        .iter()
        .filter(|event| event.parent_call_id.is_empty())
    {
        shown.push((turn, event));
        if matches!(event.kind, Some(pb::session_event::Kind::TurnEnded(_))) {
            turn += 1;
        }
    }
    // The agent's last reply is what a reader most often came for, so it
    // gets its room first and goes in whole if it can.
    let mut room = SESSION_TEXT;
    let last_reply = shown.iter().rposition(|(_, event)| {
        matches!(
            event.kind,
            Some(pb::session_event::Kind::AssistantMessage(_))
        )
    });
    let mut answer = last_reply.and_then(|at| {
        let (turn, event) = shown[at];
        event_text(event, 0, SESSION_REPLY, SESSION_TOOL, Some(turn))
    });
    if let Some(answer) = &answer {
        room = room.saturating_sub(answer.len() + 1);
    }
    // Then the newest of the rest that fit.
    let mut kept: Vec<String> = Vec::new();
    let mut left_out = false;
    for (index, (turn, event)) in shown.iter().enumerate().rev() {
        if Some(index) == last_reply {
            kept.extend(answer.take());
            continue;
        }
        let Some(part) = event_text(event, 0, SESSION_MESSAGE, SESSION_TOOL, Some(*turn)) else {
            continue;
        };
        if part.len() + 1 > room {
            left_out = true;
            break;
        }
        room -= part.len() + 1;
        kept.push(part);
    }
    // What followed the reply took all the room: the reply still goes in.
    if let Some(answer) = answer {
        kept.push("(What came between is left out; read_turn reaches it.)".to_owned());
        kept.push(answer);
    }
    if kept.is_empty() && !left_out {
        text.push_str("Nothing was said or done in it yet.\n");
        return text;
    }
    if left_out {
        text.push_str(
            "(Its earlier conversation is left out; search_sessions and read_turn reach it.)\n",
        );
    }
    for part in kept.iter().rev() {
        text.push_str(part);
        text.push('\n');
    }
    text
}

/// A whole number from `argument`, which clients send as an integer, a
/// float or a number in a string: at most `cap`, and 0 for anything else
/// or below 0.
fn whole(argument: &Value, cap: u64) -> u64 {
    let number = match argument {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    };
    match number {
        Some(number) if number >= cap as f64 => cap,
        // Above 0 and below the cap, so it fits; what follows the point
        // is dropped.
        Some(number) if number > 0.0 => number as u64,
        _ => 0,
    }
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

/// Fails when `text`, given as `field`, is longer than what an agent asks
/// the person may be.
fn short_enough(field: &str, text: &str) -> Result<(), String> {
    let limit = crate::daemon::REQUEST_LIMIT;
    if text.len() > limit {
        return Err(format!(
            "{field} is {} bytes and the most it may be is {limit}. Put the detail in a file or in your reply, and give a shorter {field} that names it",
            text.len()
        ));
    }
    Ok(())
}

/// The ids a proposal's `depends_on` gives, each once; none if it gives
/// none.
pub(crate) fn depends_on(arguments: &Value) -> Result<Vec<String>, String> {
    let Some(listed) = arguments
        .get("depends_on")
        .filter(|listed| !listed.is_null())
    else {
        return Ok(Vec::new());
    };
    let bad = || "depends_on must be a list of item ids from board".to_owned();
    let mut ids: Vec<String> = Vec::new();
    for id in listed.as_array().ok_or_else(bad)? {
        let id = id.as_str().map(str::trim).ok_or_else(bad)?;
        if !id.is_empty() && !ids.iter().any(|known| known == id) {
            ids.push(id.to_owned());
        }
    }
    Ok(ids)
}

/// The kind a proposed item's `kind` asks for; unspecified if it gives
/// none, which is a task.
pub(crate) fn item_kind(arguments: &Value) -> Result<pb::ItemKind, String> {
    match arguments["kind"].as_str().map(str::trim) {
        None | Some("") => Ok(pb::ItemKind::Unspecified),
        Some("task") => Ok(pb::ItemKind::Task),
        Some("milestone") => {
            let under = arguments["parent_id"]
                .as_str()
                .is_some_and(|parent| !parent.trim().is_empty());
            if under {
                Err("an item under parent_id is a task; leave out parent_id to propose a milestone, or kind to propose a task under it".to_owned())
            } else {
                Ok(pb::ItemKind::Milestone)
            }
        }
        Some(other) => Err(format!("kind is milestone or task, not {other:?}")),
    }
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
fn instructions(offered: Offered) -> String {
    let place = offered.place;
    let mut text = String::from("Humata is directing this session.");
    if place.has_board() {
        text.push_str(
            " board and read_item show the project's tasks, subtasks and specs; you can't \
             change them",
        );
        text.push_str(if offered.read_only {
            "."
        } else {
            ", but propose_task and propose_spec_edit suggest changes the person applies."
        });
    }
    text.push_str(" ask_person asks the person something.");
    if place.has_task() && !offered.read_only {
        text.push_str(" request_move asks them to move your task once it's done.");
    }
    if place.has_board() {
        text.push_str(" Answer each review thread you're sent with reply_to_thread.");
    }
    text.push_str(" search_sessions finds earlier work and read_turn opens a hit.");
    if place.has_board() {
        text.push_str(" read_skill reads a project skill your instructions list.");
    }
    if offered.sessions && offered.read_only {
        text.push_str(" read_session reads what another session did.");
    } else if offered.sessions {
        text.push_str(
            " start_session hands work to a new session of your own, send_message writes to \
             it and read_session reads what it did; the person may have to allow each.",
        );
    }
    text
}

/// The tools a session has: outside a project, none for a board,
/// proposals, review or project skills; on no task, none to ask to move it;
/// where its project has agents' sessions off, none for other sessions. A
/// side chat, which only reads and answers, gets none that starts or writes
/// to a session, proposes a change or asks for a move; it still reads other
/// sessions and asks the person.
/// Each listed tool costs tokens on every request, so only those that work
/// there are offered.
fn tools(offered: Offered) -> Value {
    let place = offered.place;
    let wanted = |tool: &Value| match tool["name"].as_str().unwrap_or_default() {
        "ask_person" | "search_sessions" | "read_turn" => true,
        "start_session" | "send_message" => offered.sessions && !offered.read_only,
        "read_session" => offered.sessions,
        "propose_task" | "propose_spec_edit" | "propose_dependency" => {
            place.has_board() && !offered.read_only
        }
        "request_move" => place.has_task() && !offered.read_only,
        _ => place.has_board(),
    };
    let Value::Array(every) = every_tool() else {
        return json!([]);
    };
    Value::Array(every.into_iter().filter(|tool| wanted(tool)).collect())
}

/// `tools` without those for other sessions whose Agent Access switch is
/// among `off`.
fn without(tools: Value, off: &[String]) -> Value {
    let switch = |tool: &Value| match tool["name"].as_str().unwrap_or_default() {
        "start_session" => "agent_start_session",
        "send_message" => "agent_send_message",
        "read_session" => "agent_read_session",
        _ => "",
    };
    match tools {
        Value::Array(tools) => Value::Array(
            tools
                .into_iter()
                .filter(|tool| !off.iter().any(|off| off == switch(tool)))
                .collect(),
        ),
        other => other,
    }
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
                "kind": string("milestone, for one that holds tasks, or task; leave out for a task. Only an item with no parent_id can be a milestone."),
                "depends_on": {
                    "type": "array",
                    "description": "Ids from board of the items it waits on: it is blocked until they are done.",
                    "items": {"type": "string"},
                },
                "note": string("Why you propose it, if it isn't plain from the spec."),
            }), &["title", "spec"]),
        },
        {
            "name": "propose_dependency",
            "description": "Propose that an item on the board waits on others: it shows as blocked until they are done, and comes after them in the plan. Milestones and tasks can wait on either. You can't change the board yourself: the links are made only when the person applies your proposal.",
            "inputSchema": object(json!({
                "item_id": string("From board: the item that waits."),
                "depends_on": {
                    "type": "array",
                    "description": "Ids from board of the items it waits on.",
                    "items": {"type": "string"},
                },
                "note": string("Why it has to wait for them."),
            }), &["item_id", "depends_on"]),
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
                "from_byte": {"type": "integer", "minimum": 0, "description": "Where to carry on in the message at event_id, from the note a cut message ends with."},
            }), &["session_id", "turn"]),
        },
        {
            "name": "read_skill",
            "description": "A project skill's text, by the name your instructions list it under. Read it before work it covers.",
            "inputSchema": object(json!({"name": string("The skill's name.")}), &["name"]),
        },
        // Each of these says whether it only reads, changes something or
        // destroys something, for clients and for switches that gate them.
        {
            "name": "start_session",
            "description": "Start another session, as a child of this one, to hand it a piece of work: it runs where this one does, on the same task, with the same agent and model, and knows nothing of this conversation, so the message must say all it needs. The person may have to allow it first; then your next message says it started. A session may have only so many, nested only so deep.",
            "inputSchema": object(json!({
                "name": string("What to call it, a few words, as the person sees it in Humata."),
                "message": string("Its first message: what to do, what to read first, and what to answer when done."),
            }), &["name", "message"]),
            "annotations": {"readOnlyHint": false, "destructiveHint": false},
        },
        {
            "name": "send_message",
            "description": "Write to another session, which reads it as from this session, not the person, and starts a turn on it. Unless the person set the project otherwise, only to sessions this one started; they may have to allow each message first.",
            "inputSchema": object(json!({
                "session_id": string("The session's id, from start_session."),
                "text": string("The message, in Markdown."),
            }), &["session_id", "text"]),
            "annotations": {"readOnlyHint": false, "destructiveHint": false},
        },
        {
            "name": "read_session",
            "description": "How another session stands and the end of its conversation: what was said to it, what its agent answered and the tools it ran, the newest that fit. To read what a session you started has done.",
            "inputSchema": object(json!({
                "session_id": string("The session's id, from start_session."),
                "wait_seconds": {"type": "integer", "minimum": 0, "maximum": 600,
                    "description": "Wait up to this long for it to stop working before reading; 0, the default, reads it as it is."},
            }), &["session_id"]),
            "annotations": {"readOnlyHint": true, "destructiveHint": false},
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIPE: usize = 1024 * 1024;
    const ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

    /// The next answer the server writes.
    async fn next_answer(
        answers: &mut tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
    ) -> Value {
        match tokio::time::timeout(ANSWER_WAIT, answers.next_line()).await {
            Ok(Ok(Some(line))) => {
                serde_json::from_str(&line).unwrap_or_else(|error| panic!("{error}"))
            }
            other => panic!("no answer: {other:?}"),
        }
    }

    /// The server's answers to `messages`, one per line in and out, in the
    /// order of the requests they answer. The input stays open until each
    /// request is answered.
    async fn exchange(server: &Arc<Server>, messages: &[Value]) -> Vec<Value> {
        let (mut requests, input) = tokio::io::duplex(PIPE);
        let (output, answers) = tokio::io::duplex(PIPE);
        let serving = tokio::spawn(serve(Arc::clone(server), input, output));
        for message in messages {
            requests
                .write_all(format!("{message}\n").as_bytes())
                .await
                .unwrap_or_else(|error| panic!("{error}"));
        }
        let asked = |answer: &Value| {
            messages
                .iter()
                .position(|message| message.get("id") == answer.get("id"))
        };
        let expected = messages
            .iter()
            .filter(|message| message.get("id").is_some())
            .count();
        let mut answers = BufReader::new(answers).lines();
        let mut answered = Vec::new();
        while answered.len() < expected {
            answered.push(next_answer(&mut answers).await);
        }
        drop(requests);
        let served = tokio::time::timeout(ANSWER_WAIT, serving).await;
        assert!(matches!(served, Ok(Ok(Ok(())))), "{served:?}");
        answered.sort_by_key(asked);
        answered
    }

    pub(crate) async fn call(server: &Arc<Server>, tool: &str, arguments: Value) -> (String, bool) {
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

    pub(crate) fn server(client: Client, session: &str) -> Arc<Server> {
        Arc::new(Server::new(client, session.to_owned()))
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
        let second = turn_text(&events, 2, "c", 0).unwrap_or_default();
        assert!(
            second.contains("second ask") && !second.contains("first ask"),
            "{second}"
        );
        assert_eq!(turn_text(&events, 4, "", 0), None);
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
        offered_names(Offered {
            place,
            sessions: false,
            read_only: false,
        })
    }

    fn offered_names(offered: Offered) -> Vec<String> {
        tools(offered)
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

    #[test]
    fn the_tools_for_other_sessions_are_offered_only_where_the_project_allows_them() {
        let with = offered_names(Offered {
            place: Place::Alone,
            sessions: true,
            read_only: false,
        });
        for tool in ["start_session", "send_message", "read_session"] {
            assert!(with.iter().any(|name| name == tool), "{tool}: {with:?}");
            assert!(!names(Place::Task).iter().any(|name| name == tool));
        }
    }

    #[test]
    fn a_tool_for_other_sessions_that_agent_access_has_off_is_left_out() {
        let offered = Offered {
            place: Place::Alone,
            sessions: true,
            read_only: false,
        };
        let off = ["agent_start_session".to_owned(), "search".to_owned()];
        let left: Vec<String> = without(tools(offered), &off)
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect();
        assert!(!left.iter().any(|name| name == "start_session"), "{left:?}");
        for tool in [
            "send_message",
            "read_session",
            "ask_person",
            "search_sessions",
        ] {
            assert!(left.iter().any(|name| name == tool), "{tool}: {left:?}");
        }
    }

    #[test]
    fn a_side_chat_reads_other_sessions_and_asks_but_starts_writes_and_proposes_nothing() {
        let side = offered_names(Offered {
            place: Place::Task,
            sessions: true,
            read_only: true,
        });
        for tool in ["read_session", "ask_person", "board", "read_item"] {
            assert!(side.iter().any(|name| name == tool), "{tool}: {side:?}");
        }
        for tool in [
            "start_session",
            "send_message",
            "propose_task",
            "propose_spec_edit",
            "request_move",
        ] {
            assert!(!side.iter().any(|name| name == tool), "{tool}: {side:?}");
        }
    }

    #[tokio::test]
    async fn a_side_chat_that_calls_a_tool_it_isnt_offered_is_refused_and_nothing_waits() {
        use crate::tests::{daemon, git_repository, project, start_on};
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let repository = git_repository();
        let client = daemon(state.path(), crate::hosts::Hosts::default()).await;
        let project = project(&client, repository.path()).await;
        let task = crate::tests::item(&client, &project.id, "", "login").await;
        let session = crate::tests::started(&client, start_on(&task.id)).await;
        let sent = client
            .call(pb::request::Kind::SendMessage(pb::SendMessage {
                session_id: session.id.clone(),
                text: "hello".to_owned(),
                mode: pb::SendMode::Enqueue.into(),
                ..Default::default()
            }))
            .await;
        assert!(sent.is_ok(), "{sent:?}");
        crate::tests::after_turns(&client, &session.id, 1).await;
        let side = match client
            .call(pb::request::Kind::ForkSession(pb::ForkSession {
                session_id: session.id.clone(),
                read_only: true,
                ..Default::default()
            }))
            .await
        {
            Ok(pb::response::Kind::SessionForked(pb::SessionForked {
                session: Some(side),
            })) => side,
            other => panic!("not forked: {other:?}"),
        };
        let server = server(client, &side.id);

        for (tool, arguments) in [
            ("propose_task", json!({"title": "copy", "spec": "Words."})),
            (
                "propose_spec_edit",
                json!({"item_id": task.id, "title": "sign-in"}),
            ),
            (
                "request_move",
                json!({"status": "done", "summary": "Login works."}),
            ),
            (
                "start_session",
                json!({"name": "Tests", "message": "hello"}),
            ),
            (
                "send_message",
                json!({"session_id": session.id, "text": "hello"}),
            ),
        ] {
            let (_, failed) = call(&server, tool, arguments.clone()).await;
            assert!(failed, "{tool} {arguments}");
        }
        // It still reads, and asks the person.
        let (read, failed) = call(&server, "read_session", json!({"session_id": session.id})).await;
        assert!(!failed, "{read}");
        let (_, failed) = call(&server, "ask_person", json!({"question": "Which one?"})).await;
        assert!(!failed);
        let inbox = match server
            .client
            .call(pb::request::Kind::ListInbox(pb::ListInbox {}))
            .await
        {
            Ok(pb::response::Kind::Inbox(inbox)) => inbox,
            other => panic!("{other:?}"),
        };
        let kinds: Vec<pb::AgentRequestKind> = inbox
            .entries
            .iter()
            .filter_map(|entry| entry.request.as_ref())
            .map(pb::AgentRequest::kind)
            .collect();
        assert_eq!(kinds, [pb::AgentRequestKind::Question]);
    }

    /// A session in a project, and another whose turn is held working.
    async fn beside_a_working_session(state: &Path) -> (Arc<Server>, String, tempfile::TempDir) {
        use crate::tests::{daemon, git_repository, project, start_on};
        let repository = git_repository();
        let client = daemon(state, crate::hosts::Hosts::default()).await;
        let project = project(&client, repository.path()).await;
        let task = crate::tests::item(&client, &project.id, "", "login").await;
        let session = crate::tests::started(&client, start_on(&task.id)).await;
        let mut held = start_on(&task.id);
        if let pb::request::Kind::StartSession(start) = &mut held {
            start.backend_config = r#"{"rules": [
                {"when": "", "steps": [{"step": "wait_for", "path": "never-made"}]}
            ]}"#
            .to_owned();
        }
        let held = crate::tests::started(&client, held).await;
        let sent = client
            .call(pb::request::Kind::SendMessage(pb::SendMessage {
                session_id: held.id.clone(),
                text: "work".to_owned(),
                mode: pb::SendMode::Enqueue.into(),
                ..Default::default()
            }))
            .await;
        assert!(sent.is_ok(), "{sent:?}");
        // Until it lists as working, a read of it wouldn't wait.
        let mut working = false;
        for _ in 0..400 {
            let listed = client
                .call(pb::request::Kind::ListSessions(pb::ListSessions::default()))
                .await;
            let Ok(pb::response::Kind::Sessions(sessions)) = listed else {
                panic!("{listed:?}")
            };
            working = sessions
                .sessions
                .iter()
                .any(|info| info.id == held.id && info.state() == pb::SessionState::Working);
            if working {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(working, "the held session never worked");
        (server(client, &session.id), held.id, repository)
    }

    #[tokio::test]
    async fn a_call_is_answered_while_a_read_of_a_working_session_waits() {
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (server, held, _repository) = beside_a_working_session(state.path()).await;
        let (mut requests, input) = tokio::io::duplex(PIPE);
        let (output, answers) = tokio::io::duplex(PIPE);
        let serving = tokio::spawn(serve(server, input, output));
        let mut answers = BufReader::new(answers).lines();

        let read = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                          "params": {"name": "read_session",
                                     "arguments": {"session_id": held, "wait_seconds": 600}}});
        let ping = json!({"jsonrpc": "2.0", "id": 2, "method": "ping"});
        requests
            .write_all(format!("{read}\n{ping}\n").as_bytes())
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        // The session never stops working, so the read is still waiting.
        assert_eq!(next_answer(&mut answers).await["id"], json!(2));

        // A read the agent gave up on is answered with nothing, and the
        // server goes on answering.
        let cancelled = json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                               "params": {"requestId": 1}});
        let again = json!({"jsonrpc": "2.0", "id": 3, "method": "ping"});
        requests
            .write_all(format!("{cancelled}\n{again}\n").as_bytes())
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(next_answer(&mut answers).await["id"], json!(3));

        drop(requests);
        let served = tokio::time::timeout(ANSWER_WAIT, serving).await;
        assert!(matches!(served, Ok(Ok(Ok(())))), "{served:?}");
    }

    #[tokio::test]
    async fn the_server_stops_when_its_input_ends_though_a_read_still_waits() {
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (server, held, _repository) = beside_a_working_session(state.path()).await;
        let (mut requests, input) = tokio::io::duplex(PIPE);
        let (output, answers) = tokio::io::duplex(PIPE);
        let serving = tokio::spawn(serve(server, input, output));
        let mut answers = BufReader::new(answers).lines();

        let read = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                          "params": {"name": "read_session",
                                     "arguments": {"session_id": held, "wait_seconds": 600}}});
        let ping = json!({"jsonrpc": "2.0", "id": 2, "method": "ping"});
        requests
            .write_all(format!("{read}\n{ping}\n").as_bytes())
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        // Both were taken in, and the read is the one still under way.
        assert_eq!(next_answer(&mut answers).await["id"], json!(2));

        drop(requests);
        let served = tokio::time::timeout(ANSWER_WAIT, serving).await;
        assert!(matches!(served, Ok(Ok(Ok(())))), "{served:?}");
        // The read was never answered.
        let rest = answers.next_line().await;
        assert!(matches!(rest, Ok(None)), "{rest:?}");
    }

    #[test]
    fn a_session_is_read_from_its_end_within_a_size_and_names_who_wrote() {
        use pb::session_event::Kind;
        let message = |id: &str, text: String| {
            said(
                id,
                Kind::UserMessage(pb::UserMessage {
                    text,
                    ..Default::default()
                }),
            )
        };
        let mut events = vec![message("first", "the oldest ask".to_owned())];
        // More than fits.
        for index in 0..40 {
            events.push(said(
                &format!("reply-{index}"),
                Kind::AssistantMessage(pb::AssistantMessage {
                    text: "words ".repeat(200),
                }),
            ));
        }
        events.push(message(
            "from-parent",
            crate::sender::headed("parent-id", "Parent", "the newest ask"),
        ));
        let text = session_text("child-id", None, &events);
        assert!(text.len() <= SESSION_TEXT + 1024, "{}", text.len());
        assert!(text.contains("the newest ask"), "{text}");
        assert!(!text.contains("the oldest ask"));
        // Its sender is named; it isn't put down to the person.
        assert!(text.contains("parent-id") && !text.contains("The person: "));

        let short = session_text("child-id", None, &events[..1]);
        assert!(short.contains("The person: the oldest ask"), "{short}");
    }

    /// A humatad that can't list its sessions, and counts what else it is
    /// asked.
    struct NotListing {
        others: std::sync::atomic::AtomicUsize,
    }

    impl humata_wire::Service for NotListing {
        fn handle(self: Arc<Self>, request: pb::request::Kind) -> humata_wire::ServiceFuture {
            Box::pin(async move {
                if !matches!(request, pb::request::Kind::ListSessions(_)) {
                    self.others
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                humata_wire::Reply::Response(humata_wire::error(
                    pb::ErrorCode::HostUnavailable,
                    "not answering",
                ))
            })
        }
    }

    #[tokio::test]
    async fn a_session_humatad_cant_place_is_offered_and_run_only_what_a_side_chat_is() {
        let humatad = Arc::new(NotListing {
            others: std::sync::atomic::AtomicUsize::new(0),
        });
        let server = server(crate::tests::client_of(Arc::clone(&humatad)).await, "s-1");
        let kept_back = [
            ("propose_task", json!({"title": "copy", "spec": "Words."})),
            (
                "propose_spec_edit",
                json!({"item_id": "i-1", "title": "sign-in"}),
            ),
            (
                "request_move",
                json!({"status": "done", "summary": "Login works."}),
            ),
            (
                "start_session",
                json!({"name": "Tests", "message": "hello"}),
            ),
            (
                "send_message",
                json!({"session_id": "s-2", "text": "hello"}),
            ),
        ];
        for (tool, arguments) in &kept_back {
            let (_, failed) = call(&server, tool, arguments.clone()).await;
            assert!(failed, "{tool} {arguments}");
        }
        // Nothing but which session this is was asked of humatad.
        assert_eq!(humatad.others.load(std::sync::atomic::Ordering::SeqCst), 0);

        let listed = exchange(
            &server,
            &[json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})],
        )
        .await;
        let names: Vec<&str> = listed[0]["result"]["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool["name"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        assert!(names.contains(&"ask_person"), "{names:?}");
        for (tool, _) in &kept_back {
            assert!(!names.contains(tool), "{tool}: {names:?}");
        }
    }

    #[test]
    fn a_session_is_read_with_its_last_reply_whole_though_longer_than_other_messages_may_be() {
        use pb::session_event::Kind;
        let reply = format!("{}THE-LAST-WORDS", "found ".repeat(10 * 1024 / 6));
        let mut events = vec![said(
            "ask",
            Kind::UserMessage(pb::UserMessage {
                text: "report what you found".to_owned(),
                ..Default::default()
            }),
        )];
        // An earlier reply, as long, is still cut.
        events.push(said(
            "earlier",
            Kind::AssistantMessage(pb::AssistantMessage {
                text: format!("{}THE-EARLIER-END", "looking ".repeat(10 * 1024 / 8)),
            }),
        ));
        events.push(said(
            "report",
            Kind::AssistantMessage(pb::AssistantMessage { text: reply }),
        ));
        events.push(said("end", Kind::TurnEnded(pb::TurnEnded::default())));

        let text = session_text("child-id", None, &events);
        assert!(text.contains("THE-LAST-WORDS"), "{text}");
        assert!(!text.contains("THE-EARLIER-END"), "{text}");
        assert!(text.len() <= SESSION_TEXT + 1024, "{}", text.len());
    }

    #[test]
    fn a_cut_message_names_the_event_that_read_turn_opens_it_at() {
        use pb::session_event::Kind;
        let message = |id: &str, text: String| {
            said(
                id,
                Kind::UserMessage(pb::UserMessage {
                    text,
                    ..Default::default()
                }),
            )
        };
        let ended = |id: &str| said(id, Kind::TurnEnded(pb::TurnEnded::default()));
        let mut events = vec![
            message("first-ask", "an earlier ask".to_owned()),
            ended("first-end"),
            message("second-ask", "THE-TURNS-START".to_owned()),
        ];
        // More of the turn than read_turn gives at once.
        for index in 0..20 {
            events.push(said(
                &format!("output-{index}"),
                Kind::ToolCompleted(pb::ToolCompleted {
                    success: true,
                    output: "line ".repeat(400),
                    ..Default::default()
                }),
            ));
        }
        events.push(message(
            "long-steer",
            format!("{}THE-STEERS-END", "also ".repeat(6 * 1024 / 5)),
        ));
        events.push(said(
            "reply",
            Kind::AssistantMessage(pb::AssistantMessage {
                text: "done".to_owned(),
            }),
        ));
        events.push(ended("second-end"));

        let read = session_text("child-id", None, &events);
        assert!(!read.contains("THE-STEERS-END"), "{read}");
        assert!(read.contains("long-steer"), "{read}");

        let opened = turn_text(&events, 2, "long-steer", 0).unwrap_or_default();
        assert!(opened.contains("THE-STEERS-END"), "{opened}");
        assert!(!opened.contains("THE-TURNS-START"), "{opened}");
        // In another turn, the event opens nothing.
        let other = turn_text(&events, 1, "long-steer", 0).unwrap_or_default();
        assert!(!other.contains("THE-STEERS-END"), "{other}");
    }

    #[test]
    fn a_long_turn_read_with_no_event_named_ends_with_its_reply() {
        use pb::session_event::Kind;
        let mut events = vec![said(
            "ask",
            Kind::UserMessage(pb::UserMessage {
                text: "THE-ASK".to_owned(),
                ..Default::default()
            }),
        )];
        for index in 0..40 {
            events.push(said(
                &format!("output-{index}"),
                Kind::ToolCompleted(pb::ToolCompleted {
                    success: true,
                    output: "line ".repeat(400),
                    ..Default::default()
                }),
            ));
        }
        events.push(said(
            "reply",
            Kind::AssistantMessage(pb::AssistantMessage {
                text: "THE-REPLY".to_owned(),
            }),
        ));
        events.push(said("end", Kind::TurnEnded(pb::TurnEnded::default())));

        let text = turn_text(&events, 1, "", 0).unwrap_or_default();
        assert!(text.contains("THE-REPLY"), "{text}");
        assert!(!text.contains("THE-ASK"), "{text}");
        assert!(text.len() <= TURN_TEXT + 1024, "{}", text.len());
    }

    #[test]
    fn a_text_read_part_by_part_is_read_whole() {
        // Characters of two and three bytes, so cuts fall inside them.
        let text = "héllo wörld ✓ ".repeat(40);
        for most in [1, 7, 64] {
            let mut read = String::new();
            let mut from = Some(0);
            while let Some(at) = from {
                let (part, rest) = part_of(&text, at, most);
                assert!(rest.is_none_or(|rest| rest > at), "no further from {at}");
                read.push_str(part);
                from = rest;
            }
            assert_eq!(read, text, "{most} bytes at a time");
        }
    }

    /// One turn whose agent's reply, event "reply", is `text`.
    fn replied(text: &str) -> Vec<pb::SessionEvent> {
        use pb::session_event::Kind;
        vec![
            said(
                "ask",
                Kind::UserMessage(pb::UserMessage {
                    text: "THE-ASK".to_owned(),
                    ..Default::default()
                }),
            ),
            said(
                "reply",
                Kind::AssistantMessage(pb::AssistantMessage {
                    text: text.to_owned(),
                }),
            ),
            said("end", Kind::TurnEnded(pb::TurnEnded::default())),
        ]
    }

    #[test]
    fn a_long_reply_read_from_a_byte_near_its_end_ends_with_its_last_words() {
        let reply = format!("THE-FIRST-WORDS {}THE-LAST-WORDS", "word ".repeat(8 * 1024));
        let events = replied(&reply);

        let whole_turn = turn_text(&events, 1, "", 0).unwrap_or_default();
        assert!(whole_turn.contains("THE-FIRST-WORDS"), "{whole_turn}");
        assert!(!whole_turn.contains("THE-LAST-WORDS"), "{whole_turn}");

        let end = turn_text(&events, 1, "reply", reply.len() - 2000).unwrap_or_default();
        assert!(end.contains("THE-LAST-WORDS"), "{end}");
        assert!(!end.contains("THE-FIRST-WORDS"), "{end}");
        // It starts at the reply, short as what's left of the turn is.
        assert!(!end.contains("THE-ASK"), "{end}");
    }

    #[test]
    fn a_reply_cut_by_read_session_is_carried_on_by_read_turn_with_nothing_skipped() {
        // Numbered, so no part of it is found anywhere but in its place.
        let reply: String = (0..3000).map(|number| format!("word{number} ")).collect();
        let events = replied(&reply);
        let (first, rest) = part_of(&reply, 0, SESSION_REPLY);
        let rest = rest.unwrap_or_else(|| panic!("the reply fits read_session"));

        let read = session_text("child-id", None, &events);
        assert!(read.contains(first), "{read}");
        let carried_on = turn_text(&events, 1, "reply", rest).unwrap_or_default();
        assert!(carried_on.contains(&reply[rest..]), "{carried_on}");
    }

    #[test]
    fn a_wait_is_read_from_an_integer_a_float_or_a_string() {
        for sixty in [json!(60), json!(60.0), json!("60")] {
            assert_eq!(whole(&sixty, 600), 60, "{sixty}");
        }
        for none in [Value::Null, json!(-5), json!("soon")] {
            assert_eq!(whole(&none, 600), 0, "{none}");
        }
        assert_eq!(whole(&json!(1e300), 600), 600);
    }

    #[test]
    fn what_humata_told_a_session_isnt_read_as_the_persons() {
        use pb::session_event::Kind;
        let message = |text: String| {
            said(
                "told",
                Kind::UserMessage(pb::UserMessage {
                    text,
                    ..Default::default()
                }),
            )
        };
        let own = session_text("s", None, &[message("it was declined".to_owned())]);
        let told = session_text(
            "s",
            None,
            &[message(crate::sender::noticed("it was declined"))],
        );
        assert!(told.contains("it was declined"), "{told}");
        // Named apart from the person's own message of the same words.
        assert_ne!(told, own);
        assert!(!told.contains("The person: "), "{told}");
    }

    #[tokio::test]
    async fn an_agent_starts_a_session_once_the_person_allows_it_and_reads_it() {
        use crate::tests::{daemon, git_repository, project, start_on};
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let repository = git_repository();
        let client = daemon(state.path(), crate::hosts::Hosts::default()).await;
        let project = project(&client, repository.path()).await;
        let task = crate::tests::item(&client, &project.id, "", "login").await;
        let session = crate::tests::started(&client, start_on(&task.id)).await;
        let server = server(client, &session.id);

        let (_, failed) = call(
            &server,
            "start_session",
            json!({"name": "Tests", "message": "hello"}),
        )
        .await;
        assert!(!failed);
        // A session to start needs its first message.
        let (_, failed) = call(&server, "start_session", json!({"name": "Tests"})).await;
        assert!(failed);

        // Nothing starts until the person allows it.
        let sessions = || async {
            match server
                .client
                .call(pb::request::Kind::ListSessions(pb::ListSessions::default()))
                .await
            {
                Ok(pb::response::Kind::Sessions(sessions)) => sessions.sessions,
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(sessions().await.len(), 1);
        let inbox = match server
            .client
            .call(pb::request::Kind::ListInbox(pb::ListInbox {}))
            .await
        {
            Ok(pb::response::Kind::Inbox(inbox)) => inbox,
            other => panic!("{other:?}"),
        };
        let asked: Vec<pb::AgentRequest> = inbox
            .entries
            .into_iter()
            .filter_map(|entry| entry.request)
            .collect();
        let [request] = asked.as_slice() else {
            panic!("{asked:?}")
        };
        assert_eq!(request.kind(), pb::AgentRequestKind::StartSession);
        let allowed = server
            .client
            .call(pb::request::Kind::AnswerRequest(pb::AnswerRequest {
                request_id: request.id.clone(),
                answer: Some(pb::answer_request::Answer::Approve(true)),
            }))
            .await;
        assert!(allowed.is_ok(), "{allowed:?}");

        let listed = sessions().await;
        let child = listed
            .iter()
            .find(|listed| listed.started_by_session_id == session.id)
            .unwrap_or_else(|| panic!("no child in {listed:?}"));
        crate::tests::after_turns(&server.client, &child.id, 1).await;
        let (read, failed) = call(&server, "read_session", json!({"session_id": child.id})).await;
        assert!(!failed, "{read}");
        // What this session wrote, as from it, and the child's answer.
        assert!(
            read.contains("hello") && read.contains(&session.id),
            "{read}"
        );
        assert!(read.contains("hi there"), "{read}");
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
        use crate::tests::{daemon, git_repository, project, start_on};
        let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let repository = git_repository();
        let client = daemon(state.path(), crate::hosts::Hosts::default()).await;
        let project = project(&client, repository.path()).await;
        let task = crate::tests::item(&client, &project.id, "", "login").await;
        let session = crate::tests::started(&client, start_on(&task.id)).await;
        let server = server(client, &session.id);
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
