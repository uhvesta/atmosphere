//! The Copilot backend: GitHub's Copilot CLI, driven through its server mode.
//!
//! The CLI keeps the conversation and runs the tools; [`LocalAgent`] keeps
//! our history, queue, approvals and aborts around it, as for any runner.
//! Each of our sessions is one Copilot session, created at its first turn
//! and resumed after a restart. A fork forks the Copilot session at the same
//! event, at its own first turn.
//!
//! The CLI uses GitHub's models once signed in (`copilot login`). Given a
//! provider instead, it runs on any OpenAI-compatible server, such as a local
//! Ollama, with no sign-in and no token cost.
//!
//! [`LocalAgent`]: crate::LocalAgent

mod rpc;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use crate::backup::{self, BackendFile};
use crate::claude::McpServer;
use crate::env::Env;
use crate::error::AgentError;
use crate::event::{ApprovalMode, EventKind, PermissionKind};
use crate::event::{SessionId, SessionRecord};
use crate::import::{self, Importable, Imported};
use crate::links::{Cut, Links};
use crate::runtime::{Runner, Turn};
use rpc::{Cli, PROTOCOL_VERSION};

/// The Copilot CLI version this backend is written against, pinned in
/// `//apps/humata/copilot`.
pub const COPILOT_VERSION: &str = "1.0.110-1";

/// How often a running turn passes on steering messages.
const STEER_EVERY: Duration = Duration::from_millis(100);

/// Persisted events read at a time when forking.
const PAGE: u64 = 500;

pub struct CopilotOptions {
    /// The `copilot` program.
    pub program: PathBuf,
    /// Where to keep which of our sessions is which of Copilot's.
    pub state: PathBuf,
    /// Set for the program, such as `COPILOT_HOME`, when it starts.
    pub env: Env,
    /// For sessions that name none: a model server to use instead of GitHub's.
    pub provider: Option<CopilotProvider>,
    /// Humata's MCP server, which each session starts for its board and
    /// review tools.
    pub mcp: Option<McpServer>,
}

/// An OpenAI-compatible model server, as JSON:
/// `{"type": "openai", "base_url": "...", "api_key": "..."}`. The type and key
/// are optional.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopilotProvider {
    #[serde(rename = "type", default = "openai")]
    pub kind: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

fn openai() -> String {
    "openai".to_owned()
}

/// A session's backend config, as JSON: `{"provider": {...}}`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CopilotConfig {
    provider: Option<CopilotProvider>,
}

