//! The local-model backend: a model behind an OpenAI-compatible server, such
//! as Ollama or llama.cpp's, so iterating costs no tokens. The model works in
//! the session's folder with four tools: `shell` and `write_file`, which
//! ask the person first, and `read_file` and `list_folder`.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::Model;
use crate::claude::McpServer;
use crate::env::Env;
use crate::error::AgentError;
use crate::event::{Event, EventKind, PermissionKind};
use crate::mcp_client::McpClient;
use crate::paths::inside;
use crate::runtime::{Runner, Turn, describe};

/// Ollama's address; llama.cpp's server and LM Studio need theirs set.
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434/v1";

/// How often a turn waiting for the app's model server asks again.
const SERVER_RETRY: Duration = Duration::from_secs(2);

/// Where the Humata app serves the Mac's models while it runs: a port on
/// 127.0.0.1, on the Mac or forwarded to a host over SSH. Kept in a file, so
/// sessions still find it, and wait for it, after humatad restarts while
/// the app is closed.
#[derive(Clone, Debug, Default)]
pub struct ModelServer {
    port: Arc<RwLock<Option<u16>>>,
    file: Option<PathBuf>,
}

impl ModelServer {
    /// The server named in `file`, if it names one.
    pub fn open(file: PathBuf) -> Self {
        let port = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| text.trim().parse().ok());
        Self {
            port: Arc::new(RwLock::new(port)),
            file: Some(file),
        }
    }

    pub fn port(&self) -> Option<u16> {
        *self.port.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Serves from `port` from now on, and keeps it for after a restart.
    pub fn set_port(&self, port: u16) -> std::io::Result<()> {
        *self.port.write().unwrap_or_else(PoisonError::into_inner) = Some(port);
        match &self.file {
            Some(file) => {
                if let Some(folder) = file.parent() {
                    std::fs::create_dir_all(folder)?;
                }
                std::fs::write(file, port.to_string())
            }
            None => Ok(()),
        }
    }

    fn base_url(&self) -> Option<String> {
        self.port()
            .map(|port| format!("http://127.0.0.1:{port}/v1"))
    }
}

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
        "Humata's model server at {url} isn't answering; open the Humata app on the Mac, which runs it, or set base_url in the session's backend config to another server"
    )]
    AppServerDown {
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
        "the model server at {url} answered {status} when asked for its models: {body}. Check base_url, and api_key if it needs one"
    )]
    ListStatus {
        url: String,
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
    /// The app's server, for sessions whose config and defaults name none.
    server: ModelServer,
    http: ureq::Agent,
    env: Env,
    /// Humata's tools, started for each turn and offered to the model.
    mcp: Option<McpServer>,
}

impl LocalModelRunner {
    /// `defaults` fills in what a session's own config leaves out.
    pub fn new(defaults: LocalModelConfig) -> Self {
        let http = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .build()
            .into();
        Self {
            defaults,
            server: ModelServer::default(),
            http,
            env: Env::default(),
            mcp: None,
        }
    }

    /// Uses the app's model server, when it has one, for sessions that name
    /// no other.
    #[must_use]
    pub fn with_server(mut self, server: ModelServer) -> Self {
        self.server = server;
        self
    }

    /// The server a session uses: the one its config names, this runner's
    /// default, the app's, or Ollama's; and whether it's the app's.
    fn base_url(&self, own: Option<String>) -> (String, bool) {
        if let Some(url) = own.or_else(|| self.defaults.base_url.clone()) {
            return (url, false);
        }
        match self.server.base_url() {
            Some(url) => (url, true),
            None => (DEFAULT_BASE_URL.to_owned(), false),
        }
    }

    /// Sets `env` for the commands its sessions run.
    #[must_use]
    pub fn with_env(mut self, env: Env) -> Self {
        self.env = env;
        self
    }

    pub fn with_mcp(mut self, mcp: Option<McpServer>) -> Self {
        self.mcp = mcp;
        self
    }
}

