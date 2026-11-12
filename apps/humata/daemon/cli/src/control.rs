//! `humata mcp`: Humata as MCP tools for an agent outside it, such as
//! Claude Code in a terminal, on stdin and stdout. Each tool is one request
//! of the daemon's protocol, made over its socket as the app and the rest of
//! this tool make them, so a tool can do only what the protocol allows.
//!
//! This is not `humatad mcp`, which serves one session's agent its board.

use std::time::Duration;

use humata_wire::{Client, pb};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::error::CliError;
use crate::print;

/// The MCP versions this server speaks, newest first.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// How long send_message and approve wait for the session by default.
const WAIT_SECONDS: u64 = 120;

/// The most search hits a tool gives, so they fit the agent's context.
const SEARCH_HITS: u32 = 20;

/// Answers each JSON-RPC message, one per line, until the input ends.
pub async fn serve(
    client: &Client,
    input: impl AsyncRead + Unpin,
    mut output: impl AsyncWrite + Unpin,
) -> Result<(), CliError> {
    let mut lines = BufReader::new(input).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|source| CliError::Pipe { source })?
    {
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(client, &message).await,
            Err(_) => Some(failure(&Value::Null, -32700, "that isn't JSON")),
        };
        if let Some(answer) = answer {
            let mut text = answer.to_string();
            text.push('\n');
            output
                .write_all(text.as_bytes())
                .await
                .map_err(|source| CliError::Pipe { source })?;
            output
                .flush()
                .await
                .map_err(|source| CliError::Pipe { source })?;
        }
    }
    Ok(())
}

fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The answer to one message; none for a notification.
async fn handle(client: &Client, message: &Value) -> Option<Value> {
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
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default();
            let empty = json!({});
            let arguments = params.get("arguments").unwrap_or(&empty);
            let (text, failed) = match call(client, name, &Arguments(arguments)).await {
                Ok(text) => (text, false),
                Err(text) => (text, true),
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
struct Tool {
    name: &'static str,
    description: &'static str,
    properties: Value,
    required: &'static [&'static str],
    effect: Effect,
}

#[derive(Clone, Copy, PartialEq)]
enum Effect {
    Reads,
    Changes,
    Destroys,
}

fn string(description: &str) -> Value {
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
                "approval": {"type": "string", "enum": ["ask", "edits", "everything"],
                    "description": "What it may do without asking; default ask"},
                "backend_config": string("The backend's config, such as a scripted agent's script as JSON"),
            }),
            required: &["backend"],
            effect: Effect::Changes,
        },
        Tool {
            name: "send_message",
            description: "Sends a session a message, then waits until it is idle or asks for something, and answers what it did: one event per line, `id<TAB>kind<TAB>detail`. A permission_requested line's detail starts with the request's id, for approve.",
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
            description: "A session's events, one per line, as send_message prints them.",
            properties: json!({
                "session_id": session(),
                "last": {"type": "integer", "description": "Only the last this many"},
            }),
            required: &["session_id"],
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
            name: "create_item",
            description: "Adds a task to a project's board, or a subtask under one, with a Markdown spec. Answers its id.",
            properties: json!({
                "project_id": project(),
                "title": string("Its title"),
                "spec": string("What it is and when it's done, in Markdown"),
                "parent_id": string("The task to put it under, for a subtask"),
            }),
            required: &["project_id", "title"],
            effect: Effect::Changes,
        },
        Tool {
            name: "read_spec",
            description: "A task's spec, in Markdown.",
            properties: json!({"project_id": project(), "item_id": string("The task's id, from board")}),
            required: &["project_id", "item_id"],
            effect: Effect::Reads,
        },
        Tool {
            name: "edit_spec",
            description: "Replaces a task's spec. The board's log keeps the old one, to undo it from.",
            properties: json!({
                "item_id": string("The task's id, from board"),
                "spec": string("The whole new spec, in Markdown"),
            }),
            required: &["item_id", "spec"],
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

fn tools() -> Vec<Value> {
    tool_list()
        .into_iter()
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
struct Arguments<'a>(&'a Value);

impl Arguments<'_> {
    /// A string, trimmed; empty when not given.
    fn text(&self, field: &str) -> String {
        self.0[field]
            .as_str()
            .map(str::trim)
            .unwrap_or_default()
            .to_owned()
    }

    fn required(&self, field: &str) -> Result<String, String> {
        Some(self.text(field))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("give {field}"))
    }

    fn flag(&self, field: &str) -> bool {
        self.0[field].as_bool().unwrap_or_default()
    }

    fn number(&self, field: &str) -> Option<u64> {
        self.0[field].as_u64()
    }
}

async fn ask(client: &Client, request: pb::request::Kind) -> Result<pb::response::Kind, String> {
    client
        .call(request)
        .await
        .map_err(|error| humata_wire::describe(&error))
}

fn unexpected(got: &pb::response::Kind) -> String {
    format!("humatad answered with {got:?}; update Humata so humata and humatad match")
}

fn lines<T>(items: &[T], line: impl Fn(&T) -> String, none: &str) -> String {
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
            });
            match ask(client, request).await? {
                Response::TurnAborted(_) => Ok("Stopped its turn.".to_owned()),
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
            let shown: Vec<_> = events.iter().filter(|event| !is_delta(event)).collect();
            let skip = arguments
                .number("last")
                .and_then(|last| usize::try_from(last).ok())
                .map_or(0, |last| shown.len().saturating_sub(last));
            Ok(lines(
                &shown[skip..],
                |event| print::event(event),
                "It has no events yet.",
            ))
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
                "The board is empty; add a task with create_item.",
            ))
        }
        "create_item" => {
            let request = Request::CreateItem(pb::CreateItem {
                project_id: arguments.required("project_id")?,
                parent_id: arguments.text("parent_id"),
                title: arguments.required("title")?,
                spec: arguments.0["spec"].as_str().unwrap_or_default().to_owned(),
                ..Default::default()
            });
            match ask(client, request).await? {
                Response::ItemCreated(pb::ItemCreated { item: Some(item) }) => {
                    Ok(format!("Added {}: {}.", item.id, item.title))
                }
                other => Err(unexpected(&other)),
            }
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
        "edit_spec" => {
            let spec = arguments.0["spec"]
                .as_str()
                .filter(|spec| !spec.trim().is_empty())
                .ok_or("give spec: the whole new spec, in Markdown")?;
            let request = Request::UpdateItem(pb::UpdateItem {
                item_id: arguments.required("item_id")?,
                spec: Some(spec.to_owned()),
                ..Default::default()
            });
            match ask(client, request).await? {
                Response::ItemUpdated(_) => {
                    Ok("Replaced the spec; the board's log can undo it.".to_owned())
                }
                other => Err(unexpected(&other)),
            }
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
        "show" => show(arguments).await,
        _ => Err(format!("humata has no tool {name:?}; see tools/list")),
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
        other => {
            return Err(format!(
                "{other:?} isn't an approval mode; give ask, edits or everything"
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
                events.push(print::event(event));
            }
        }),
    )
    .await;
    let mut text = events.join("\n");
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
    use super::*;

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

    #[test]
    fn only_destructive_tools_say_so_and_readers_change_nothing() {
        let tools = tools();
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
