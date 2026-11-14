//! The Claude Code backend: the `claude` CLI in print mode, speaking JSON
//! lines on its standard input and output.
//!
//! Each turn runs one `claude -p`. The CLI keeps the conversation in a
//! session of its own, so a turn resumes it; a fork resumes its parent's up
//! to the last event it keeps, as a new session, at its own first turn. The
//! CLI asks before using a tool over the same pipe, and [`Turn::ask`] answers
//! it, so the session's approval mode and the person decide, not the CLI's
//! settings. [`LocalAgent`] keeps our history, queue, approvals and aborts
//! around it, as for any runner.
//!
//! It runs as whoever the CLI is signed in as (`claude`, then `/login`, with
//! a Claude subscription), or on `ANTHROPIC_API_KEY` in its environment.
//!
//! [`LocalAgent`]: crate::LocalAgent

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};

use crate::Compaction;
use crate::backup::{self, BackendFile};
use crate::children;
use crate::env::Env;
use crate::error::AgentError;
use crate::event::{ApprovalMode, EventKind, PermissionKind, SessionId, SessionRecord};
use crate::import::{self, Importable, Imported};
use crate::links::Links;
use crate::runtime::{Runner, Turn};

mod subagents;

/// The Claude Code version this backend is written against, pinned in
/// `//apps/humata/claude`.
pub const CLAUDE_VERSION: &str = "2.1.322";

/// How often a running turn passes on steering messages.
const STEER_EVERY: Duration = Duration::from_millis(100);

/// The folder under the state one where running CLIs are noted.
const RUNNING: &str = "running";

/// How long the CLI gets to exit after a turn before it's stopped.
const EXIT_WAIT: Duration = Duration::from_secs(10);

/// The most of the CLI's standard error kept, from its end, to say why it
/// stopped.
const STDERR_KEPT: usize = 4096;

/// Tools left out: Humata makes the worktrees, and the rest need the CLI's
/// own terminal or outlive the turn.
const LEFT_OUT: &str = "AskUserQuestion,EnterPlanMode,ExitPlanMode,EnterWorktree,ExitWorktree,CronCreate,CronDelete,CronList";

/// What the model is told when the person denies a tool.
const DENIED: &str = "The person directing this session didn't allow this.";

pub struct ClaudeOptions {
    /// The `claude` program.
    pub program: PathBuf,
    /// Where to keep which of our sessions is which of Claude Code's.
    pub state: PathBuf,
    /// Set for the program when it starts, over humatad's own environment.
    pub env: Env,
    /// Given to every session as the MCP server `humata`.
    pub mcp: Option<McpServer>,
}

/// An MCP server the CLI starts on standard input and output. It's told the
/// session it serves in `HUMATA_SESSION`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServer {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, thiserror::Error)]
pub enum ClaudeError {
    #[error(
        "couldn't start Claude Code at {program}; install it (https://claude.ai/install.sh) and sign in by running `claude`, or point HUMATA_CLAUDE at it (the version Humata is tested with is {CLAUDE_VERSION})"
    )]
    Start {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't read the image {path} sent with the message; send the message again with the image attached"
    )]
    ReadImage {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("lost the connection to Claude Code; send again to start it again")]
    Pipe {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "Claude Code stopped before the turn ended ({status}): {stderr}. Send again to retry; if it keeps stopping, run `claude` in a terminal in the session's folder to see why"
    )]
    Exited { status: String, stderr: String },

    #[error(
        "Claude Code failed: {message}. If it's about signing in, run `claude` and `/login`. If it's a usage limit, wait for it to reset; a new session won't get around it and loses the conversation"
    )]
    Turn { message: String },

    #[error(
        "Claude Code no longer has this session's conversation ({session}.jsonl isn't under {}), so it can't go on with it; something removed it, such as Claude Code's own clean-up of old sessions. Sending again or forking won't help, as both need it: start a new session, and copy what it needs from this one's history, which Humata keeps", projects.display()
    )]
    Gone { session: String, projects: PathBuf },

    #[error(
        "Claude Code has no conversation of session {session} to sum up, as it starts a new one at its next message; send a message first"
    )]
    NoConversation { session: SessionId },

    #[error(
        "Claude Code ended /compact without summing up the conversation ({said}). There may be too little to sum up yet; if not, compare `claude --version` with {CLAUDE_VERSION}, the version Humata is tested with"
    )]
    NotCompacted { said: String },

    #[error(transparent)]
    Agent(#[from] AgentError),
}