#[derive(Debug, thiserror::Error)]
pub enum CopilotError {
    #[error(
        "couldn't start the Copilot CLI at {program}; install it, or point HUMATA_COPILOT at it (the version Humata is tested with is {COPILOT_VERSION})"
    )]
    Start {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the Copilot CLI stopped; send again to restart it. If it keeps stopping, run `copilot` in a terminal to see why"
    )]
    Exited,

    #[error("lost the connection to the Copilot CLI; send again to restart it")]
    Pipe {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the Copilot CLI speaks protocol version {found}, and Humata speaks {PROTOCOL_VERSION}; use the version Humata is tested with, {COPILOT_VERSION}"
    )]
    Protocol { found: String },

    #[error(
        "the Copilot CLI refused {method} ({code}): {message}. If the session is gone from Copilot's own state, fork this session into a new one"
    )]
    Rpc {
        method: String,
        code: i64,
        message: String,
    },

    #[error(
        "the Copilot CLI's answer to {method} had no {field}; use the version Humata is tested with, {COPILOT_VERSION}"
    )]
    Missing {
        method: &'static str,
        field: &'static str,
    },

    #[error(
        "Copilot failed: {message}. If it's about signing in, run `copilot login` or set COPILOT_GITHUB_TOKEN; to use a local model instead, give the session a provider"
    )]
    Session { message: String },

    #[error(
        "the session's Copilot config isn't valid; give JSON like {{\"provider\": {{\"base_url\": \"http://127.0.0.1:11434/v1\"}}}}, or none, and start a new session with it"
    )]
    BadConfig {
        #[source]
        source: serde_json::Error,
    },

    #[error(transparent)]
    Plugins(#[from] crate::plugins::OverlayError),

    #[error(transparent)]
    Agent(#[from] AgentError),
}

/// Where a backup keeps Copilot's session folders.
const SESSION_STATE: &str = "session-state";

/// Plays turns on the Copilot CLI, started when first needed and again if it
/// stops.
/// The folder under the state one where the running CLI is noted.
const RUNNING: &str = "running";
/// Where the CLI running with its experimental features is noted.
const RUNNING_EXPERIMENTAL: &str = "running-experimental";

/// Copilot Fusion, which picks the models for each turn, by its id.
const FUSION: &str = "fusion";
/// Fusion's tiers, least first; it takes them in place of an effort.
const FUSION_TIERS: [&str; 3] = ["standard", "high", "max"];

pub struct CopilotRunner {
    options: CopilotOptions,
    /// The CLI, and the one running with its experimental features.
    cli: [tokio::sync::Mutex<Option<Arc<Cli>>>; 2],
    /// Aborts sent for stopped turns, by Copilot session, so its next turn
    /// starts after the last one has stopped.
    aborting: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl CopilotRunner {
    pub fn new(options: CopilotOptions) -> Self {
        crate::children::stop_left(&options.state.join(RUNNING));
        crate::children::stop_left(&options.state.join(RUNNING_EXPERIMENTAL));
        Self {
            options,
            cli: [tokio::sync::Mutex::new(None), tokio::sync::Mutex::new(None)],
            aborting: Mutex::new(HashMap::new()),
        }
    }

    /// Where Copilot keeps its sessions, one folder each.
    fn session_state(&self) -> Option<PathBuf> {
        import::backend_home(&self.options.env, "COPILOT_HOME", ".copilot")
            .map(|home| home.join("session-state"))
    }

    /// The CLI, or the one with its experimental features on.
    async fn cli(&self, experimental: bool) -> Result<Arc<Cli>, CopilotError> {
        let mut cli = self.cli[usize::from(experimental)].lock().await;
        if let Some(running) = cli.as_ref().filter(|cli| cli.is_running()) {
            return Ok(running.clone());
        }
        let (flag, running) = if experimental {
            ("--experimental", RUNNING_EXPERIMENTAL)
        } else {
            ("--no-experimental", RUNNING)
        };
        let started = Cli::start(
            &self.options.program,
            flag,
            &self.options.env.vars(),
            &self.options.state.join(running),
        )
        .await?;
        *cli = Some(started.clone());
        Ok(started)
    }

    /// The Copilot session for the turn's session: resumed, forked or
    /// created, as its links say.
    async fn open(
        &self,
        cli: &Cli,
        turn: &Turn,
        provider: Option<&CopilotProvider>,
        links: &mut Links,
        path: &std::path::Path,
    ) -> Result<String, CopilotError> {
        let record = turn.record();
        // What the session was opened with; opened again when it changes.
        let opened_with = format!("{:?}|{:?}", record.model, record.effort);
        // Copilot reads them as it opens the session.
        if !links
            .session
            .as_ref()
            .is_some_and(|session| cli.is_live(session, &opened_with))
        {
            let checkouts: Vec<_> = record
                .checkouts
                .iter()
                .map(|checkout| checkout.path.clone())
                .collect();
            crate::plugins::write_copilot(&checkouts, &record.plugins, fusion_tier(&record))?;
        }
        if let Some(session) = &links.session {
            if !cli.is_live(session, &opened_with) {
                let params =
                    session_params(turn, provider, self.options.mcp.as_ref(), Some(session));
                cli.call("session.resume", params).await?;
                cli.set_live(session, &opened_with);
            }
            return Ok(session.clone());
        }
        let forked = match links.fork.take() {
            Some(fork) => fork_at(cli, &fork.source, links).await?,
            None => None,
        };
        let session = match forked {
            Some(session) => {
                cli.call(
                    "session.resume",
                    session_params(turn, provider, self.options.mcp.as_ref(), Some(&session)),
                )
                .await?;
                session
            }
            None => {
                links.events.clear();
                if crate::links::had_turns(&turn.history()) {
                    turn.emit(EventKind::Notice {
                        text: crate::links::STARTED_OVER.to_owned(),
                    })?;
                }
                let created = cli
                    .call(
                        "session.create",
                        session_params(turn, provider, self.options.mcp.as_ref(), None),
                    )
                    .await?;
                string(&created, "session.create", "sessionId")?
            }
        };
        cli.set_live(&session, &opened_with);
        links.session = Some(session.clone());
        links.save(path)?;
        Ok(session)
    }

    /// Waits for the abort of the session's last turn, if it was stopped.
    /// True if it was.
    async fn settle(&self, session: &str) -> bool {
        let aborting = self
            .aborting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(session);
        match aborting {
            Some(aborting) => {
                let _ = aborting.await;
                true
            }
            None => false,
        }
    }
}

/// Forks `source` where `links` say. None if it keeps nothing of it.
async fn fork_at(cli: &Cli, source: &str, links: &Links) -> Result<Option<String>, CopilotError> {
    let mut ids = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let mut params = json!({"sessionId": source, "max": PAGE});
        if !cursor.is_null() {
            params["cursor"] = cursor;
        }
        let page = cli.call("sessions.readPersistedEvents", params).await?;
        let events = page
            .get("events")
            .and_then(Value::as_array)
            .ok_or(CopilotError::Missing {
                method: "sessions.readPersistedEvents",
                field: "events",
            })?;
        ids.extend(
            events
                .iter()
                .filter_map(|event| event.get("id").and_then(Value::as_str))
                .map(str::to_owned),
        );
        cursor = page.get("cursor").cloned().unwrap_or(Value::Null);
        let more = page.get("hasMore").and_then(Value::as_bool) == Some(true);
        if !more || cursor.is_null() || events.is_empty() {
            break;
        }
    }
    let params = match links.cut(&ids) {
        Cut::Nothing => return Ok(None),
        Cut::All => json!({"sessionId": source}),
        Cut::Before(event) => json!({"sessionId": source, "toEventId": event}),
    };
    let forked = cli.call("sessions.fork", params).await?;
    string(&forked, "sessions.fork", "sessionId").map(Some)
}

/// The tier a Fusion session runs at, if it's one.
fn fusion_tier(record: &SessionRecord) -> Option<&str> {
    (record.model.as_deref() == Some(FUSION))
        .then(|| record.effort.as_deref().unwrap_or(FUSION_TIERS[0]))
}

/// The strings in `value`, an array of them.
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// The models in Copilot's answer to `models.list`: those the account may
/// use, each with what it costs in premium requests, and Fusion with its
/// tiers.
fn copilot_models(listed: &Value) -> Vec<crate::Model> {
    let mut models: Vec<crate::Model> = listed["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| {
            model["policy"]["state"]
                .as_str()
                .is_none_or(|state| state == "enabled")
        })
        .filter_map(|model| {
            let id = model["id"].as_str()?;
            let multiplier = model["billing"]["multiplier"].as_f64();
            Some(crate::Model {
                id: id.to_owned(),
                name: model["name"].as_str().unwrap_or(id).to_owned(),
                detail: multiplier.map(|times| match times {
                    0.0 => "Included".to_owned(),
                    1.0 => "1 premium request".to_owned(),
                    times => format!("{times} premium requests"),
                }),
                // Said for each model; taken as yes when it isn't.
                reads_images: model["capabilities"]["supports"]["vision"]
                    .as_bool()
                    .unwrap_or(true),
                efforts: if id == FUSION {
                    FUSION_TIERS.iter().map(|&tier| tier.to_owned()).collect()
                } else {
                    strings(&model["supportedReasoningEfforts"])
                },
                default_effort: if id == FUSION {
                    Some(FUSION_TIERS[0].to_owned())
                } else {
                    model["defaultReasoningEffort"].as_str().map(str::to_owned)
                },
            })
        })
        .collect();
    // Fusion bills as the models it picks; the CLI may not list it.
    if !models.is_empty() && !models.iter().any(|model| model.id == FUSION) {
        models.push(crate::Model {
            id: FUSION.to_owned(),
            name: "Copilot Fusion".to_owned(),
            detail: Some("Billed as the models it picks".to_owned()),
            reads_images: true,
            efforts: FUSION_TIERS.iter().map(|&tier| tier.to_owned()).collect(),
            default_effort: Some(FUSION_TIERS[0].to_owned()),
        });
    }
    models
}

