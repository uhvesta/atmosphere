use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;

use crate::error::AgentError;

/// How to start and resume one kind of agent. In `start_args`,
/// `resume_args` and `conversation_file`, `{conversation}` is the pinned
/// conversation id (PER-12), `{folder}` the session folder, `{folder_dashed}`
/// the folder as Claude Code names its per-folder record
/// ([`humata2_conversation::claude_project_name`]: every character that
/// isn't an ASCII letter or digit replaced by `-`, cut and hashed past 200),
/// and `{home}` the home folder.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct AgentKind {
    pub name: String,
    pub program: PathBuf,
    pub start_args: Vec<String>,
    pub resume_args: Vec<String>,
    pub conversation_file: String,
    /// Where this agent's turn ends come from (SE-21, D169).
    #[serde(default)]
    pub turn_end: TurnEnd,
    /// What the agent's conversation file holds, so its turns can be read
    /// for a fork before one (FK-3). None unless the kind says otherwise.
    #[serde(default)]
    pub conversation_format: ConversationFormat,
    /// What a person sees for this kind (see [`AgentKind::person_name`]),
    /// when the kind names one; None uses the kind's `name`.
    #[serde(default)]
    display_name: Option<String>,
    /// The CLI's own dir when the daemon runs with it set (`CLAUDE_CONFIG_DIR`,
    /// `CODEX_HOME`): see [`Agents::with_cli_home`].
    #[serde(skip)]
    cli_home: Option<PathBuf>,
    /// The arguments that make `program` print its version and exit, such
    /// as `--version`; what it prints is kept with each launch (CV-3,
    /// [`AgentKind::cli_version`]). When the kind doesn't name them, its
    /// conversation format's (`ConversationFormat::version_args`), run
    /// after the kind's wrapper arguments ([`AgentKind::wrapper_args`]);
    /// `[]` asks none. What the kind names is the whole argument list
    /// after `program`: the override for a shape that can't be known.
    #[serde(default)]
    pub version_args: Option<Vec<String>>,
    /// Whether Humata picks the conversation's id or the CLI does and
    /// Humata finds it once written (Codex, D670).
    #[serde(default)]
    pub conversation_id: ConversationId,
    /// What puts the CLI on the session's chosen model, after its start and
    /// resume arguments, `{model}` standing for the model: `--model
    /// {model}` for every built-in kind. Added only when a model was
    /// chosen, so a session started on none runs the CLI's own default.
    /// Empty: the kind can't be told a model.
    #[serde(default)]
    model_args: Vec<String>,
    /// Which built-in CLI this kind is, for the arguments that give it
    /// Humata's MCP server and pre-allow its tools (D748); None for a kind
    /// from an agents file, which names `{humata_mcp}` where it wants the
    /// server's config file.
    #[serde(skip)]
    builtin: Option<Builtin>,
    /// Those arguments, filled by [`Agents::with_humata_tools`], after the
    /// start and resume arguments and before the model's. Empty until then
    /// and for every other kind.
    #[serde(skip)]
    tools_args: Vec<String>,
}

/// A built-in agent CLI Humata tells of its MCP server (D748).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Builtin {
    Claude,
    Codex,
    Copilot,
}

/// Where Humata's MCP server is for this daemon's agents (D748), as
/// `install_mcp_configs` wrote it into the state folder: the config files
/// Claude Code and Copilot load, and the value of Codex's `-c
/// mcp_servers.humata=…`. None holds the session's token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HumataTools {
    pub claude_mcp: PathBuf,
    pub copilot_mcp: PathBuf,
    pub codex_mcp: String,
}

impl Builtin {
    /// The arguments that load Humata's server for the session, on top of
    /// the person's own servers, and pre-allow its tools and the `humata`
    /// command, as each CLI's `--help` names them: Claude Code 2.1
    /// (`--mcp-config`, never `--strict-mcp-config`; `--allowedTools`),
    /// Copilot 1.0 (`--additional-mcp-config`, `--allow-tool`), Codex 0.169
    /// (`-c mcp_servers.humata=…`, its tools approved). Each flag and its
    /// value are one argument: the flags take many values, so a value
    /// after a space would take the next argument too.
    fn args(self, tools: &HumataTools) -> Vec<String> {
        match self {
            Self::Claude => vec![
                format!("--mcp-config={}", tools.claude_mcp.to_string_lossy()),
                "--allowedTools=mcp__humata,Bash(humata:*)".into(),
            ],
            Self::Codex => vec!["-c".into(), format!("mcp_servers.humata={}", tools.codex_mcp)],
            Self::Copilot => vec![
                format!("--additional-mcp-config=@{}", tools.copilot_mcp.to_string_lossy()),
                "--allow-tool=humata".into(),
                "--allow-tool=shell(humata:*)".into(),
            ],
        }
    }

    /// Whether `argv[at]` is one of [`Self::args`], as any daemon wrote
    /// them, and how many arguments it is: what a stored launch drops
    /// before taking this daemon's ([`Agents::with_tools_in`]).
    fn arg_at(argv: &[OsString], at: usize) -> usize {
        let text = |at: usize| argv.get(at).and_then(|arg| arg.to_str()).unwrap_or_default();
        let one = ["--mcp-config=", "--allowedTools=mcp__humata,", "--additional-mcp-config=", "--allow-tool=humata", "--allow-tool=shell(humata:"];
        if one.iter().any(|prefix| text(at).starts_with(prefix)) {
            return 1;
        }
        if text(at) == "-c" && text(at + 1).starts_with("mcp_servers.humata=") {
            return 2;
        }
        0
    }
}

/// How a kind's conversation id is known (D670).
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConversationId {
    /// Humata picks it and the CLI is started with it (`{conversation}`,
    /// PER-12): Claude's `--session-id`, Copilot's `--session-id`.
    #[default]
    Pinned,
    /// The CLI picks its own and can't be told one (Codex 0.169 has no
    /// flag for it). `{conversation}` and the conversation file stand for
    /// Humata's placeholder id until the CLI has written its conversation;
    /// then [`Agents::bind_found`] finds it by the folder the agent runs in,
    /// which no other session's agent runs in, and the launch takes its id.
    FoundInFolder,
}

/// How long an agent's CLI gets to print its version at launch: a CLI
/// that takes longer is launched with no version recorded.
const VERSION_DEADLINE: Duration = Duration::from_secs(5);
/// The most characters of a CLI's version line kept.
const VERSION_MAX: usize = 200;

/// The format of an agent's conversation file.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConversationFormat {
    /// Not read: the agent has no turns to fork before, only forks from now.
    #[default]
    None,
    /// Claude's JSONL, one JSON object per line, read by
    /// `humata2_conversation`'s Claude adapter, or line by line
    /// (`humata2_mirror::turns_of`) when kept where Claude Code doesn't.
    ClaudeJsonl,
    /// Codex's rollout files under its home (`CODEX_HOME`), carried by
    /// `humata2_conversation`'s Codex adapter.
    CodexRollout,
    /// Copilot's session-state folder under its home (`COPILOT_HOME`),
    /// carried by `humata2_conversation`'s Copilot adapter.
    CopilotSessionState,
}

impl ConversationFormat {
    /// The `humata2_conversation` format whose adapter reads and carries a
    /// conversation in this format; None when the agent has no file to read.
    pub fn adapter_format(self) -> Option<humata2_conversation::Format> {
        match self {
            Self::None => None,
            Self::ClaudeJsonl => Some(humata2_conversation::Format::ClaudeJsonl),
            Self::CodexRollout => Some(humata2_conversation::Format::CodexRollout),
            Self::CopilotSessionState => Some(humata2_conversation::Format::CopilotSessionState),
        }
    }

    /// The arguments that make the CLI writing this format print its
    /// version, as the conversation layout fixtures ask it: Copilot with
    /// `--no-auto-update`, so asking doesn't update it. None for a format
    /// no adapter reads.
    fn version_args(self) -> &'static [&'static str] {
        match self {
            Self::None => &[],
            Self::ClaudeJsonl | Self::CodexRollout => &["--version"],
            Self::CopilotSessionState => &["--no-auto-update", "--version"],
        }
    }
}

/// Why a conversation couldn't follow its folder ([`AgentKind::follow_folder`]).
#[derive(Debug, thiserror::Error)]
pub enum FollowError {
    #[error(
        "couldn't make room at {} to move conversation {id} to its agent's new folder; free the disk space or fix the permissions of that folder, then try again",
        path.display()
    )]
    Stage {
        id: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "conversation {id} is at {} but its agent's CLI doesn't find it there, so it can't follow the channel to the new folder; put the file back where the CLI keeps it, or remove it when it is no longer needed, then rename the channel again",
        file.display()
    )]
    Misplaced {
        id: String,
        file: PathBuf,
        #[source]
        source: Option<humata2_conversation::ConversationError>,
    },
    #[error(
        "conversation {id} couldn't be moved to its agent's new folder {}; its files name the old folder until it is, so check that they are there and readable by this user, then rename the channel again",
        to.display()
    )]
    Conversation {
        id: String,
        to: PathBuf,
        #[source]
        source: humata2_conversation::ConversationError,
    },
}

impl humata2_revive::CliVersions for Agents {
    fn current<'a>(&'a self, agent: &'a str) -> humata2_revive::VersionAsked<'a> {
        Box::pin(async move {
            match self.get(agent) {
                Ok(kind) => kind.cli_version().await,
                Err(_) => None,
            }
        })
    }

    /// D801 D: a conversation follows its agent's folder, as a rename does
    /// for an agent that was stopped (`follow_folder`), once the agent that
    /// kept running under the old folder is resumed.
    fn follow<'a>(&'a self, step: &'a humata2_revive::FollowStep) -> humata2_revive::Followed<'a> {
        Box::pin(async move {
            let (agents, step) = (self.clone(), step.clone());
            let refused = |reason: String| humata2_revive::FollowFailed { reason };
            tokio::task::spawn_blocking(move || {
                let kind = agents.get(&step.agent).map_err(|error| refused(error.to_string()))?;
                kind.follow_folder(&step.home, &step.id, &step.file, &step.from, &step.to, &step.scratch)
                    .map(|_| ())
                    .map_err(|error| refused(error.to_string()))
            })
            .await
            .map_err(|error| refused(format!("the task that moves the conversation stopped: {error}")))?
        })
    }
}