/// Where a backup keeps Claude Code's transcripts.
const TRANSCRIPTS: &str = "transcripts";

/// Plays turns on Claude Code, one `claude -p` per turn.
pub struct ClaudeRunner {
    options: ClaudeOptions,
}

impl ClaudeRunner {
    pub fn new(options: ClaudeOptions) -> Self {
        children::stop_left(&options.state.join(RUNNING));
        Self { options }
    }

    /// Where Claude Code keeps its transcripts, one folder per project.
    fn transcripts(&self) -> Option<PathBuf> {
        import::backend_home(&self.options.env, "CLAUDE_CONFIG_DIR", ".claude")
            .map(|home| home.join("projects"))
    }

    /// The CLI for a turn of `record`, in the session `links` say: resumed,
    /// forked, or new. The Claude Code session it plays in.
    fn command(&self, record: &SessionRecord, links: &mut Links) -> (Command, String) {
        let mut command = Command::new(&self.options.program);
        command.args([
            "--print",
            "--verbose",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--include-partial-messages",
            "--replay-user-messages",
            "--permission-prompt-tool",
            "stdio",
            "--permission-mode",
            permission_mode(record.approval),
            "--disallowedTools",
            LEFT_OUT,
        ]);
        let session = match (&links.session, &links.fork) {
            (Some(session), _) => {
                command.args(["--resume", session]);
                session.clone()
            }
            (None, Some(fork)) => {
                let session = uuid::Uuid::new_v4().to_string();
                command.args([
                    "--resume",
                    &fork.source,
                    "--fork-session",
                    "--session-id",
                    &session,
                ]);
                if let Some(last) = links.last() {
                    command.args(["--resume-session-at", last]);
                }
                session
            }
            (None, None) => {
                links.events.clear();
                let session = uuid::Uuid::new_v4().to_string();
                command.args(["--session-id", &session]);
                session
            }
        };
        if let Some(model) = &record.model {
            command.args(["--model", model]);
        }
        if let Some(effort) = &record.effort {
            command.args(["--effort", effort]);
        }
        if let Some(instructions) = &record.instructions {
            command.args(["--append-system-prompt", instructions]);
        }
        if let Some(mcp) = &self.options.mcp {
            let mut env: serde_json::Map<String, Value> = mcp
                .env
                .iter()
                .map(|(name, value)| (name.clone(), json!(value)))
                .collect();
            env.insert("HUMATA_SESSION".into(), json!(record.id.as_str()));
            let config = json!({"mcpServers": {"humata": {
                "type": "stdio",
                "command": mcp.command,
                "args": mcp.args,
                "env": env,
            }}});
            command.args(["--mcp-config", &config.to_string()]);
            // Its tools change only Humata's board and review threads, which
            // the person sees and can revert, so they aren't asked about.
            command.args(["--allowedTools", "mcp__humata"]);
        }
        // Over the person's own settings, for this session alone.
        if !record.plugins.is_empty() {
            let settings = json!({"enabledPlugins": record.plugins});
            command.args(["--settings", &settings.to_string()]);
        }
        command
            .current_dir(&record.working_directory)
            .envs(self.options.env.vars())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        (command, session)
    }

