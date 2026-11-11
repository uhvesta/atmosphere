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

use crate::env::Env;
use crate::error::AgentError;
use crate::event::{EventKind, PermissionKind};
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
    Agent(#[from] AgentError),
}

/// Plays turns on the Copilot CLI, started when first needed and again if it
/// stops.
pub struct CopilotRunner {
    options: CopilotOptions,
    cli: tokio::sync::Mutex<Option<Arc<Cli>>>,
    /// Aborts sent for stopped turns, by Copilot session, so its next turn
    /// starts after the last one has stopped.
    aborting: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl CopilotRunner {
    pub fn new(options: CopilotOptions) -> Self {
        Self {
            options,
            cli: tokio::sync::Mutex::new(None),
            aborting: Mutex::new(HashMap::new()),
        }
    }

    async fn cli(&self) -> Result<Arc<Cli>, CopilotError> {
        let mut cli = self.cli.lock().await;
        if let Some(running) = cli.as_ref().filter(|cli| cli.is_running()) {
            return Ok(running.clone());
        }
        let started = Cli::start(&self.options.program, &self.options.env.vars()).await?;
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
        if let Some(session) = &links.session {
            if !cli.is_live(session) {
                let params = session_params(turn, provider, Some(session));
                cli.call("session.resume", params).await?;
                cli.set_live(session);
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
                    session_params(turn, provider, Some(&session)),
                )
                .await?;
                session
            }
            None => {
                links.events.clear();
                let created = cli
                    .call("session.create", session_params(turn, provider, None))
                    .await?;
                string(&created, "session.create", "sessionId")?
            }
        };
        cli.set_live(&session);
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

fn session_params(turn: &Turn, provider: Option<&CopilotProvider>, id: Option<&str>) -> Value {
    let record = turn.record();
    let mut params = json!({
        "workingDirectory": record.working_directory,
        "requestPermission": true,
        "streaming": true,
    });
    if let Some(id) = id {
        params["sessionId"] = json!(id);
    }
    if let Some(model) = &record.model {
        params["model"] = json!(model);
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
    params
}

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

    async fn run_turn(self: Arc<Self>, turn: Turn) -> Result<(), CopilotError> {
        let config = match &turn.record().backend_config {
            Some(json) => serde_json::from_str::<CopilotConfig>(json)
                .map_err(|source| CopilotError::BadConfig { source })?,
            None => CopilotConfig::default(),
        };
        let provider = config.provider.or_else(|| self.options.provider.clone());
        let path = Links::path(&self.options.state, turn.record().id.as_str());
        let mut links = Links::load(&path)?;
        let cli = self.cli().await?;
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
        cli.call(
            "session.send",
            json!({"sessionId": session, "prompt": turn.text()}),
        )
        .await?;
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
        self.links.events.push((ours, theirs.to_owned()));
        self.links.save(&self.path)
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
                EventKind::UserMessage { text } => Some((event.id.clone(), text.as_str())),
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
        let approved = self.turn.ask(kind, summary).await?;
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
