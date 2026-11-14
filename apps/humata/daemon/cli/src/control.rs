//! `humata mcp`: Humata as MCP tools for an agent outside it, such as
//! Claude Code in a terminal, on stdin and stdout. Each tool is one request
//! of the daemon's protocol, made over its socket as the app and the rest of
//! this tool make them, so a tool can do only what the protocol allows.
//!
//! This is not `humatad mcp`, which serves one session's agent its board.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use humata_wire::pb;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::task::{AbortHandle, JoinSet};

use crate::approved::Client;
use crate::error::CliError;
use crate::print;

/// The MCP versions this server speaks, newest first.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// How long send_message and approve wait for the session by default.
const WAIT_SECONDS: u64 = 120;

/// The most search hits a tool gives, so they fit the agent's context.
const SEARCH_HITS: u32 = 20;

/// The most bytes of one event's line an agent is given: of a message or a
/// reply, of the last reply shown, and of any other event.
const MESSAGE_BYTES: usize = 4 * 1024;
const LAST_REPLY_BYTES: usize = 16 * 1024;
const EVENT_BYTES: usize = 600;

/// The most bytes of events one answer holds, kept from its end.
const ANSWER_BYTES: usize = 32 * 1024;

/// How long calls under way when the input ends still have to be answered.
const ENDED_GRACE: Duration = Duration::from_secs(5);

type Connecting =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Client, CliError>> + Send>>;

/// The daemon as `humata mcp` reaches it for as long as it runs: the
/// connection in hand, if one has been made, and how to make another once
/// that one has ended, as it does whenever the daemon is restarted or
/// updated. It starts with none, so `humata mcp` runs before the daemon does.
pub struct Connection {
    client: tokio::sync::Mutex<Option<Arc<Client>>>,
    connect: Box<dyn Fn() -> Connecting + Send + Sync>,
}

impl Connection {
    /// `client` is the connection already made, if any; `connect` makes
    /// the next.
    pub fn new<Connect, Made>(client: Option<Arc<Client>>, connect: Connect) -> Self
    where
        Connect: Fn() -> Made + Send + Sync + 'static,
        Made: std::future::Future<Output = Result<Client, CliError>> + Send + 'static,
    {
        Self {
            client: tokio::sync::Mutex::new(client),
            connect: Box::new(move || -> Connecting { Box::pin(connect()) }),
        }
    }

    /// The connection in hand while it is open, else a new one, kept in its
    /// place. Calls that ask at once connect once: the others wait for it.
    /// A connection that can't be made is the caller's error, and the next
    /// call tries again.
    async fn client(&self) -> Result<Arc<Client>, CliError> {
        let mut client = self.client.lock().await;
        if let Some(open) = client.as_ref().filter(|client| client.is_connected()) {
            return Ok(open.clone());
        }
        let made = Arc::new((self.connect)().await?);
        *client = Some(made.clone());
        Ok(made)
    }
}

/// Answers each JSON-RPC message, one per line, until the input ends. Each
/// request is answered as its own task, so one that waits, as send_message
/// does, holds up no other; answers go out as they're ready, each naming
/// its request, and a request the client cancels stops. When the input
/// ends, calls already under way still have [`ENDED_GRACE`] to be answered,
/// since a client may close its input as soon as it has written. A tool
/// call made after the daemon's connection ended is made on a new one; a
/// call under way when it ended is never made again.
pub async fn serve(
    connection: Connection,
    input: impl AsyncRead + Unpin,
    mut output: impl AsyncWrite + Unpin,
) -> Result<(), CliError> {
    let connection = Arc::new(connection);
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
                let Some(line) = line.map_err(|source| CliError::Pipe { source })? else {
                    ended = Some(Box::pin(tokio::time::sleep(ENDED_GRACE)));
                    continue;
                };
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    write(&mut output, &failure(&Value::Null, -32700, "that isn't JSON")).await?;
                    continue;
                };
                let Some(id) = message.get("id") else {
                    // A notification is answered with nothing; this one
                    // stops the request it names. A send_message stopped
                    // has sent its message or hasn't: only its wait ends.
                    if message["method"] == json!("notifications/cancelled")
                        && let Some(cancelled) =
                            asked.remove(&message["params"]["requestId"].to_string())
                    {
                        cancelled.abort();
                    }
                    continue;
                };
                let id = id.to_string();
                let connection = Arc::clone(&connection);
                let handle =
                    answering.spawn(async move { handle(&connection, &message).await });
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
                            failure(&id, -32603, "humata mcp failed on this call; call it again")
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

/// Writes one answer on its line.
async fn write(output: &mut (impl AsyncWrite + Unpin), answer: &Value) -> Result<(), CliError> {
    let mut text = answer.to_string();
    text.push('\n');
    output
        .write_all(text.as_bytes())
        .await
        .map_err(|source| CliError::Pipe { source })?;
    output
        .flush()
        .await
        .map_err(|source| CliError::Pipe { source })
}

fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The answer to one message; none for a notification.
async fn handle(connection: &Connection, message: &Value) -> Option<Value> {
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
                "instructions": "Humata directs coding agents in sessions, each in its own \
                    copy of a project's repositories, with a board of tasks and specs. Set \
                    projects up, start sessions, talk to them and answer what they ask, as \
                    the person would in the app; then show them what you made with show.",
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools(refused(connection).await.as_deref())}),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default();
            let empty = json!({});
            let arguments = params.get("arguments").unwrap_or(&empty);
            // Only a tool needs the daemon, so only here is it reached again
            // if its connection ended.
            let (text, failed) = match connection.client().await {
                Ok(client) => match call(&client, name, &Arguments(arguments)).await {
                    Ok(text) => (text, false),
                    Err(text) => (text, true),
                },
                Err(error) => (humata_wire::describe(&error), true),
            };
            json!({"content": [{"type": "text", "text": text}], "isError": failed})
        }
        method => {
            return Some(failure(
                id,
                -32601,
                &format!("humata mcp has no method {method:?}"),
            ));
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// One tool: its name, what it does, its arguments' schema, and whether it
/// changes nothing or destroys something.
pub(crate) struct Tool {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) properties: Value,
    pub(crate) required: &'static [&'static str],
    pub(crate) effect: Effect,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Effect {
    Reads,
    Changes,
    Destroys,
}