    /// Plays `turn` on the CLI: its message, or, `compacting`, the CLI's
    /// own `/compact` with the turn's text as what to keep in mind.
    async fn play(&self, turn: &Turn, compacting: bool) -> Result<(), ClaudeError> {
        let record = turn.record();
        let path = Links::path(&self.options.state, record.id.as_str());
        let mut links = Links::load(&path)?;
        // Resumed, Claude Code would fail on every turn without saying why.
        if let (Some(session), Some(projects)) = (&links.session, self.transcripts())
            && projects.is_dir()
            && import::claude_transcript(&projects, session).is_err()
        {
            return Err(ClaudeError::Gone {
                session: session.clone(),
                projects,
            });
        }
        if links.session.is_none() && links.fork.is_none() {
            if compacting {
                return Err(ClaudeError::NoConversation { session: record.id });
            }
            if turn.with_history(crate::links::had_turns)? {
                turn.emit(EventKind::Notice {
                    text: crate::links::STARTED_OVER.to_owned(),
                })?;
            }
        }
        let (mut command, session) = self.command(&record, &mut links);
        let mut child = command.spawn().map_err(|source| ClaudeError::Start {
            program: self.options.program.clone(),
            source,
        })?;
        let _noted = children::Noted::note(&self.options.state.join(RUNNING), child.id());
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(ClaudeError::Pipe {
                source: std::io::Error::other("the CLI's standard streams weren't piped"),
            });
        };
        let stderr = tokio::spawn(tail(stderr));
        let mut playing = Playing {
            turn,
            stdin: Some(stdin),
            links,
            path,
            session,
            model: record.model.clone().unwrap_or_default(),
            sent: 0,
            replayed: 0,
            failure: None,
            mode: permission_mode(record.approval),
            subagents: subagents::Subagents::default(),
            compact: compacting.then_some(false),
        };
        if compacting {
            playing.send(&compact_command(turn.text())).await?;
        } else {
            let images = read_images(turn.images())?;
            playing.send_with(turn.text(), &images).await?;
        }
        let mut lines = BufReader::new(stdout).lines();
        let mut steer = tokio::time::interval(STEER_EVERY);
        let ended = loop {
            tokio::select! {
                line = lines.next_line() => {
                    let line = line.map_err(|source| ClaudeError::Pipe { source })?;
                    let Some(line) = line else { break false };
                    // Anything else it writes, such as a warning, is skipped.
                    let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
                    if playing.handle(&event).await? {
                        break true;
                    }
                }
                _ = steer.tick() => {
                    for text in turn.take_steering() {
                        playing.send(&text).await?;
                    }
                    let mode = permission_mode(turn.approval());
                    if mode != playing.mode {
                        playing.set_mode(mode).await?;
                    }
                }
            }
        };
        // Closing its input lets it finish writing its session and exit.
        playing.stdin = None;
        let exited = tokio::time::timeout(EXIT_WAIT, child.wait()).await;
        if !ended && playing.failure.is_none() {
            let status = match exited {
                Ok(Ok(status)) => status.to_string(),
                Ok(Err(error)) => error.to_string(),
                Err(_) => "still running".to_owned(),
            };
            let stderr = stderr.await.unwrap_or_default();
            return Err(ClaudeError::Exited {
                status,
                stderr: stderr.trim().to_owned(),
            });
        }
        playing.failure.map_or(Ok(()), Err)
    }
}

/// The efforts Claude Code's `--effort` takes, least first.
const CLAUDE_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Claude Code's model aliases, which follow the newest of each.
const CLAUDE_MODELS: [(&str, &str, &str); 5] = [
    ("fable", "Fable", "The most capable, for the hardest work"),
    ("opus", "Opus", "Very capable, and quicker than Fable"),
    ("sonnet", "Sonnet", "Fast and capable"),
    ("haiku", "Haiku", "The fastest and cheapest"),
    (
        "opusplan",
        "Opus Plan",
        "Opus to plan, Sonnet to carry it out",
    ),
];

impl Runner for ClaudeRunner {
    type Error = ClaudeError;