fn session_params(
    turn: &Turn,
    provider: Option<&CopilotProvider>,
    mcp: Option<&McpServer>,
    id: Option<&str>,
) -> Value {
    let record = turn.record();
    let mut params = json!({
        "workingDirectory": record.working_directory,
        "requestPermission": true,
        "streaming": true,
        "permissionMode": permission_mode(record.approval),
    });
    if let Some(id) = id {
        params["sessionId"] = json!(id);
    }
    if let Some(model) = &record.model {
        params["model"] = json!(model);
    }
    // Fusion takes its tier from the local settings instead.
    if let Some(effort) = record
        .effort
        .as_ref()
        .filter(|_| fusion_tier(&record).is_none())
    {
        params["reasoningEffort"] = json!(effort);
    }
    if let Some(provider) = provider {
        let mut server = json!({"type": provider.kind, "baseUrl": provider.base_url});
        if let Some(key) = &provider.api_key {
            server["apiKey"] = json!(key);
        }
        params["provider"] = server;
    }
    if let Some(instructions) = &record.instructions {
        params["systemMessage"] = json!({"mode": "append", "content": instructions});
    }
    if let Some(mcp) = mcp {
        let mut env: serde_json::Map<String, Value> = mcp
            .env
            .iter()
            .map(|(name, value)| (name.clone(), json!(value)))
            .collect();
        env.insert("HUMATA_SESSION".into(), json!(record.id.as_str()));
        params["mcpServers"] = json!({HUMATA_MCP: {
            "type": "local",
            "command": mcp.command,
            "args": mcp.args,
            "env": env,
            "tools": ["*"],
        }});
    }
    params
}