pub(crate) fn string(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn boolean(description: &str) -> Value {
    json!({"type": "boolean", "description": description})
}

fn tool_list() -> Vec<Tool> {
    let session = || string("The session's id, from list_sessions");
    let project = || string("The project's id, from list_projects");
    let wait = || {
        json!({"type": "integer", "description":
            "Seconds to wait for the session to be idle or to ask for something; 0 doesn't wait. Default 120."})
    };
    vec![
        Tool {
            name: "list_projects",
            description: "Every project: its id, its repositories, and its name.",
            properties: json!({}),
            required: &[],
            effect: Effect::Reads,
        },
        Tool {
            name: "create_project",
            description: "Adds a project of one or more repositories, each a folder on this Mac or a git URL, such as https://github.com/owner/repo, which the host clones. Answers its id.",
            properties: json!({
                "name": string("What to call it"),
                "repositories": {"type": "array", "items": {"type": "string"},
                    "description": "Each repository: a folder, a git URL, or NAME=folder-or-URL"},
                "instructions": string("Instructions every session in it reads"),
            }),
            required: &["name", "repositories"],
            effect: Effect::Changes,
        },
        Tool {
            name: "delete_project",
            description: "Deletes a project and its board. Destructive: its tasks and specs are gone; its sessions stay, outside any project.",
            properties: json!({"project_id": project()}),
            required: &["project_id"],
            effect: Effect::Destroys,
        },
        Tool {
            name: "list_hosts",
            description: "The machines sessions can run on besides this Mac: name, SSH destination, variable names, setup.",
            properties: json!({}),
            required: &[],
            effect: Effect::Reads,
        },
        Tool {
            name: "host_status",
            description: "Whether a host is reachable, and its Humata and agents' versions.",
            properties: json!({"host": string("The host's name, from list_hosts")}),
            required: &["host"],
            effect: Effect::Reads,
        },
        Tool {
            name: "list_sessions",
            description: "Sessions, one per line: id, host, state, parent, project, approval mode, backend, model, marks, folder, name.",
            properties: json!({"project_id": string("Only this project's; empty for all")}),
            required: &[],
            effect: Effect::Reads,
        },
        Tool {
            name: "start_session",
            description: "Starts a session: a free chat in a folder, a project's session (with item_id, on a task), or a planning session that proposes tasks and specs. Answers its id; send it a message to begin.",
            properties: json!({
                "backend": {"type": "string", "enum": ["claude", "copilot", "local", "scripted"],
                    "description": "The agent: Claude Code, Copilot, a local model, or a scripted one for tests"},
                "model": string("The backend's model; empty for its default"),
                "project_id": string("The project to work in; empty for a free chat"),
                "item_id": string("A task on the project's board to work on"),
                "planning": boolean("A planning session, which proposes tasks and edits specs"),
                "name": string("What to call it"),
                "host": string("The host to run on; empty for this Mac"),
                "directory": string("For a free chat, the folder to work in"),
                "approval": {"type": "string", "enum": ["ask", "edits", "everything", "auto"],
                    "description": "What it may do without asking; default ask. auto lets the agent's own judge decide, for claude and copilot"},
                "backend_config": string("The backend's config, such as a scripted agent's script as JSON"),
            }),
            required: &["backend"],
            effect: Effect::Changes,
        },
        Tool {
            name: "send_message",
            description: "Sends a session a message, then waits until it is idle or asks for something, and answers what it did: one event per line, `id<TAB>kind<TAB>detail`. A permission_requested line's detail starts with the request's id, for approve. Long events are cut, each saying how many bytes were left out and the from_byte that read_event takes to read on, and when there are too many the earliest are left out, with a first line naming the before_event_id that read_history takes to show them.",
            properties: json!({
                "session_id": session(),
                "text": string("The message"),
                "wait_seconds": wait(),
            }),
            required: &["session_id", "text"],
            effect: Effect::Changes,
        },
        Tool {
            name: "approve",
            description: "Approves or denies what a session asked to do, then waits as send_message does.",
            properties: json!({
                "session_id": session(),
                "request_id": string("The permission request's id"),
                "deny": boolean("Deny it instead"),
                "wait_seconds": wait(),
            }),
            required: &["session_id", "request_id"],
            effect: Effect::Changes,
        },
        Tool {
            name: "stop_turn",
            description: "Stops what a session is doing now.",
            properties: json!({"session_id": session()}),
            required: &["session_id"],
            effect: Effect::Changes,
        },
        Tool {
            name: "rewind_session",
            description: "Cuts a session's chat back to before one of the person's messages or before a compaction, stopping its turn and dropping what's queued; with files, its code goes back as it was then too. undo_rewind brings it all back until the next message.",
            properties: json!({
                "session_id": session(),
                "before_event_id": string("The id of the person's message, or of the compaction, to cut from"),
                "files": boolean("Put the code back as it was then too"),
            }),
            required: &["session_id", "before_event_id"],
            effect: Effect::Changes,
        },
        Tool {
            name: "undo_rewind",
            description: "Undoes a session's last rewind, chat and files, if no message was sent since.",
            properties: json!({"session_id": session()}),
            required: &["session_id"],
            effect: Effect::Changes,
        },
        Tool {
            name: "compact_session",
            description: "Has a session's agent sum up its conversation so far to make room, in a turn of its own; the history then holds a compaction, and rewind_session to it brings the full conversation back. A working session is refused unless stop is given.",
            properties: json!({
                "session_id": session(),
                "focus": string("What the summary should keep in mind, for an agent that takes it"),
                "stop": boolean("Stop the running turn first; what's queued runs after the summary"),
            }),
            required: &["session_id"],
            effect: Effect::Changes,
        },
        Tool {
            name: "fork_session",
            description: "A new session with this one's history and a copy of its code as it is. Answers its id.",
            properties: json!({"session_id": session(), "name": string("What to call the fork")}),
            required: &["session_id"],
            effect: Effect::Changes,
        },
        Tool {
            name: "archive_session",
            description: "Archives a session. Destructive: its copy of the code is removed; its history stays, and unarchiving makes a new copy without uncommitted changes.",
            properties: json!({"session_id": session()}),
            required: &["session_id"],
            effect: Effect::Destroys,
        },
        Tool {
            name: "read_history",
            description: "A session's events, one per line, as send_message prints them. Long events are cut, each saying how many bytes were left out and the from_byte that read_event takes to read on. When they don't all fit, the latest are shown and a first line says how many earlier ones aren't: call again with before_event_id set to the id it names to read those.",
            properties: json!({
                "session_id": session(),
                "last": {"type": "integer", "description": "Only the last this many"},
                "before_event_id": string("Only events before the one with this id; with last, the last that many before it"),
            }),
            required: &["session_id"],
            effect: Effect::Reads,
        },
        Tool {
            name: "read_event",
            description: "One event's line from a byte of it, to read on where read_history or send_message cut it. A part that isn't the event's end says the from_byte of the next.",
            properties: json!({
                "session_id": session(),
                "event_id": string("The event's id, the first field of its line"),
                "from_byte": {"type": "integer", "minimum": 0, "description": "Where to read from, as the note at a cut gives it. Default 0, the line's start."},
            }),
            required: &["session_id", "event_id"],
            effect: Effect::Reads,
        },
        Tool {
            name: "list_inbox",
            description: "What waits on the person: approvals, agents' questions and requests to move tasks, failures, replies. One per line: session, reason, project, task, name, then for a question or move its request id, status asked for and text.",
            properties: json!({}),
            required: &[],
            effect: Effect::Reads,
        },
        Tool {
            name: "answer_request",
            description: "Answers an agent's question or request to move its task: reply sends the session a message; approve moves the task as asked; dismiss takes it out of the inbox.",
            properties: json!({
                "request_id": string("The request's id, from list_inbox"),
                "reply": string("A reply, sent to the session"),
                "approve": boolean("Move the task as asked"),
                "dismiss": boolean("Take it out of the inbox, doing nothing"),
            }),
            required: &["request_id"],
            effect: Effect::Changes,
        },
        Tool {
            name: "board",
            description: "A project's tasks and subtasks, one per line: id, parent, status, status given, priority, sessions, title.",
            properties: json!({"project_id": project()}),
            required: &["project_id"],
            effect: Effect::Reads,
        },
        Tool {
            name: "propose_task",
            description: "Proposes a task for a project's board, or a subtask under one, with a Markdown spec. It isn't added until the person applies it on the board in Humata, perhaps edited; use show to put the board in front of them.",
            properties: json!({
                "project_id": project(),
                "title": string("Its title"),
                "spec": string("What it is and when it's done, in Markdown"),
                "parent_id": string("The task to put it under, for a subtask"),
                "kind": string("milestone, for one that holds tasks, or task; a task if left out. Not milestone with parent_id"),
                "depends_on": {"type": "array", "items": {"type": "string"},
                    "description": "Ids, from board, of the items it waits on"},
                "note": string("Why, for the person deciding"),
            }),
            required: &["project_id", "title"],
            effect: Effect::Changes,
        },
        Tool {
            name: "propose_dependency",
            description: "Proposes that an item on a project's board waits on others: it is blocked until they are done, and comes after them in plan. Milestones and tasks can wait on either. The links are made only when the person applies it on the board in Humata, and refused then if they would close a loop.",
            properties: json!({
                "project_id": project(),
                "item_id": string("The item that waits, from board"),
                "depends_on": {"type": "array", "items": {"type": "string"},
                    "description": "Ids, from board, of the items it waits on"},
                "note": string("Why, for the person deciding"),
            }),
            required: &["project_id", "item_id", "depends_on"],
            effect: Effect::Changes,
        },
        Tool {
            name: "plan",
            description: "A project's items in the order to take them, each after what it depends on and a milestone's tasks under it, one per line: id, milestone or task, status, ready or blocked or finished, the ids it waits on, title.",
            properties: json!({"project_id": project()}),
            required: &["project_id"],
            effect: Effect::Reads,
        },
        Tool {
            name: "read_spec",
            description: "A task's spec, in Markdown.",
            properties: json!({"project_id": project(), "item_id": string("The task's id, from board")}),
            required: &["project_id", "item_id"],
            effect: Effect::Reads,
        },
        Tool {
            name: "propose_spec_edit",
            description: "Proposes a new spec or title for a task or subtask. It changes only when the person applies it on the board in Humata; until then read_spec shows it as it was.",
            properties: json!({
                "project_id": project(),
                "item_id": string("The task's id, from board"),
                "title": string("Its new title, if it changes"),
                "spec": string("The whole new spec, in Markdown, if it changes"),
                "note": string("Why, for the person deciding"),
            }),
            required: &["project_id", "item_id"],
            effect: Effect::Changes,
        },
        Tool {
            name: "search",
            description: "Searches every session's messages, replies and tool calls, review comments and specs, with related words too.",
            properties: json!({
                "query": string("The words to find"),
                "project_id": string("Only this project's"),
            }),
            required: &["query"],
            effect: Effect::Reads,
        },
        Tool {
            name: "show",
            description: "Opens a session, a task or a project's board in the Humata app, behind other windows unless bring_forward is set.",
            properties: json!({
                "session_id": string("A session to open"),
                "project_id": string("A project whose board to open"),
                "item_id": string("With project_id, a task to open"),
                "bring_forward": boolean("Bring Humata in front of other windows"),
            }),
            required: &[],
            effect: Effect::Reads,
        },
    ]
}

/// The ids of the Agent Access switches the person has off for this
/// connection, as the daemon says; None when no daemon says, as before it
/// runs.
async fn refused(connection: &Connection) -> Option<Vec<String>> {
    let client = connection.client().await.ok()?;
    refused_for(&client).await
}

async fn refused_for(client: &Client) -> Option<Vec<String>> {
    let request = pb::request::Kind::GetAgentAccess(pb::GetAgentAccess {});
    match client.call(request).await {
        Ok(pb::response::Kind::AgentAccess(access)) => Some(access.refused),
        _ => None,
    }
}

/// The ids of the Agent Access switches tool `name` goes by: those of the
/// requests it makes of the daemon. Most tools make one, whose switch has
/// the tool's name.
fn switches(name: &'static str) -> Vec<&'static str> {
    match name {
        // They wait on the session by following its chat.
        "send_message" | "approve" => vec![name, "read_history"],
        "read_event" => vec!["read_history"],
        "read_spec" | "list_proposals" | "plan" => vec!["board"],
        "propose_task" | "propose_spec_edit" | "propose_dependency" => vec!["propose"],
        "decline_proposal" => vec!["apply_proposal"],
        "update_item" => vec!["create_item"],
        // Following an approval is never off.
        "approval_status" => Vec::new(),
        _ => vec![name],
    }
}

/// The tools to offer: all but those a switch in `refused` turns off, so
/// an agent isn't offered what the daemon would refuse. With None, when no
/// daemon has said what is off, those that destroy are left out, as they
/// are until the person turns them on.
fn tools(refused: Option<&[String]>) -> Vec<Value> {
    #[allow(unused_mut)]
    let mut listed = tool_list();
    #[cfg(feature = "admin")]
    listed.extend(crate::admin::tools());
    listed
        .into_iter()
        .filter(|tool| match refused {
            Some(refused) => !switches(tool.name)
                .iter()
                .any(|switch| refused.iter().any(|off| off == switch)),
            None => tool.effect != Effect::Destroys,
        })
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": {"type": "object", "properties": tool.properties, "required": tool.required},
                "annotations": {
                    "readOnlyHint": tool.effect == Effect::Reads,
                    "destructiveHint": tool.effect == Effect::Destroys,
                },
            })
        })
        .collect()
}