    async fn models(&self, _config: Option<String>) -> Result<Vec<crate::Model>, AgentError> {
        Ok(CLAUDE_MODELS
            .iter()
            .map(|(id, name, detail)| crate::Model {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                detail: Some((*detail).to_owned()),
                reads_images: true,
                // Haiku thinks as hard as it does; the others take an effort.
                efforts: if *id == "haiku" {
                    Vec::new()
                } else {
                    CLAUDE_EFFORTS
                        .iter()
                        .map(|&effort| effort.to_owned())
                        .collect()
                },
                default_effort: None,
            })
            .collect())
    }

    fn importable(&self) -> Result<Vec<Importable>, AgentError> {
        let Some(projects) = self.transcripts() else {
            return Ok(Vec::new());
        };
        import::claude_importable(&projects, &self.options.state)
    }

    fn backup(&self, record: &SessionRecord) -> Result<Vec<BackendFile>, AgentError> {
        let Some(links) = backup::links_file(&self.options.state, record.id.as_str())? else {
            return Ok(Vec::new());
        };
        let mut files = vec![links];
        let Some(projects) = self.transcripts() else {
            return Ok(files);
        };
        for session in backup::linked(&files)? {
            // A transcript that is gone is the backend's loss; the session
            // still comes back with its own history.
            let Ok(path) = import::claude_transcript(&projects, &session) else {
                continue;
            };
            if let Some(contents) = backup::read_optional(&path)? {
                files.push(BackendFile {
                    name: format!("{TRANSCRIPTS}/{session}.jsonl"),
                    contents,
                });
            }
        }
        Ok(files)
    }

    fn forget(&self, record: &SessionRecord) -> Result<(), AgentError> {
        let theirs = backup::forget_links(&self.options.state, record.id.as_str())?;
        let Some(projects) = self.transcripts() else {
            return Ok(());
        };
        for session in theirs {
            if let Ok(path) = import::claude_transcript(&projects, &session) {
                backup::remove(&path)?;
            }
        }
        Ok(())
    }

    fn restore(&self, record: &SessionRecord, files: &[BackendFile]) -> Result<(), AgentError> {
        // Claude Code finds a session's transcript in its working
        // directory's project folder, which is the restored one's.
        let project: String = record
            .working_directory
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        for file in files {
            if file.name == backup::LINKS {
                backup::write_links(&self.options.state, record.id.as_str(), &file.contents)?;
            } else if let Some(transcript) = file.name.strip_prefix(&format!("{TRANSCRIPTS}/")) {
                let Some(projects) = self.transcripts() else {
                    continue;
                };
                backup::write_under(&projects.join(&project), transcript, &file.contents)?;
            }
        }
        Ok(())
    }

    fn import(&self, external: &str, session: &SessionId) -> Result<Imported, AgentError> {
        let projects = self
            .transcripts()
            .ok_or_else(|| AgentError::UnknownImport {
                id: external.to_owned(),
            })?;
        import::import_into(&self.options.state, external, session, || {
            import::claude_read(&import::claude_transcript(&projects, external)?)
        })
    }

    async fn run_turn(self: Arc<Self>, turn: Turn) -> Result<(), ClaudeError> {
        self.play(&turn, false).await
    }

    async fn compaction(&self, _record: &SessionRecord) -> Compaction {
        Compaction::Supported { focus: true }
    }

    /// Has Claude Code run its own `/compact` on the conversation.
    async fn compact(self: Arc<Self>, turn: Turn) -> Result<(), ClaudeError> {
        self.play(&turn, true).await
    }

    fn forked(
        &self,
        parent: &crate::SessionId,
        fork: &SessionRecord,
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
        session: &SessionRecord,
        history: &[crate::Event],
        rewind: usize,
    ) -> Result<(), AgentError> {
        Links::rewind(&self.options.state, session.id.as_str(), history, rewind)
    }

    fn rewind_undone(&self, session: &SessionRecord, rewind: usize) -> Result<(), AgentError> {
        Links::undo_rewind(&self.options.state, session.id.as_str(), rewind)
    }
}