/// Copilot's permission mode for `approval`. Assisted has its judge assess
/// each request, and Humata lets through those it approves; otherwise every
/// request comes to Humata and the approval mode answers.
fn permission_mode(approval: ApprovalMode) -> &'static str {
    match approval {
        ApprovalMode::Auto => "assisted",
        ApprovalMode::AskEveryTime | ApprovalMode::AllowEdits | ApprovalMode::AllowEverything => {
            "manual"
        }
    }
}

/// The name Humata's MCP server goes by in each session.
const HUMATA_MCP: &str = "humata";

fn string(
    value: &Value,
    method: &'static str,
    field: &'static str,
) -> Result<String, CopilotError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(CopilotError::Missing { method, field })
}

impl Runner for CopilotRunner {
    type Error = CopilotError;

    async fn models(&self, config: Option<String>) -> Result<Vec<crate::Model>, AgentError> {
        let config = match config {
            Some(json) => serde_json::from_str::<CopilotConfig>(&json)
                .map_err(|source| CopilotError::BadConfig { source })?,
            None => CopilotConfig::default(),
        };
        if let Some(provider) = config.provider.or_else(|| self.options.provider.clone()) {
            let http = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_connect(Some(Duration::from_secs(10)))
                .build()
                .into();
            return Ok(crate::local_model::server_models(
                http,
                &provider.base_url,
                provider.api_key,
            )
            .await?);
        }
        let listed = self
            .cli(false)
            .await?
            .call("models.list", json!({}))
            .await?;
        Ok(copilot_models(&listed))
    }

    fn importable(&self) -> Result<Vec<Importable>, AgentError> {
        let Some(sessions) = self.session_state() else {
            return Ok(Vec::new());
        };
        import::not_imported(&self.options.state, import::copilot_importable(&sessions)?)
    }

    fn backup(&self, record: &SessionRecord) -> Result<Vec<BackendFile>, AgentError> {
        let Some(links) = backup::links_file(&self.options.state, record.id.as_str())? else {
            return Ok(Vec::new());
        };
        let mut files = vec![links];
        let Some(sessions) = self.session_state() else {
            return Ok(files);
        };
        for session in backup::linked(&files)? {
            if import::is_session_id(&session) {
                let prefix = format!("{SESSION_STATE}/{session}");
                files.extend(backup::read_tree(&sessions.join(&session), &prefix)?);
            }
        }
        Ok(files)
    }