/// A tool call's arguments.
pub(crate) struct Arguments<'a>(pub(crate) &'a Value);

impl Arguments<'_> {
    /// A string, trimmed; empty when not given.
    pub(crate) fn text(&self, field: &str) -> String {
        self.0[field]
            .as_str()
            .map(str::trim)
            .unwrap_or_default()
            .to_owned()
    }

    /// A list of strings, each trimmed, without the empty ones; none when
    /// not given.
    pub(crate) fn texts(&self, field: &str) -> Vec<String> {
        self.0[field]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|text| text.as_str().map(str::trim))
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .collect()
    }

    pub(crate) fn required(&self, field: &str) -> Result<String, String> {
        Some(self.text(field))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("give {field}"))
    }

    fn flag(&self, field: &str) -> bool {
        self.0[field].as_bool().unwrap_or_default()
    }

    /// A whole number of 0 or more, written as an integer, as a float with
    /// no fraction or as either in a string; not given otherwise.
    pub(crate) fn number(&self, field: &str) -> Option<u64> {
        // A float is turned through its digits, which fails where a cast
        // would quietly give another number.
        let whole = |number: f64| {
            (number >= 0.0 && number.fract() == 0.0)
                .then(|| format!("{number:.0}").parse::<u64>().ok())
                .flatten()
        };
        match &self.0[field] {
            Value::Number(number) => number.as_u64().or_else(|| number.as_f64().and_then(whole)),
            Value::String(text) => {
                let text = text.trim();
                text.parse::<u64>()
                    .ok()
                    .or_else(|| text.parse::<f64>().ok().and_then(whole))
            }
            _ => None,
        }
    }
}

/// The part of `line`, the line of the event `id`, from byte `from`, moved
/// back to a character's start, that is at most `most` bytes long. A part
/// that isn't the line's end says how many bytes were left out and how
/// read_event reads on. It always takes at least one character, so reading
/// from each note in turn reaches the end.
fn part(line: &str, from: usize, most: usize, id: &str) -> String {
    let mut from = from.min(line.len());
    while !line.is_char_boundary(from) {
        from -= 1;
    }
    let mut end = from.saturating_add(most).min(line.len());
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    if end == from {
        end = line[from..]
            .chars()
            .next()
            .map_or(from, |first| from + first.len_utf8());
    }
    let mut part = line[from..end].to_owned();
    if end < line.len() {
        part.push_str(&format!(
            " [cut here: {} more bytes left out; read_event with event_id {id} and from_byte {end} reads on]",
            line.len() - end
        ));
    }
    part
}

/// An event's line as [`print::event`] gives it, cut for an agent's
/// context; `last_reply` for the last reply of what is shown, which gets
/// more room.
fn event_line(event: &pb::SessionEvent, last_reply: bool) -> String {
    use humata_wire::pb::session_event::Kind;
    let limit = match &event.kind {
        Some(Kind::AssistantMessage(_)) if last_reply => LAST_REPLY_BYTES,
        Some(Kind::UserMessage(_) | Kind::AssistantMessage(_)) => MESSAGE_BYTES,
        _ => EVENT_BYTES,
    };
    part(&print::event(event), 0, limit, &event.id)
}

/// The events as an answer for an agent: a line each, cut, the latest that
/// fit [`ANSWER_BYTES`]. When earlier ones didn't fit, a first line says how
/// many and names the `before_event_id` that reads them.
fn shown(events: &[&pb::SessionEvent]) -> String {
    use humata_wire::pb::session_event::Kind;
    let last_reply = events.iter().rposition(|event| {
        event.parent_call_id.is_empty() && matches!(event.kind, Some(Kind::AssistantMessage(_)))
    });
    let mut kept = Vec::new();
    let mut size = 0;
    let mut first = events.len();
    for (index, event) in events.iter().enumerate().rev() {
        let line = event_line(event, Some(index) == last_reply);
        if !kept.is_empty() && size + line.len() + 1 > ANSWER_BYTES {
            break;
        }
        size += line.len() + 1;
        kept.push(line);
        first = index;
    }
    if first > 0
        && let Some(event) = events.get(first)
    {
        kept.push(format!(
            "[{first} earlier events aren't shown; read_history with before_event_id {} shows them]",
            event.id
        ));
    }
    kept.reverse();
    kept.join("\n")
}

/// The events of a history to show: those before the event `before` names,
/// when it names one, and of them the last `last`, when given. Parts of a
/// reply as it streamed are never shown.
fn before<'a>(
    events: &'a [pb::SessionEvent],
    before: &str,
    last: Option<u64>,
) -> Result<Vec<&'a pb::SessionEvent>, String> {
    let end = if before.is_empty() {
        events.len()
    } else {
        events
            .iter()
            .position(|event| event.id == before)
            .ok_or_else(|| {
                format!(
                    "the session's history has no event {before}; it may have been rewound since. Call read_history without before_event_id to see the ids it has now"
                )
            })?
    };
    let shown: Vec<_> = events[..end]
        .iter()
        .filter(|event| !is_delta(event))
        .collect();
    let skip = last
        .and_then(|last| usize::try_from(last).ok())
        .map_or(0, |last| shown.len().saturating_sub(last));
    Ok(shown[skip..].to_vec())
}

/// The kind `given` names for a new item; unspecified, a task, for none.
pub(crate) fn item_kind(given: &str) -> Result<pb::ItemKind, String> {
    match given {
        "" => Ok(pb::ItemKind::Unspecified),
        "task" => Ok(pb::ItemKind::Task),
        "milestone" => Ok(pb::ItemKind::Milestone),
        other => Err(format!("kind is milestone or task, not {other:?}")),
    }
}

/// Proposes `propose`, which waits for the person on the board.
async fn proposed(client: &Client, propose: pb::Propose) -> Result<String, String> {
    match ask(client, pb::request::Kind::Propose(propose)).await? {
        pb::response::Kind::Proposed(pb::Proposed {
            proposal: Some(proposal),
        }) => Ok(format!(
            "Proposed {:?} ({}). It waits on the project's board until the person applies or declines it; don't count on it until then.",
            proposal.title, proposal.id
        )),
        other => Err(unexpected(&other)),
    }
}

pub(crate) async fn ask(
    client: &Client,
    request: pb::request::Kind,
) -> Result<pb::response::Kind, String> {
    client
        .call(request)
        .await
        .map_err(|error| humata_wire::describe(&error))
}

pub(crate) fn unexpected(got: &pb::response::Kind) -> String {
    format!("humatad answered with {got:?}; update Humata so humata and humatad match")
}