impl Runner for LocalModelRunner {
    type Error = LocalModelError;

    async fn models(&self, config: Option<String>) -> Result<Vec<Model>, AgentError> {
        let own = match config {
            Some(json) => serde_json::from_str::<LocalModelConfig>(&json)
                .map_err(|source| LocalModelError::BadConfig { source })?,
            None => LocalModelConfig::default(),
        };
        let (base_url, apps) = self.base_url(own.base_url);
        let api_key = own.api_key.or_else(|| self.defaults.api_key.clone());
        match server_models(self.http.clone(), &base_url, api_key).await {
            Err(LocalModelError::Connect { url, source }) if apps => {
                Err(LocalModelError::AppServerDown { url, source }.into())
            }
            listed => Ok(listed?),
        }
    }

    async fn run_turn(self: Arc<Self>, turn: Turn) -> Result<(), LocalModelError> {
        let own = match &turn.record().backend_config {
            Some(json) => serde_json::from_str::<LocalModelConfig>(json)
                .map_err(|source| LocalModelError::BadConfig { source })?,
            None => LocalModelConfig::default(),
        };
        let (base_url, apps) = self.base_url(own.base_url);
        let request = Request {
            url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
            api_key: own.api_key.or_else(|| self.defaults.api_key.clone()),
            model: turn
                .record()
                .model
                .clone()
                .ok_or(LocalModelError::NoModel)?,
        };

        // Without Humata's tools the turn still has the files to work on.
        let mut offered = tools();
        let mut humata = match &self.mcp {
            None => None,
            Some(server) => {
                match McpClient::start(server, turn.record().id.as_str(), &self.env).await {
                    Ok((client, listed)) => {
                        if let Some(offered) = offered.as_array_mut() {
                            offered.extend(listed);
                        }
                        Some(client)
                    }
                    Err(failure) => {
                        eprintln!(
                            "humatad: the local model works without Humata's tools this turn: {}",
                            describe(&failure)
                        );
                        None
                    }
                }
            }
        };

        let mut calls = 0_usize;
        for _ in 0..MAX_STEPS {
            let reply = self.reply(&turn, &request, &offered, apps).await?;
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
                let (success, output) = match humata.as_mut() {
                    Some(humata) if humata.owns(&call.name) => humata
                        .call(&call.name, &call.arguments)
                        .await
                        .unwrap_or_else(|failure| (false, describe(&failure))),
                    _ => run_tool(&turn, &self.env, &call.name, &call.arguments).await?,
                };
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
    /// The model's next reply. While the app's server is down, the turn
    /// says so and waits for it, until it's back or the turn is stopped.
    async fn reply(
        &self,
        turn: &Turn,
        request: &Request,
        tools: &Value,
        apps: bool,
    ) -> Result<Reply, LocalModelError> {
        let mut waiting = false;
        loop {
            // Asked afresh each time, so steering sent while it waited is in.
            let messages = messages(turn.record().instructions.as_deref(), &turn.history());
            match self.complete(turn, request, tools, messages).await {
                Err(LocalModelError::Connect { .. }) if apps => {
                    if !waiting {
                        waiting = true;
                        turn.emit(EventKind::Waiting {
                            reason: "Humata's model server isn't running. Open the Humata app on the Mac; the turn goes on when it's back.".to_owned(),
                        })?;
                    }
                    tokio::time::sleep(SERVER_RETRY).await;
                }
                reply => return reply,
            }
        }
    }

    /// Streams one reply, recording its text and reasoning as they come.
    async fn complete(
        &self,
        turn: &Turn,
        request: &Request,
        tools: &Value,
        messages: Vec<Value>,
    ) -> Result<Reply, LocalModelError> {
        let body = json!({
            "model": request.model,
            "messages": messages,
            "tools": tools,
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

/// The models an OpenAI-compatible server at `base_url` has, by its
/// `/models`.
pub(crate) async fn server_models(
    http: ureq::Agent,
    base_url: &str,
    api_key: Option<String>,
) -> Result<Vec<Model>, LocalModelError> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    // ureq blocks, so it's asked on its own thread; one that panicked lists
    // nothing.
    tokio::task::spawn_blocking(move || list_models(&http, &url, api_key.as_deref()))
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
}

fn list_models(
    http: &ureq::Agent,
    url: &str,
    api_key: Option<&str>,
) -> Result<Vec<Model>, LocalModelError> {
    let mut request = http.get(url).header("Accept", "application/json");
    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {key}"));
    }
    let response = request.call().map_err(|source| LocalModelError::Connect {
        url: url.to_owned(),
        source: Box::new(source),
    })?;
    let status = response.status().as_u16();
    let mut text = String::new();
    response
        .into_body()
        .into_reader()
        .take(4 * 1024 * 1024)
        .read_to_string(&mut text)
        .map_err(|source| LocalModelError::Read {
            url: url.to_owned(),
            source,
        })?;
    if status != 200 {
        text.truncate(4096);
        return Err(LocalModelError::ListStatus {
            url: url.to_owned(),
            status,
            body: text.trim().to_owned(),
        });
    }
    let listed: Value = serde_json::from_str(&text).map_err(|source| LocalModelError::Decode {
        url: url.to_owned(),
        source,
    })?;
    let mut models: Vec<Model> = listed["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| model["id"].as_str())
        .map(|id| Model {
            id: id.to_owned(),
            name: id.to_owned(),
            detail: None,
        })
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
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
            "Returns a file's contents, or a folder's list.",
            json!({"type": "object", "properties": {"path": path}, "required": ["path"]}),
        ),
        tool(
            "list_folder",
            "Lists what a folder holds, folders ending in /; the project folder when no path is given.",
            json!({"type": "object", "properties": {"path": path}}),
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
struct ListArguments {
    #[serde(default)]
    path: PathBuf,
}

#[derive(Deserialize)]
struct WriteArguments {
    path: PathBuf,
    contents: String,
}

/// Runs a tool the model called. What goes wrong is told to the model, as
/// the tool's failed output, so it can try another way.
async fn run_tool(
    turn: &Turn,
    env: &Env,
    name: &str,
    arguments: &str,
) -> Result<(bool, String), AgentError> {
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
                    shell(&root, env, &command).await
                } else {
                    denied()
                }
            }
        },
        "read_file" => match serde_json::from_str::<ReadArguments>(arguments) {
            Err(error) => bad(error),
            Ok(ReadArguments { path }) => match inside(&root, &path) {
                None => outside(&path),
                Some(target) if target.is_dir() => list(&target, &path).await,
                Some(target) => match tokio::fs::read_to_string(&target).await {
                    Ok(contents) => (true, truncated(contents)),
                    Err(error) => (false, format!("Couldn't read {}: {error}", path.display())),
                },
            },
        },
        "list_folder" => match serde_json::from_str::<ListArguments>(arguments) {
            Err(error) => bad(error),
            Ok(ListArguments { path }) if path.as_os_str().is_empty() || path == Path::new(".") => {
                list(&root, &path).await
            }
            Ok(ListArguments { path }) => match inside(&root, &path) {
                None => outside(&path),
                Some(target) => list(&target, &path).await,
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

async fn shell(root: &Path, env: &Env, command: &str) -> (bool, String) {
    let child = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .envs(env.vars())
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

/// The names in a folder, sorted, folders ending in `/`; Git's own folder
/// is left out.
async fn list(target: &Path, path: &Path) -> (bool, String) {
    let couldnt =
        |error: std::io::Error| (false, format!("Couldn't list {}: {error}", path.display()));
    let mut entries = match tokio::fs::read_dir(target).await {
        Ok(entries) => entries,
        Err(error) => return couldnt(error),
    };
    let mut names = Vec::new();
    loop {
        match entries.next_entry().await {
            Ok(Some(entry)) => {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == ".git" {
                    continue;
                }
                let folder = entry.file_type().await.is_ok_and(|kind| kind.is_dir());
                names.push(if folder { format!("{name}/") } else { name });
            }
            Ok(None) => break,
            Err(error) => return couldnt(error),
        }
    }
    if names.is_empty() {
        return (true, "(empty)".into());
    }
    names.sort();
    (true, truncated(names.join("\n")))
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
        serve_on(listener, replies)
    }

    fn serve_on(listener: TcpListener, replies: Vec<(u16, String)>) -> FakeServer {
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
    async fn the_model_lists_a_folder_without_asking() {
        let server = serve(vec![
            (200, sse(&call("c1", "list_folder", json!({})))),
            (200, sse(&call("c2", "read_file", json!({"path": "src"})))),
            (200, sse(&[text("Seen.")])),
        ])
        .await;
        let fixture = fixture(&server);
        std::fs::create_dir_all(fixture.work.path().join("src")).expect("a folder");
        std::fs::write(fixture.work.path().join("src/a.txt"), "one").expect("a file");
        std::fs::write(fixture.work.path().join("b.txt"), "two").expect("a file");
        let id = fixture.start(Some("qwen3")).await;
        fixture.send(&id, "look around").await;

        wait_for(&fixture.agent, &id, idle_turns(1)).await;

        let requests = server.requests();
        let tools: Vec<_> = said(&requests[2])
            .into_iter()
            .filter(|said| said.starts_with("tool: "))
            .collect();
        assert_eq!(tools, ["tool: b.txt\nsrc/", "tool: a.txt"]);
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

    /// A port nothing listens on, for now.
    fn free_port() -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("listens");
        listener.local_addr().expect("an address").port()
    }

    #[tokio::test]
    async fn a_turn_waits_for_the_apps_server_and_goes_on_when_it_starts() {
        let port = free_port();
        let state = tempfile::tempdir().expect("a temporary folder");
        let work = tempfile::tempdir().expect("a temporary folder");
        let server = ModelServer::open(state.path().join("model-server"));
        server.set_port(port).expect("the port is kept");
        let runner = LocalModelRunner::new(LocalModelConfig::default()).with_server(server);
        let fixture = Fixture {
            agent: LocalAgent::open(state.path().join("local"), runner).expect("the agent opens"),
            work,
            _state: state,
        };
        let id = fixture.start(Some("apple")).await;
        fixture.send(&id, "hi").await;

        let is_waiting = |events: &[Event]| {
            events
                .iter()
                .any(|event| matches!(event.kind, EventKind::Waiting { .. }))
        };
        wait_for(&fixture.agent, &id, is_waiting).await;

        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .expect("listens");
        let app = serve_on(listener, vec![(200, sse(&[text("back")]))]);
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;

        assert_eq!(
            events
                .iter()
                .filter(|event| is_waiting(std::slice::from_ref(event)))
                .count(),
            1
        );
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            EventKind::AssistantMessage { text } if text == "back"
        )));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.kind, EventKind::Failed { .. }))
        );
        assert_eq!(app.requests()[0]["model"], "apple");
    }

    #[test]
    fn the_apps_server_is_kept_for_after_a_restart() {
        let state = tempfile::tempdir().expect("a temporary folder");
        let file = state.path().join("model-server");
        assert_eq!(ModelServer::open(file.clone()).port(), None);
        ModelServer::open(file.clone())
            .set_port(47830)
            .expect("the port is kept");
        assert_eq!(ModelServer::open(file).port(), Some(47830));
    }

    #[test]
    fn a_server_the_session_names_comes_before_the_apps() {
        let server = ModelServer::default();
        server.set_port(47830).expect("the port is kept");
        let runner = LocalModelRunner::new(LocalModelConfig::default()).with_server(server);
        assert_eq!(
            runner.base_url(None),
            ("http://127.0.0.1:47830/v1".to_owned(), true)
        );
        let own = "http://gpu:11434/v1".to_owned();
        assert_eq!(runner.base_url(Some(own.clone())), (own, false));
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
