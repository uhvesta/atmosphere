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
                    "instructions": "Humata is directing this session. Its board has the \
                        project's tasks and their subtasks; keep the one you work on up to \
                        date, and add subtasks as you find work. Answer each review thread \
                        you're sent with reply_to_thread.",
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({"tools": tools()}),
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
            "board" => self.board().await,
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
                Ok(format!(
                    "Proposed {title:?}. It reaches the board only if the person applies it, perhaps edited; don't work on it until then."
                ))
            }
            "propose_spec_edit" => {
                let item_id = required(text("item_id"), "item_id")?;
                if arguments["spec"]
                    .as_str()
                    .is_none_or(|spec| spec.trim().is_empty())
                {
                    return Err("give spec: the whole new spec, in Markdown".to_owned());
                }
                Ok(format!(
                    "Proposed a new spec for {item_id}. It replaces the old one only if the person applies it; until then read_item shows the old one."
                ))
            }
            "update_item" => {
                let item_id = required(text("item_id"), "item_id")?;
                let status = text("status")
                    .map(|status| {
                        status_named(&status).ok_or_else(|| {
                            format!(
                                "{status:?} isn't a status; give one of {}",
                                STATUSES.join(", ")
                            )
                        })
                    })
                    .transpose()?;
                if let Some(status) = status.filter(|status| asked_for(*status)) {
                    return Err(format!(
                        "moving a task to {} is the person's call; ask with request_move, saying what you did",
                        status_name(status)
                    ));
                }
                let title = text("title").filter(|title| !title.is_empty());
                let spec = arguments["spec"].as_str().map(str::to_owned);
                if status.is_none() && title.is_none() && spec.is_none() {
                    return Err("say what to change: status, title or spec".to_owned());
                }
                let update = pb::UpdateItem {
                    item_id,
                    title,
                    spec,
                    status: status.map(Into::into),
                    priority: None,
                };
                match self.ask(pb::request::Kind::UpdateItem(update)).await? {
                    pb::response::Kind::ItemUpdated(pb::ItemUpdated { item: Some(item) }) => {
                        Ok(format!(
                            "Updated {}: {} ({}). The person can undo it from the board's log.",
                            item.id,
                            item.title,
                            status_name(item.given_status())
                        ))
                    }
                    other => Err(unexpected(&other)),
                }
            }
            "add_subtask" => {
                let title = required(text("title"), "title")?;
                let parent = match text("task_id").filter(|id| !id.is_empty()) {
                    Some(task) => self.item(&task).await?,
                    None => {
                        let session = self.session().await?;
                        if session.item_id.is_empty() {
                            return Err(
                                "this session works on no task; give task_id, from the board"
                                    .to_owned(),
                            );
                        }
                        let item = self.item(&session.item_id).await?;
                        if item.parent_id.is_empty() {
                            item
                        } else {
                            self.item(&item.parent_id).await?
                        }
                    }
                };
                let create = pb::CreateItem {
                    project_id: parent.project_id.clone(),
                    parent_id: parent.id.clone(),
                    title,
                    spec: text("spec").unwrap_or_default(),
                    ..Default::default()
                };
                match self.ask(pb::request::Kind::CreateItem(create)).await? {
                    pb::response::Kind::ItemCreated(pb::ItemCreated { item: Some(item) }) => Ok(
                        format!("Added {} under {}: {}.", item.id, parent.title, item.title),
                    ),
                    other => Err(unexpected(&other)),
                }
            }
            "ask_person" => {
                let ask = pb::AskPerson {
                    session_id: self.session.clone(),
                    text: required(text("question"), "question")?,
                };
                match self.ask(pb::request::Kind::AskPerson(ask)).await? {
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
                let item_id = match text("item_id").filter(|id| !id.is_empty()) {
                    Some(id) => id,
                    None => {
                        let session = self.session().await?;
                        if session.item_id.is_empty() {
                            return Err("this session works on no task; give item_id, from board"
                                .to_owned());
                        }
                        session.item_id
                    }
                };
                let asked = pb::RequestMove {
                    session_id: self.session.clone(),
                    item_id,
                    status: status.into(),
                    summary,
                };
                match self.ask(pb::request::Kind::RequestMove(asked)).await? {
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

    /// One of the session's project's items.
    async fn item(&self, id: &str) -> Result<pb::Item, String> {
        let session = self.session().await?;
        if session.project_id.is_empty() {
            return Err("this session isn't in a project, so it has no board".to_owned());
        }
        self.items(&session.project_id)
            .await?
            .into_iter()
            .find(|item| item.id == id)
            .ok_or_else(|| format!("the board has no item {id}; see board for their ids"))
    }

    async fn board(&self) -> Result<String, String> {
        let session = self.session().await?;
        if session.project_id.is_empty() {
            return Ok("This session isn't in a project, so it has no board.".to_owned());
        }
        let items = self.items(&session.project_id).await?;
        if items.is_empty() {
            return Ok(
                "The board is empty; add_subtask needs a task, which the person adds.".to_owned(),
            );
        }
        let mut text = String::from("Tasks, with their subtasks under them:\n");
        for item in &items {
            let indent = if item.parent_id.is_empty() { "" } else { "  " };
            let yours = if item.id == session.item_id {
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
        Ok(text)
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
            pb::CommentAuthor::You => "The person",
            pb::CommentAuthor::Agent => "You",
        };
        text.push_str(&format!("{who}: {}\n", comment.text.trim()));
    }
    text
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

const STATUSES: [&str; 6] = [
    "todo",
    "in_progress",
    "waiting_on_you",
    "in_review",
    "done",
    "wont_do",
];

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

fn tools() -> Value {
    let object = |properties: Value, required: &[&str]| json!({"type": "object", "properties": properties, "required": required});
    let string = |description: &str| json!({"type": "string", "description": description});
    json!([
        {
            "name": "board",
            "description": "The board of the project this session is in: each task, its subtasks, their ids and statuses, and which one this session works on.",
            "inputSchema": object(json!({}), &[]),
        },
        {
            "name": "read_item",
            "description": "A task's or subtask's title and spec, as it is now.",
            "inputSchema": object(json!({"item_id": string("From board.")}), &["item_id"]),
        },
        {
            "name": "update_item",
            "description": "Change a task's or subtask's status, title or spec. Moving it to in_review, done or wont_do is the person's call: use request_move for those.",
            "inputSchema": object(json!({
                "item_id": string("From board."),
                "status": {"type": "string", "enum": STATUSES},
                "title": string("The new title."),
                "spec": string("The whole new spec, in Markdown; it replaces the old one."),
            }), &["item_id"]),
        },
        {
            "name": "add_subtask",
            "description": "Add a subtask, under this session's task unless task_id says another.",
            "inputSchema": object(json!({
                "title": string("Short, like a commit subject."),
                "spec": string("What it is and why, in Markdown."),
                "task_id": string("A task from board; a subtask can't have subtasks."),
            }), &["title"]),
        },
        {
            "name": "propose_task",
            "description": "Propose a task, or a subtask under parent_id, with its spec. Nothing is added until the person applies it from your proposal in the chat, perhaps after editing it.",
            "inputSchema": object(json!({
                "title": string("Short, like a commit subject."),
                "spec": string("What it is, why, and when it's done, in Markdown."),
                "parent_id": string("A task from board, to propose a subtask under it."),
                "note": string("Why you propose it, if it isn't plain from the spec."),
            }), &["title", "spec"]),
        },
        {
            "name": "propose_spec_edit",
            "description": "Propose a new spec for a task or subtask. It replaces the old one only when the person applies it; sessions working on it are told to read it again.",
            "inputSchema": object(json!({
                "item_id": string("From board."),
                "spec": string("The whole new spec, in Markdown."),
                "note": string("What changed and why."),
            }), &["item_id", "spec"]),
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
        assert!(names.contains(&"update_item") && names.contains(&"reply_to_thread"));
        assert_eq!(answers[2]["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn the_agent_moves_its_item_and_adds_subtasks_under_its_task() {
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

        let (_, failed) = call(
            &server,
            "update_item",
            json!({"item_id": subtask.id, "status": "waiting_on_you"}),
        )
        .await;
        assert!(!failed);
        let (_, failed) = call(&server, "add_subtask", json!({"title": "copy"})).await;
        assert!(!failed);
        use pb::ItemStatus::{Todo, WaitingOnYou};
        assert_eq!(
            board(&server.client, &project.id).await,
            [
                ("login".to_owned(), Todo),
                ("form".to_owned(), WaitingOnYou),
                ("copy".to_owned(), Todo),
            ]
        );

        for (tool, arguments) in [
            (
                "update_item",
                json!({"item_id": subtask.id, "status": "finished"}),
            ),
            ("update_item", json!({"item_id": subtask.id})),
            ("update_item", json!({"item_id": "nope", "status": "todo"})),
            (
                "update_item",
                json!({"item_id": subtask.id, "status": "done"}),
            ),
            (
                "add_subtask",
                json!({"title": "deeper", "task_id": subtask.id}),
            ),
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