pub(crate) fn lines<T>(items: &[T], line: impl Fn(&T) -> String, none: &str) -> String {
    if items.is_empty() {
        return none.to_owned();
    }
    items.iter().map(line).collect::<Vec<_>>().join("\n")
}

/// What a tool answers, or why it failed, for the agent to read.
async fn call(client: &Client, name: &str, arguments: &Arguments<'_>) -> Result<String, String> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    match name {
        "list_projects" => match ask(client, Request::ListProjects(pb::ListProjects {})).await? {
            Response::Projects(projects) => Ok(lines(
                &projects.projects,
                print::project,
                "There are no projects; add one with create_project.",
            )),
            other => Err(unexpected(&other)),
        },
        "create_project" => {
            let name = arguments.required("name")?;
            let given: Vec<String> = arguments.0["repositories"]
                .as_array()
                .map(|all| {
                    all.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            if given.is_empty() {
                return Err("give repositories: at least one folder or git URL".to_owned());
            }
            let repositories = given
                .iter()
                .map(|given| crate::repository(given))
                .collect::<Result<_, _>>()
                .map_err(|error| humata_wire::describe(&error))?;
            let request = Request::CreateProject(pb::CreateProject {
                name,
                repositories,
                instructions: arguments.text("instructions"),
                skills: Vec::new(),
            });
            match ask(client, request).await? {
                Response::ProjectCreated(pb::ProjectCreated {
                    project: Some(project),
                }) => Ok(format!("Created project {}.", project.id)),
                other => Err(unexpected(&other)),
            }
        }
        "delete_project" => {
            let request = Request::DeleteProject(pb::DeleteProject {
                project_id: arguments.required("project_id")?,
            });
            match ask(client, request).await? {
                Response::ProjectDeleted(_) => Ok("Deleted the project.".to_owned()),
                other => Err(unexpected(&other)),
            }
        }
        "list_hosts" => match ask(client, Request::ListHosts(pb::ListHosts {})).await? {
            Response::Hosts(hosts) => Ok(lines(
                &hosts.hosts,
                print::host,
                "No hosts besides this Mac.",
            )),
            other => Err(unexpected(&other)),
        },
        "host_status" => {
            let request = Request::CheckHost(pb::CheckHost {
                name: arguments.required("host")?,
            });
            match ask(client, request).await? {
                Response::HostHealth(health) => Ok(print::health(&health)),
                other => Err(unexpected(&other)),
            }
        }
        "list_sessions" => {
            let request = Request::ListSessions(pb::ListSessions {
                project_id: arguments.text("project_id"),
            });
            match ask(client, request).await? {
                Response::Sessions(sessions) => Ok(lines(
                    &sessions.sessions,
                    print::session,
                    "There are no sessions.",
                )),
                other => Err(unexpected(&other)),
            }
        }
        "start_session" => start(client, arguments).await,
        "send_message" => {
            let session = arguments.required("session_id")?;
            let request = Request::SendMessage(pb::SendMessage {
                session_id: session.clone(),
                text: arguments.required("text")?,
                mode: pb::SendMode::Enqueue.into(),
                client_id: String::new(),
                images: Vec::new(),
            });
            let sent = |response: &Response| matches!(response, Response::MessageSent(_));
            wait(client, &session, request, sent, "MessageSent", arguments).await
        }
        "approve" => {
            let session = arguments.required("session_id")?;
            let request = Request::Approve(pb::Approve {
                session_id: session.clone(),
                permission_request_id: arguments.required("request_id")?,
                approved: !arguments.flag("deny"),
            });
            let approved = |response: &Response| matches!(response, Response::Approved(_));
            wait(client, &session, request, approved, "Approved", arguments).await
        }
        "stop_turn" => {
            let request = Request::AbortTurn(pb::AbortTurn {
                session_id: arguments.required("session_id")?,
                drop_queued: false,
            });
            match ask(client, request).await? {
                Response::TurnAborted(_) => Ok("Stopped its turn.".to_owned()),
                other => Err(unexpected(&other)),
            }
        }
        "rewind_session" => {
            let request = Request::RewindSession(pb::RewindSession {
                session_id: arguments.required("session_id")?,
                before_event_id: arguments.required("before_event_id")?,
                files: arguments.flag("files"),
            });
            match ask(client, request).await? {
                Response::SessionRewound(_) => Ok("Rewound it.".to_owned()),
                other => Err(unexpected(&other)),
            }
        }
        "undo_rewind" => {
            let request = Request::UndoRewind(pb::UndoRewind {
                session_id: arguments.required("session_id")?,
            });
            match ask(client, request).await? {
                Response::RewindUndone(_) => Ok("Undid its rewind.".to_owned()),
                other => Err(unexpected(&other)),
            }
        }
        "compact_session" => {
            let request = Request::CompactSession(pb::CompactSession {
                session_id: arguments.required("session_id")?,
                focus: arguments.text("focus"),
                stop: arguments.flag("stop"),
            });
            match ask(client, request).await? {
                Response::CompactStarted(_) => Ok("Compacting it.".to_owned()),
                other => Err(unexpected(&other)),
            }
        }
        "fork_session" => {
            let request = Request::ForkSession(pb::ForkSession {
                session_id: arguments.required("session_id")?,
                name: arguments.text("name"),
                ..Default::default()
            });
            match ask(client, request).await? {
                Response::SessionForked(pb::SessionForked {
                    session: Some(session),
                }) => Ok(format!("Forked as session {}.", session.id)),
                other => Err(unexpected(&other)),
            }
        }
        "archive_session" => {
            let request = Request::UpdateSession(pb::UpdateSession {
                session_id: arguments.required("session_id")?,
                archived: Some(true),
                ..Default::default()
            });
            match ask(client, request).await? {
                Response::SessionUpdated(_) => Ok("Archived the session.".to_owned()),
                other => Err(unexpected(&other)),
            }
        }
        "read_history" => {
            let session = arguments.required("session_id")?;
            let events = crate::history(client, &session)
                .await
                .map_err(|error| humata_wire::describe(&error))?;
            let from = arguments.text("before_event_id");
            let events = before(&events, &from, arguments.number("last"))?;
            Ok(if !events.is_empty() {
                shown(&events)
            } else if from.is_empty() {
                "It has no events yet.".to_owned()
            } else {
                "It has no events before that one.".to_owned()
            })
        }
        "read_event" => {
            let session = arguments.required("session_id")?;
            let id = arguments.required("event_id")?;
            let events = crate::history(client, &session)
                .await
                .map_err(|error| humata_wire::describe(&error))?;
            let event = events.iter().find(|event| event.id == id).ok_or_else(|| {
                format!(
                    "the session's history has no event {id}; it may have been rewound since. Call read_history to see the ids it has now"
                )
            })?;
            let line = print::event(event);
            let from = arguments
                .number("from_byte")
                .map_or(0, |from| usize::try_from(from).unwrap_or(usize::MAX));
            if from > 0 && from >= line.len() {
                return Err(format!(
                    "the event's line is {} bytes long, so nothing starts at byte {from}; give a from_byte below that, or none to read it from its start",
                    line.len()
                ));
            }
            Ok(part(&line, from, LAST_REPLY_BYTES, &id))
        }
        "list_inbox" => match ask(client, Request::ListInbox(pb::ListInbox {})).await? {
            Response::Inbox(inbox) => Ok(lines(
                &inbox.entries,
                print::inbox_entry,
                "Nothing waits on the person.",
            )),
            other => Err(unexpected(&other)),
        },
        "answer_request" => {
            let answer = if !arguments.text("reply").is_empty() {
                pb::answer_request::Answer::Reply(arguments.text("reply"))
            } else if arguments.flag("approve") {
                pb::answer_request::Answer::Approve(true)
            } else if arguments.flag("dismiss") {
                pb::answer_request::Answer::Dismiss(true)
            } else {
                return Err("say how to answer: reply, approve or dismiss".to_owned());
            };
            let request = Request::AnswerRequest(pb::AnswerRequest {
                request_id: arguments.required("request_id")?,
                answer: Some(answer),
            });
            match ask(client, request).await? {
                Response::RequestAnswered(_) => Ok("Answered.".to_owned()),
                other => Err(unexpected(&other)),
            }
        }
        "board" => {
            let items = items(client, arguments.required("project_id")?).await?;
            Ok(lines(
                &items,
                print::item,
                "The board is empty; suggest a task with propose_task.",
            ))
        }
        "propose_task" => {
            let propose = pb::Propose {
                project_id: arguments.required("project_id")?,
                kind: pb::ProposalKind::Task.into(),
                title: arguments.required("title")?,
                spec: arguments.0["spec"].as_str().unwrap_or_default().to_owned(),
                parent_id: arguments.text("parent_id"),
                note: arguments.text("note"),
                item_kind: item_kind(&arguments.text("kind"))?.into(),
                depends_on: arguments.texts("depends_on"),
                ..Default::default()
            };
            proposed(client, propose).await
        }
        "propose_dependency" => {
            let depends_on = arguments.texts("depends_on");
            if depends_on.is_empty() {
                return Err(
                    "give depends_on, the ids from board of the items it waits on".to_owned(),
                );
            }
            let propose = pb::Propose {
                project_id: arguments.required("project_id")?,
                kind: pb::ProposalKind::Dependency.into(),
                item_id: arguments.required("item_id")?,
                depends_on,
                note: arguments.text("note"),
                ..Default::default()
            };
            proposed(client, propose).await
        }
        "plan" => {
            let mut items = items(client, arguments.required("project_id")?).await?;
            items.sort_by_key(|item| item.plan_order);
            Ok(lines(
                &items,
                print::planned,
                "The board is empty; suggest a task with propose_task.",
            ))
        }
        "read_spec" => {
            let item = arguments.required("item_id")?;
            let found = items(client, arguments.required("project_id")?)
                .await?
                .into_iter()
                .find(|found| found.id == item)
                .ok_or_else(|| format!("the board has no task {item}; see board for their ids"))?;
            Ok(if found.spec.trim().is_empty() {
                format!("{} has no spec yet.", found.title)
            } else {
                found.spec
            })
        }
        "propose_spec_edit" => {
            let propose = pb::Propose {
                project_id: arguments.required("project_id")?,
                kind: pb::ProposalKind::SpecEdit.into(),
                item_id: arguments.required("item_id")?,
                title: arguments.text("title"),
                spec: arguments.0["spec"].as_str().unwrap_or_default().to_owned(),
                note: arguments.text("note"),
                ..Default::default()
            };
            proposed(client, propose).await
        }
        "search" => {
            let request = Request::Search(pb::Search {
                query: arguments.required("query")?,
                project_id: arguments.text("project_id"),
                limit: SEARCH_HITS,
                ..pb::Search::default()
            });
            match ask(client, request).await? {
                Response::SearchResults(results) => Ok(lines(
                    &results.hits,
                    print::search_hit,
                    "Nothing found; try fewer words, or others that mean the same.",
                )),
                other => Err(unexpected(&other)),
            }
        }
        "show" => {
            // It asks nothing of the daemon, which so can't refuse it.
            let off = refused_for(client).await.unwrap_or_default();
            if off.iter().any(|off| off == "show") {
                return Err(
                    "show is turned off for agents in Humata's Settings › Agent Access. The person can turn it on there, in the Humata app; tell them what you'd have shown instead"
                        .to_owned(),
                );
            }
            show(arguments).await
        }
        _ => {
            #[cfg(feature = "admin")]
            if let Some(answer) = crate::admin::call(client, name, arguments).await {
                return answer;
            }
            Err(format!("humata has no tool {name:?}; see tools/list"))
        }
    }
}

async fn start(client: &Client, arguments: &Arguments<'_>) -> Result<String, String> {
    let backend = match arguments.required("backend")?.as_str() {
        "claude" => pb::Backend::Claude,
        "copilot" => pb::Backend::Copilot,
        "local" => pb::Backend::Local,
        "scripted" => pb::Backend::Scripted,
        other => {
            return Err(format!(
                "{other:?} isn't a backend; give claude, copilot, local or scripted"
            ));
        }
    };
    let approval = match arguments.text("approval").as_str() {
        "" | "ask" => pb::ApprovalMode::AskEveryTime,
        "edits" => pb::ApprovalMode::AllowEdits,
        "everything" => pb::ApprovalMode::AllowEverything,
        "auto" => pb::ApprovalMode::Auto,
        other => {
            return Err(format!(
                "{other:?} isn't an approval mode; give ask, edits, everything or auto"
            ));
        }
    };
    let request = pb::request::Kind::StartSession(pb::StartSession {
        backend: backend.into(),
        working_directory: arguments.text("directory"),
        model: arguments.text("model"),
        name: arguments.text("name"),
        host: arguments.text("host"),
        backend_config: arguments.0["backend_config"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        approval_mode: approval.into(),
        project_id: arguments.text("project_id"),
        item_id: arguments.text("item_id"),
        planning: arguments.flag("planning"),
        ..Default::default()
    });
    match ask(client, request).await? {
        pb::response::Kind::SessionStarted(pb::SessionStarted {
            session: Some(session),
        }) => Ok(format!(
            "Started session {}; send it a message to begin.",
            session.id
        )),
        other => Err(unexpected(&other)),
    }
}

/// Makes `request`, then answers the session's new events until it is idle
/// or asks for something, or `wait_seconds` pass.
async fn wait(
    client: &Client,
    session: &str,
    request: pb::request::Kind,
    answered: impl Fn(&pb::response::Kind) -> bool,
    expected: &'static str,
    arguments: &Arguments<'_>,
) -> Result<String, String> {
    let seconds = arguments.number("wait_seconds").unwrap_or(WAIT_SECONDS);
    if seconds == 0 {
        let response = ask(client, request).await?;
        return if answered(&response) {
            Ok("Done; it carries on without being waited for.".to_owned())
        } else {
            Err(unexpected(&response))
        };
    }
    let mut events = Vec::new();
    let waited = tokio::time::timeout(
        Duration::from_secs(seconds),
        crate::call_and_wait(client, session, request, answered, expected, |event| {
            if !is_delta(event) {
                events.push(event.clone());
            }
        }),
    )
    .await;
    let mut text = shown(&events.iter().collect::<Vec<_>>());
    match waited {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            text.push_str(&format!("\n{}", humata_wire::describe(&error)));
            return Err(text);
        }
        Err(_) => text.push_str(&format!(
            "\nStill working after {seconds} seconds; read_history shows how it goes on."
        )),
    }
    Ok(text.trim_start().to_owned())
}

fn is_delta(event: &pb::SessionEvent) -> bool {
    matches!(event.kind, Some(pb::session_event::Kind::AssistantDelta(_)))
}

async fn items(client: &Client, project: String) -> Result<Vec<pb::Item>, String> {
    crate::items(client, project)
        .await
        .map_err(|error| humata_wire::describe(&error))
}

/// The `humata://` URL that opens what the arguments name in the app.
fn link(arguments: &Arguments<'_>) -> Result<String, String> {
    let session = arguments.text("session_id");
    let project = arguments.text("project_id");
    let item = arguments.text("item_id");
    let safe = |id: &str| {
        id.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    if ![&session, &project, &item].iter().all(|id| safe(id)) {
        return Err("ids are letters, digits, - and _; copy them from the other tools".to_owned());
    }
    match (session.is_empty(), project.is_empty(), item.is_empty()) {
        (false, true, true) => Ok(format!("humata://session/{session}")),
        (true, false, true) => Ok(format!("humata://board/{project}")),
        (true, false, false) => Ok(format!("humata://task/{project}/{item}")),
        _ => Err(
            "give session_id, or project_id for its board, or project_id and item_id for a task"
                .to_owned(),
        ),
    }
}

async fn show(arguments: &Arguments<'_>) -> Result<String, String> {
    let url = link(arguments)?;
    let mut open = tokio::process::Command::new("open");
    if !arguments.flag("bring_forward") {
        open.arg("-g");
    }
    let status = open
        .arg(&url)
        .status()
        .await
        .map_err(|error| format!("couldn't run open for {url}: {error}; is this a Mac?"))?;
    if status.success() {
        Ok(format!("Opened {url} in Humata."))
    } else {
        Err(format!(
            "open {url} failed; check that the Humata app is installed, then try again"
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use humata_wire::pb::session_event::Kind;
    use humata_wire::{Reply, Service, ServiceFuture};
    use tokio::io::{duplex, split};
    use tokio::sync::mpsc;

    use super::*;

    fn event(id: &str, kind: Kind) -> pb::SessionEvent {
        pb::SessionEvent {
            id: id.to_owned(),
            kind: Some(kind),
            ..Default::default()
        }
    }

    fn user(text: impl Into<String>) -> Kind {
        Kind::UserMessage(pb::UserMessage {
            text: text.into(),
            ..Default::default()
        })
    }

    fn reply(text: impl Into<String>) -> Kind {
        Kind::AssistantMessage(pb::AssistantMessage { text: text.into() })
    }

    fn ids<'a>(events: &[&'a pb::SessionEvent]) -> Vec<&'a str> {
        events.iter().map(|event| event.id.as_str()).collect()
    }

    #[test]
    fn a_number_is_read_however_it_is_written() {
        let number = |value: Value| {
            let arguments = json!({ "n": value });
            Arguments(&arguments).number("n")
        };
        assert_eq!(number(json!(20)), Some(20));
        assert_eq!(number(json!(20.0)), Some(20));
        assert_eq!(number(json!("20")), Some(20));
        assert_eq!(number(json!(" 20.0 ")), Some(20));
        assert_eq!(number(json!(0)), Some(0));
        assert_eq!(number(json!(20.5)), None);
        assert_eq!(number(json!(-1)), None);
        assert_eq!(number(json!("-3")), None);
        assert_eq!(number(json!("many")), None);
        assert_eq!(number(json!(true)), None);
        assert_eq!(number(Value::Null), None);
    }

    /// How many bytes of `whole` the `cut` line starts with.
    fn kept(cut: &str, whole: &str) -> usize {
        cut.bytes()
            .zip(whole.bytes())
            .take_while(|(cut, whole)| cut == whole)
            .count()
    }

    #[test]
    fn a_long_event_is_cut_and_says_how_much_is_left_out() {
        // Two bytes a character, so a cut has boundaries to respect.
        let long = "\u{e9}".repeat(5_000);
        let message = event("e1", user(long.clone()));
        let whole = print::event(&message);
        let line = event_line(&message, false);
        let start = kept(&line, &whole);
        assert!(
            start <= MESSAGE_BYTES && start + 4 > MESSAGE_BYTES,
            "{start}"
        );
        assert!(line.len() < whole.len());
        assert!(
            line[start..].contains(&(whole.len() - start).to_string()),
            "{}",
            &line[start..]
        );

        let output = event(
            "e2",
            Kind::ToolCompleted(pb::ToolCompleted {
                call_id: "c1".to_owned(),
                success: true,
                output: long.clone(),
            }),
        );
        assert!(kept(&event_line(&output, false), &print::event(&output)) <= EVENT_BYTES);

        // The last reply shown has more room, and a short event is whole.
        let answer = event("e3", reply(long));
        assert_eq!(event_line(&answer, true), print::event(&answer));
        assert!(event_line(&answer, false).len() < print::event(&answer).len());
        let short = event("e4", user("hi"));
        assert_eq!(event_line(&short, false), print::event(&short));
    }

    /// The `from_byte` a cut part's note names, if it was cut.
    fn next_byte(part: &str) -> Option<usize> {
        let (_, rest) = part.rsplit_once("from_byte ")?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    }

    #[test]
    fn a_cut_event_is_read_to_its_end_from_the_byte_each_part_names() {
        // Three bytes a character, and a limit they don't divide.
        let whole = format!("e1\tuser\t{}", "\u{20ac}".repeat(400));
        let mut read = String::new();
        let mut from = 0;
        let mut parts = 0;
        loop {
            let part = part(&whole, from, 100, "e1");
            let taken = kept(&part, &whole[from..]);
            assert!(taken > 0 && taken <= 100, "{taken}");
            read.push_str(&whole[from..from + taken]);
            parts += 1;
            assert!(parts <= whole.len(), "it never reaches the end");
            match next_byte(&part) {
                Some(next) if from + taken < whole.len() => {
                    assert_eq!(next, from + taken);
                    from = next;
                }
                _ => break,
            }
        }
        assert_eq!(read, whole);
        assert!(parts > 1);

        // A byte inside a character reads from that character's start.
        let inside = part(&whole, 9, 100, "e1");
        assert!(inside.starts_with('\u{20ac}'), "{inside}");
    }

    #[test]
    fn a_history_too_long_keeps_its_end_and_names_where_the_rest_starts() {
        let events: Vec<pb::SessionEvent> = (0..40)
            .map(|number| event(&format!("e{number}"), user("x".repeat(3_000))))
            .collect();
        let all: Vec<&pb::SessionEvent> = events.iter().collect();

        let text = shown(&all);
        let rows: Vec<&str> = text.lines().collect();
        let listed: Vec<&str> = rows[1..]
            .iter()
            .map(|row| row.split('\t').next().unwrap_or_default())
            .collect();
        let first = events.len() - listed.len();
        assert!(first > 0 && listed.len() > 1, "{first}");
        assert_eq!(listed, ids(&all[first..]));
        assert!(text.len() <= ANSWER_BYTES + 200, "{}", text.len());
        // The first line names the event to read on from, which gives
        // exactly the events left out.
        assert!(rows[0].contains(&events[first].id), "{}", rows[0]);
        let rest = before(&events, &events[first].id, None).expect("it holds the event");
        assert_eq!(ids(&rest), ids(&all[..first]));

        // A history that fits is its events and nothing else.
        assert_eq!(shown(&all[..3]).lines().count(), 3);
    }

    #[test]
    fn events_before_an_id_are_the_ones_shown() {
        let events = [
            event("e1", user("one")),
            event(
                "e2",
                Kind::AssistantDelta(pb::AssistantDelta {
                    text: "tw".to_owned(),
                }),
            ),
            event("e3", reply("two")),
            event("e4", user("three")),
            event("e5", reply("four")),
        ];
        let shown = |from: &str, last| before(&events, from, last).map(|events| ids(&events));
        assert_eq!(shown("e4", None), Ok(vec!["e1", "e3"]));
        assert_eq!(shown("e4", Some(1)), Ok(vec!["e3"]));
        assert_eq!(shown("", Some(2)), Ok(vec!["e4", "e5"]));
        assert_eq!(shown("e1", None), Ok(Vec::new()));
        assert!(shown("e9", None).is_err());
    }

    type Items = mpsc::Sender<Result<pb::StreamItem, pb::Error>>;

    /// A daemon with one session: a read answers `history`, each watch
    /// hands the test the sender of its events, and a message is taken.
    struct Daemon {
        history: Mutex<Vec<pb::SessionEvent>>,
        /// How many times the history was read.
        reads: AtomicUsize,
        watched: mpsc::UnboundedSender<Items>,
        /// The Agent Access switches that are off for the client.
        refused: Mutex<Vec<String>>,
    }

    impl Service for Daemon {
        fn handle(self: Arc<Self>, request: pb::request::Kind) -> ServiceFuture {
            Box::pin(async move {
                match request {
                    pb::request::Kind::ReadHistory(_) => {
                        self.reads.fetch_add(1, Ordering::Relaxed);
                        let events = self.history.lock().expect("not poisoned").clone();
                        Reply::Response(pb::response::Kind::History(pb::History { events }))
                    }
                    pb::request::Kind::WatchSession(_) => {
                        let (items, receiver) = mpsc::channel(8);
                        let _ = self.watched.send(items);
                        Reply::Stream {
                            response: pb::response::Kind::Watching(pb::Watching {}),
                            items: receiver,
                        }
                    }
                    pb::request::Kind::SendMessage(_) => {
                        Reply::Response(pb::response::Kind::MessageSent(pb::MessageSent {}))
                    }
                    pb::request::Kind::GetAgentAccess(_) => {
                        Reply::Response(pb::response::Kind::AgentAccess(pb::AgentAccess {
                            refused: self.refused.lock().expect("not poisoned").clone(),
                            ..Default::default()
                        }))
                    }
                    _ => Reply::Response(humata_wire::error(
                        pb::ErrorCode::InvalidRequest,
                        "not in this test",
                    )),
                }
            })
        }
    }

    fn daemon() -> (Arc<Daemon>, mpsc::UnboundedReceiver<Items>) {
        let (watched, watches) = mpsc::unbounded_channel();
        let daemon = Arc::new(Daemon {
            history: Mutex::default(),
            reads: AtomicUsize::new(0),
            watched,
            refused: Mutex::default(),
        });
        (daemon, watches)
    }

    /// A connection to `daemon`, served over its own in-memory pipe, and
    /// the task serving it: stopping the task is the daemon going away.
    async fn served(daemon: Arc<Daemon>) -> (Client, tokio::task::JoinHandle<()>) {
        let (ours, theirs) = duplex(64 * 1024);
        let (reader, writer) = split(theirs);
        let serving = tokio::spawn(async move {
            let _ = humata_wire::serve(reader, writer, pb::Role::Daemon, "test", daemon).await;
        });
        let (reader, writer) = split(ours);
        let wire = humata_wire::Client::connect(reader, writer, pb::Role::Client, "test")
            .await
            .expect("connects");
        let wait = crate::approved::Wait::For(Duration::ZERO);
        (Client::new(wire, wait, false), serving)
    }

    async fn connected() -> (Arc<Client>, Arc<Daemon>, mpsc::UnboundedReceiver<Items>) {
        let (daemon, watches) = daemon();
        let (client, _serving) = served(daemon.clone()).await;
        (Arc::new(client), daemon, watches)
    }

    /// A connection with no way to make another.
    fn only(client: Arc<Client>) -> Connection {
        Connection::new(Some(client), || async { Err(CliError::NoSocket) })
    }

    /// A connection whose daemon has gone away, once the client knows.
    async fn gone() -> Arc<Client> {
        let (client, serving) = served(daemon().0).await;
        serving.abort();
        tokio::time::timeout(Duration::from_secs(5), async {
            while client.is_connected() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the client sees the daemon go");
        Arc::new(client)
    }

    fn item(event: pb::SessionEvent) -> Result<pb::StreamItem, pb::Error> {
        Ok(pb::StreamItem {
            kind: Some(pb::stream_item::Kind::SessionEvent(event)),
        })
    }

    fn requested(id: &str, request: &str) -> pb::SessionEvent {
        event(
            id,
            Kind::PermissionRequested(pb::PermissionRequested {
                request_id: request.to_owned(),
                ..Default::default()
            }),
        )
    }

    fn message() -> pb::request::Kind {
        pb::request::Kind::SendMessage(pb::SendMessage {
            session_id: "s".to_owned(),
            text: "hi".to_owned(),
            ..Default::default()
        })
    }

    fn sent(response: &pb::response::Kind) -> bool {
        matches!(response, pb::response::Kind::MessageSent(_))
    }

    #[tokio::test]
    async fn a_wait_goes_on_past_a_request_the_mode_allowed() {
        let (client, daemon, mut watches) = connected().await;
        let mut seen = Vec::new();
        let waiting = crate::call_and_wait(&client, "s", message(), sent, "MessageSent", |event| {
            seen.push(event.id.clone());
        });
        let drive = async {
            let items = watches.recv().await.expect("the wait watches");
            let allowed = event(
                "e2",
                Kind::PermissionCompleted(pb::PermissionCompleted {
                    request_id: "r1".to_owned(),
                    approved: true,
                    automatic: true,
                    judged: false,
                }),
            );
            for event in [requested("e1", "r1"), allowed] {
                items.send(item(event)).await.expect("sent");
            }
            // Longer than a request is held before the history is asked.
            tokio::time::sleep(Duration::from_millis(600)).await;
            for event in [
                event("e3", reply("done")),
                event("e4", Kind::Idle(pb::SessionIdle {})),
            ] {
                items.send(item(event)).await.expect("sent");
            }
            items
        };
        let (waited, _items) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(waiting, drive)
        })
        .await
        .expect("the wait ends");

        waited.expect("waited");
        assert_eq!(seen, ["e1", "e2", "e3", "e4"]);
        // Once, for what was there before the message; not again for the
        // request.
        assert_eq!(daemon.reads.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn a_wait_ends_at_a_request_no_one_answered() {
        let (client, daemon, mut watches) = connected().await;
        let mut seen = Vec::new();
        let waiting = crate::call_and_wait(&client, "s", message(), sent, "MessageSent", |event| {
            seen.push(event.id.clone());
        });
        let drive = async {
            let items = watches.recv().await.expect("the wait watches");
            let asked = requested("e1", "r1");
            daemon
                .history
                .lock()
                .expect("not poisoned")
                .push(asked.clone());
            items.send(item(asked)).await.expect("sent");
            items
        };
        let (waited, _items) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(waiting, drive)
        })
        .await
        .expect("the wait ends");

        waited.expect("waited");
        assert_eq!(seen, ["e1"]);
    }

    #[tokio::test]
    async fn a_wait_ends_at_a_request_already_open_when_it_starts() {
        let (client, daemon, mut watches) = connected().await;
        daemon
            .history
            .lock()
            .expect("not poisoned")
            .push(requested("e1", "r1"));
        let mut seen = Vec::new();
        let waiting = crate::call_and_wait(&client, "s", message(), sent, "MessageSent", |event| {
            seen.push(event.id.clone());
        });
        // Nothing is streamed: the session is still stopped at the request.
        let drive = async { watches.recv().await.expect("the wait watches") };
        let (waited, _items) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(waiting, drive)
        })
        .await
        .expect("the wait ends");

        waited.expect("waited");
        assert_eq!(seen, ["e1"]);
    }

    #[tokio::test]
    async fn a_request_open_at_the_start_and_answered_since_doesnt_end_the_wait() {
        let (client, daemon, mut watches) = connected().await;
        daemon
            .history
            .lock()
            .expect("not poisoned")
            .push(requested("e1", "r1"));
        let mut seen = Vec::new();
        let waiting = crate::call_and_wait(&client, "s", message(), sent, "MessageSent", |event| {
            seen.push(event.id.clone());
        });
        let drive = async {
            let items = watches.recv().await.expect("the wait watches");
            let answered = event(
                "e2",
                Kind::PermissionCompleted(pb::PermissionCompleted {
                    request_id: "r1".to_owned(),
                    approved: true,
                    automatic: false,
                    judged: false,
                }),
            );
            daemon
                .history
                .lock()
                .expect("not poisoned")
                .push(answered.clone());
            items.send(item(answered)).await.expect("sent");
            // Longer than a request is held before the history is asked.
            tokio::time::sleep(Duration::from_millis(600)).await;
            let idle = event("e3", Kind::Idle(pb::SessionIdle {}));
            items.send(item(idle)).await.expect("sent");
            items
        };
        let (waited, _items) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(waiting, drive)
        })
        .await
        .expect("the wait ends");

        waited.expect("waited");
        assert_eq!(seen, ["e2", "e3"]);
    }

    #[tokio::test]
    async fn a_request_left_by_a_turn_that_ended_doesnt_end_the_wait() {
        let (client, daemon, mut watches) = connected().await;
        daemon.history.lock().expect("not poisoned").extend([
            requested("e1", "r1"),
            event("e2", Kind::Idle(pb::SessionIdle {})),
        ]);
        let mut seen = Vec::new();
        let waiting = crate::call_and_wait(&client, "s", message(), sent, "MessageSent", |event| {
            seen.push(event.id.clone());
        });
        let drive = async {
            let items = watches.recv().await.expect("the wait watches");
            items
                .send(item(event("e3", reply("on it"))))
                .await
                .expect("sent");
            // Longer than a request is held before the history is asked.
            tokio::time::sleep(Duration::from_millis(600)).await;
            let idle = event("e4", Kind::Idle(pb::SessionIdle {}));
            items.send(item(idle)).await.expect("sent");
            items
        };
        let (waited, _items) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(waiting, drive)
        })
        .await
        .expect("the wait ends");

        waited.expect("waited");
        assert_eq!(seen, ["e3", "e4"]);
    }

    fn line(message: Value) -> Vec<u8> {
        format!("{message}\n").into_bytes()
    }

    fn send_message(id: u64) -> Vec<u8> {
        line(json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": "send_message", "arguments": {"session_id": "s", "text": "hi"}},
        }))
    }

    fn ping(id: u64) -> Vec<u8> {
        line(json!({"jsonrpc": "2.0", "id": id, "method": "ping"}))
    }

    #[tokio::test]
    async fn a_ping_is_answered_while_a_call_waits() {
        let (client, _daemon, mut watches) = connected().await;
        let (mut requests, input) = duplex(4096);
        let (output, answers) = duplex(64 * 1024);
        let mut answers = BufReader::new(answers).lines();
        let drive = async {
            requests.write_all(&send_message(1)).await.expect("written");
            // The session never goes idle, so the call waits from here on.
            let items = watches.recv().await.expect("the call watches");
            requests.write_all(&ping(2)).await.expect("written");
            let answer = answers.next_line().await.expect("read").expect("an answer");
            (answer, items)
        };

        let answered = tokio::select! {
            ended = serve(only(client), input, output) => panic!("it stopped serving: {ended:?}"),
            answered = tokio::time::timeout(Duration::from_secs(10), drive) => answered,
        };

        let (answer, _items) = answered.expect("the ping is answered");
        let answer: Value = serde_json::from_str(&answer).expect("JSON");
        assert_eq!(answer["id"], json!(2));
        assert!(answer.get("error").is_none(), "{answer}");
    }

    #[tokio::test]
    async fn a_cancelled_call_is_answered_with_nothing_and_the_next_is_answered() {
        let (client, _daemon, mut watches) = connected().await;
        let (mut requests, input) = duplex(4096);
        let (output, answers) = duplex(64 * 1024);
        let mut answers = BufReader::new(answers).lines();
        let drive = async move {
            requests.write_all(&send_message(1)).await.expect("written");
            let items = watches.recv().await.expect("the call watches");
            let cancel = line(json!({
                "jsonrpc": "2.0", "method": "notifications/cancelled",
                "params": {"requestId": 1},
            }));
            requests.write_all(&cancel).await.expect("written");
            requests.write_all(&ping(2)).await.expect("written");
            // The input ends: a call still waiting would hold the server
            // open for its grace.
            drop(requests);
            let mut answered = Vec::new();
            while let Some(answer) = answers.next_line().await.expect("read") {
                let answer: Value = serde_json::from_str(&answer).expect("JSON");
                answered.push(answer["id"].clone());
            }
            (answered, items)
        };

        let (ended, (answered, _items)) = tokio::time::timeout(ENDED_GRACE / 2, async {
            tokio::join!(serve(only(client), input, output), drive)
        })
        .await
        .expect("it ends once the input does, with no call left waiting");

        ended.expect("served");
        assert_eq!(answered, [json!(2)]);
    }

    /// A send_message that doesn't wait for the session.
    fn send_and_go(id: u64) -> Vec<u8> {
        line(json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {
                "name": "send_message",
                "arguments": {"session_id": "s", "text": "hi", "wait_seconds": 0},
            },
        }))
    }

    /// A connector that counts its tries; the first `failing` of them can't
    /// reach a daemon, and each after that reaches a new one.
    fn connector(
        failing: usize,
    ) -> (
        Arc<AtomicUsize>,
        impl Fn() -> Connecting + Send + Sync + 'static,
    ) {
        let tries = Arc::new(AtomicUsize::new(0));
        let counted = tries.clone();
        let connect = move || -> Connecting {
            let attempt = counted.fetch_add(1, Ordering::Relaxed);
            Box::pin(async move {
                if attempt < failing {
                    return Err(CliError::NoSocket);
                }
                Ok(served(daemon().0).await.0)
            })
        };
        (tries, connect)
    }

    #[tokio::test]
    async fn a_call_after_the_daemon_restarted_is_answered_by_the_new_one() {
        let (tries, connect) = connector(0);
        let connection = Connection::new(Some(gone().await), connect);
        let (mut requests, input) = duplex(4096);
        let (output, answers) = duplex(64 * 1024);
        let mut answers = BufReader::new(answers).lines();
        let drive = async {
            let mut answered = Vec::new();
            for id in [1, 2] {
                requests.write_all(&send_and_go(id)).await.expect("written");
                let answer = answers.next_line().await.expect("read").expect("an answer");
                answered.push(serde_json::from_str::<Value>(&answer).expect("JSON"));
            }
            answered
        };

        let answered = tokio::select! {
            ended = serve(connection, input, output) => panic!("it stopped serving: {ended:?}"),
            answered = tokio::time::timeout(Duration::from_secs(10), drive) => answered,
        };

        for answer in answered.expect("the calls are answered") {
            assert_eq!(answer["result"]["isError"], json!(false), "{answer}");
        }
        // The second call found the connection the first one made.
        assert_eq!(tries.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn a_daemon_that_cant_be_reached_says_so_and_the_next_call_tries_again() {
        let (tries, connect) = connector(1);
        let connection = Connection::new(Some(gone().await), connect);
        let (mut requests, input) = duplex(4096);
        let (output, answers) = duplex(64 * 1024);
        let mut answers = BufReader::new(answers).lines();
        let drive = async {
            let mut answered = Vec::new();
            for id in [1, 2] {
                requests.write_all(&send_and_go(id)).await.expect("written");
                let answer = answers.next_line().await.expect("read").expect("an answer");
                answered.push(serde_json::from_str::<Value>(&answer).expect("JSON"));
            }
            answered
        };

        let answered = tokio::select! {
            ended = serve(connection, input, output) => panic!("it stopped serving: {ended:?}"),
            answered = tokio::time::timeout(Duration::from_secs(10), drive) => answered,
        };

        let answered = answered.expect("the calls are answered");
        assert_eq!(answered[0]["id"], json!(1));
        assert_eq!(
            answered[0]["result"]["isError"],
            json!(true),
            "{}",
            answered[0]
        );
        assert_eq!(answered[1]["id"], json!(2));
        assert_eq!(
            answered[1]["result"]["isError"],
            json!(false),
            "{}",
            answered[1]
        );
        assert_eq!(tries.load(Ordering::Relaxed), 2);
    }

    /// A request with no parameters.
    fn request(id: u64, method: &str) -> Vec<u8> {
        line(json!({"jsonrpc": "2.0", "id": id, "method": method}))
    }

    #[tokio::test]
    async fn it_serves_before_a_daemon_runs_and_reaches_one_that_starts_later() {
        let (tries, connect) = connector(2);
        let connection = Connection::new(None, connect);
        let (mut requests, input) = duplex(4096);
        let (output, answers) = duplex(64 * 1024);
        let mut answers = BufReader::new(answers).lines();
        let drive = async {
            let mut answered = Vec::new();
            let lines = [
                request(1, "initialize"),
                request(2, "tools/list"),
                send_and_go(3),
                send_and_go(4),
            ];
            for line in lines {
                requests.write_all(&line).await.expect("written");
                let answer = answers.next_line().await.expect("read").expect("an answer");
                answered.push(serde_json::from_str::<Value>(&answer).expect("JSON"));
            }
            answered
        };

        let answered = tokio::select! {
            ended = serve(connection, input, output) => panic!("it stopped serving: {ended:?}"),
            answered = tokio::time::timeout(Duration::from_secs(10), drive) => answered,
        };

        let answered = answered.expect("each is answered");
        assert!(
            answered[0]["result"]["protocolVersion"].is_string(),
            "{}",
            answered[0]
        );
        assert!(
            answered[1]["result"]["tools"]
                .as_array()
                .is_some_and(|tools| !tools.is_empty()),
            "{}",
            answered[1]
        );
        // The tool list asked for Agent Access and, with no daemon, listed
        // what is on by default.
        assert_eq!(answered[2]["id"], json!(3));
        assert_eq!(
            answered[2]["result"]["isError"],
            json!(true),
            "{}",
            answered[2]
        );
        assert_eq!(answered[3]["id"], json!(4));
        assert_eq!(
            answered[3]["result"]["isError"],
            json!(false),
            "{}",
            answered[3]
        );
        assert_eq!(tries.load(Ordering::Relaxed), 3);
    }

    fn link_to(arguments: Value) -> Result<String, String> {
        link(&Arguments(&arguments))
    }

    #[test]
    fn show_links_to_a_session_a_board_or_a_task() {
        assert_eq!(
            link_to(json!({"session_id": "s1"})).as_deref(),
            Ok("humata://session/s1")
        );
        assert_eq!(
            link_to(json!({"project_id": "p1"})).as_deref(),
            Ok("humata://board/p1")
        );
        assert_eq!(
            link_to(json!({"project_id": "p1", "item_id": "i2"})).as_deref(),
            Ok("humata://task/p1/i2")
        );
        assert!(link_to(json!({})).is_err());
        assert!(link_to(json!({"session_id": "s1", "project_id": "p1"})).is_err());
        assert!(link_to(json!({"session_id": "../etc"})).is_err());
    }

    /// The names of the tools `connection` lists.
    async fn listed(connection: &Connection) -> Vec<String> {
        let ask = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
        let answer = handle(connection, &ask).await.expect("an answer");
        answer["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect()
    }

    #[tokio::test]
    async fn the_tool_list_leaves_out_what_agent_access_has_off() {
        let (client, daemon, _watches) = connected().await;
        let connection = only(client);
        let has = |tools: &[String], name: &str| tools.iter().any(|tool| tool == name);

        // Nothing off: every tool, those that destroy too.
        let all = listed(&connection).await;
        for name in ["list_projects", "send_message", "delete_project", "show"] {
            assert!(has(&all, name), "{name} in {all:?}");
        }

        *daemon.refused.lock().expect("not poisoned") = vec![
            "delete_project".to_owned(),
            "read_history".to_owned(),
            "show".to_owned(),
        ];
        let some = listed(&connection).await;
        // What is off, and what couldn't work without it.
        for name in [
            "delete_project",
            "read_history",
            "read_event",
            "send_message",
            "show",
        ] {
            assert!(!has(&some, name), "{name} in {some:?}");
        }
        for name in ["list_projects", "archive_session", "start_session"] {
            assert!(has(&some, name), "{name} in {some:?}");
        }
    }

    #[tokio::test]
    async fn with_no_daemon_to_ask_the_tool_list_leaves_out_what_destroys() {
        let connection = only(gone().await);
        let tools = listed(&connection).await;
        assert!(
            tools.iter().any(|tool| tool == "list_projects"),
            "{tools:?}"
        );
        assert!(tools.iter().any(|tool| tool == "send_message"), "{tools:?}");
        assert!(
            !tools.iter().any(|tool| tool == "delete_project"),
            "{tools:?}"
        );
        assert!(
            !tools.iter().any(|tool| tool == "archive_session"),
            "{tools:?}"
        );
    }

    #[test]
    fn only_destructive_tools_say_so_and_readers_change_nothing() {
        let nothing_off: &[String] = &[];
        let tools = tools(Some(nothing_off));
        let flagged = |hint: &str| -> Vec<String> {
            tools
                .iter()
                .filter(|tool| tool["annotations"][hint] == json!(true))
                .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
                .collect()
        };
        assert_eq!(
            flagged("destructiveHint"),
            ["delete_project", "archive_session"]
        );
        assert!(flagged("readOnlyHint").contains(&"board".to_owned()));
        assert!(!flagged("readOnlyHint").contains(&"send_message".to_owned()));
    }
}
