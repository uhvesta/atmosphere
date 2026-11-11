//! The local-model backend: a model behind an OpenAI-compatible server, such
//! as Ollama or llama.cpp's, so iterating costs no tokens. The model works in
//! the session's folder with three tools: `shell` and `write_file`, which
//! ask the person first, and `read_file`.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::error::AgentError;
use crate::event::{Event, EventKind, PermissionKind};
use crate::paths::inside;
use crate::runtime::{Runner, Turn};

/// Ollama's address; llama.cpp's server and LM Studio need theirs set.
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434/v1";

/// Model replies a turn may take, each after the last one's tool calls.
const MAX_STEPS: usize = 32;
/// A tool's output beyond this is cut, to keep the model's context small.
const MAX_OUTPUT: usize = 16 * 1024;
const SHELL_TIMEOUT: Duration = Duration::from_secs(120);

/// The server and model a session uses, from its backend config, as JSON:
/// `{"base_url": "...", "api_key": "..."}`. Both are optional.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalModelConfig {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LocalModelError {
    #[error(
        "the session's local-model config isn't valid; give JSON like {{\"base_url\": \"{DEFAULT_BASE_URL}\"}} and start a new session with it"
    )]
    BadConfig {
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "the session names no model; start it with one the server has (for Ollama, `ollama list` shows them)"
    )]
    NoModel,

    #[error(
        "couldn't reach the model server at {url}; start it (for Ollama, `ollama serve`), or set base_url in the session's backend config to where it listens"
    )]
    Connect {
        url: String,
        #[source]
        source: Box<ureq::Error>,
    },

    #[error(
        "the model server at {url} answered {status}: {body}. Check that it has the model {model} (for Ollama, `ollama pull {model}`)"
    )]
    Status {
        url: String,
        model: String,
        status: u16,
        body: String,
    },

    #[error(
        "the connection to the model server at {url} broke mid-reply; check the server, then send again"
    )]
    Read {
        url: String,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the model server at {url} sent a reply this backend can't read; check that base_url points at an OpenAI-compatible API (it usually ends in /v1)"
    )]
    Decode {
        url: String,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "the model kept calling tools for {MAX_STEPS} replies without finishing; send a narrower request, or stop the turn and steer it"
    )]
    TooManySteps,

    #[error(transparent)]
    Agent(#[from] AgentError),
}

/// Plays turns on a local model server.
pub struct LocalModelRunner {
    defaults: LocalModelConfig,
    http: ureq::Agent,
}

impl LocalModelRunner {
    /// `defaults` fills in what a session's own config leaves out.
    pub fn new(defaults: LocalModelConfig) -> Self {
        let http = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .build()
            .into();
        Self { defaults, http }
    }
}

impl Runner for LocalModelRunner {
    type Error = LocalModelError;

    async fn run_turn(self: Arc<Self>, turn: Turn) -> Result<(), LocalModelError> {
        let own = match &turn.record().backend_config {
            Some(json) => serde_json::from_str::<LocalModelConfig>(json)
                .map_err(|source| LocalModelError::BadConfig { source })?,
            None => LocalModelConfig::default(),
        };
        let base_url = own
            .base_url
            .or_else(|| self.defaults.base_url.clone())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
        let request = Request {
            url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
            api_key: own.api_key.or_else(|| self.defaults.api_key.clone()),
            model: turn
                .record()
                .model
                .clone()
                .ok_or(LocalModelError::NoModel)?,
        };

        let mut calls = 0_usize;
        for _ in 0..MAX_STEPS {
            let messages = messages(turn.record().instructions.as_deref(), &turn.history());
            let reply = self.complete(&turn, &request, messages).await?;
            if reply.calls.is_empty() {
                return Ok(());
            }
            for call in reply.calls {
                calls += 1;
                let call_id = call.id.unwrap_or_else(|| format!("call-{calls}"));
                turn.emit(EventKind::ToolStarted {
                    call_id: call_id.clone(),
                    tool: call.name.clone(),
                    arguments_json: call.arguments.clone(),
                })?;
                let (success, output) = run_tool(&turn, &call.name, &call.arguments).await?;
                turn.emit(EventKind::ToolCompleted {
                    call_id,
                    success,
                    output,
                })?;
            }
            // Steering messages are in the history already, so the next
            // reply sees them.
            turn.take_steering();
        }
        Err(LocalModelError::TooManySteps)
    }
}