    fn forget(&self, record: &SessionRecord) -> Result<(), AgentError> {
        let theirs = backup::forget_links(&self.options.state, record.id.as_str())?;
        let Some(sessions) = self.session_state() else {
            return Ok(());
        };
        for session in theirs
            .iter()
            .filter(|session| import::is_session_id(session))
        {
            backup::remove(&sessions.join(session))?;
        }
        Ok(())
    }

    fn restore(&self, record: &SessionRecord, files: &[BackendFile]) -> Result<(), AgentError> {
        for file in files {
            if file.name == backup::LINKS {
                backup::write_links(&self.options.state, record.id.as_str(), &file.contents)?;
            } else if let Some(name) = file.name.strip_prefix(&format!("{SESSION_STATE}/")) {
                let Some(sessions) = self.session_state() else {
                    continue;
                };
                backup::write_under(&sessions, name, &file.contents)?;
            }
        }
        Ok(())
    }

    fn import(&self, external: &str, session: &SessionId) -> Result<Imported, AgentError> {
        let sessions = self
            .session_state()
            .ok_or_else(|| AgentError::UnknownImport {
                id: external.to_owned(),
            })?;
        import::import_into(&self.options.state, external, session, || {
            import::copilot_read(&import::copilot_folder(&sessions, external)?)
        })
    }

    async fn run_turn(self: Arc<Self>, turn: Turn) -> Result<(), CopilotError> {
        let config = match &turn.record().backend_config {
            Some(json) => serde_json::from_str::<CopilotConfig>(json)
                .map_err(|source| CopilotError::BadConfig { source })?,
            None => CopilotConfig::default(),
        };
        let provider = config.provider.or_else(|| self.options.provider.clone());
        let path = Links::path(&self.options.state, turn.record().id.as_str());
        let mut links = Links::load(&path)?;
        let cli = self.cli(turn.record().experimental).await?;
        let session = self
            .open(&cli, &turn, provider.as_ref(), &mut links, &path)
            .await?;
        let after_abort = self.settle(&session).await;

        let mut events = cli.subscribe(&session);
        let mut stop = StopOnDrop {
            runner: self.clone(),
            cli: Some(cli.clone()),
            session: session.clone(),
        };
        // Images go as files it reads itself, from the session's folder.
        let attachments: Vec<Value> = turn
            .images()
            .iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned());
                json!({"type": "file", "path": path, "displayName": name})
            })
            .collect();
        let mut send = json!({"sessionId": session, "prompt": turn.text()});
        if !attachments.is_empty() {
            send["attachments"] = Value::Array(attachments);
        }
        cli.call("session.send", send).await?;
        let mut playing = Playing {
            turn: &turn,
            cli: &cli,
            session: &session,
            links,
            path,
            failure: None,
            // The aborted turn's end can come after the abort answered.
            started: !after_abort,
        };
        let mut mode = permission_mode(turn.approval());
        let mut steer = tokio::time::interval(STEER_EVERY);
        loop {
            tokio::select! {
                event = events.recv() => {
                    let Some(event) = event else {
                        return Err(CopilotError::Exited);
                    };
                    if playing.handle(&event).await? {
                        break;
                    }
                }
                _ = steer.tick() => {
                    for text in turn.take_steering() {
                        let params = json!({"sessionId": session, "prompt": text, "mode": "immediate"});
                        cli.call("session.send", params).await?;
                    }
                    let now = permission_mode(turn.approval());
                    if now != mode {
                        let params = json!({"sessionId": session, "mode": now});
                        cli.call("session.permissions.setMode", params).await?;
                        mode = now;
                    }
                }
            }
        }
        stop.cli = None;
        playing.failure.map_or(Ok(()), Err)
    }

    fn forked(
        &self,
        parent: &crate::SessionId,
        fork: &crate::SessionRecord,
        history: &[crate::Event],
    ) -> Result<(), AgentError> {
        let parent = Links::load(&Links::path(&self.options.state, parent.as_str()))?;
        let links = parent.fork(history);
        if links == Links::default() {
            return Ok(());
        }
        links.save(&Links::path(&self.options.state, fork.id.as_str()))
    }

    fn rewound(
        &self,
        session: &crate::SessionRecord,
        history: &[crate::Event],
        rewind: usize,
    ) -> Result<(), AgentError> {
        Links::rewind(&self.options.state, session.id.as_str(), history, rewind)
    }

    fn rewind_undone(
        &self,
        session: &crate::SessionRecord,
        rewind: usize,
    ) -> Result<(), AgentError> {
        Links::undo_rewind(&self.options.state, session.id.as_str(), rewind)
    }
}