/// The end of what `stream` writes until it closes, as text.
async fn tail(mut stream: impl AsyncRead + Unpin) -> String {
    let mut kept = Vec::new();
    let mut buffer = [0; 1024];
    while let Ok(read) = stream.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        kept.extend_from_slice(&buffer[..read]);
        if kept.len() > STDERR_KEPT {
            kept.drain(..kept.len() - STDERR_KEPT);
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

/// A turn as Claude Code plays it.
struct Playing<'a> {
    turn: &'a Turn,
    /// None once the turn has ended.
    stdin: Option<ChildStdin>,
    links: Links,
    path: PathBuf,
    /// The Claude Code session the turn plays in; linked once it has said
    /// something, as only then can it be resumed.
    session: String,
    model: String,
    /// Messages sent, the turn's own and those steering it.
    sent: usize,
    /// Messages the CLI has taken, as it echoes them back.
    replayed: usize,
    /// What the CLI reported going wrong; the turn fails with it at its end.
    failure: Option<ClaudeError>,
    /// The CLI's permission mode, as last set.
    mode: &'static str,
    subagents: subagents::Subagents,
    /// For a turn that has the CLI sum up the conversation: whether it has
    /// said it did. None for a turn that replies to a message.
    compact: Option<bool>,
}

/// The CLI's command to sum up the conversation, keeping `focus` in mind;
/// on one line, as the command ends with its line.
fn compact_command(focus: &str) -> String {
    let focus = focus.split_whitespace().collect::<Vec<_>>().join(" ");
    if focus.is_empty() {
        "/compact".to_owned()
    } else {
        format!("/compact {focus}")
    }
}

/// The CLI's permission mode for `approval`. In auto its classifier lets
/// through what it finds safe and refuses the rest, and asks over the pipe
/// after several refusals; otherwise every tool asks over the pipe and the
/// approval mode answers.
fn permission_mode(approval: ApprovalMode) -> &'static str {
    match approval {
        ApprovalMode::Auto => "auto",
        ApprovalMode::AskEveryTime | ApprovalMode::AllowEdits | ApprovalMode::AllowEverything => {
            "manual"
        }
    }
}

/// The turn's images, each with its media type.
fn read_images(paths: &[PathBuf]) -> Result<Vec<(&'static str, Vec<u8>)>, ClaudeError> {
    paths
        .iter()
        .map(|path| {
            std::fs::read(path)
                .map(|data| (crate::images::media_type(path), data))
                .map_err(|source| ClaudeError::ReadImage {
                    path: path.clone(),
                    source,
                })
        })
        .collect()
}

