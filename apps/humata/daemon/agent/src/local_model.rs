//! The local-model backend: a model behind an OpenAI-compatible server, such
//! as Ollama or llama.cpp's, so iterating costs no tokens. The model works in
//! the session's folder with its tools: `shell`, `write_file` and
//! `edit_file`, which ask the person first, and `read_file`, `list_folder`,
//! `search_files` and `find_files`, which don't.

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
        "the chat no longer fits in the context of {model} (the server at {url} said: {body}). Fork the chat from a later message, start a new session, or load the model with a longer context"
    )]
    ContextFull {
        url: String,
        model: String,
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

        // Sized once a turn, so what's left out moves only a step at a time.
        let context = server_context(
            self.http.clone(),
            &base_url,
            request.api_key.clone(),
            request.model.clone(),
        )
        .await;
        let budget = budget(context, &offered);

        let mut calls = 0_usize;
        for _ in 0..MAX_STEPS {
            let reply = self.reply(&turn, &request, &offered, budget, apps).await?;
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
                        .map(|(success, output)| (success, truncated(output)))
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
        budget: usize,
        apps: bool,
    ) -> Result<Reply, LocalModelError> {
        let mut waiting = false;
        loop {
            // Asked afresh each time, so steering sent while it waited is in.
            let messages = messages(
                turn.record().instructions.as_deref(),
                &turn.history(),
                Some(&turn.images_folder()),
                budget,
            );
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
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
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
    tokio::task::spawn_blocking(move || {
        listing(&http, &url, api_key.as_deref()).map(|listed| list_models(&listed))
    })
    .await
    .unwrap_or_else(|_| Ok(Vec::new()))
}

/// How many tokens `model` takes in, if the server at `base_url` lists it:
/// llama.cpp, vLLM, LM Studio and others each name it differently. None
/// when the server is down or doesn't say.
async fn server_context(
    http: ureq::Agent,
    base_url: &str,
    api_key: Option<String>,
    model: String,
) -> Option<u64> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    tokio::task::spawn_blocking(move || {
        listing(&http, &url, api_key.as_deref())
            .ok()
            .and_then(|listed| listed_context(&listed, &model))
    })
    .await
    .ok()
    .flatten()
}

fn listed_context(listed: &Value, model: &str) -> Option<u64> {
    let entry = listed["data"]
        .as_array()?
        .iter()
        .find(|entry| entry["id"] == model)?;
    [
        &entry["context_length"],
        &entry["max_context_length"],
        &entry["max_model_len"],
        &entry["context_window"],
        &entry["meta"]["n_ctx"],
        &entry["meta"]["n_ctx_train"],
    ]
    .into_iter()
    .find_map(Value::as_u64)
    .filter(|tokens| *tokens > 0)
}

/// The server's `/models` answer.
fn listing(http: &ureq::Agent, url: &str, api_key: Option<&str>) -> Result<Value, LocalModelError> {
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
    serde_json::from_str(&text).map_err(|source| LocalModelError::Decode {
        url: url.to_owned(),
        source,
    })
}

/// The models in a `/models` answer, by id.
fn list_models(listed: &Value) -> Vec<Model> {
    let mut models: Vec<Model> = listed["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let id = model["id"].as_str()?;
            // Humata's own server says, for one, how well it calls tools.
            let detail = model["detail"]
                .as_str()
                .map(str::trim)
                .filter(|detail| !detail.is_empty());
            Some(Model {
                id: id.to_owned(),
                name: id.to_owned(),
                detail: detail.map(str::to_owned),
                // Humata's own server lists what each model takes besides text.
                reads_images: model["inputs"]
                    .as_array()
                    .is_some_and(|inputs| inputs.iter().any(|input| input == "image")),
                efforts: Vec::new(),
                default_effort: None,
            })
        })
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models
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
        if context_full(status, &text) {
            return Err(LocalModelError::ContextFull {
                url: url.to_owned(),
                model: model.to_owned(),
                body: text.trim().to_owned(),
            });
        }
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

/// A user message's content: its text alone, or with its images first as
/// image_url parts. An image gone from the folder is said to be missing.
fn user_content(text: &str, names: &[String], folder: Option<&Path>) -> Value {
    let Some(folder) = folder.filter(|_| !names.is_empty()) else {
        return json!(text);
    };
    let mut parts: Vec<Value> = names
        .iter()
        .map(|name| {
            let path = folder.join(name);
            match std::fs::read(&path) {
                Ok(data) => json!({
                    "type": "image_url",
                    "image_url": {"url": format!(
                        "data:{};base64,{}",
                        crate::images::media_type(&path),
                        crate::images::base64(&data),
                    )},
                }),
                Err(_) => json!({"type": "text", "text": "[an image sent here is missing]"}),
            }
        })
        .collect();
    if !text.is_empty() {
        parts.push(json!({"type": "text", "text": text}));
    }
    Value::Array(parts)
}

/// The conversation so far as chat messages: the instructions, then each
/// message, reply and tool call in the history. A reply's text and the
/// calls it made go in one message, each call followed by its result, and
/// a message the person sent while calls ran comes after their results.
/// The images of the latest message that has any, read from `images`, go
/// with it as data URLs; an earlier message's are only noted, since
/// sending every image again each turn costs tokens and can make the
/// request too large. Once what's sent outgrows `budget` bytes, the oldest
/// tool calls' long arguments, such as a write's contents, and their
/// output are left out, a step at a time so what's sent stays the same
/// between steps and the server can reuse what it read before.
fn messages(
    instructions: Option<&str>,
    history: &[Event],
    images: Option<&Path>,
    budget: usize,
) -> Vec<Value> {
    let mut system = String::from(
        "You are a coding agent working in the person's project folder. Use the tools to read, change and run things there, then say briefly what you did.",
    );
    if let Some(instructions) = instructions {
        system.push_str("\n\n");
        system.push_str(instructions);
    }
    let latest = history.iter().rposition(
        |event| matches!(&event.kind, EventKind::UserMessage { images, .. } if !images.is_empty()),
    );
    // Each call's arguments with their long strings left out, where any are.
    let shortened: Vec<Option<String>> = history
        .iter()
        .map(|event| match &event.kind {
            EventKind::ToolStarted { arguments_json, .. } => shortened(arguments_json),
            _ => None,
        })
        .collect();
    let dropped = left_out(system.len(), history, budget);
    let mut messages = vec![json!({"role": "system", "content": system})];
    // How much leaving out the calls so far saves, oldest first.
    let mut seen = 0_usize;
    let mut reply = Said::default();
    // Sent while the reply's calls ran, so after their results.
    let mut waiting: Vec<Value> = Vec::new();
    for (index, event) in history.iter().enumerate() {
        match &event.kind {
            EventKind::UserMessage {
                text,
                images: names,
            } => {
                let content = if Some(index) == latest || names.is_empty() {
                    user_content(text, names, images)
                } else {
                    let count = names.len();
                    let noun = if count == 1 { "image" } else { "images" };
                    json!(format!("[{count} {noun} sent here earlier]\n{text}"))
                };
                waiting.push(json!({"role": "user", "content": content}));
                if reply.calls.is_empty() {
                    reply.end(&mut messages, &mut waiting);
                }
            }
            EventKind::AssistantMessage { text } => {
                reply.end(&mut messages, &mut waiting);
                reply.text = Some(text.clone());
            }
            // A new reply, or a new turn, starts here.
            EventKind::Usage { .. } | EventKind::TurnStarted | EventKind::TurnEnded { .. } => {
                reply.end(&mut messages, &mut waiting);
            }
            EventKind::ToolStarted {
                call_id,
                tool,
                arguments_json,
            } => {
                let arguments = match &shortened[index] {
                    Some(short) => {
                        seen += arguments_json.len().saturating_sub(short.len());
                        if seen <= dropped {
                            short
                        } else {
                            arguments_json
                        }
                    }
                    None => arguments_json,
                };
                reply.calls.push((
                    json!({
                        "id": call_id,
                        "type": "function",
                        "function": {"name": tool, "arguments": arguments},
                    }),
                    None,
                ));
            }
            EventKind::ToolCompleted {
                call_id, output, ..
            } => {
                seen += output.len();
                let content = if seen <= dropped {
                    DROPPED.to_owned()
                } else {
                    output.clone()
                };
                if let Some((_, result)) = reply
                    .calls
                    .iter_mut()
                    .find(|(call, result)| result.is_none() && call["id"] == call_id.as_str())
                {
                    *result = Some(content);
                }
            }
            _ => {}
        }
    }
    reply.end(&mut messages, &mut waiting);
    messages
}

/// About how many bytes of text a token is; few, so a budget sized from
/// tokens errs small.
const BYTES_PER_TOKEN: usize = 3;
/// What's sent of the conversation when the server doesn't say how much
/// the model takes in.
const UNKNOWN_CONTEXT_SENT: usize = 64 * 1024;
/// The least that's sent, however small the model's context, so a step
/// still leaves room for a turn's work.
const LEAST_SENT: usize = 8 * 1024;

/// How many bytes of the conversation are sent with each request: three
/// quarters of a model's context of `context_tokens`, leaving the rest for
/// its reply, less what describing `tools` takes, since that goes with
/// every request too.
fn budget(context_tokens: Option<u64>, tools: &Value) -> usize {
    let whole = match context_tokens.and_then(|tokens| usize::try_from(tokens).ok()) {
        Some(tokens) => tokens.saturating_mul(BYTES_PER_TOKEN) / 4 * 3,
        None => UNKNOWN_CONTEXT_SENT,
    };
    whole
        .saturating_sub(tools.to_string().len())
        .max(LEAST_SENT)
}

/// How many bytes of the oldest tool calls' arguments and output are left
/// out to keep what's sent within `budget`: everything counts, the
/// instructions (`system` bytes) and every message as well, but only tool
/// calls are left out. None until there's more than the budget, then whole
/// steps of half of it, so the point moves only when another step's worth
/// has come.
fn left_out(system: usize, history: &[Event], budget: usize) -> usize {
    let sent: usize = system
        + history
            .iter()
            .map(|event| match &event.kind {
                EventKind::UserMessage { text, .. } | EventKind::AssistantMessage { text } => {
                    text.len()
                }
                EventKind::ToolStarted { arguments_json, .. } => arguments_json.len(),
                EventKind::ToolCompleted { output, .. } => output.len(),
                _ => 0,
            })
            .sum::<usize>();
    let step = (budget / 2).max(1);
    match sent.checked_sub(budget) {
        None | Some(0) => 0,
        Some(over) => over.div_ceil(step) * step,
    }
}

/// A string argument longer than this is left out of an old call.
const ARGUMENT_KEPT: usize = 200;

/// A call's arguments with each string longer than [`ARGUMENT_KEPT`], such
/// as a write's contents or an edit's texts, replaced by a note of its
/// size; None when there's none to leave out or they aren't JSON.
fn shortened(arguments: &str) -> Option<String> {
    fn shorten(value: &mut Value) -> bool {
        match value {
            Value::String(text) if text.len() > ARGUMENT_KEPT => {
                *text = format!("({} bytes, left out to save room)", text.len());
                true
            }
            Value::Array(values) => values
                .iter_mut()
                .fold(false, |cut, value| shorten(value) || cut),
            Value::Object(fields) => fields
                .values_mut()
                .fold(false, |cut, value| shorten(value) || cut),
            _ => false,
        }
    }
    if arguments.len() <= ARGUMENT_KEPT {
        return None;
    }
    let mut parsed: Value = serde_json::from_str(arguments).ok()?;
    shorten(&mut parsed).then(|| parsed.to_string())
}

/// What an old tool result is replaced with once it's left out.
const DROPPED: &str =
    "(This output was left out to save room. Run the tool again if it's still needed.)";

/// A reply being put together from the history: its text and its calls,
/// each with its result once that's in.
#[derive(Default)]
struct Said {
    text: Option<String>,
    calls: Vec<(Value, Option<String>)>,
}

impl Said {
    /// Adds the reply, if there is one, and then the messages that waited
    /// for it. A call that never got a result, as when a restart cut it
    /// off, is given one: servers refuse a call without its result.
    fn end(&mut self, messages: &mut Vec<Value>, waiting: &mut Vec<Value>) {
        let Said { text, calls } = std::mem::take(self);
        if !calls.is_empty() {
            let tool_calls: Vec<&Value> = calls.iter().map(|(call, _)| call).collect();
            messages.push(json!({"role": "assistant", "content": text, "tool_calls": tool_calls}));
            for (call, result) in &calls {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call["id"],
                    "content": result.as_deref().unwrap_or(INTERRUPTED),
                }));
            }
        } else if let Some(text) = text {
            messages.push(json!({"role": "assistant", "content": text}));
        }
        messages.append(waiting);
    }
}