/// Why an agent's conversation gave no turns ([`AgentKind::turns`]).
#[derive(Debug, thiserror::Error)]
pub enum TurnsError {
    /// The conversation holds what Humata hasn't seen its CLI write (CV-3).
    #[error(
        "the conversation holds records this version of Humata doesn't know, so its turns can't be \
         read and it can't be cut or copied safely; update Humata, or fork and continue from a \
         session started with a CLI version Humata supports"
    )]
    Unsupported {
        #[source]
        source: humata2_conversation::ConversationError,
    },
    /// The conversation couldn't be read or doesn't hold what its CLI writes.
    #[error(
        "the conversation couldn't be read for its turns; check that its files are there and \
         readable by this user, then try again"
    )]
    Unreadable {
        #[source]
        source: humata2_conversation::ConversationError,
    },
    /// The conversation file couldn't be read.
    #[error(
        "couldn't read the conversation file {}; check that it is there and readable by this \
         user, then try again",
        path.display()
    )]
    File {
        path: PathBuf,
        #[source]
        source: humata2_mirror::MirrorError,
    },
}

/// Conversation `id` at `at`, when `adapter` finds its transcript at `file`,
/// where the session's agent record keeps it.
fn kept_at(
    adapter: &dyn humata2_conversation::Adapter,
    at: &humata2_conversation::Placement,
    id: &str,
    file: &Path,
) -> Option<humata2_conversation::Conversation> {
    adapter.locate(at, id).ok().filter(|found| {
        found
            .files
            .first()
            .is_some_and(|transcript| transcript.path == file)
    })
}

/// Where an agent's turn ends are learned from.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TurnEnd {
    /// The agent's own hooks (`{hooks}` in its args is the hooks file).
    Hooks,
    /// Output that has gone quiet (SE-22).
    #[default]
    Output,
    /// A plain program with no turns.
    None,
}

/// What `AgentKind::launch` fills in; maps onto `humata2_store::AgentLaunch`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    pub argv: Vec<OsString>,
    pub resume_argv: Vec<OsString>,
    pub conversation_file: PathBuf,
}

impl AgentKind {
    /// The name a person knows this agent by, for who made a version of a
    /// document: its display name when the kind sets one (OpenCode), else its
    /// CLI's own name ("Claude Code", "Codex", "Copilot"),
    /// "Terminal" for a plain program with no turns, else the kind's name.
    pub fn person_name(&self) -> String {
        if let Some(name) = &self.display_name {
            return name.clone();
        }
        if let Some(format) = self.conversation_format.adapter_format() {
            return format.to_string();
        }
        if self.turn_end == TurnEnd::None {
            return "Terminal".to_owned();
        }
        self.name.clone()
    }

    /// Whether this kind is a plain shell (zsh, bash, fish, …): a program
    /// with no turns and no conversation of its own.
    #[must_use]
    pub fn is_shell(&self) -> bool {
        self.turn_end == TurnEnd::None
    }

    /// The commands and conversation file for `conversation` in `folder`,
    /// with the CLI on its own default model.
    pub fn launch(&self, home: &Path, folder: &Path, conversation: &str) -> LaunchPlan {
        self.launch_with_model(home, folder, conversation, None)
    }

    /// [`Self::launch`] with the CLI started and resumed on `model`, when
    /// one was chosen ([`AgentKind::model_args`]). None or empty keeps the
    /// CLI's own default.
    pub fn launch_with_model(
        &self,
        home: &Path,
        folder: &Path,
        conversation: &str,
        model: Option<&str>,
    ) -> LaunchPlan {
        let model_args: Vec<OsString> = match model.filter(|model| !model.is_empty()) {
            Some(model) => self.model_args.iter().map(|arg| OsString::from(arg.replace("{model}", model))).collect(),
            None => Vec::new(),
        };
        let folder_text = folder.to_string_lossy();
        let fill = |t: &str| {
            t.replace("{conversation}", conversation)
                .replace("{folder_dashed}", &humata2_conversation::claude_project_name(folder))
                .replace("{folder}", &folder_text)
                .replace("{home}", &home.to_string_lossy())
        };
        let argv = |args: &[String]| {
            std::iter::once(self.program.clone().into_os_string())
                .chain(args.iter().map(|a| OsString::from(fill(a))))
                .chain(self.tools_args.iter().map(OsString::from))
                .chain(model_args.iter().cloned())
                .collect()
        };
        LaunchPlan {
            argv: argv(&self.start_args),
            resume_argv: argv(&self.resume_args),
            conversation_file: PathBuf::from(fill(&self.conversation_file)),
        }
    }

    /// The arguments a kind that wraps its CLI (`/usr/bin/env copilot ...`,
    /// `npx x ...`) puts between `program` and the CLI's own: the leading
    /// arguments `start_args` and `resume_args` share, up to the first
    /// that starts with `-` or holds a `{` (the CLI's flags and
    /// placeholders). Empty for a kind run directly.
    fn wrapper_args(&self) -> &[String] {
        let shared = self
            .start_args
            .iter()
            .zip(&self.resume_args)
            .take_while(|(start, resume)| start == resume && !start.starts_with('-') && !start.contains('{'))
            .count();
        &self.start_args[..shared]
    }

    /// What this kind's CLI prints first when run with its `version_args`,
    /// or its conversation format's when it names none, kept with a launch
    /// (CV-3) so its conversation's adapter checks it against what that
    /// version writes. None when there are no version arguments, or the CLI
    /// doesn't start, fails, prints nothing or takes longer than
    /// [`VERSION_DEADLINE`].
    pub async fn cli_version(&self) -> Option<String> {
        let (leading, args): (&[String], Vec<&str>) = match &self.version_args {
            Some(args) => (&[], args.iter().map(String::as_str).collect()),
            None => (self.wrapper_args(), self.conversation_format.version_args().to_vec()),
        };
        if args.is_empty() {
            return None;
        }
        let child = tokio::process::Command::new(&self.program)
            .args(leading)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .ok()?;
        let output = tokio::time::timeout(VERSION_DEADLINE, child.wait_with_output())
            .await
            .ok()?
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
        Some(line.chars().take(VERSION_MAX).collect())
    }

    /// The turns of conversation `id`, which the session's agent record
    /// keeps at `file`, read the way its fork and carry read it (CV-2):
    /// through its format's adapter (strictly, `Adapter::check`, given
    /// `cli_version`, the version the record kept from its launch) when the
    /// adapter finds it at `file`, else, for Claude's JSONL kept elsewhere,
    /// line by line as its byte copy cuts it. No turns while nothing is
    /// written yet; None for a kind whose turns can't be read. Blocking.
    pub fn turns(
        &self,
        home: &Path,
        folder: &Path,
        id: &str,
        file: &Path,
        cli_version: Option<&str>,
    ) -> Result<Option<Vec<humata2_conversation::Turn>>, TurnsError> {
        let (Some(format), Some(at)) = (
            self.conversation_format.adapter_format(),
            self.placement(home, folder),
        ) else {
            return Ok(None);
        };
        // A file that can't be looked at is read below, which says why.
        let written = std::fs::metadata(file).map_or_else(
            |error| error.kind() != std::io::ErrorKind::NotFound,
            |meta| meta.len() > 0,
        );
        if !written {
            return Ok(Some(Vec::new()));
        }
        let adapter = humata2_conversation::adapter(format);
        if let Some(found) = kept_at(adapter, &at, id, file) {
            let read = |source: humata2_conversation::ConversationError| match source {
                humata2_conversation::ConversationError::Unsupported { .. } => {
                    TurnsError::Unsupported { source }
                }
                source => TurnsError::Unreadable { source },
            };
            adapter.check(&found, cli_version).map_err(read)?;
            return adapter.turns(&found).map(Some).map_err(read);
        }
        if self.conversation_format != ConversationFormat::ClaudeJsonl {
            return Ok(None);
        }
        let marks = humata2_mirror::turns_of(file).map_err(|source| TurnsError::File {
            path: file.to_owned(),
            source,
        })?;
        Ok(Some(
            marks
                .into_iter()
                .map(|mark| humata2_conversation::Turn {
                    index: mark.index,
                    first_line: mark.first_line,
                    at: mark.at,
                })
                .collect(),
        ))
    }

    /// Where turn `turn` (from 1) of conversation `id`, which the session's
    /// agent record keeps at `file`, starts in `file`, in bytes: what came
    /// before it is what a fork before it holds. Read through the format's
    /// adapter when it finds the conversation at `file`, and for Claude's
    /// JSONL, whose fork keeps only the current path, line by line as its
    /// byte copy cuts it. None when the turn isn't there or the conversation
    /// can't be read. Blocking.
    pub fn turn_start(
        &self,
        home: &Path,
        folder: &Path,
        id: &str,
        file: &Path,
        turn: u32,
    ) -> Option<u64> {
        let position = usize::try_from(turn).ok()?.checked_sub(1)?;
        if self.conversation_format == ConversationFormat::ClaudeJsonl {
            let marks = humata2_mirror::turns_of(file).ok()?;
            return marks.into_iter().find(|mark| mark.index == turn).map(|mark| mark.offset);
        }
        let format = self.conversation_format.adapter_format()?;
        let at = self.placement(home, folder)?;
        let adapter = humata2_conversation::adapter(format);
        let found = kept_at(adapter, &at, id, file)?;
        adapter.turn_starts(&found).ok()?.get(position).copied()
    }