struct Request {
    url: String,
    api_key: Option<String>,
    model: String,
}

/// A tool call, assembled from the parts it streamed in.
#[derive(Default)]
struct Call {
    id: Option<String>,
    name: String,
    arguments: String,
}

struct Reply {
    calls: Vec<Call>,
}

/// What the reader thread passes on as the reply streams.
enum Part {
    Text(String),
    Reasoning(String),
    Call {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments: Option<String>,
    },
    Usage {
        input: u64,
        output: u64,
    },
}

impl LocalModelRunner {
    /// Streams one reply, recording its text and reasoning as they come.
    async fn complete(
        &self,
        turn: &Turn,
        request: &Request,
        messages: Vec<Value>,
    ) -> Result<Reply, LocalModelError> {
        let body = json!({
            "model": request.model,
            "messages": messages,
            "tools": tools(),
            "stream": true,
            "stream_options": {"include_usage": true},
        })
        .to_string();
        let (parts, mut received) = mpsc::channel(256);
        let http = self.http.clone();
        let url = request.url.clone();
        let model = request.model.clone();
        let api_key = request.api_key.clone();
        // ureq blocks, so the reply is read on its own thread. If the turn is
        // stopped, `received` is dropped and the thread stops at its next part.
        let reading = tokio::task::spawn_blocking(move || {
            stream(&http, &url, &model, api_key.as_deref(), &body, &parts)
        });

        let mut text = String::new();
        let mut reasoning = String::new();
        let mut calls: Vec<Call> = Vec::new();
        while let Some(part) = received.recv().await {
            match part {
                Part::Text(delta) => {
                    text.push_str(&delta);
                    turn.emit(EventKind::AssistantDelta { text: delta })?;
                }
                Part::Reasoning(delta) => reasoning.push_str(&delta),
                Part::Call {
                    index,
                    id,
                    name,
                    arguments,
                } => {
                    if calls.len() <= index {
                        calls.resize_with(index + 1, Call::default);
                    }
                    let call = &mut calls[index];
                    if id.is_some() {
                        call.id = id;
                    }
                    if let Some(name) = name {
                        call.name.push_str(&name);
                    }
                    if let Some(arguments) = arguments {
                        call.arguments.push_str(&arguments);
                    }
                }
                Part::Usage { input, output } => {
                    turn.emit(EventKind::Usage {
                        model: request.model.clone(),
                        input_tokens: input,
                        output_tokens: output,
                        premium_requests: 0.0,
                    })?;
                }
            }
        }
        // If the thread panicked, nothing more will come.
        if let Ok(result) = reading.await {
            result?;
        }
        if !reasoning.is_empty() {
            turn.emit(EventKind::Reasoning { text: reasoning })?;
        }
        if !text.is_empty() {
            turn.emit(EventKind::AssistantMessage { text })?;
        }
        calls.retain(|call| !call.name.is_empty());
        Ok(Reply { calls })
    }
}

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Delta,
}

#[derive(Default, Deserialize)]
struct Delta {
    content: Option<String>,
    /// Ollama's name for it, and llama.cpp's.
    reasoning: Option<String>,
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<CallDelta>,
}

#[derive(Deserialize)]
struct CallDelta {
    #[serde(default)]
    index: usize,
    id: Option<String>,
    function: Option<FunctionDelta>,
}

#[derive(Deserialize)]
struct FunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