/// The result given a tool call that was cut off before it finished.
const INTERRUPTED: &str = "This tool call was interrupted before it finished, so its result is unknown. Check what it was meant to change before relying on it.";

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
            "Returns a file's lines, or a folder's list. A long file comes a part at a time: give offset, the first line to read (from 1), and limit, how many.",
            json!({"type": "object", "properties": {
                "path": path,
                "offset": {"type": "integer", "minimum": 1},
                "limit": {"type": "integer", "minimum": 1},
            }, "required": ["path"]}),
        ),
        tool(
            "list_folder",
            "Lists what a folder holds, folders ending in /; the project folder when no path is given.",
            json!({"type": "object", "properties": {"path": path}}),
        ),
        tool(
            "search_files",
            "Finds the lines that hold text, any case, in the files under a folder (the project folder when no path is given), as path:line: text. Asks nobody, so use it rather than grep in shell.",
            json!({"type": "object", "properties": {"text": {"type": "string"}, "path": path}, "required": ["text"]}),
        ),
        tool(
            "find_files",
            "Lists the files and folders under a folder (the project folder when no path is given) whose path holds name, any case. Asks nobody, so use it rather than find in shell.",
            json!({"type": "object", "properties": {"name": {"type": "string"}, "path": path}, "required": ["name"]}),
        ),
        tool(
            "write_file",
            "Writes a file, replacing what it held. To change part of a file, use edit_file.",
            json!({"type": "object", "properties": {"path": path, "contents": {"type": "string"}}, "required": ["path", "contents"]}),
        ),
        tool(
            "edit_file",
            "Replaces old_text in a file with new_text. old_text must be there exactly once, unless every is true, which replaces each.",
            json!({"type": "object", "properties": {
                "path": path,
                "old_text": {"type": "string"},
                "new_text": {"type": "string"},
                "every": {"type": "boolean"},
            }, "required": ["path", "old_text", "new_text"]}),
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
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct EditArguments {
    path: PathBuf,
    old_text: String,
    new_text: String,
    #[serde(default)]
    every: bool,
}

#[derive(Deserialize)]
struct ListArguments {
    #[serde(default)]
    path: PathBuf,
}

#[derive(Deserialize)]
struct SearchArguments {
    text: String,
    #[serde(default)]
    path: PathBuf,
}

#[derive(Deserialize)]
struct FindArguments {
    name: String,
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
            Ok(ReadArguments {
                path,
                offset,
                limit,
            }) => match inside(&root, &path) {
                None => outside(&path),
                Some(target) if target.is_dir() => list(&target, &path).await,
                Some(target) => match tokio::fs::read_to_string(&target).await {
                    Ok(contents) => (true, lines(&contents, offset, limit)),
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
        "search_files" => match serde_json::from_str::<SearchArguments>(arguments) {
            Err(error) => bad(error),
            Ok(SearchArguments { text, .. }) if text.is_empty() => {
                (false, "Give text to search for.".to_owned())
            }
            Ok(SearchArguments { text, path }) => match folder(&root, &path) {
                None => outside(&path),
                Some(folder) => looked(move || search_files(&root, &folder, &text)).await,
            },
        },
        "find_files" => match serde_json::from_str::<FindArguments>(arguments) {
            Err(error) => bad(error),
            Ok(FindArguments { name, .. }) if name.is_empty() => {
                (false, "Give name, part of the paths to find.".to_owned())
            }
            Ok(FindArguments { name, path }) => match folder(&root, &path) {
                None => outside(&path),
                Some(folder) => looked(move || find_files(&root, &folder, &name)).await,
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
        "edit_file" => match serde_json::from_str::<EditArguments>(arguments) {
            Err(error) => bad(error),
            Ok(edit) => match inside(&root, &edit.path) {
                None => outside(&edit.path),
                Some(target) => {
                    if turn
                        .ask(
                            PermissionKind::Write,
                            format!("Edit {}", edit.path.display()),
                        )
                        .await?
                    {
                        edited(&target, &edit).await
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
    let spawned = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .envs(env.vars())
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => return (false, format!("Couldn't run it: {error}")),
    };
    let mut printed = Capped::default();
    let run = async {
        let (mut out, mut err) = (child.stdout.take(), child.stderr.take());
        let (mut a, mut b) = ([0u8; 8192], [0u8; 8192]);
        while out.is_some() || err.is_some() {
            tokio::select! {
                read = read_some(&mut out, &mut a) => match read {
                    Some(n) => printed.push(&a[..n]),
                    None => out = None,
                },
                read = read_some(&mut err, &mut b) => match read {
                    Some(n) => printed.push(&b[..n]),
                    None => err = None,
                },
            }
        }
        child.wait().await
    };
    let finished = tokio::time::timeout(SHELL_TIMEOUT, run).await;
    match finished {
        Err(_) => {
            let _ = child.start_kill();
            let mut text = printed.text();
            text.push_str(&format!(
                "\n(stopped after {} seconds without finishing; this is what it printed)",
                SHELL_TIMEOUT.as_secs()
            ));
            (false, text)
        }
        Ok(Err(error)) => (false, format!("Couldn't run it: {error}")),
        Ok(Ok(status)) => {
            let mut text = printed.text();
            if let Some(code) = status.code().filter(|code| *code != 0) {
                text.push_str(&format!("\n(exit status {code})"));
            }
            (status.success(), text)
        }
    }
}

/// Up to `buffer`'s length from `reader`; None at its end, or with none.
async fn read_some<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut Option<R>,
    buffer: &mut [u8],
) -> Option<usize> {
    use tokio::io::AsyncReadExt as _;
    match reader {
        Some(reader) => match reader.read(buffer).await {
            Ok(0) | Err(_) => None,
            Ok(read) => Some(read),
        },
        None => std::future::pending().await,
    }
}

/// What a command printed, as it prints it: its start and its end, so a
/// noisy one can't take the daemon's memory, and the end, where errors
/// usually are, is always there.
#[derive(Default)]
struct Capped {
    head: Vec<u8>,
    tail: std::collections::VecDeque<u8>,
    cut: usize,
}

impl Capped {
    const HEAD: usize = MAX_OUTPUT / 4;
    const TAIL: usize = MAX_OUTPUT - Self::HEAD;

    fn push(&mut self, mut bytes: &[u8]) {
        if self.head.len() < Self::HEAD {
            let taken = (Self::HEAD - self.head.len()).min(bytes.len());
            self.head.extend_from_slice(&bytes[..taken]);
            bytes = &bytes[taken..];
        }
        self.tail.extend(bytes);
        if self.tail.len() > Self::TAIL {
            let over = self.tail.len() - Self::TAIL;
            self.tail.drain(..over);
            self.cut += over;
        }
    }

    fn text(self) -> String {
        let mut text = String::from_utf8_lossy(&self.head).into_owned();
        if self.cut > 0 {
            text.push_str(&format!("\n(… {} bytes cut here …)\n", self.cut));
        }
        let tail: Vec<u8> = self.tail.into();
        text.push_str(&String::from_utf8_lossy(&tail));
        text
    }
}

/// The lines asked for, or as many from the start as fit, saying which
/// they are and where to go on when there are more.
fn lines(contents: &str, offset: Option<usize>, limit: Option<usize>) -> String {
    let all: Vec<&str> = contents.lines().collect();
    let first = offset.unwrap_or(1).max(1);
    if first > all.len().max(1) {
        return format!(
            "The file has {} lines, so none start at {first}.",
            all.len()
        );
    }
    let mut text = String::new();
    let mut last = first - 1;
    for line in all.iter().skip(first - 1).take(limit.unwrap_or(usize::MAX)) {
        if text.len() + line.len() + 1 > MAX_OUTPUT && last >= first {
            break;
        }
        text.push_str(line);
        text.push('\n');
        last += 1;
    }
    if first > 1 || last < all.len() {
        text.push_str(&format!("(lines {first}–{last} of {}", all.len()));
        if last < all.len() {
            text.push_str(&format!("; offset {} goes on", last + 1));
        }
        text.push(')');
    }
    truncated(text)
}

/// Replaces the text an edit names: once when it's there once, or each
/// time when asked to; otherwise says why it didn't.
async fn edited(target: &Path, edit: &EditArguments) -> (bool, String) {
    let path = edit.path.display();
    let contents = match tokio::fs::read_to_string(target).await {
        Ok(contents) => contents,
        Err(error) => return (false, format!("Couldn't read {path}: {error}")),
    };
    if edit.old_text.is_empty() {
        return (false, "old_text is empty; give the text to replace.".into());
    }
    let count = contents.matches(edit.old_text.as_str()).count();
    if count == 0 {
        return (
            false,
            format!("{path} doesn't hold old_text; read it again and copy the text exactly."),
        );
    }
    if count > 1 && !edit.every {
        return (
            false,
            format!(
                "{path} holds old_text {count} times; give more of the text around it, or set every to replace each."
            ),
        );
    }
    let changed = contents.replace(edit.old_text.as_str(), &edit.new_text);
    match tokio::fs::write(target, changed).await {
        Ok(()) => (true, format!("Edited {path} ({count} replaced)")),
        Err(error) => (false, format!("Couldn't write {path}: {error}")),
    }
}

/// Whether a server refused a request for being longer than the model's
/// context, as Ollama, llama.cpp, LM Studio, MLX and vLLM say it.
fn context_full(status: u16, body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    matches!(status, 400 | 413 | 422 | 500)
        && (body.contains("context")
            || body.contains("too long")
            || body.contains("too many tokens"))
        && (body.contains("exceed")
            || body.contains("length")
            || body.contains("window")
            || body.contains("too long")
            || body.contains("too many"))
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

/// The folder a search looks under: the project folder when `path` is
/// empty or `.`, else `path` in it.
fn folder(root: &Path, path: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() || path == Path::new(".") {
        Some(root.to_owned())
    } else {
        inside(root, path)
    }
}

/// Runs a search off the executor, since it reads many files.
async fn looked(search: impl FnOnce() -> (bool, String) + Send + 'static) -> (bool, String) {
    tokio::task::spawn_blocking(search)
        .await
        .unwrap_or_else(|_| (false, "The search failed; try again.".to_owned()))
}

/// The most a search_files or find_files call answers with.
const MOST_FOUND: usize = 100;
/// The most files a search looks at, so one over a huge folder ends.
const MOST_LOOKED_AT: usize = 20_000;
/// Files larger than this aren't searched: they're seldom source.
const LARGEST_SEARCHED: u64 = 1024 * 1024;
/// Folders a search doesn't go into: version control, dependencies and
/// build output, which are large and seldom what's meant.
const NOT_SEARCHED: [&str; 7] = [
    ".git",
    "node_modules",
    "target",
    ".build",
    "DerivedData",
    "__pycache__",
    ".venv",
];

/// Calls `each` with every file and folder under `folder`, folders before
/// what they hold, not following links, until it returns false or
/// [`MOST_LOOKED_AT`] have been seen. Says whether it saw them all.
fn walk(folder: &Path, each: &mut dyn FnMut(&Path, bool) -> bool) -> bool {
    let mut looked_at = 0_usize;
    let mut folders = vec![folder.to_owned()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        let mut inner = Vec::new();
        for entry in entries {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let path = entry.path();
            if kind.is_dir() {
                if NOT_SEARCHED.contains(&name.as_ref()) || name.starts_with("bazel-") {
                    continue;
                }
                inner.push(path.clone());
            } else if !kind.is_file() {
                continue;
            }
            looked_at += 1;
            if looked_at > MOST_LOOKED_AT || !each(&path, kind.is_dir()) {
                return false;
            }
        }
        // Popped from the end, so reversed to go in name order.
        folders.extend(inner.into_iter().rev());
    }
    true
}

/// `path` as the model gives it to the other tools: relative to the
/// project folder.
fn shown(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// The lines under `folder` that hold `text`, in any case.
fn search_files(root: &Path, folder: &Path, text: &str) -> (bool, String) {
    let wanted = text.to_lowercase();
    let mut found = Vec::new();
    let all = walk(folder, &mut |path, is_folder| {
        if is_folder || std::fs::metadata(path).map_or(true, |data| data.len() > LARGEST_SEARCHED) {
            return true;
        }
        let Ok(bytes) = std::fs::read(path) else {
            return true;
        };
        // A NUL early on means it isn't text.
        if bytes.iter().take(8192).any(|byte| *byte == 0) {
            return true;
        }
        let contents = String::from_utf8_lossy(&bytes);
        for (number, line) in contents.lines().enumerate() {
            if line.to_lowercase().contains(&wanted) {
                let line = line.trim();
                let mut end = line.len().min(200);
                while !line.is_char_boundary(end) {
                    end -= 1;
                }
                found.push(format!(
                    "{}:{}: {}",
                    shown(root, path),
                    number + 1,
                    &line[..end]
                ));
                if found.len() >= MOST_FOUND {
                    return false;
                }
            }
        }
        true
    });
    answer(found, all, &format!("No file holds {text:?}."))
}

/// The files and folders under `folder` whose path holds `name`, in any
/// case; folders end in /.
fn find_files(root: &Path, folder: &Path, name: &str) -> (bool, String) {
    let wanted = name.to_lowercase();
    let mut found = Vec::new();
    let all = walk(folder, &mut |path, is_folder| {
        // Matched below the folder, so its own name doesn't match it all.
        let below = shown(folder, path);
        if below.to_lowercase().contains(&wanted) {
            let path = shown(root, path);
            found.push(if is_folder { format!("{path}/") } else { path });
        }
        found.len() < MOST_FOUND
    });
    answer(found, all, &format!("No path holds {name:?}."))
}

fn answer(found: Vec<String>, all: bool, none: &str) -> (bool, String) {
    if found.is_empty() {
        let none = if all {
            none.to_owned()
        } else {
            format!("{none} Not every file was looked at; give a path to look in.")
        };
        return (true, none);
    }
    let mut text = found.join("\n");
    if !all {
        text.push_str("\n(There may be more; give a path, or more of the text, to narrow it.)");
    }
    (true, truncated(text))
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
        serve_listing(listener, json!({"object": "list", "data": []}), replies)
    }

    /// Like [`serve_on`], answering each `GET` with `listing` and not
    /// taking a reply for it.
    fn serve_listing(
        listener: TcpListener,
        listing: Value,
        replies: Vec<(u16, String)>,
    ) -> FakeServer {
        let port = listener.local_addr().expect("an address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let kept = requests.clone();
        tokio::spawn(async move {
            let mut replies = replies.into_iter();
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                let mut socket = tokio::io::BufReader::new(socket);
                let mut length = 0;
                let mut get = false;
                loop {
                    let mut line = String::new();
                    if socket.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    get |= line.starts_with("GET ");
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
                if get {
                    let body = listing.to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.get_mut().write_all(response.as_bytes()).await;
                    let _ = socket.get_mut().shutdown().await;
                    continue;
                }
                let Some((status, body)) = replies.next() else {
                    return;
                };
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
            .find(|event| {
                event.kind
                    == EventKind::UserMessage {
                        text: "two".into(),
                        images: Vec::new(),
                    }
            })
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
    async fn listed_models_keep_what_the_server_says_sets_them_apart() {
        let list = json!({"object": "list", "data": [
            {"id": "qwen", "object": "model", "detail": "Calls tools well"},
            {"id": "apple", "object": "model"},
            {"id": "blank", "object": "model", "detail": "  "},
        ]});
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listens");
        let server = serve_listing(listener, list, Vec::new());
        let runner = LocalModelRunner::new(LocalModelConfig::default());
        let models = server_models(runner.http.clone(), &server.base_url, None)
            .await
            .expect("the models are listed");

        let details: Vec<(&str, Option<&str>)> = models
            .iter()
            .map(|model| (model.id.as_str(), model.detail.as_deref()))
            .collect();
        assert_eq!(
            details,
            [
                ("apple", None),
                ("blank", None),
                ("qwen", Some("Calls tools well"))
            ]
        );
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

    #[test]
    fn a_tool_call_cut_off_before_its_result_still_gets_one() {
        let history = [
            EventKind::UserMessage {
                text: "go".into(),
                images: Vec::new(),
            },
            EventKind::ToolStarted {
                call_id: "c1".into(),
                tool: "shell".into(),
                arguments_json: "{}".into(),
            },
            EventKind::ToolStarted {
                call_id: "c2".into(),
                tool: "shell".into(),
                arguments_json: "{}".into(),
            },
            EventKind::ToolCompleted {
                call_id: "c2".into(),
                success: true,
                output: "ok".into(),
            },
            EventKind::TurnEnded { aborted: true },
            EventKind::UserMessage {
                text: "again".into(),
                images: Vec::new(),
            },
        ]
        .map(Event::now);
        let messages = messages(None, &history, None, UNKNOWN_CONTEXT_SENT);
        let results: Vec<&str> = messages
            .iter()
            .filter(|message| message["role"] == "tool")
            .filter_map(|message| message["tool_call_id"].as_str())
            .collect();
        assert_eq!(results.len(), 2);
        assert!(results.contains(&"c1") && results.contains(&"c2"));
        // Each result comes before the person's next message.
        assert_eq!(
            messages.last().map(|message| &message["content"]),
            Some(&json!("again"))
        );
    }

    #[test]
    fn only_the_latest_images_are_sent_again() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        std::fs::write(folder.path().join("old.png"), [1]).expect("written");
        std::fs::write(folder.path().join("new.png"), [2]).expect("written");
        let history = [
            EventKind::UserMessage {
                text: "first".into(),
                images: vec!["old.png".into()],
            },
            EventKind::AssistantMessage {
                text: "a cat".into(),
            },
            EventKind::UserMessage {
                text: "and this?".into(),
                images: vec!["new.png".into()],
            },
            EventKind::AssistantMessage {
                text: "a dog".into(),
            },
            EventKind::UserMessage {
                text: "compare them".into(),
                images: Vec::new(),
            },
        ]
        .map(Event::now);
        let messages = messages(None, &history, Some(folder.path()), UNKNOWN_CONTEXT_SENT);
        let images: Vec<&str> = messages
            .iter()
            .filter_map(|message| message["content"].as_array())
            .flatten()
            .filter_map(|part| part["image_url"]["url"].as_str())
            .collect();
        assert_eq!(
            images,
            vec![format!(
                "data:image/png;base64,{}",
                crate::images::base64(&[2])
            )]
        );
    }

    fn started(id: &str) -> EventKind {
        EventKind::ToolStarted {
            call_id: id.into(),
            tool: "shell".into(),
            arguments_json: "{}".into(),
        }
    }

    fn completed(id: &str, output: &str) -> EventKind {
        EventKind::ToolCompleted {
            call_id: id.into(),
            success: true,
            output: output.into(),
        }
    }

    fn user(text: &str) -> EventKind {
        EventKind::UserMessage {
            text: text.into(),
            images: Vec::new(),
        }
    }

    #[test]
    fn a_message_sent_while_a_tool_runs_comes_after_its_result() {
        let history = [
            user("go"),
            EventKind::AssistantMessage {
                text: "Looking.".into(),
            },
            started("c1"),
            user("also check the tests"),
            completed("c1", "ok"),
            started("c2"),
            completed("c2", "ok"),
        ]
        .map(Event::now);
        let messages = messages(None, &history, None, UNKNOWN_CONTEXT_SENT);
        let roles: Vec<&str> = messages
            .iter()
            .skip(1)
            .filter_map(|message| message["role"].as_str())
            .collect();
        assert_eq!(roles, vec!["user", "assistant", "tool", "tool", "user"]);
        // The reply's text and both its calls are one message.
        assert_eq!(messages[2]["content"], json!("Looking."));
        assert_eq!(messages[2]["tool_calls"].as_array().map(Vec::len), Some(2));
        assert_eq!(messages[5]["content"], json!("also check the tests"));
    }

    fn outputs(history: &[Event]) -> Vec<String> {
        messages(None, history, None, UNKNOWN_CONTEXT_SENT)
            .iter()
            .filter(|message| message["role"] == "tool")
            .filter_map(|message| message["content"].as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn old_tool_output_is_left_out_a_step_at_a_time() {
        let big = "x".repeat(UNKNOWN_CONTEXT_SENT / 4);
        let mut history = vec![Event::now(user("go"))];
        for index in 0..3 {
            history.push(Event::now(started(&format!("c{index}"))));
            history.push(Event::now(completed(&format!("c{index}"), &big)));
        }
        // Within the budget: nothing left out.
        assert!(outputs(&history).iter().all(|output| *output == big));

        history.push(Event::now(started("c3")));
        history.push(Event::now(completed("c3", &big)));
        let first = outputs(&history);
        assert_eq!(first[..2], [DROPPED, DROPPED]);
        assert_eq!(first[2..], [big.clone(), big.clone()]);

        // A little more doesn't move what's left out, so what was sent
        // before is sent the same.
        history.push(Event::now(started("c4")));
        history.push(Event::now(completed("c4", "small")));
        assert_eq!(outputs(&history)[..4], first[..]);
    }

    /// The arguments each call in `history` is sent with.
    fn arguments(history: &[Event], budget: usize) -> Vec<Value> {
        messages(None, history, None, budget)
            .iter()
            .filter_map(|message| message["tool_calls"].as_array())
            .flatten()
            .filter_map(|call| call["function"]["arguments"].as_str())
            .map(|arguments| serde_json::from_str(arguments).expect("still JSON"))
            .collect()
    }

    #[test]
    fn old_writes_are_sent_without_their_contents_and_the_newest_whole() {
        let budget = 32 * 1024;
        let contents = "y".repeat(budget / 2);
        let mut history = vec![Event::now(user("write three files"))];
        for index in 0..3 {
            history.push(Event::now(EventKind::ToolStarted {
                call_id: format!("w{index}"),
                tool: "write_file".into(),
                arguments_json: json!({"path": format!("f{index}"), "contents": contents})
                    .to_string(),
            }));
            history.push(Event::now(completed(&format!("w{index}"), "Wrote it.")));
        }
        let sent = arguments(&history, budget);
        assert_eq!(sent.len(), 3);
        // Each is still the same call, to the same file.
        for (index, call) in sent.iter().enumerate() {
            assert_eq!(call["path"], format!("f{index}"));
        }
        assert_ne!(sent[0]["contents"], contents.as_str());
        assert_eq!(sent[2]["contents"], contents.as_str());
        let size: usize = messages(None, &history, None, budget)
            .iter()
            .map(|message| message.to_string().len())
            .sum();
        assert!(size <= budget + budget / 8, "{size} bytes sent");

        // Under a budget they fit in, every write goes whole.
        let roomy = arguments(&history, 4 * budget);
        assert!(
            roomy
                .iter()
                .all(|call| call["contents"] == contents.as_str())
        );
    }

    #[test]
    fn long_messages_count_toward_the_budget_too() {
        let budget = 16 * 1024;
        let output = "o".repeat(budget / 4);
        let with = |message: &str| {
            [
                started("c0"),
                completed("c0", &output),
                user(message),
                started("c1"),
                completed("c1", "new"),
            ]
            .map(Event::now)
        };
        let results = |history: &[Event]| -> Vec<String> {
            messages(None, history, None, budget)
                .iter()
                .filter(|message| message["role"] == "tool")
                .filter_map(|message| message["content"].as_str().map(str::to_owned))
                .collect()
        };
        assert_eq!(results(&with("short")), [output.clone(), "new".to_owned()]);
        assert_eq!(results(&with(&"m".repeat(budget)))[0], DROPPED);
    }

    #[test]
    fn the_budget_is_sized_from_the_models_context() {
        let tools = tools();
        let small = budget(Some(8_192), &tools);
        let large = budget(Some(131_072), &tools);
        assert!(small < large, "{small} {large}");
        // Under the context's bytes, so the reply has room.
        assert!(small < 8_192 * BYTES_PER_TOKEN, "{small}");
        // The tools' descriptions go with every request, so they come off it.
        assert_eq!(
            budget(None, &tools),
            UNKNOWN_CONTEXT_SENT - tools.to_string().len()
        );
        // However small the context, a turn still has room to work.
        assert_eq!(budget(Some(16), &tools), LEAST_SENT);
    }

    #[test]
    fn a_models_context_is_read_from_how_its_server_lists_it() {
        let listed = json!({"data": [
            {"id": "lm", "context_length": 32_768},
            {"id": "vllm", "max_model_len": 16_384},
            {"id": "llama", "meta": {"n_ctx_train": 8_192}},
            {"id": "ollama"},
        ]});
        assert_eq!(listed_context(&listed, "lm"), Some(32_768));
        assert_eq!(listed_context(&listed, "vllm"), Some(16_384));
        assert_eq!(listed_context(&listed, "llama"), Some(8_192));
        assert_eq!(listed_context(&listed, "ollama"), None);
        assert_eq!(listed_context(&listed, "missing"), None);
    }

    #[test]
    fn files_are_searched_and_found_without_asking_and_skip_git() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let root = folder.path();
        std::fs::create_dir_all(root.join("src/deep")).expect("made");
        std::fs::create_dir_all(root.join(".git")).expect("made");
        std::fs::write(
            root.join("src/deep/login.rs"),
            "fn a() {}\nfn Sign_In() {}\n",
        )
        .expect("written");
        std::fs::write(root.join("README"), "nothing here\n").expect("written");
        std::fs::write(root.join(".git/config"), "sign_in\n").expect("written");

        let (ok, found) = search_files(root, root, "sign_in");
        assert!(ok);
        assert_eq!(found, "src/deep/login.rs:2: fn Sign_In() {}");

        let (ok, found) = find_files(root, &root.join("src"), "LOGIN");
        assert!(ok);
        assert_eq!(found, "src/deep/login.rs");
        let (_, folders) = find_files(root, root, "deep");
        assert_eq!(folders, "src/deep/\nsrc/deep/login.rs");

        // Nothing found isn't a failure.
        let (ok, _) = search_files(root, root, "absent");
        assert!(ok);
        assert_eq!(folder_of(root, "../up"), None);
    }

    fn folder_of(root: &Path, path: &str) -> Option<PathBuf> {
        folder(root, Path::new(path))
    }

    #[test]
    fn a_long_file_is_read_a_part_at_a_time() {
        let contents: String = (1..=10).map(|line| format!("line {line}\n")).collect();
        let part = lines(&contents, Some(3), Some(2));
        assert!(part.starts_with("line 3\nline 4\n"));
        assert!(!part.contains("line 5"));
        assert!(part.contains("offset 5"));
        assert_eq!(lines(&contents, None, None).matches("line ").count(), 10);
        assert!(!lines(&contents, Some(20), None).contains("line 1"));
    }

    #[tokio::test]
    async fn an_edit_replaces_text_found_once_and_refuses_text_found_twice() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let target = folder.path().join("a.py");
        std::fs::write(&target, "x = 1\ny = 1\n").expect("written");
        let edit = |old: &str, every: bool| EditArguments {
            path: "a.py".into(),
            old_text: old.into(),
            new_text: "= 2".into(),
            every,
        };
        assert!(edited(&target, &edit("x = 1", false)).await.0);
        assert_eq!(
            std::fs::read_to_string(&target).ok().as_deref(),
            Some("= 2\ny = 1\n")
        );
        assert!(!edited(&target, &edit("= ", false)).await.0);
        assert!(!edited(&target, &edit("z", false)).await.0);
        assert!(edited(&target, &edit("= ", true)).await.0);
    }

    #[test]
    fn long_command_output_keeps_its_start_and_end() {
        let mut printed = Capped::default();
        printed.push(b"start\n");
        for _ in 0..1000 {
            printed.push(&[b'x'; 1000]);
        }
        printed.push(b"\nexit status 1");
        let kept = printed.text();
        assert!(kept.len() < MAX_OUTPUT + 100);
        assert!(kept.starts_with("start"));
        assert!(kept.ends_with("exit status 1"));
    }

    #[tokio::test]
    async fn a_command_prints_its_output_and_its_failure() {
        let root = std::env::temp_dir();
        let (ok, text) = shell(&root, &Env::default(), "echo out; echo err >&2; exit 3").await;
        assert!(!ok);
        assert!(text.contains("out"));
        assert!(text.contains("err"));
        assert!(text.contains('3'));
    }

    #[test]
    fn a_context_refusal_is_told_apart_from_other_failures() {
        assert!(context_full(
            400,
            r#"{"error":"the request exceeds the available context size"}"#
        ));
        assert!(context_full(
            400,
            "This model's maximum context length is 8192 tokens."
        ));
        assert!(!context_full(404, "model not found"));
        assert!(!context_full(400, "bad request"));
    }
}