    /// The folder a Codex rollout or a Copilot transcript records for itself
    /// (`session_meta`'s `payload.cwd`, `session.start`'s `data.context.cwd`),
    /// read from its first lines; `None` for any other kind, or when the
    /// file records none.
    pub fn recorded_folder(&self, file: &Path) -> Option<PathBuf> {
        use std::io::BufRead;
        let format = self.conversation_format.adapter_format()?;
        if !matches!(format, humata2_conversation::Format::CodexRollout | humata2_conversation::Format::CopilotSessionState) {
            return None;
        }
        let lines = std::io::BufReader::new(std::fs::File::open(file).ok()?).lines();
        for line in lines.take(8).map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            let cwd = match value.get("type").and_then(serde_json::Value::as_str) {
                Some("session_meta") => value.pointer("/payload/cwd"),
                Some("session.start") => value.pointer("/data/context/cwd"),
                _ => None,
            };
            if let Some(cwd) = cwd.and_then(serde_json::Value::as_str) {
                return Some(PathBuf::from(cwd));
            }
        }
        None
    }

    /// MV-16: conversation `id`, which the session's agent record keeps at
    /// `file`, follows its agent's working folder from `from` to `to`. Its
    /// adapter places it again where it is, staged in `scratch`: it names
    /// `to` in the files that name `from` and, for Codex, in the CLI's
    /// thread row. `Ok(false)` when nothing is to follow: Claude's
    /// conversation moves with its key, a kind with no adapter, a
    /// conversation file that is gone. A file that is there but not where
    /// the adapter finds it is [`FollowError::Misplaced`]. Can run again.
    /// Blocking.
    pub fn follow_folder(
        &self,
        home: &Path,
        id: &str,
        file: &Path,
        from: &Path,
        to: &Path,
        scratch: &Path,
    ) -> Result<bool, FollowError> {
        if self.conversation_format == ConversationFormat::ClaudeJsonl {
            return Ok(false);
        }
        let Some(format) = self.conversation_format.adapter_format() else {
            return Ok(false);
        };
        let (Some(at), Some(there)) = (self.placement(home, from), self.placement(home, to)) else {
            return Ok(false);
        };
        let adapter = humata2_conversation::adapter(format);
        if matches!(std::fs::symlink_metadata(file), Err(error) if error.kind() == std::io::ErrorKind::NotFound) {
            return Ok(false);
        }
        let misplaced = |source| FollowError::Misplaced { id: id.to_owned(), file: file.to_owned(), source };
        let found = match adapter.locate(&at, id) {
            Ok(found) if found.files.first().is_some_and(|transcript| transcript.path == file) => found,
            Ok(_) => return Err(misplaced(None)),
            Err(source) => return Err(misplaced(Some(source))),
        };
        let stage = |source: std::io::Error| FollowError::Stage { id: id.to_owned(), path: scratch.to_owned(), source };
        match std::fs::remove_dir_all(scratch) {
            Err(source) if source.kind() != std::io::ErrorKind::NotFound => return Err(stage(source)),
            _ => {}
        }
        std::fs::create_dir_all(scratch).map_err(stage)?;
        let follow = |source: humata2_conversation::ConversationError| FollowError::Conversation {
            id: id.to_owned(),
            to: to.to_owned(),
            source,
        };
        let placed = adapter
            .pack(&found, scratch)
            .map_err(follow)
            .and_then(|packed| adapter.place(&packed, &at, &there).map_err(follow));
        let _ = std::fs::remove_dir_all(scratch);
        placed.map(|_| true)
    }

    /// Where this kind's conversations for `folder` are kept, as
    /// `humata2_conversation` names it (MV-16): `home`, the CLI's own dir
    /// when one is set for the kind (`Agents::with_cli_home`), and
    /// `folder`. None for a kind whose format no adapter knows.
    pub fn placement(&self, home: &Path, folder: &Path) -> Option<humata2_conversation::Placement> {
        self.conversation_format.adapter_format()?;
        Some(humata2_conversation::Placement {
            home: home.to_owned(),
            cli_home: self.cli_home.clone(),
            folder: folder.to_owned(),
        })
    }
}

/// OpenCode, run by the `humata opencode` wrapper (D201, D201a), which
/// starts its server, attaches its TUI to the conversation's session, and
/// reports turns and asks as the session's agent. It starts and resumes the
/// same way: the session is named after the conversation. Its conversation
/// lives in OpenCode's own database, so there is no file to read.
fn opencode() -> AgentKind {
    let args: Vec<String> = [
        "HUMATA_SOCKET={humata_socket}",
        "{humata}",
        "opencode",
        "--conversation",
        "{conversation}",
        "--state",
        "{state}/opencode/{conversation}.json",
    ]
    .map(String::from)
    .into();
    AgentKind {
        name: "opencode".into(),
        display_name: Some("OpenCode".into()),
        // Sets the socket the wrapper talks to, with no shell between.
        program: "/usr/bin/env".into(),
        start_args: args.clone(),
        resume_args: args,
        conversation_file: String::new(),
        turn_end: TurnEnd::Hooks,
        conversation_format: ConversationFormat::None,
        cli_home: None,
        version_args: None,
        conversation_id: ConversationId::Pinned,
        model_args: model_flag(),
        builtin: None,
        tools_args: Vec::new(),
    }
}

/// Codex (codex-cli 0.169.0), run by the `humata codex` wrapper (D795). The
/// wrapper reads Codex's app-server, names the thread as the conversation,
/// and passes asks to the Inbox. It hands Humata's hooks to plain Codex
/// (`-c hooks=…`, HOOKS, D193) only when the app-server can't be had. Codex
/// can't be started on a given id, so its conversation is found once written
/// (D670) and resumed by the id it wrote. Its rollout is under
/// `{home}/.codex/sessions/YYYY/MM/DD/`; until it is found the file named
/// here is one that is never there. The version check runs plain `codex`,
/// not the wrapper.
fn codex() -> AgentKind {
    let args: Vec<String> = [
        "HUMATA_SOCKET={humata_socket}",
        "{humata}",
        "codex",
        "--hooks",
        "{codex_hooks}",
        "--",
    ]
    .map(String::from)
    .into();
    let resume_args: Vec<String> = args
        .iter()
        .cloned()
        .chain(["resume".to_owned(), "{conversation}".to_owned()])
        .collect();
    AgentKind {
        name: "codex".into(),
        display_name: None,
        // Sets the socket the wrapper talks to, with no shell between.
        program: "/usr/bin/env".into(),
        start_args: args,
        resume_args,
        conversation_file: "{home}/.codex/sessions/{conversation}.jsonl".into(),
        turn_end: TurnEnd::Hooks,
        conversation_format: ConversationFormat::CodexRollout,
        cli_home: None,
        version_args: Some(vec!["codex".into(), "--version".into()]),
        conversation_id: ConversationId::FoundInFolder,
        model_args: model_flag(),
        builtin: Some(Builtin::Codex),
        tools_args: Vec::new(),
    }
}

/// GitHub Copilot CLI (1.0.113): `--session-id` starts a new conversation
/// on the given id and resumes it on a later run, and its hooks come as a
/// plugin folder (HOOKS, D193). Its `events.jsonl` is written once the
/// first prompt is taken.
fn copilot() -> AgentKind {
    let args = vec![
        "--session-id".into(),
        "{conversation}".into(),
        "--plugin-dir".into(),
        "{copilot_plugin}".into(),
    ];
    AgentKind {
        name: "copilot".into(),
        display_name: None,
        program: "copilot".into(),
        start_args: args.clone(),
        resume_args: args,
        conversation_file: "{home}/.copilot/session-state/{conversation}/events.jsonl".into(),
        turn_end: TurnEnd::Hooks,
        conversation_format: ConversationFormat::CopilotSessionState,
        cli_home: None,
        version_args: None,
        conversation_id: ConversationId::Pinned,
        model_args: model_flag(),
        builtin: Some(Builtin::Copilot),
        tools_args: Vec::new(),
    }
}

/// How every built-in agent's CLI is put on a model: `--model <model>` for
/// Claude Code, Codex (`codex` and `codex resume`), Copilot, and the `humata
/// opencode` wrapper, which hands it to OpenCode's TUI.
fn model_flag() -> Vec<String> {
    vec!["--model".into(), "{model}".into()]
}

/// The name a Terminal session's plain shell runs under. The app asks for it
/// by its command, `zsh`, whatever the login shell is (SPEC §1, D312).
const SHELL: &str = "zsh";

/// The agents a session can run.
#[derive(Clone, Debug)]
pub struct Agents {
    kinds: Vec<AgentKind>,
    /// The agent switches file (`agents.toml`) this daemon reads, when it
    /// has a home to keep one in.
    settings: Option<PathBuf>,
}

impl Agents {
    pub fn builtin() -> Self {
        Self {
            settings: None,
            kinds: vec![
                AgentKind {
                    name: "claude".into(),
                    program: "claude".into(),
                    start_args: vec![
                        "--session-id".into(),
                        "{conversation}".into(),
                        "--settings".into(),
                        "{hooks}".into(),
                    ],
                    resume_args: vec![
                        "--resume".into(),
                        "{conversation}".into(),
                        "--settings".into(),
                        "{hooks}".into(),
                    ],
                    conversation_file:
                        "{home}/.claude/projects/{folder_dashed}/{conversation}.jsonl".into(),
                    turn_end: TurnEnd::Hooks,
                    conversation_format: ConversationFormat::ClaudeJsonl,
                    display_name: None,
                    cli_home: None,
                    version_args: None,
                    conversation_id: ConversationId::Pinned,
                    model_args: model_flag(),
                    builtin: Some(Builtin::Claude),
                    tools_args: Vec::new(),
                },
                codex(),
                copilot(),
                opencode(),
            ],
        }
    }

    /// Names the agent switches file the daemon reads for which kinds are
    /// enabled and installed on demand.
    pub fn with_settings(mut self, path: PathBuf) -> Self {
        self.settings = Some(path);
        self
    }

    /// The agent switches file, when the daemon has one.
    pub fn settings_file(&self) -> Option<&Path> {
        self.settings.as_deref()
    }

    /// The kinds, in order.
    pub fn kinds(&self) -> &[AgentKind] {
        &self.kinds
    }

    /// Every `{hooks}` in a kind's `start_args` and `resume_args` becomes
    /// `file`, the settings file with the agent's turn hooks (D169).
    pub fn with_hooks(mut self, file: &Path) -> Self {
        let file = file.to_string_lossy();
        for kind in &mut self.kinds {
            for arg in kind.start_args.iter_mut().chain(&mut kind.resume_args) {
                *arg = arg.replace("{hooks}", &file);
            }
        }
        self
    }