/// Sends the request and passes on each part of the streamed reply, until it
/// ends or nobody is listening.
fn stream(
    http: &ureq::Agent,
    url: &str,
    model: &str,
    api_key: Option<&str>,
    body: &str,
    parts: &mpsc::Sender<Part>,
) -> Result<(), LocalModelError> {
    let mut request = http
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream");
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {key}"));
    }
    let response = request
        .send(body)
        .map_err(|source| LocalModelError::Connect {
            url: url.to_owned(),
            source: Box::new(source),
        })?;
    let status = response.status().as_u16();
    let mut reader = response.into_body().into_reader();
    if status != 200 {
        let mut text = String::new();
        let _ = reader.by_ref().take(4096).read_to_string(&mut text);
        return Err(LocalModelError::Status {
            url: url.to_owned(),
            model: model.to_owned(),
            status,
            body: text.trim().to_owned(),
        });
    }
    for line in BufReader::new(reader).lines() {
        let line = line.map_err(|source| LocalModelError::Read {
            url: url.to_owned(),
            source,
        })?;
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        let chunk: Chunk =
            serde_json::from_str(data).map_err(|source| LocalModelError::Decode {
                url: url.to_owned(),
                source,
            })?;
        for part in chunk_parts(chunk) {
            if parts.blocking_send(part).is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn chunk_parts(chunk: Chunk) -> Vec<Part> {
    let mut parts = Vec::new();
    for choice in chunk.choices {
        let delta = choice.delta;
        if let Some(text) = delta.reasoning.or(delta.reasoning_content)
            && !text.is_empty()
        {
            parts.push(Part::Reasoning(text));
        }
        if let Some(text) = delta.content
            && !text.is_empty()
        {
            parts.push(Part::Text(text));
        }
        for call in delta.tool_calls {
            let (name, arguments) = match call.function {
                Some(function) => (function.name, function.arguments),
                None => (None, None),
            };
            parts.push(Part::Call {
                index: call.index,
                id: call.id,
                name,
                arguments,
            });
        }
    }
    if let Some(usage) = chunk.usage {
        parts.push(Part::Usage {
            input: usage.prompt_tokens,
            output: usage.completion_tokens,
        });
    }
    parts
}

/// The conversation so far as chat messages: the instructions, then each
/// message, reply and tool call in the history.
fn messages(instructions: Option<&str>, history: &[Event]) -> Vec<Value> {
    let mut system = String::from(
        "You are a coding agent working in the person's project folder. Use the tools to read, change and run things there, then say briefly what you did.",
    );
    if let Some(instructions) = instructions {
        system.push_str("\n\n");
        system.push_str(instructions);
    }
    let mut messages = vec![json!({"role": "system", "content": system})];
    for event in history {
        match &event.kind {
            EventKind::UserMessage { text } => {
                messages.push(json!({"role": "user", "content": text}));
            }
            EventKind::AssistantMessage { text } => {
                messages.push(json!({"role": "assistant", "content": text}));
            }
            EventKind::ToolStarted {
                call_id,
                tool,
                arguments_json,
            } => messages.push(json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": {"name": tool, "arguments": arguments_json},
                }],
            })),
            EventKind::ToolCompleted {
                call_id, output, ..
            } => messages.push(json!({"role": "tool", "tool_call_id": call_id, "content": output})),
            _ => {}
        }
    }
    messages
}