/// Stops the Copilot turn if ours is cut off before it ends.
struct StopOnDrop {
    runner: Arc<CopilotRunner>,
    /// None once the turn has ended.
    cli: Option<Arc<Cli>>,
    session: String,
}

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        let Some(cli) = self.cli.take() else { return };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let session = self.session.clone();
        let aborting = runtime.spawn(async move {
            let _ = cli
                .call("session.abort", json!({"sessionId": session}))
                .await;
        });
        self.runner
            .aborting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(self.session.clone(), aborting);
    }
}

/// A turn as Copilot plays it.
struct Playing<'a> {
    turn: &'a Turn,
    cli: &'a Cli,
    session: &'a str,
    links: Links,
    path: PathBuf,
    /// What Copilot reported going wrong; the turn fails with it at its end.
    failure: Option<CopilotError>,
    /// Whether Copilot has taken this turn's prompt; until then, an idle is
    /// another turn's.
    started: bool,
}

impl Playing<'_> {
    /// Records one of Copilot's events. True once the turn has ended.
    async fn handle(&mut self, event: &Value) -> Result<bool, CopilotError> {
        let data = &event["data"];
        let text = |field: &str| data[field].as_str().unwrap_or_default().to_owned();
        let kind = match event["type"].as_str().unwrap_or_default() {
            "session.idle" => return Ok(self.started),
            "user.message" => {
                self.started = true;
                self.link_user_message(event, &text("content"))?;
                return Ok(false);
            }
            "session.error" => {
                self.failure = Some(CopilotError::Session {
                    message: text("message"),
                });
                return Ok(false);
            }
            "permission.requested" => {
                if data["resolvedByHook"].as_bool() != Some(true) {
                    self.ask(data).await?;
                }
                return Ok(false);
            }
            "session.compaction_complete" => EventKind::Compacted,
            "assistant.message_delta" => EventKind::AssistantDelta {
                text: text("deltaContent"),
            },
            "assistant.message" => EventKind::AssistantMessage {
                text: text("content"),
            },
            "assistant.reasoning" => EventKind::Reasoning {
                text: text("content"),
            },
            "tool.execution_start" => EventKind::ToolStarted {
                call_id: text("toolCallId"),
                tool: text("toolName"),
                arguments_json: data["arguments"].to_string(),
            },
            "tool.execution_complete" => {
                let output = data["result"]["content"]
                    .as_str()
                    .or_else(|| data["error"]["message"].as_str())
                    .or_else(|| data["error"].as_str())
                    .unwrap_or_default();
                EventKind::ToolCompleted {
                    call_id: text("toolCallId"),
                    success: data["success"].as_bool().unwrap_or(false),
                    output: output.to_owned(),
                }
            }
            "assistant.usage" => EventKind::Usage {
                model: text("model"),
                input_tokens: data["inputTokens"].as_u64().unwrap_or(0),
                output_tokens: data["outputTokens"].as_u64().unwrap_or(0),
                premium_requests: data["cost"].as_f64().unwrap_or(0.0),
                cache_read_tokens: data["cacheReadTokens"].as_u64().unwrap_or(0),
                cache_write_tokens: data["cacheWriteTokens"].as_u64().unwrap_or(0),
            },
            _ => return Ok(false),
        };
        let empty = matches!(
            &kind,
            EventKind::AssistantDelta { text }
                | EventKind::AssistantMessage { text }
                | EventKind::Reasoning { text } if text.is_empty()
        );
        if empty {
            return Ok(false);
        }
        let ours = self.turn.emit(kind)?;
        self.link(event, ours.id)?;
        Ok(false)
    }

    /// Links a lasting event of Copilot's to ours.
    fn link(&mut self, event: &Value, ours: crate::EventId) -> Result<(), AgentError> {
        let ephemeral = event["ephemeral"].as_bool() == Some(true);
        let Some(theirs) = event["id"].as_str().filter(|_| !ephemeral) else {
            return Ok(());
        };
        self.links.link(&self.path, ours, theirs.to_owned())
    }

    /// Links Copilot's record of a message to ours: the turn's own, or one
    /// sent to steer it, whichever has its text and isn't linked yet.
    fn link_user_message(&mut self, event: &Value, content: &str) -> Result<(), AgentError> {
        let history = self.turn.history();
        let start = history
            .iter()
            .rposition(|event| matches!(event.kind, EventKind::TurnStarted))
            .map_or(0, |started| started.saturating_sub(1));
        let unlinked: Vec<_> = history[start..]
            .iter()
            .filter(|event| !self.links.is_linked(&event.id))
            .filter_map(|event| match &event.kind {
                EventKind::UserMessage { text, .. } => Some((event.id.clone(), text.as_str())),
                _ => None,
            })
            .collect();
        let ours = unlinked
            .iter()
            .find(|(_, text)| *text == content)
            .or_else(|| unlinked.first())
            .map(|(id, _)| id.clone());
        match ours {
            Some(ours) => self.link(event, ours),
            None => Ok(()),
        }
    }

    /// Asks the person about Copilot's permission request, and answers it.
    async fn ask(&mut self, data: &Value) -> Result<(), CopilotError> {
        let request = &data["permissionRequest"];
        let field = |name: &str| request[name].as_str().map(str::to_owned);
        // Humata's tools change only its board and review threads, which the
        // person sees and can revert, so they aren't asked about.
        let ours = request["kind"] == "mcp" && request["serverName"] == HUMATA_MCP;
        let (kind, summary) = match request["kind"].as_str().unwrap_or_default() {
            "shell" => (PermissionKind::Shell, field("fullCommandText")),
            "write" => (PermissionKind::Write, field("fileName")),
            "read" => (PermissionKind::Read, field("path")),
            "url" => (PermissionKind::Other, field("url")),
            _ => (PermissionKind::Other, None),
        };
        let summary = summary
            .or_else(|| field("intention"))
            .or_else(|| field("kind"))
            .unwrap_or_default();
        let judged = self.turn.approval() == ApprovalMode::Auto
            && data["assistedApprovalAssessment"]["status"] == "completed"
            && data["assistedApprovalAssessment"]["recommendation"] == "approve";
        let approved = if ours {
            true
        } else if judged {
            self.turn.allowed_by_judge(kind, summary)?;
            true
        } else {
            self.turn.ask(kind, summary).await?
        };
        let result = if approved {
            json!({"kind": "approve-once"})
        } else {
            json!({"kind": "reject"})
        };
        let params = json!({
            "sessionId": self.session,
            "requestId": data["requestId"],
            "result": result,
        });
        self.cli
            .call("session.permissions.handlePendingPermissionRequest", params)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::copilot_models;

    #[test]
    fn copilot_lists_the_models_the_account_may_use_with_their_cost() {
        let listed = json!({"models": [
            {"id": "gpt-5", "name": "GPT-5", "billing": {"multiplier": 1.0}},
            {"id": "mini", "billing": {"multiplier": 0.0}},
            {"id": "locked", "name": "Locked", "policy": {"state": "disabled"}},
            {"name": "no id"},
        ]});
        let models = copilot_models(&listed);
        let ids: Vec<_> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["gpt-5", "mini", "fusion"]);
        assert_eq!(models[1].name, "mini");
        assert!(models.iter().all(|model| model.detail.is_some()));
        assert_eq!(models[2].efforts, ["standard", "high", "max"]);
    }

    #[test]
    fn each_model_says_the_efforts_it_takes() {
        let listed = json!({"models": [
            {"id": "gpt-5", "supportedReasoningEfforts": ["low", "high"], "defaultReasoningEffort": "low"},
            {"id": "fusion", "name": "Fusion"},
        ]});
        let models = copilot_models(&listed);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].efforts, ["low", "high"]);
        assert_eq!(models[0].default_effort.as_deref(), Some("low"));
        assert_eq!(models[1].efforts, ["standard", "high", "max"]);
    }

    #[test]
    fn an_answer_without_models_lists_none() {
        assert!(copilot_models(&json!({"error": "nope"})).is_empty());
    }
}