    /// Every `{codex_hooks}` in a kind's args becomes `codex`, the value of
    /// Codex's `-c hooks=…`, and every `{copilot_plugin}` the folder of the
    /// plugin with Copilot's hooks (HOOKS, D193). Each is one argument, so
    /// no shell ever splits it.
    pub fn with_agent_hooks(mut self, codex: &str, copilot_plugin: &Path) -> Self {
        let plugin = copilot_plugin.to_string_lossy();
        for kind in &mut self.kinds {
            for arg in kind.start_args.iter_mut().chain(&mut kind.resume_args) {
                *arg = arg
                    .replace("{codex_hooks}", codex)
                    .replace("{copilot_plugin}", &plugin);
            }
        }
        self
    }

    /// Every `{humata}` in a kind's args becomes `humata`, the CLI beside
    /// the daemon, every `{humata_socket}` the daemon's socket, and every
    /// `{state}` its state folder: what the `humata opencode` wrapper is run
    /// with (D201a).
    pub fn with_wrapper(mut self, humata: &Path, socket: &Path, state: &Path) -> Self {
        let (humata, socket, state) = (
            humata.to_string_lossy(),
            socket.to_string_lossy(),
            state.to_string_lossy(),
        );
        for kind in &mut self.kinds {
            for arg in kind.start_args.iter_mut().chain(&mut kind.resume_args) {
                *arg = arg
                    .replace("{humata_socket}", &socket)
                    .replace("{humata}", &humata)
                    .replace("{state}", &state);
            }
        }
        self
    }

    /// Humata's MCP server and tools for every session's agent (D748): each
    /// built-in CLI is started and resumed with Humata's server loaded on
    /// top of the person's own, scoped to the session by the token in its
    /// environment, and its tools and the `humata` command pre-allowed
    /// ([`Builtin::args`]). A kind from an agents file gets the server
    /// where its args name `{humata_mcp}`: the config file Claude Code's
    /// format writes. Nothing goes into the agent's own config, and the
    /// plain shell gets none of it.
    pub fn with_humata_tools(mut self, tools: &HumataTools) -> Self {
        let file = tools.claude_mcp.to_string_lossy();
        for kind in &mut self.kinds {
            for arg in kind.start_args.iter_mut().chain(&mut kind.resume_args) {
                *arg = arg.replace("{humata_mcp}", &file);
            }
            if let Some(builtin) = kind.builtin {
                kind.tools_args = builtin.args(tools);
            }
        }
        self
    }

    /// Makes `launch`, saved by an earlier daemon, start and resume with
    /// this daemon's Humata tools arguments for its kind (D748): the ones
    /// it has, from any daemon, are dropped and this daemon's added after
    /// the rest. True when it changed. A kind with none leaves it alone.
    pub fn with_tools_in(&self, launch: &mut humata2_store::AgentLaunch) -> bool {
        let Ok(kind) = self.get(&launch.agent) else {
            return false;
        };
        if kind.tools_args.is_empty() {
            return false;
        }
        let mut changed = false;
        for argv in [&mut launch.argv, &mut launch.resume_argv] {
            if argv.is_empty() {
                continue;
            }
            let mut kept = Vec::with_capacity(argv.len() + kind.tools_args.len());
            let mut at = 0;
            while at < argv.len() {
                match Builtin::arg_at(argv, at) {
                    0 => {
                        kept.push(argv[at].clone());
                        at += 1;
                    }
                    n => at += n,
                }
            }
            kept.extend(kind.tools_args.iter().map(OsString::from));
            if kept != *argv {
                *argv = kept;
                changed = true;
            }
        }
        changed
    }

    /// The plain shell a Terminal session runs (D312): `program` with
    /// `args`, the shell new shell tabs run, with no turns and no
    /// conversation, so it comes back as a fresh shell in its folder. Added
    /// unless the kinds already name one.
    pub fn with_shell(mut self, program: &Path, args: &[OsString]) -> Self {
        if self.kinds.iter().any(|kind| kind.name == SHELL) {
            return self;
        }
        let args: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        self.kinds.push(AgentKind {
            name: SHELL.into(),
            program: program.to_owned(),
            start_args: args.clone(),
            resume_args: args,
            conversation_file: String::new(),
            turn_end: TurnEnd::None,
            conversation_format: ConversationFormat::None,
            display_name: None,
            cli_home: None,
            version_args: None,
            conversation_id: ConversationId::Pinned,
            model_args: Vec::new(),
            builtin: None,
            tools_args: Vec::new(),
        });
        self
    }

    /// Makes `dir` the CLI's own dir for every kind whose conversations are
    /// in `format` (Claude's `CLAUDE_CONFIG_DIR`, Codex's `CODEX_HOME`), as
    /// the daemon finds it set (MV-16): their conversation files are kept
    /// under it in place of the default dir in the home (`{home}/.claude`),
    /// and their placements name it.
    pub fn with_cli_home(mut self, format: ConversationFormat, dir: &Path) -> Self {
        for kind in self.kinds.iter_mut().filter(|kind| kind.conversation_format == format) {
            if let Some(rest) = kind.conversation_file.strip_prefix("{home}/.")
                && let Some((_, tail)) = rest.split_once('/')
            {
                kind.conversation_file = format!("{}/{tail}", dir.to_string_lossy());
            }
            kind.cli_home = Some(dir.to_owned());
        }
        self
    }

    /// A JSON list of kinds that replaces the built-ins.
    pub fn from_json(bytes: &[u8]) -> Result<Self, AgentError> {
        let kinds: Vec<AgentKind> = serde_json::from_slice(bytes).map_err(AgentError::Parse)?;
        for (i, k) in kinds.iter().enumerate() {
            if kinds[..i].iter().any(|o| o.name == k.name) {
                return Err(AgentError::Duplicate {
                    name: k.name.clone(),
                });
            }
        }
        Ok(Self {
            kinds,
            settings: None,
        })
    }

    pub fn get(&self, name: &str) -> Result<&AgentKind, AgentError> {
        self.kinds
            .iter()
            .find(|k| k.name == name)
            .ok_or_else(|| AgentError::Unknown {
                name: name.to_string(),
                known: self
                    .kinds
                    .iter()
                    .map(|k| k.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            })
    }

    /// The name a person knows agent `id` by (see [`AgentKind::person_name`]);
    /// the id itself when no kind has it.
    pub fn person_name(&self, id: &str) -> String {
        self.get(id)
            .map_or_else(|_| id.to_owned(), AgentKind::person_name)
    }

    /// Whether `agent`'s kind finds its conversation once written (D670)
    /// and `launch` hasn't found it yet: its conversation file isn't there.
    pub fn unbound(&self, launch: &humata2_store::AgentLaunch) -> bool {
        self.get(&launch.agent)
            .is_ok_and(|kind| kind.conversation_id == ConversationId::FoundInFolder)
            && !launch.conversation_file.is_file()
    }

    /// For an [`Self::unbound`] launch, finds the conversation its CLI
    /// wrote for the folder it runs in, newest first, passing over the ids
    /// in `taken` (other sessions' agents', such as a fork that shares the
    /// folder), and makes the launch name it: its id in place of the
    /// placeholder in the resume command, and its file. False, and the
    /// launch as it was, when none is written yet. Blocking: it reads the
    /// CLI's conversations folder.
    pub fn bind_found(
        &self,
        home: &Path,
        launch: &mut humata2_store::AgentLaunch,
        taken: &HashSet<String>,
    ) -> bool {
        if !self.unbound(launch) {
            return false;
        }
        let Ok(kind) = self.get(&launch.agent) else {
            return false;
        };
        let Some((id, file)) = kind.found_conversation(home, &launch.folder, taken) else {
            return false;
        };
        bind(launch, &id, file);
        true
    }

    /// Learns the conversation session `session`'s agent runs in, when its
    /// kind finds it in its folder (D670) and the CLI has written it since:
    /// saves the launch naming it, so a resume, fork or read finds that
    /// conversation. Ids other sessions' agents hold are passed over. True
    /// when it saved one; false when there was nothing to learn. Blocking.
    pub fn learn_conversation(
        &self,
        store: &humata2_store::Store,
        home: &Path,
        session: &str,
    ) -> Result<bool, humata2_store::StoreError> {
        let Some(agent) = store.agent(session)? else {
            return Ok(false);
        };
        if !self.unbound(&agent.launch) {
            return Ok(false);
        }
        let taken: HashSet<String> = store
            .agents()?
            .into_iter()
            .filter(|other| other.session != session)
            .map(|other| other.launch.conversation_id)
            .collect();
        let mut launch = agent.launch.clone();
        if !self.bind_found(home, &mut launch, &taken) {
            return Ok(false);
        }
        // Read again just before saving, so a state saved meanwhile is kept.
        let Some(mut now) = store.agent(session)? else {
            return Ok(false);
        };
        if now.launch.conversation_id != agent.launch.conversation_id {
            return Ok(false);
        }
        now.launch = launch;
        store.put_agent(&now)?;
        Ok(true)
    }

    /// Follows session `session`'s agent into conversation `id`, which its
    /// own CLI's hook says it is in now: Copilot's `/clear` and `/restart`
    /// start another conversation in the same running agent. Saves the
    /// launch naming `id`, in its commands and its file, so a resume, fork
    /// or read takes up that conversation. Nothing changes, and false, when
    /// the launch names `id` already; when `id`'s file isn't written yet (a
    /// resume needs it, and the CLI writes it by the end of the first turn,
    /// whose hook says it again); when another session's agent holds `id`;
    /// or when the launch's file doesn't name its id, so `id`'s file can't
    /// be told. Blocking: it reads the store and checks the file.
    pub fn follow_conversation(
        &self,
        store: &humata2_store::Store,
        session: &str,
        id: &str,
    ) -> Result<bool, humata2_store::StoreError> {
        let Some(agent) = store.agent(session)? else {
            return Ok(false);
        };
        let old = agent.launch.conversation_id.as_str();
        if old.is_empty() || old == id || self.get(&agent.launch.agent).is_err() {
            return Ok(false);
        }
        let Ok(kind) = self.get(&agent.launch.agent) else {
            return Ok(false);
        };
        let file = if kind.conversation_format == ConversationFormat::CodexRollout {
            // Codex names a rollout by its start time and keeps it in that
            // day's folder, so the old file's name can't be edited.
            codex_rollout(&agent.launch.conversation_file, id)
        } else {
            agent
                .launch
                .conversation_file
                .to_str()
                .filter(|file| file.contains(old))
                .map(|file| PathBuf::from(file.replace(old, id)))
        };
        let Some(file) = file else {
            return Ok(false);
        };
        if !file.is_file() {
            return Ok(false);
        }
        let taken = store
            .agents()?
            .iter()
            .any(|other| other.session != session && other.launch.conversation_id == id);
        if taken {
            return Ok(false);
        }
        let mut launch = agent.launch.clone();
        bind(&mut launch, id, file);
        // Read again just before saving, so a state saved meanwhile is kept.
        let Some(mut now) = store.agent(session)? else {
            return Ok(false);
        };
        if now.launch.conversation_id != agent.launch.conversation_id {
            return Ok(false);
        }
        // The conversation left behind stays the session's, as a start
        // fresh keeps it (PER-15), so it can still be read and found.
        if agent.launch.conversation_file.is_file() {
            let ended = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX));
            store.push_previous_conversation(session, old, &agent.launch.conversation_file, ended)?;
        }
        now.launch = launch;
        store.put_agent(&now)?;
        Ok(true)
    }
}