fn tools() -> Value {
    let tool = |name: &str, description: &str, parameters: Value| {
        json!({"type": "function", "function": {
            "name": name, "description": description, "parameters": parameters,
        }})
    };
    let path = json!({"type": "string", "description": "Relative to the project folder."});
    json!([
        tool(
            "shell",
            "Runs a shell command in the project folder and returns its output.",
            json!({"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}),
        ),
        tool(
            "read_file",
            "Returns a file's contents.",
            json!({"type": "object", "properties": {"path": path}, "required": ["path"]}),
        ),
        tool(
            "write_file",
            "Writes a file, replacing what it held.",
            json!({"type": "object", "properties": {"path": path, "contents": {"type": "string"}}, "required": ["path", "contents"]}),
        ),
    ])
}

#[derive(Deserialize)]
struct ShellArguments {
    command: String,
}

#[derive(Deserialize)]
struct ReadArguments {
    path: PathBuf,
}

#[derive(Deserialize)]
struct WriteArguments {
    path: PathBuf,
    contents: String,
}

/// Runs a tool the model called. What goes wrong is told to the model, as
/// the tool's failed output, so it can try another way.
async fn run_tool(turn: &Turn, name: &str, arguments: &str) -> Result<(bool, String), AgentError> {
    let root = turn.record().working_directory.clone();
    let bad = |error: serde_json::Error| (false, format!("Bad arguments for {name}: {error}"));
    Ok(match name {
        "shell" => match serde_json::from_str::<ShellArguments>(arguments) {
            Err(error) => bad(error),
            Ok(ShellArguments { command }) => {
                if turn
                    .ask(PermissionKind::Shell, format!("Run `{command}`"))
                    .await?
                {
                    shell(&root, &command).await
                } else {
                    denied()
                }
            }
        },
        "read_file" => match serde_json::from_str::<ReadArguments>(arguments) {
            Err(error) => bad(error),
            Ok(ReadArguments { path }) => match inside(&root, &path) {
                None => outside(&path),
                Some(target) => match tokio::fs::read_to_string(&target).await {
                    Ok(contents) => (true, truncated(contents)),
                    Err(error) => (false, format!("Couldn't read {}: {error}", path.display())),
                },
            },
        },
        "write_file" => match serde_json::from_str::<WriteArguments>(arguments) {
            Err(error) => bad(error),
            Ok(WriteArguments { path, contents }) => match inside(&root, &path) {
                None => outside(&path),
                Some(target) => {
                    if turn
                        .ask(PermissionKind::Write, format!("Write {}", path.display()))
                        .await?
                    {
                        write(&target, &path, &contents).await
                    } else {
                        denied()
                    }
                }
            },
        },
        other => (false, format!("There is no tool {other}.")),
    })
}

async fn shell(root: &Path, command: &str) -> (bool, String) {
    let child = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(root)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(SHELL_TIMEOUT, child).await {
        Err(_) => (
            false,
            format!(
                "Stopped after {} seconds without finishing.",
                SHELL_TIMEOUT.as_secs()
            ),
        ),
        Ok(Err(error)) => (false, format!("Couldn't run it: {error}")),
        Ok(Ok(output)) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            if let Some(code) = output.status.code().filter(|code| *code != 0) {
                text.push_str(&format!("\n(exit status {code})"));
            }
            (output.status.success(), truncated(text))
        }
    }
}

async fn write(target: &Path, path: &Path, contents: &str) -> (bool, String) {
    if let Some(parent) = target.parent()
        && let Err(error) = tokio::fs::create_dir_all(parent).await
    {
        return (
            false,
            format!("Couldn't create the folder for {}: {error}", path.display()),
        );
    }
    match tokio::fs::write(target, contents).await {
        Ok(()) => (true, format!("Wrote {}", path.display())),
        Err(error) => (false, format!("Couldn't write {}: {error}", path.display())),
    }
}

fn denied() -> (bool, String) {
    (false, "The person denied this.".into())
}

fn outside(path: &Path) -> (bool, String) {
    (
        false,
        format!(
            "{} is outside the project folder; use a relative path without `..`.",
            path.display()
        ),
    )
}

fn truncated(mut text: String) -> String {
    if text.len() > MAX_OUTPUT {
        let mut end = MAX_OUTPUT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n(cut off here)");
    }
    text
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::Agent;
    use crate::event::{ForkOptions, SendMode, SessionId, StartOptions};
    use crate::runtime::LocalAgent;
    use crate::runtime::tests::{idle_turns, permission_request, wait_for, waiting};

    /// An OpenAI-compatible server that answers each request with the next
    /// of its replies, and keeps the requests it got.
    struct FakeServer {
        base_url: String,
        requests: Arc<Mutex<Vec<Value>>>,
    }

    /// A streamed reply, one chunk per element.
    fn sse(chunks: &[Value]) -> String {
        let mut body = String::new();
        for chunk in chunks {
            body.push_str(&format!("data: {chunk}\n\n"));
        }
        body.push_str("data: [DONE]\n\n");
        body
    }

    fn text(delta: &str) -> Value {
        json!({"choices": [{"delta": {"content": delta}}]})
    }

    fn call(id: &str, name: &str, arguments: Value) -> Vec<Value> {
        let arguments = arguments.to_string();
        let (first, rest) = arguments.split_at(arguments.len() / 2);
        vec![
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 0, "id": id, "function": {"name": name, "arguments": first}},
            ]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [
                {"index": 0, "function": {"arguments": rest}},
            ]}}]}),
        ]
    }

    async fn serve(replies: Vec<(u16, String)>) -> FakeServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listens");
        let port = listener.local_addr().expect("an address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let kept = requests.clone();
        tokio::spawn(async move {
            for (status, body) in replies {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                let mut socket = tokio::io::BufReader::new(socket);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if socket.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim_end().to_ascii_lowercase();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut request = vec![0; length];
                if socket.read_exact(&mut request).await.is_err() {
                    return;
                }
                if let Ok(request) = serde_json::from_slice(&request) {
                    kept.lock().expect("not poisoned").push(request);
                }
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.get_mut().write_all(response.as_bytes()).await;
                let _ = socket.get_mut().shutdown().await;
            }
        });
        FakeServer {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            requests,
        }
    }

    impl FakeServer {
        fn requests(&self) -> Vec<Value> {
            self.requests.lock().expect("not poisoned").clone()
        }
    }

    struct Fixture {
        agent: LocalAgent<LocalModelRunner>,
        work: tempfile::TempDir,
        _state: tempfile::TempDir,
    }

    fn fixture(server: &FakeServer) -> Fixture {
        let state = tempfile::tempdir().expect("a temporary folder");
        let work = tempfile::tempdir().expect("a temporary folder");
        let runner = LocalModelRunner::new(LocalModelConfig {
            base_url: Some(server.base_url.clone()),
            api_key: None,
        });
        let agent = LocalAgent::open(state.path(), runner).expect("the agent opens");
        Fixture {
            agent,
            work,
            _state: state,
        }
    }

    impl Fixture {
        async fn start(&self, model: Option<&str>) -> SessionId {
            let options = StartOptions {
                name: "test".into(),
                working_directory: self.work.path().to_owned(),
                model: model.map(str::to_owned),
                ..StartOptions::default()
            };
            self.agent.start(options).await.expect("starts").id
        }

        async fn send(&self, id: &SessionId, text: &str) {
            self.agent
                .send(id, text.into(), SendMode::Enqueue)
                .await
                .expect("sent");
        }
    }

    /// What the conversation sent to the model said, as `role: content`.
    fn said(request: &Value) -> Vec<String> {
        request["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|message| message["role"] != "system")
            .map(|message| {
                format!(
                    "{}: {}",
                    message["role"].as_str().unwrap_or_default(),
                    message["content"].as_str().unwrap_or("(tool call)")
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn a_reply_streams_and_is_recorded() {
        let server = serve(vec![(200, sse(&[text("hel"), text("lo")]))]).await;
        let fixture = fixture(&server);
        let id = fixture.start(Some("qwen3")).await;
        fixture.send(&id, "hi").await;

        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;

        let replies: Vec<_> = events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::AssistantDelta { text } => Some(format!("delta {text}")),
                EventKind::AssistantMessage { text } => Some(format!("message {text}")),
                _ => None,
            })
            .collect();
        assert_eq!(replies, ["delta hel", "delta lo", "message hello"]);
        let requests = server.requests();
        assert_eq!(requests[0]["model"], "qwen3");
        assert_eq!(said(&requests[0]), ["user: hi"]);
    }

    #[tokio::test]
    async fn an_approved_write_writes_the_file_and_the_model_sees_it() {
        let mut first = call(
            "c1",
            "write_file",
            json!({"path": "src/a.txt", "contents": "one"}),
        );
        first.insert(0, text("Writing it."));
        let server = serve(vec![(200, sse(&first)), (200, sse(&[text("Done.")]))]).await;
        let fixture = fixture(&server);
        let id = fixture.start(Some("qwen3")).await;
        fixture.send(&id, "write a").await;

        let events = wait_for(&fixture.agent, &id, waiting).await;
        let request = permission_request(&events).expect("it asked");
        fixture
            .agent
            .approve(&id, &request, true)
            .await
            .expect("approved");
        wait_for(&fixture.agent, &id, idle_turns(1)).await;

        let written = std::fs::read_to_string(fixture.work.path().join("src/a.txt"));
        assert_eq!(written.expect("the file exists"), "one");
        let requests = server.requests();
        assert_eq!(
            said(&requests[1]),
            [
                "user: write a",
                "assistant: Writing it.",
                "assistant: (tool call)",
                "tool: Wrote src/a.txt",
            ]
        );
    }

    #[tokio::test]
    async fn a_denied_shell_command_does_not_run() {
        let server = serve(vec![
            (
                200,
                sse(&call("c1", "shell", json!({"command": "touch ran"}))),
            ),
            (200, sse(&[text("Okay.")])),
        ])
        .await;
        let fixture = fixture(&server);
        let id = fixture.start(Some("qwen3")).await;
        fixture.send(&id, "run it").await;

        let events = wait_for(&fixture.agent, &id, waiting).await;
        let request = permission_request(&events).expect("it asked");
        fixture
            .agent
            .approve(&id, &request, false)
            .await
            .expect("answered");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;

        assert!(!fixture.work.path().join("ran").exists());
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, EventKind::ToolCompleted { success: false, .. }))
        );
    }

    #[tokio::test]
    async fn a_fork_asks_the_model_with_only_the_history_before_it() {
        let server = serve(vec![
            (200, sse(&[text("One.")])),
            (200, sse(&[text("Two.")])),
            (200, sse(&[text("Three.")])),
        ])
        .await;
        let fixture = fixture(&server);
        let id = fixture.start(Some("qwen3")).await;
        fixture.send(&id, "one").await;
        wait_for(&fixture.agent, &id, idle_turns(1)).await;
        fixture.send(&id, "two").await;
        let events = wait_for(&fixture.agent, &id, idle_turns(2)).await;
        let two = events
            .iter()
            .find(|event| event.kind == EventKind::UserMessage { text: "two".into() })
            .expect("the message is in the history");

        let options = ForkOptions {
            before: Some(two.id.clone()),
            ..ForkOptions::default()
        };
        let fork = fixture.agent.fork(&id, options).await.expect("forked");
        fixture.send(&fork.id, "three").await;
        wait_for(&fixture.agent, &fork.id, idle_turns(2)).await;

        let requests = server.requests();
        assert_eq!(
            said(&requests[2]),
            ["user: one", "assistant: One.", "user: three"]
        );
    }

    #[tokio::test]
    async fn a_session_without_a_model_fails_its_turn() {
        let server = serve(Vec::new()).await;
        let fixture = fixture(&server);
        let id = fixture.start(None).await;
        fixture.send(&id, "hi").await;

        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;

        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, EventKind::Failed { .. }))
        );
        assert!(server.requests().is_empty());
    }

    #[tokio::test]
    async fn a_missing_model_fails_the_turn_with_the_status() {
        let server = serve(vec![(404, r#"{"error":"model not found"}"#.into())]).await;
        let runner = LocalModelRunner::new(LocalModelConfig {
            base_url: Some(server.base_url.clone()),
            api_key: None,
        });
        let request = Request {
            url: format!("{}/chat/completions", server.base_url),
            api_key: None,
            model: "nope".into(),
        };
        let (parts, _received) = mpsc::channel(8);
        let http = runner.http.clone();
        let result = tokio::task::spawn_blocking(move || {
            stream(&http, &request.url, &request.model, None, "{}", &parts)
        })
        .await
        .expect("the thread finishes");

        assert!(matches!(
            result,
            Err(LocalModelError::Status { status: 404, .. })
        ));
    }

    #[tokio::test]
    async fn a_server_that_is_not_there_is_a_connect_error() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("listens");
        let port = listener.local_addr().expect("an address").port();
        drop(listener);
        let runner = LocalModelRunner::new(LocalModelConfig::default());
        let (parts, _received) = mpsc::channel(8);
        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let result = tokio::task::spawn_blocking(move || {
            stream(&runner.http, &url, "m", None, "{}", &parts)
        })
        .await
        .expect("the thread finishes");

        assert!(matches!(result, Err(LocalModelError::Connect { .. })));
    }

    #[test]
    fn paths_outside_the_folder_are_refused() {
        let root = Path::new("/work");
        assert_eq!(
            inside(root, Path::new("a/b")),
            Some(PathBuf::from("/work/a/b"))
        );
        assert_eq!(inside(root, Path::new("../a")), None);
        assert_eq!(inside(root, Path::new("/etc/passwd")), None);
        assert_eq!(inside(root, Path::new("")), None);
    }
}