impl Playing<'_> {
    async fn send(&mut self, text: &str) -> Result<(), ClaudeError> {
        self.send_with(text, &[]).await
    }

    /// Sends `text` with `images`, each a media type and its bytes.
    async fn send_with(
        &mut self,
        text: &str,
        images: &[(&'static str, Vec<u8>)],
    ) -> Result<(), ClaudeError> {
        let content = if images.is_empty() {
            json!(text)
        } else {
            let mut blocks: Vec<Value> = images
                .iter()
                .map(|(media_type, data)| {
                    json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": media_type,
                            "data": crate::images::base64(data),
                        },
                    })
                })
                .collect();
            if !text.is_empty() {
                blocks.push(json!({"type": "text", "text": text}));
            }
            Value::Array(blocks)
        };
        let message = json!({"type": "user", "message": {"role": "user", "content": content}});
        self.write(&message).await?;
        self.sent += 1;
        Ok(())
    }

    /// Switches the running CLI to `mode`, as the person changed the
    /// session's approvals mid-turn.
    async fn set_mode(&mut self, mode: &'static str) -> Result<(), ClaudeError> {
        let request = json!({"type": "control_request",
            "request_id": uuid::Uuid::new_v4().to_string(),
            "request": {"subtype": "set_permission_mode", "mode": mode},
        });
        self.write(&request).await?;
        self.mode = mode;
        Ok(())
    }

    async fn write(&mut self, message: &Value) -> Result<(), ClaudeError> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Ok(());
        };
        let mut line = message.to_string();
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|source| ClaudeError::Pipe { source })?;
        stdin
            .flush()
            .await
            .map_err(|source| ClaudeError::Pipe { source })
    }

    /// Records one of the CLI's events. True once the turn has ended.
    async fn handle(&mut self, event: &Value) -> Result<bool, ClaudeError> {
        // A subagent's own steps are kept under the tool call that started
        // it, apart from the chat.
        let tagged = self.subagents.take(event);
        if !event["parent_tool_use_id"].is_null() && event["type"] != "control_request" {
            self.record(tagged)?;
            return Ok(false);
        }
        match event["type"].as_str().unwrap_or_default() {
            "system" if event["subtype"] == "init" => {
                if let Some(model) = event["model"].as_str() {
                    model.clone_into(&mut self.model);
                }
                // The session it says it plays in is the one the next turn
                // resumes, should a version give a resumed one a new id.
                if let Some(session) = event["session_id"].as_str().filter(|id| !id.is_empty()) {
                    session.clone_into(&mut self.session);
                }
            }
            // Its context was summed up to make room.
            "system" if event["subtype"] == "compact_boundary" => {
                self.turn.emit(EventKind::Compacted)?;
                if let Some(summed) = &mut self.compact {
                    *summed = true;
                }
            }
            "stream_event" => {
                let delta = &event["event"]["delta"];
                if event["event"]["type"] == "content_block_delta" && delta["type"] == "text_delta"
                {
                    let text = delta["text"].as_str().unwrap_or_default();
                    if !text.is_empty() {
                        self.turn.emit(EventKind::AssistantDelta {
                            text: text.to_owned(),
                        })?;
                    }
                }
            }
            // The CLI's own message about a failed request; its result says
            // the same, as a failure.
            "assistant" if !event["error"].is_null() => {}
            "assistant" => {
                self.started()?;
                self.assistant(event)?;
            }
            "user" if event["isReplay"].as_bool() == Some(true) => self.replayed += 1,
            "user" => self.tool_results(event)?,
            "control_request" => self.control(event).await?,
            "result" => {
                let finished = self.subagents.finish();
                self.record(finished)?;
                return self.result(event);
            }
            _ => {}
        }
        self.record(tagged)?;
        Ok(false)
    }

    /// Records subagents' steps and news, each under its call.
    fn record(&self, tagged: Vec<subagents::Tagged>) -> Result<(), AgentError> {
        for (parent, kind) in tagged {
            self.turn.emit_under(Some(parent), kind)?;
        }
        Ok(())
    }

    /// Links the session once the CLI has it, so the next turn resumes it.
    fn started(&mut self) -> Result<(), AgentError> {
        if self.links.session.as_deref() == Some(self.session.as_str()) {
            return Ok(());
        }
        self.links.session = Some(self.session.clone());
        self.links.fork = None;
        self.links.save(&self.path)
    }

    fn assistant(&mut self, event: &Value) -> Result<(), AgentError> {
        let blocks = event["message"]["content"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        for block in blocks {
            let text = |field: &str| block[field].as_str().unwrap_or_default().to_owned();
            match block["type"].as_str().unwrap_or_default() {
                "text" if !text("text").is_empty() => {
                    let ours = self
                        .turn
                        .emit(EventKind::AssistantMessage { text: text("text") })?;
                    // A fork can resume up to a reply; not up to a tool call,
                    // as its result would be missing.
                    self.link(event, ours.id)?;
                }
                "thinking" if !text("thinking").is_empty() => {
                    self.turn.emit(EventKind::Reasoning {
                        text: text("thinking"),
                    })?;
                }
                "tool_use" => {
                    self.turn.emit(EventKind::ToolStarted {
                        call_id: text("id"),
                        tool: text("name"),
                        arguments_json: block["input"].to_string(),
                    })?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn tool_results(&mut self, event: &Value) -> Result<(), AgentError> {
        let blocks = event["message"]["content"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        for block in blocks.iter().filter(|block| block["type"] == "tool_result") {
            let output = match &block["content"] {
                Value::String(text) => text.clone(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            let ours = self.turn.emit(EventKind::ToolCompleted {
                call_id: block["tool_use_id"].as_str().unwrap_or_default().to_owned(),
                success: block["is_error"].as_bool() != Some(true),
                output,
            })?;
            self.link(event, ours.id)?;
        }
        Ok(())
    }

    fn link(&mut self, event: &Value, ours: crate::EventId) -> Result<(), AgentError> {
        let Some(theirs) = event["uuid"].as_str() else {
            return Ok(());
        };
        if self.links.session.is_none() {
            return Ok(());
        }
        self.links.link(&self.path, ours, theirs.to_owned())
    }

    /// Answers the CLI's request: whether a tool may run, asked of the
    /// person unless the approval mode allows it.
    async fn control(&mut self, event: &Value) -> Result<(), ClaudeError> {
        let request = &event["request"];
        let id = event["request_id"].clone();
        if request["subtype"] != "can_use_tool" {
            let subtype = request["subtype"].as_str().unwrap_or_default();
            let refused = json!({"type": "control_response", "response": {
                "subtype": "error",
                "request_id": id,
                "error": format!("Humata doesn't answer {subtype} requests"),
            }});
            return self.write(&refused).await;
        }
        let input = &request["input"];
        let field = |name: &str| input[name].as_str().map(str::to_owned);
        let tool = request["tool_name"].as_str().unwrap_or_default();
        let (kind, summary) = match tool {
            "Bash" => (PermissionKind::Shell, field("command")),
            "Edit" | "MultiEdit" | "Write" => (PermissionKind::Write, field("file_path")),
            "NotebookEdit" => (PermissionKind::Write, field("notebook_path")),
            "Read" => (PermissionKind::Read, field("file_path")),
            "Glob" | "Grep" => (PermissionKind::Read, field("pattern")),
            "WebFetch" => (PermissionKind::Other, field("url")),
            "WebSearch" => (PermissionKind::Other, field("query")),
            _ => (PermissionKind::Other, None),
        };
        let summary = summary
            .or_else(|| request["description"].as_str().map(str::to_owned))
            .or_else(|| request["display_name"].as_str().map(str::to_owned))
            .unwrap_or_else(|| tool.to_owned());
        let approved = self.turn.ask(kind, summary).await?;
        let answer = if approved {
            json!({"behavior": "allow", "updatedInput": input})
        } else {
            json!({"behavior": "deny", "message": DENIED})
        };
        let response = json!({"type": "control_response", "response": {
            "subtype": "success",
            "request_id": id,
            "response": answer,
        }});
        self.write(&response).await
    }

    /// Records the end of a reply. True if it ends the turn: it failed, the
    /// CLI has taken every message sent, or it was asked to sum up the
    /// conversation, which it may not echo back as a message.
    fn result(&mut self, event: &Value) -> Result<bool, ClaudeError> {
        let usage = &event["usage"];
        let count = |field: &str| usage[field].as_u64().unwrap_or(0);
        self.turn.emit(EventKind::Usage {
            model: self.model.clone(),
            input_tokens: count("input_tokens")
                + count("cache_creation_input_tokens")
                + count("cache_read_input_tokens"),
            output_tokens: count("output_tokens"),
            premium_requests: 0.0,
            cache_read_tokens: count("cache_read_input_tokens"),
            cache_write_tokens: count("cache_creation_input_tokens"),
        })?;
        if event["is_error"].as_bool() == Some(true) {
            let errors = event["errors"]
                .as_array()
                .map(|errors| {
                    errors
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .unwrap_or_default();
            let message = event["result"]
                .as_str()
                .filter(|result| !result.is_empty())
                .map_or(errors, str::to_owned);
            self.failure = Some(ClaudeError::Turn { message });
            return Ok(true);
        }
        self.started()?;
        let Some(summed) = self.compact else {
            return Ok(self.replayed >= self.sent);
        };
        if !summed {
            let said = event["result"]
                .as_str()
                .map(str::trim)
                .filter(|said| !said.is_empty())
                .unwrap_or("it said nothing more");
            self.failure = Some(ClaudeError::NotCompacted {
                said: said.to_owned(),
            });
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, SendMode, SessionState, StartOptions};
    use crate::runtime::tests::{idle_turns, wait_for};
    use crate::{Agent, LocalAgent};

    /// Replies to a message itself; asked to sum up, takes `written` as
    /// what the CLI wrote in answer to its `/compact`.
    struct Written {
        written: Vec<Value>,
        links: PathBuf,
    }

    impl Runner for Written {
        type Error = ClaudeError;

        async fn run_turn(self: Arc<Self>, turn: Turn) -> Result<(), ClaudeError> {
            turn.emit(EventKind::AssistantMessage { text: "hi".into() })?;
            Ok(())
        }

        async fn compaction(&self, _record: &SessionRecord) -> Compaction {
            Compaction::Supported { focus: true }
        }

        async fn compact(self: Arc<Self>, turn: Turn) -> Result<(), ClaudeError> {
            let mut playing = Playing {
                turn: &turn,
                stdin: None,
                links: Links::default(),
                path: self.links.clone(),
                session: "theirs".into(),
                model: String::new(),
                sent: 1,
                replayed: 0,
                failure: None,
                mode: permission_mode(turn.approval()),
                subagents: subagents::Subagents::default(),
                compact: Some(false),
            };
            let mut ended = false;
            for event in &self.written {
                if playing.handle(event).await? {
                    ended = true;
                    break;
                }
            }
            if !ended && playing.failure.is_none() {
                return Err(ClaudeError::Exited {
                    status: "the test's events ran out".into(),
                    stderr: String::new(),
                });
            }
            playing.failure.map_or(Ok(()), Err)
        }
    }

    /// A session's history after a turn and then a compaction for which
    /// the CLI wrote `written`.
    async fn compacted_with(written: Vec<Value>) -> Vec<Event> {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let runner = Written {
            written,
            links: dir.path().join("links").join("session.json"),
        };
        let agent = LocalAgent::open(dir.path().join("state"), runner).expect("the agent opens");
        let options = StartOptions {
            name: "test".into(),
            working_directory: dir.path().to_owned(),
            ..StartOptions::default()
        };
        let id = agent.start(options).await.expect("starts").id;
        agent
            .send(&id, "hello".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        wait_for(&agent, &id, idle_turns(1)).await;
        agent
            .compact(&id, String::new(), false)
            .await
            .expect("compacting");
        wait_for(&agent, &id, idle_turns(2)).await
    }

    fn result() -> Value {
        json!({"type": "result", "subtype": "success", "is_error": false, "result": "", "usage": {}})
    }

    #[tokio::test]
    async fn a_compaction_the_cli_reports_is_recorded_and_its_result_ends_the_turn() {
        let boundary = json!({"type": "system", "subtype": "compact_boundary"});

        let events = compacted_with(vec![boundary, result()]).await;

        assert!(
            events
                .iter()
                .any(|event| event.kind == EventKind::Compacted)
        );
        assert_eq!(SessionState::after(&events), SessionState::Idle);
    }

    #[tokio::test]
    async fn a_result_without_a_compaction_fails_the_turn_and_records_none() {
        let events = compacted_with(vec![result()]).await;

        assert!(
            !events
                .iter()
                .any(|event| event.kind == EventKind::Compacted)
        );
        assert_eq!(SessionState::after(&events), SessionState::Failed);
    }

    #[test]
    fn what_to_keep_in_mind_goes_on_the_commands_one_line() {
        assert_eq!(compact_command(""), "/compact");
        assert_eq!(
            compact_command("the API notes\nand the plan"),
            "/compact the API notes and the plan"
        );
    }
}