/// Makes `launch` name conversation `id`, kept at `file`: the placeholder id
/// it was launched with becomes `id` in its commands (D670).
fn bind(launch: &mut humata2_store::AgentLaunch, id: &str, file: PathBuf) {
    let old = std::mem::replace(&mut launch.conversation_id, id.to_owned());
    for arg in launch.argv.iter_mut().chain(&mut launch.resume_argv) {
        if let Some(text) = arg.to_str()
            && text.contains(old.as_str())
        {
            *arg = OsString::from(text.replace(old.as_str(), id));
        }
    }
    launch.conversation_file = file;
}

impl AgentKind {
    /// The conversation this kind's CLI wrote for `folder` that isn't in
    /// `taken`, newest first: its id and file. Only Codex's rollouts are
    /// found so (`<codex>/sessions/YYYY/MM/DD/rollout-*.jsonl`, whose first
    /// line, `session_meta`, names its `id` and `cwd`); None for any other
    /// kind or when none is written. Blocking.
    fn found_conversation(
        &self,
        home: &Path,
        folder: &Path,
        taken: &HashSet<String>,
    ) -> Option<(String, PathBuf)> {
        if self.conversation_id != ConversationId::FoundInFolder
            || self.conversation_format != ConversationFormat::CodexRollout
        {
            return None;
        }
        let codex = self.cli_home.clone().unwrap_or_else(|| home.join(".codex"));
        let want = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_owned());
        let mut found: Option<(std::time::SystemTime, String, PathBuf)> = None;
        // Newest day first: a conversation started today is in today's folder.
        for file in rollouts(&codex.join("sessions")) {
            let Some((id, cwd)) = rollout_meta(&file) else {
                continue;
            };
            let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
            if cwd != want || taken.contains(&id) {
                continue;
            }
            let modified = file
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if found.as_ref().is_none_or(|(at, _, _)| modified > *at) {
                found = Some((modified, id, file));
            }
        }
        found.map(|(_, id, file)| (id, file))
    }
}

/// The one rollout of Codex conversation `id` under the `sessions` folder
/// that holds `current` (its placeholder or bound file), found at year,
/// month, day depth without following symlinks. None when `id` isn't a
/// plain id (so it can't name a path or pattern), when there is no such
/// folder or rollout, when more than one matches, or when a folder can't
/// be read (the next hook names `id` again).
fn codex_rollout(current: &Path, id: &str) -> Option<PathBuf> {
    if id.is_empty()
        || id.len() > 128
        || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    let sessions = current.ancestors().find(|dir| dir.file_name().is_some_and(|name| name == "sessions"))?;
    let entries = |dir: &Path, want_dir: bool| -> Option<Vec<PathBuf>> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            if (want_dir && kind.is_dir()) || (!want_dir && kind.is_file()) {
                found.push(entry.path());
            }
        }
        Some(found)
    };
    let suffix = format!("-{id}.jsonl");
    let mut matches = Vec::new();
    for year in entries(sessions, true)? {
        for month in entries(&year, true)? {
            for day in entries(&month, true)? {
                matches.extend(entries(&day, false)?.into_iter().filter(|file| {
                    file.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(&suffix))
                }));
            }
        }
    }
    if matches.len() == 1 { matches.pop() } else { None }
}

/// Every `rollout-*.jsonl` three folders down in `sessions` (year, month,
/// day); none when it can't be read.
fn rollouts(sessions: &Path) -> Vec<PathBuf> {
    let dirs = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.is_dir())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut files = Vec::new();
    for year in dirs(sessions) {
        for month in dirs(&year) {
            for day in dirs(&month) {
                let Ok(entries) = std::fs::read_dir(&day) else {
                    continue;
                };
                files.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
                }));
            }
        }
    }
    files
}

/// The id and folder a Codex rollout's first line, its `session_meta`,
/// names; None when it isn't one.
fn rollout_meta(file: &Path) -> Option<(String, PathBuf)> {
    use std::io::BufRead;
    let mut line = String::new();
    std::io::BufReader::new(std::fs::File::open(file).ok()?)
        .read_line(&mut line)
        .ok()?;
    let value: serde_json::Value = serde_json::from_str(&line).ok()?;
    if value.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let payload = value.get("payload")?;
    let id = payload.get("id")?.as_str()?.to_owned();
    let cwd = PathBuf::from(payload.get("cwd")?.as_str()?);
    Some((id, cwd))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claude_conversation_in_a_long_folder_is_where_claude_code_keeps_it() {
        // X2, Claude Code 2.1.326: a folder over 200 units once dashed is
        // cut and named by its hash.
        let home = Path::new("/home/me");
        let folder = format!("/private/tmp/claude-501/h2x2fix/w/{}", "b".repeat(167));
        let agents = Agents::builtin();
        let claude = agents.get("claude").unwrap();

        assert_eq!(
            claude.launch(home, Path::new(&folder), "abc").conversation_file,
            home.join(".claude/projects")
                .join(format!("-private-tmp-claude-501-h2x2fix-w-{}-svwx2v", "b".repeat(166)))
                .join("abc.jsonl")
        );
    }

    #[test]
    fn a_claude_config_dir_set_for_the_daemon_is_where_claudes_conversations_are_kept() {
        let dir = Path::new("/state/claude-config");
        let (home, folder) = (Path::new("/home/me"), Path::new("/home/me/Humata/ch/sessions/s.1"));
        let agents = Agents::builtin().with_cli_home(ConversationFormat::ClaudeJsonl, dir);
        let claude = agents.get("claude").unwrap();

        assert_eq!(
            claude.launch(home, folder, "abc").conversation_file,
            dir.join("projects/-home-me-Humata-ch-sessions-s-1/abc.jsonl")
        );
        let placement = claude.placement(home, folder).unwrap();
        assert_eq!(placement.home, home);
        assert_eq!(placement.cli_home.as_deref(), Some(dir));
        assert_eq!(placement.folder, folder);
        let plain = Agents::builtin();
        assert_eq!(plain.get("claude").unwrap().placement(home, folder).unwrap().cli_home, None);
        assert!(plain.get("opencode").unwrap().placement(home, folder).is_none(), "no adapter knows it");
    }

    #[test]
    fn claude_pins_the_conversation() {
        let agents = Agents::builtin().with_hooks(Path::new("/state/claude-hooks.json"));
        let plan = agents.get("claude").unwrap().launch(
            Path::new("/home/me"),
            Path::new("/home/me/Humata/ch/sessions/s.1"),
            "abc",
        );
        assert_eq!(
            plan.argv,
            [
                "claude",
                "--session-id",
                "abc",
                "--settings",
                "/state/claude-hooks.json"
            ]
        );
        assert_eq!(
            plan.resume_argv,
            [
                "claude",
                "--resume",
                "abc",
                "--settings",
                "/state/claude-hooks.json"
            ]
        );
        assert_eq!(agents.get("claude").unwrap().turn_end, TurnEnd::Hooks);
        assert_eq!(
            plan.conversation_file,
            Path::new("/home/me/.claude/projects/-home-me-Humata-ch-sessions-s-1/abc.jsonl")
        );
    }

    #[test]
    fn a_kind_in_config_with_no_display_name_loads_and_is_named_by_its_name() {
        let json = br#"[{"name":"tool","program":"tool","start_args":[],
            "resume_args":[],"conversation_file":""}]"#;
        let agents = Agents::from_json(json).expect("an older config still loads");
        assert_eq!(agents.person_name("tool"), "tool");
    }

    #[test]
    fn opencode_runs_through_humatas_wrapper_on_this_daemon() {
        let agents = Agents::builtin().with_wrapper(
            Path::new("/opt/humata/humata"),
            Path::new("/state dir/humatad.sock"),
            Path::new("/state dir"),
        );
        let kind = agents.get("opencode").expect("OpenCode is built in");
        assert_eq!(kind.turn_end, TurnEnd::Hooks);
        let plan = kind.launch(Path::new("/home/me"), Path::new("/work"), "abc");
        assert_eq!(
            plan.argv,
            [
                "/usr/bin/env",
                "HUMATA_SOCKET=/state dir/humatad.sock",
                "/opt/humata/humata",
                "opencode",
                "--conversation",
                "abc",
                "--state",
                "/state dir/opencode/abc.json",
            ]
        );
        // Resuming names the same session.
        assert_eq!(plan.resume_argv, plan.argv);
        assert_eq!(kind.conversation_format, ConversationFormat::None);
    }

    #[test]
    fn a_terminal_runs_the_shell_with_no_turns_beside_an_agents_file() {
        let json = br#"[{"name":"fake","program":"/bin/fake","start_args":[],
            "resume_args":[],"conversation_file":""}]"#;
        let agents = Agents::from_json(json)
            .unwrap()
            .with_shell(Path::new("/bin/sh"), &["-l".into()]);
        let kind = agents.get(SHELL).expect("the Terminal's shell is an agent");
        assert_eq!(kind.turn_end, TurnEnd::None);
        assert_eq!(kind.conversation_format, ConversationFormat::None);
        let plan = kind.launch(Path::new("/h"), Path::new("/f"), "id");
        assert_eq!(plan.argv, ["/bin/sh", "-l"]);
        // It comes back as a fresh shell.
        assert_eq!(plan.resume_argv, plan.argv);
        assert!(agents.get("fake").is_ok());
    }

    #[test]
    fn an_agents_file_that_names_the_shell_keeps_its_own() {
        let json = br#"[{"name":"zsh","program":"/bin/zsh","start_args":["-f"],
            "resume_args":[],"conversation_file":""}]"#;
        let agents = Agents::from_json(json)
            .unwrap()
            .with_shell(Path::new("/bin/sh"), &[]);
        let plan = agents
            .get(SHELL)
            .unwrap()
            .launch(Path::new("/h"), Path::new("/f"), "id");
        assert_eq!(plan.argv, ["/bin/zsh", "-f"]);
    }

    #[test]
    fn an_agents_person_name_is_never_its_id() {
        let agents = Agents::builtin().with_shell(Path::new("/bin/sh"), &[]);
        let claude = agents.person_name("claude");
        assert_eq!(claude, humata2_conversation::Format::ClaudeJsonl.to_string());
        assert_ne!(claude, "claude");
        let shell = agents.person_name(SHELL);
        assert_ne!(shell, SHELL, "a plain terminal is not named for its shell");
        // OpenCode names its display name; a person sees that, not its id.
        let configured = agents
            .get("opencode")
            .unwrap()
            .display_name
            .clone()
            .expect("the builtin OpenCode kind has a display name");
        assert_eq!(agents.person_name("opencode"), configured);
        assert_ne!(agents.person_name("opencode"), "opencode");
        // An id no kind has is shown as it is.
        assert_eq!(agents.person_name("nope"), "nope");
    }

    #[test]
    fn codex_and_copilot_are_built_in_after_claude() {
        let agents = Agents::builtin().with_agent_hooks("{ Stop = [] }", Path::new("/state/copilot-plugin"));
        let names: Vec<&str> = agents.kinds().iter().map(|kind| kind.name.as_str()).collect();
        assert_eq!(names, ["claude", "codex", "copilot", "opencode"]);
        let (home, folder) = (Path::new("/home/me"), Path::new("/home/me/Humata/ch/sessions/s.1"));
        let copilot = agents.get("copilot").unwrap();
        let plan = copilot.launch(home, folder, "abc");
        assert_eq!(
            plan.argv,
            ["copilot", "--session-id", "abc", "--plugin-dir", "/state/copilot-plugin"]
        );
        assert_eq!(plan.resume_argv, plan.argv, "the same id starts and resumes it");
        assert_eq!(plan.conversation_file, home.join(".copilot/session-state/abc/events.jsonl"));
        assert_eq!(copilot.turn_end, TurnEnd::Hooks);
        // Codex runs under `humata codex` (D795), which reads its app-server
        // and hands Humata's hooks to plain Codex only when that fails.
        let wrapped = Agents::builtin()
            .with_agent_hooks("{ Stop = [] }", Path::new("/state/copilot-plugin"))
            .with_wrapper(Path::new("/bin/humata"), Path::new("/run/h.sock"), Path::new("/state"));
        let codex = wrapped.get("codex").unwrap();
        let plan = codex.launch(home, folder, "abc");
        let wrapper = ["/usr/bin/env", "HUMATA_SOCKET=/run/h.sock", "/bin/humata", "codex", "--hooks", "{ Stop = [] }", "--"];
        assert_eq!(plan.argv, wrapper);
        let resume: Vec<&str> = wrapper.iter().copied().chain(["resume", "abc"]).collect();
        assert_eq!(plan.resume_argv, resume);
        assert_eq!(codex.turn_end, TurnEnd::Hooks);
        assert_eq!(codex.conversation_format, ConversationFormat::CodexRollout);
        let moved = Agents::builtin()
            .with_cli_home(ConversationFormat::CopilotSessionState, Path::new("/state/copilot"));
        assert_eq!(
            moved.get("copilot").unwrap().launch(home, folder, "abc").conversation_file,
            Path::new("/state/copilot/session-state/abc/events.jsonl")
        );
    }

    /// The flag and value that follow each other in `argv`, when `flag` is there.
    fn value_after<'a>(argv: &'a [OsString], flag: &str) -> Option<&'a OsString> {
        argv.iter().position(|arg| arg == flag).and_then(|at| argv.get(at + 1))
    }

    #[test]
    fn a_chosen_model_reaches_every_builtin_agents_start_and_resume() {
        let agents = Agents::builtin();
        let (home, folder) = (Path::new("/home/me"), Path::new("/home/me/Humata/ch/sessions/s.1"));
        for kind in agents.kinds() {
            let plan = kind.launch_with_model(home, folder, "abc", Some("claude-haiku-5.5"));
            for argv in [&plan.argv, &plan.resume_argv] {
                assert_eq!(
                    value_after(argv, "--model").map(|model| model.to_string_lossy()),
                    Some("claude-haiku-5.5".into()),
                    "{}: {argv:?}",
                    kind.name
                );
            }
        }
    }

    #[test]
    fn no_model_flag_is_added_when_none_was_chosen() {
        let agents = Agents::builtin();
        let (home, folder) = (Path::new("/home/me"), Path::new("/work"));
        for kind in agents.kinds() {
            for model in [None, Some("")] {
                let plan = kind.launch_with_model(home, folder, "abc", model);
                assert_eq!(plan, kind.launch(home, folder, "abc"), "{}", kind.name);
                assert!(!plan.argv.iter().any(|arg| arg == "--model"), "{}: {:?}", kind.name, plan.argv);
            }
        }
    }

    #[test]
    fn an_agents_file_kind_gets_the_model_where_its_model_args_say() {
        let json = br#"[{"name":"x","program":"x","start_args":["--id","{conversation}"],
            "resume_args":["--resume","{conversation}"],"conversation_file":"f",
            "model_args":["-m","{model}"]}]"#;
        let agents = Agents::from_json(json).unwrap();
        let plan = agents.get("x").unwrap().launch_with_model(Path::new("/h"), Path::new("/f"), "id", Some("small"));
        assert_eq!(plan.argv, ["x", "--id", "id", "-m", "small"]);
        assert_eq!(plan.resume_argv, ["x", "--resume", "id", "-m", "small"]);
        // A kind that names no model arguments starts on its own default.
        let plain = Agents::from_json(br#"[{"name":"y","program":"y","start_args":[],"resume_args":[],"conversation_file":"f"}]"#)
            .unwrap();
        let plan = plain.get("y").unwrap().launch_with_model(Path::new("/h"), Path::new("/f"), "id", Some("small"));
        assert_eq!(plan.argv, ["y"]);
    }

    /// Writes a Codex rollout for conversation `id` run in `cwd` under
    /// `codex`'s sessions, as codex-cli 0.169.0 names it.
    fn rollout(codex: &Path, id: &str, cwd: &Path) -> PathBuf {
        let day = codex.join("sessions/2026/11/16");
        std::fs::create_dir_all(&day).unwrap();
        let file = day.join(format!("rollout-2026-11-16T10-00-00-{id}.jsonl"));
        let meta = serde_json::json!({
            "timestamp": "2026-11-16T10:00:00.000Z",
            "type": "session_meta",
            "payload": {"id": id, "session_id": id, "cwd": cwd, "cli_version": "0.169.0"},
        });
        std::fs::write(&file, format!("{meta}\n")).unwrap();
        file
    }

    /// A Codex session saved before its CLI wrote its conversation learns
    /// it on the next look, and keeps it; a Claude session is left as is.
    #[test]
    fn a_saved_codex_session_learns_its_conversation_once_written() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = humata2_store::Store::open(dir.path().join("state")).unwrap();
        let agents = Agents::builtin();
        let save = |id: &str, agent: &str| {
            let folder = home.join("Humata/ch/sessions").join(id);
            std::fs::create_dir_all(&folder).unwrap();
            store
                .put_session(&humata2_store::SessionRecord {
                    id: id.to_owned(),
                    channel: "desk".to_owned(),
                    machine: String::new(),
                    title: id.to_owned(),
                    agent: agent.to_owned(),
                    folder: folder.clone(),
                    created_unix_ms: 1,
                })
                .unwrap();
            store
                .put_agent(&humata2_store::AgentRecord {
                    session: id.to_owned(),
                    tab: format!("{id}_tab"),
                    launch: launch_of(&agents, agent, &home, &folder),
                    state: humata2_store::AgentState::Running { holder_pid: 7 },
                    generation: 1,
                    interruptions: Vec::new(),
                    updated_unix_ms: 1,
                })
                .unwrap();
            folder
        };
        let codex_folder = save("se_codex", "codex");
        save("se_claude", "claude");

        assert!(!agents.learn_conversation(&store, &home, "se_codex").unwrap());
        rollout(&home.join(".codex"), "019a-real", &codex_folder);
        assert!(agents.learn_conversation(&store, &home, "se_codex").unwrap());
        assert!(!agents.learn_conversation(&store, &home, "se_codex").unwrap());
        assert!(!agents.learn_conversation(&store, &home, "se_claude").unwrap());
        assert!(!agents.learn_conversation(&store, &home, "se_missing").unwrap());

        let saved = store.agent("se_codex").unwrap().unwrap();
        assert_eq!(saved.launch.conversation_id, "019a-real");
        assert!(saved.launch.resume_argv.iter().any(|arg| arg == "019a-real"));
        assert_eq!(saved.state, humata2_store::AgentState::Running { holder_pid: 7 });
    }

    fn launch_of(agents: &Agents, name: &str, home: &Path, folder: &Path) -> humata2_store::AgentLaunch {
        let plan = agents.get(name).unwrap().launch(home, folder, "placeholder-id");
        humata2_store::AgentLaunch {
            agent: name.into(),
            folder: folder.to_owned(),
            argv: plan.argv,
            resume_argv: plan.resume_argv,
            variable_names: Vec::new(),
            conversation_id: "placeholder-id".into(),
            conversation_file: plan.conversation_file,
            holder_socket: PathBuf::from("/nowhere.sock"),
            cols: 80,
            rows: 24,
            cli_version: None,
        }
    }

    #[test]
    fn a_codex_conversation_is_found_by_its_folder_once_written_and_resumed_by_its_id() {
        let home = tempfile::tempdir().unwrap();
        let folder = home.path().join("Humata/ch/sessions/s.1");
        let other = home.path().join("Humata/ch/sessions/s.2");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let agents = Agents::builtin();
        let mut launch = launch_of(&agents, "codex", home.path(), &folder);
        let none = HashSet::new();
        assert!(agents.unbound(&launch));
        assert!(!agents.bind_found(home.path(), &mut launch, &none), "nothing written yet");
        assert_eq!(launch.conversation_id, "placeholder-id");

        let codex = home.path().join(".codex");
        rollout(&codex, "0000-other-folder", &other);
        let file = rollout(&codex, "1111-this-folder", &folder);
        assert!(agents.bind_found(home.path(), &mut launch, &none));
        assert_eq!(launch.conversation_id, "1111-this-folder");
        assert_eq!(launch.conversation_file, file);
        assert!(launch.resume_argv.iter().any(|arg| arg == "1111-this-folder"));
        assert!(!launch.resume_argv.iter().any(|arg| arg == "placeholder-id"));
        assert!(!agents.unbound(&launch), "a found conversation stays found");
        assert!(!agents.bind_found(home.path(), &mut launch, &none));
    }

    #[test]
    fn a_codex_conversation_another_session_has_is_passed_over() {
        let home = tempfile::tempdir().unwrap();
        let folder = home.path().join("work");
        std::fs::create_dir_all(&folder).unwrap();
        let agents = Agents::builtin();
        let mut launch = launch_of(&agents, "codex", home.path(), &folder);
        rollout(&home.path().join(".codex"), "2222-a-fork", &folder);
        let taken = HashSet::from(["2222-a-fork".to_owned()]);
        assert!(!agents.bind_found(home.path(), &mut launch, &taken));
        assert!(agents.unbound(&launch));
    }

    #[test]
    fn a_pinned_kind_is_never_rebound() {
        let home = tempfile::tempdir().unwrap();
        let folder = home.path().join("work");
        std::fs::create_dir_all(&folder).unwrap();
        let agents = Agents::builtin();
        rollout(&home.path().join(".codex"), "3333", &folder);
        for name in ["claude", "copilot"] {
            let mut launch = launch_of(&agents, name, home.path(), &folder);
            assert!(!agents.unbound(&launch));
            assert!(!agents.bind_found(home.path(), &mut launch, &HashSet::new()));
            assert_eq!(launch.conversation_id, "placeholder-id");
        }
    }

    #[test]
    fn unknown_agent_is_an_error() {
        assert!(matches!(
            Agents::builtin().get("nope"),
            Err(AgentError::Unknown { .. })
        ));
    }

    #[test]
    fn json_replaces_builtins() {
        let json =
            br#"[{"name":"fake","program":"/bin/fake","start_args":["{folder}","{conversation}"],
            "resume_args":[],"conversation_file":"{folder}/c-{conversation}"}]"#;
        let agents = Agents::from_json(json).unwrap();
        assert!(agents.get("claude").is_err());
        let plan = agents
            .get("fake")
            .unwrap()
            .launch(Path::new("/h"), Path::new("/f"), "id");
        assert_eq!(plan.argv, ["/bin/fake", "/f", "id"]);
        assert_eq!(plan.resume_argv, ["/bin/fake"]);
        assert_eq!(plan.conversation_file, Path::new("/f/c-id"));
    }

    #[test]
    fn turn_end_defaults_to_output_and_is_read_from_json() {
        let json = br#"[{"name":"a","program":"p","start_args":["{hooks}"],"resume_args":[],
            "conversation_file":"x","turn_end":"hooks"},
            {"name":"b","program":"p","start_args":[],"resume_args":[],"conversation_file":"x"}]"#;
        let agents = Agents::from_json(json)
            .unwrap()
            .with_hooks(Path::new("/h.json"));
        assert_eq!(agents.get("a").unwrap().turn_end, TurnEnd::Hooks);
        assert_eq!(agents.get("a").unwrap().start_args, ["/h.json"]);
        assert_eq!(agents.get("b").unwrap().turn_end, TurnEnd::Output);
    }

    #[test]
    fn codex_and_copilot_hooks_fill_their_placeholders() {
        let json =
            br#"[{"name":"codex","program":"codex","start_args":["-c","hooks={codex_hooks}"],
            "resume_args":["resume","-c","hooks={codex_hooks}"],"conversation_file":"x"},
            {"name":"copilot","program":"copilot","start_args":["--plugin-dir","{copilot_plugin}"],
            "resume_args":[],"conversation_file":"x"}]"#;
        let agents = Agents::from_json(json)
            .unwrap()
            .with_agent_hooks("{ Stop = [] }", Path::new("/state dir/copilot-plugin"));
        let codex = agents.get("codex").unwrap();
        assert_eq!(codex.start_args, ["-c", "hooks={ Stop = [] }"]);
        assert_eq!(codex.resume_args, ["resume", "-c", "hooks={ Stop = [] }"]);
        assert_eq!(
            agents.get("copilot").unwrap().start_args,
            ["--plugin-dir", "/state dir/copilot-plugin"]
        );
    }

    #[test]
    fn conversation_format_is_read_from_json_and_is_none_by_default() {
        let json = br#"[{"name":"a","program":"p","start_args":[],"resume_args":[],
            "conversation_file":"x","conversation_format":"claude-jsonl"},
            {"name":"b","program":"p","start_args":[],"resume_args":[],"conversation_file":"x"}]"#;
        let agents = Agents::from_json(json).unwrap();
        assert_eq!(agents.get("a").unwrap().conversation_format, ConversationFormat::ClaudeJsonl);
        assert_eq!(agents.get("b").unwrap().conversation_format, ConversationFormat::None);
        assert_eq!(
            Agents::builtin().get("claude").unwrap().conversation_format,
            ConversationFormat::ClaudeJsonl
        );
    }

    #[test]
    fn bad_or_duplicate_json_errors() {
        assert!(Agents::from_json(b"nope").is_err());
        let k = r#"{"name":"a","program":"p","start_args":[],"resume_args":[],"conversation_file":"x"}"#;
        assert!(Agents::from_json(format!("[{k},{k}]").as_bytes()).is_err());
    }

    /// A kind run by `/bin/sh`, asked its version with `version_args`.
    fn shell_kind(version_args: &[&str]) -> AgentKind {
        let json = serde_json::json!([{
            "name": "v",
            "program": "/bin/sh",
            "start_args": [],
            "resume_args": [],
            "conversation_file": "x",
            "version_args": version_args,
        }]);
        let agents = Agents::from_json(json.to_string().as_bytes()).unwrap();
        agents.get("v").unwrap().clone()
    }

    #[tokio::test]
    async fn a_launch_keeps_the_first_line_the_cli_prints_for_its_version() {
        let kind = shell_kind(&[
            "-c",
            "echo; echo '  GitHub Copilot CLI 1.0.112-0.  '; echo 'Run copilot update'",
        ]);
        assert_eq!(kind.cli_version().await.as_deref(), Some("GitHub Copilot CLI 1.0.112-0."));
    }

    #[tokio::test]
    async fn no_version_is_kept_when_the_kind_asks_none_or_the_cli_fails_or_prints_nothing() {
        assert_eq!(shell_kind(&[]).cli_version().await, None);
        assert_eq!(shell_kind(&["-c", "echo 1.0.112; exit 3"]).cli_version().await, None);
        assert_eq!(shell_kind(&["-c", ":"]).cli_version().await, None);
    }

    /// `testdata/fake_cli.sh` (BUILD's `HUMATA2_FAKE_CLI`): prints what
    /// Codex does for `--version` and what Copilot does for
    /// `--no-auto-update --version`, and fails for anything else.
    fn fake_cli() -> PathBuf {
        std::env::var_os("HUMATA2_FAKE_CLI")
            .map(PathBuf::from)
            .expect("set HUMATA2_FAKE_CLI to the sessions testdata/fake_cli.sh")
    }

    /// A kind an agents file names, run by `program`, whose conversation
    /// is in `format`, naming `version_args` when given.
    fn file_kind(program: &Path, format: &str, version_args: Option<&[&str]>) -> AgentKind {
        let mut kind = serde_json::json!({
            "name": "v",
            "program": program,
            "start_args": [],
            "resume_args": [],
            "conversation_file": "x",
            "conversation_format": format,
        });
        if let Some(args) = version_args {
            kind["version_args"] = args.into();
        }
        let json = serde_json::json!([kind]).to_string();
        Agents::from_json(json.as_bytes()).unwrap().get("v").unwrap().clone()
    }

    #[tokio::test]
    async fn a_codex_or_copilot_kind_naming_no_version_args_records_what_its_cli_says() {
        let cli = fake_cli();
        let codex = file_kind(&cli, "codex-rollout", None);
        assert_eq!(codex.cli_version().await.as_deref(), Some("codex-cli 0.169.0"));
        let copilot = file_kind(&cli, "copilot-session-state", None);
        assert_eq!(copilot.cli_version().await.as_deref(), Some("GitHub Copilot CLI 1.0.112-0."));
    }

    #[tokio::test]
    async fn version_args_the_agents_file_names_win_and_a_kind_with_no_format_asks_none() {
        let cli = fake_cli();
        let named = file_kind(&cli, "copilot-session-state", Some(&["--version"]));
        assert_eq!(named.cli_version().await.as_deref(), Some("codex-cli 0.169.0"));
        assert_eq!(file_kind(&cli, "copilot-session-state", Some(&[])).cli_version().await, None);
        assert_eq!(file_kind(&cli, "none", None).cli_version().await, None);
    }

    fn tools() -> HumataTools {
        HumataTools {
            claude_mcp: PathBuf::from("/state dir/humata-mcp-claude.json"),
            copilot_mcp: PathBuf::from("/state dir/humata-mcp-copilot.json"),
            codex_mcp: "{ command = \"/opt/humatad2\" }".into(),
        }
    }

    /// Whether `argv` holds `want`, one after the other.
    fn holds(argv: &[OsString], want: &[&str]) -> bool {
        argv.windows(want.len()).any(|window| window.iter().zip(want).all(|(arg, want)| arg == want))
    }

    #[test]
    fn every_builtin_cli_starts_and_resumes_with_humatas_mcp_server_and_its_tools_allowed() {
        let agents = Agents::builtin().with_humata_tools(&tools());
        let (home, folder) = (Path::new("/home/me"), Path::new("/home/me/Humata/ch/sessions/s.1"));
        let want: [(&str, &[&str]); 3] = [
            (
                "claude",
                &["--mcp-config=/state dir/humata-mcp-claude.json", "--allowedTools=mcp__humata,Bash(humata:*)"],
            ),
            ("codex", &["-c", "mcp_servers.humata={ command = \"/opt/humatad2\" }"]),
            (
                "copilot",
                &[
                    "--additional-mcp-config=@/state dir/humata-mcp-copilot.json",
                    "--allow-tool=humata",
                    "--allow-tool=shell(humata:*)",
                ],
            ),
        ];
        for (name, args) in want {
            let plan = agents.get(name).unwrap().launch_with_model(home, folder, "abc", Some("m"));
            for argv in [&plan.argv, &plan.resume_argv] {
                assert!(holds(argv, args), "{name}: {argv:?}");
                assert!(!argv.iter().any(|arg| arg == "--strict-mcp-config"), "{name} keeps the person's own servers");
                assert_eq!(value_after(argv, "--model").map(|m| m.to_string_lossy()), Some("m".into()), "{name}");
            }
        }
    }

    #[test]
    fn the_plain_shell_and_an_agents_file_kind_with_no_slot_get_no_mcp_server() {
        let json = br#"[{"name":"tool","program":"tool","start_args":["--go"],
            "resume_args":["--go"],"conversation_file":""}]"#;
        let agents = Agents::from_json(json)
            .unwrap()
            .with_shell(Path::new("/bin/zsh"), &["-l".into()])
            .with_humata_tools(&tools());
        let (home, folder) = (Path::new("/h"), Path::new("/f"));
        assert_eq!(agents.get(SHELL).unwrap().launch(home, folder, "id").argv, ["/bin/zsh", "-l"]);
        assert_eq!(agents.get("tool").unwrap().launch(home, folder, "id").argv, ["tool", "--go"]);
        let builtin = Agents::builtin().with_shell(Path::new("/bin/zsh"), &[]).with_humata_tools(&tools());
        assert_eq!(builtin.get(SHELL).unwrap().launch(home, folder, "id").argv, ["/bin/zsh"]);
    }

    #[test]
    fn an_agents_file_kind_gets_the_mcp_server_where_it_names_the_slot() {
        let json = br#"[{"name":"tool","program":"tool","start_args":["--mcp","{humata_mcp}"],
            "resume_args":["--resume","{conversation}","--mcp={humata_mcp}"],"conversation_file":""}]"#;
        let agents = Agents::from_json(json).unwrap().with_humata_tools(&tools());
        let plan = agents.get("tool").unwrap().launch(Path::new("/h"), Path::new("/f"), "id");
        assert_eq!(plan.argv, ["tool", "--mcp", "/state dir/humata-mcp-claude.json"]);
        assert_eq!(plan.resume_argv, ["tool", "--resume", "id", "--mcp=/state dir/humata-mcp-claude.json"]);
    }

    #[test]
    fn a_launch_saved_before_the_tools_resumes_with_them_once() {
        let (home, folder) = (Path::new("/home/me"), Path::new("/work"));
        let before = Agents::builtin();
        let now = Agents::builtin().with_humata_tools(&tools());
        for name in ["claude", "codex", "copilot"] {
            let mut launch = launch_of(&before, name, home, folder);
            let fresh = launch_of(&now, name, home, folder);
            assert!(now.with_tools_in(&mut launch), "{name}");
            assert_eq!(launch.resume_argv, fresh.resume_argv, "{name}");
            assert_eq!(launch.argv, fresh.argv, "{name}");
            assert!(!now.with_tools_in(&mut launch), "{name}: a second pass changes nothing");
            // A daemon whose state folder moved replaces the old paths.
            let moved = Agents::builtin().with_humata_tools(&HumataTools {
                claude_mcp: PathBuf::from("/new/c.json"),
                copilot_mcp: PathBuf::from("/new/p.json"),
                codex_mcp: "{ command = \"/new/humatad2\" }".into(),
            });
            assert!(moved.with_tools_in(&mut launch), "{name}");
            assert_eq!(launch.resume_argv, launch_of(&moved, name, home, folder).resume_argv, "{name}");
        }
        let shell = Agents::builtin().with_shell(Path::new("/bin/sh"), &[]).with_humata_tools(&tools());
        let mut launch = launch_of(&shell, SHELL, home, folder);
        assert!(!shell.with_tools_in(&mut launch), "the shell has none to take");
    }

    /// Saves a running Codex session whose launch is `launch`.
    fn save_codex(store: &humata2_store::Store, id: &str, launch: humata2_store::AgentLaunch) {
        store
            .put_session(&humata2_store::SessionRecord {
                id: id.to_owned(),
                channel: "desk".to_owned(),
                machine: String::new(),
                title: id.to_owned(),
                agent: "codex".to_owned(),
                folder: launch.folder.clone(),
                created_unix_ms: 1,
            })
            .unwrap();
        store
            .put_agent(&humata2_store::AgentRecord {
                session: id.to_owned(),
                tab: format!("{id}_tab"),
                launch,
                state: humata2_store::AgentState::Running { holder_pid: 7 },
                generation: 1,
                interruptions: Vec::new(),
                updated_unix_ms: 1,
            })
            .unwrap();
    }

    /// Writes a Codex rollout for `id` in day folder `day`, as codex-cli
    /// 0.169.0 names a conversation started that day.
    fn rollout_on(codex: &Path, day: &str, id: &str, cwd: &Path) -> PathBuf {
        let folder = codex.join("sessions").join(day);
        std::fs::create_dir_all(&folder).unwrap();
        let stamp = day.replace('/', "-");
        let file = folder.join(format!("rollout-{stamp}T09-30-00-{id}.jsonl"));
        let meta = serde_json::json!({
            "type": "session_meta",
            "payload": {"id": id, "cwd": cwd, "cli_version": "0.169.0"},
        });
        std::fs::write(&file, format!("{meta}\n")).unwrap();
        file
    }

    /// Codex's `/new` starts a conversation whose rollout is in another day
    /// folder under another name: following it finds that file by its id,
    /// and resume takes it up (AGENTS2, D799).
    #[test]
    fn a_codex_conversation_is_followed_by_id_into_a_later_day_folder() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let folder = home.join("Humata/ch/sessions/s.1");
        std::fs::create_dir_all(&folder).unwrap();
        let store = humata2_store::Store::open(dir.path().join("state")).unwrap();
        let agents = Agents::builtin();
        let codex = home.join(".codex");
        let mut launch = launch_of(&agents, "codex", &home, &folder);
        rollout_on(&codex, "2026/11/16", "1111-first", &folder);
        assert!(agents.bind_found(&home, &mut launch, &HashSet::new()));
        save_codex(&store, "se_codex", launch);

        // Named before its rollout is written: nothing changes yet.
        assert!(!agents.follow_conversation(&store, "se_codex", "2222-after-new").unwrap());
        let file = rollout_on(&codex, "2026/11/18", "2222-after-new", &folder);
        assert!(agents.follow_conversation(&store, "se_codex", "2222-after-new").unwrap());

        let saved = store.agent("se_codex").unwrap().unwrap();
        assert_eq!(saved.launch.conversation_id, "2222-after-new");
        assert_eq!(saved.launch.conversation_file, file);
        assert!(saved.launch.resume_argv.iter().any(|arg| arg == "2222-after-new"));
        assert!(!saved.launch.resume_argv.iter().any(|arg| arg == "1111-first"));
    }

    /// The thread a fresh Codex session names before any folder scan binds
    /// it, so a session started under the wrapper needs no scan (D795).
    #[test]
    fn an_unbound_codex_session_is_bound_by_the_thread_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let folder = home.join("Humata/ch/sessions/s.1");
        std::fs::create_dir_all(&folder).unwrap();
        let store = humata2_store::Store::open(dir.path().join("state")).unwrap();
        let agents = Agents::builtin();
        save_codex(&store, "se_codex", launch_of(&agents, "codex", &home, &folder));
        let file = rollout_on(&home.join(".codex"), "2026/11/18", "3333-thread", &folder);

        assert!(agents.follow_conversation(&store, "se_codex", "3333-thread").unwrap());
        let saved = store.agent("se_codex").unwrap().unwrap();
        assert_eq!(saved.launch.conversation_id, "3333-thread");
        assert_eq!(saved.launch.conversation_file, file);
        assert!(!agents.unbound(&saved.launch));
    }

    /// An id that could name a path, or match more than one rollout, is
    /// never followed.
    #[test]
    fn a_codex_id_that_could_name_a_path_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let folder = home.join("Humata/ch/sessions/s.1");
        std::fs::create_dir_all(&folder).unwrap();
        let store = humata2_store::Store::open(dir.path().join("state")).unwrap();
        let agents = Agents::builtin();
        save_codex(&store, "se_codex", launch_of(&agents, "codex", &home, &folder));
        rollout_on(&home.join(".codex"), "2026/11/18", "4444-real", &folder);
        for id in ["../4444-real", "*", "4444-*", "2026/11/18/rollout"] {
            assert!(!agents.follow_conversation(&store, "se_codex", id).unwrap(), "{id:?}");
        }
        let saved = store.agent("se_codex").unwrap().unwrap();
        assert_eq!(saved.launch.conversation_id, "placeholder-id");
    }
}
