//! The restart report (PER-29): what was running before a restart, what
//! became of it, and for anything not brought back, the step that failed,
//! the error, its holder's log, the versions and the files it relied on.
//!
//! Written as a bug report (D47) under the data folder's `restarts/`, so
//! the restart doctor (PER-30) and a person read it the same way: the
//! sessions as they were before and after each step in `state.txt`, the
//! failures and log lines in `daemon.log`. Beside them, `restart.json`
//! holds the same facts for a program: each session's status (reattached,
//! resumed, recoverable with what's needed, or failed) and its files.
//! Variable values the terminals were given are masked in both.

use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write as _};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use humata2_report::{
    Context, Daemon, DaemonError, Form, Limits, LogLine, Machine, Recording, Secrets, Session,
    Trees,
};
use humata2_store::{
    AgentRecord, AgentState, Change, Restored, ResumeFailure, SCHEMA_VERSION, SessionRecord,
    TabRecord,
};
use serde::Serialize;

use crate::reconcile::{Revived, ShellRevived};
use crate::{ReviveConfig, ReviveError};

/// The holder log lines kept for each session that wasn't brought back.
const LOG_LINES: usize = 50;
/// How far back from a log's end to look for them.
const LOG_TAIL_BYTES: u64 = 64 << 10;
/// Values shorter than this aren't masked: they'd match everywhere.
const SHORTEST_SECRET: usize = 4;

/// Where restart reports go, and what they say about this daemon.
#[derive(Clone, Debug)]
pub struct RestartReports {
    /// The data folder's `restarts/`.
    pub folder: PathBuf,
    /// humatad2's version.
    pub version: String,
    /// This machine's name.
    pub machine: String,
    /// The OS and its version, such as `macOS 26.7`.
    pub os: String,
    /// The daemon's own log from the run before this start, whose last
    /// lines say what it was doing when it stopped. None when there is
    /// none (a first start).
    pub daemon_log: Option<PathBuf>,
}

/// A file a session relied on, as it was found before reviving it.
#[derive(Clone, Debug, Serialize)]
struct Found {
    role: &'static str,
    path: String,
    /// None when it wasn't there.
    bytes: Option<u64>,
    modified_unix_ms: Option<i64>,
}

impl Found {
    fn look(role: &'static str, path: &Path) -> Self {
        let metadata = fs::metadata(path).ok();
        let modified = metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .and_then(|since| i64::try_from(since.as_millis()).ok());
        Self {
            role,
            path: path.display().to_string(),
            bytes: metadata.map(|metadata| metadata.len()),
            modified_unix_ms: modified,
        }
    }
}

/// One agent or shell, as found and as it ended up.
#[derive(Clone, Debug, Serialize)]
struct Entry {
    kind: &'static str,
    session: Option<String>,
    tab: String,
    /// The agent, or `shell`.
    agent: String,
    /// What it was saved as before the restart.
    was: &'static str,
    /// reattached, exited, relaunched, resumed, left, restarted,
    /// recoverable or failed; `pending` until it's been looked at.
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    /// What it takes to bring it back, when it's recoverable.
    #[serde(skip_serializing_if = "Option::is_none")]
    needs: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    conversation_id: Option<String>,
    holder_pid_saved: Option<u32>,
    holder_pid_now: Option<u32>,
    files: Vec<Found>,
    #[serde(skip)]
    socket: PathBuf,
    #[serde(skip)]
    log: PathBuf,
}

impl Entry {
    fn brought_back(&self) -> bool {
        !matches!(self.status, "recoverable" | "failed")
    }

    fn line(&self) -> String {
        let mut line = format!("{} {} (tab {}): was {}", self.kind, self.session.as_deref().unwrap_or("-"), self.tab, self.was);
        if let Some(pid) = self.holder_pid_saved {
            let _ = write!(line, " in holder {pid}");
        }
        let _ = write!(line, "; {}", self.status);
        if let Some(reason) = self.reason {
            let _ = write!(line, " ({reason}");
            if let Some(code) = self.exit_code {
                let _ = write!(line, ", exit code {code}");
            }
            line.push(')');
        }
        if let Some(pid) = self.holder_pid_now {
            let _ = write!(line, ", holder {pid} now");
        }
        line
    }
}

#[derive(Serialize)]
struct RestartJson<'a> {
    format: u32,
    at_unix_ms: i64,
    machine: &'a str,
    version: &'a str,
    os: &'a str,
    store_schema_version: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    restored: Option<RestoredJson>,
    sessions: &'a [Entry],
    /// The last lines the daemon logged before this start.
    daemon_log: &'a [String],
}

#[derive(Serialize)]
struct RestoredJson {
    copy_taken_unix_ms: i64,
    damaged_kept_at: String,
    damage: String,
    lost: Vec<LostRow>,
    unreadable_tables: Vec<String>,
}

#[derive(Serialize)]
struct LostRow {
    table: String,
    key: String,
    change: &'static str,
}

/// What one reconcile found and did, written when it's done.
pub(crate) struct Restart {
    at_unix_ms: i64,
    started: Instant,
    entries: Vec<Entry>,
    secrets: Vec<(String, String)>,
    recording: Recording,
    restored: Option<RestoredJson>,
    daemon_log: Vec<String>,
}

impl Restart {
    pub(crate) fn new(
        config: &ReviveConfig,
        at_unix_ms: i64,
        sessions: &[SessionRecord],
        restored: Option<&Restored>,
    ) -> Self {
        let reports = &config.reports;
        let sessions: Vec<Session> = sessions
            .iter()
            .map(|session| Session {
                id: session.id.clone(),
                title: session.title.clone(),
                channel: session.channel.clone(),
                machine: session.machine.clone(),
                agent: session.agent.clone(),
            })
            .collect();
        let context = Context {
            started_unix_ms: u64::try_from(at_unix_ms).unwrap_or(0),
            app_version: format!("humatad2 {}", reports.version),
            os: reports.os.clone(),
            daemons: vec![Daemon {
                machine: reports.machine.clone(),
                version: reports.version.clone(),
            }],
            machines: vec![Machine {
                name: reports.machine.clone(),
                kind: "local".to_owned(),
                state: "restarted".to_owned(),
            }],
            sessions,
            ..Context::default()
        };
        let trees = Trees {
            state: String::new(),
            view: "(humatad2 has no window)".to_owned(),
            layout: "(humatad2 has no window)".to_owned(),
        };
        let restored = restored.map(|restored| RestoredJson {
            copy_taken_unix_ms: restored.copy_taken_unix_ms,
            damaged_kept_at: restored.damaged_kept_at.display().to_string(),
            damage: restored.damage.clone(),
            lost: restored
                .changes
                .iter()
                .map(|row| LostRow {
                    table: row.table.clone(),
                    key: row.key.clone(),
                    change: match row.change {
                        Change::AddedSinceCopy => "added_since_copy",
                        Change::ChangedSinceCopy => "changed_since_copy",
                        Change::RemovedSinceCopy => "removed_since_copy",
                    },
                })
                .collect(),
            unreadable_tables: restored.unreadable_tables.clone(),
        });
        let daemon_log = reports.daemon_log.as_deref().map(log_tail).unwrap_or_default();
        let mut recording = Recording::start(context, trees, Limits::default());
        for line in &daemon_log {
            recording.log(LogLine {
                at_ms: 0,
                machine: reports.machine.clone(),
                line: format!("(last run) {line}"),
            });
        }
        let mut restart = Self {
            at_unix_ms,
            started: Instant::now(),
            entries: Vec::new(),
            secrets: Vec::new(),
            recording,
            restored,
            daemon_log,
        };
        restart.step("found the saved sessions");
        restart
    }

    fn at_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// The sessions as they stand, as `state.txt` shows them.
    fn state(&self) -> String {
        let mut state = format!("store schema version {SCHEMA_VERSION}\n");
        if let Some(restored) = &self.restored {
            let _ = writeln!(
                state,
                "store restored from its last good copy (taken at {} ms): {}; {} rows differ, {} tables unreadable",
                restored.copy_taken_unix_ms,
                restored.damage,
                restored.lost.len(),
                restored.unreadable_tables.len()
            );
            for row in &restored.lost {
                let _ = writeln!(state, "  {} {}: {}", row.table, row.key, row.change);
            }
        }
        for entry in &self.entries {
            state.push_str(&entry.line());
            state.push('\n');
        }
        state
    }

    fn step(&mut self, action: &str) {
        let (at, state) = (self.at_ms(), self.state());
        self.recording.action(at, action, &state);
    }

    /// Notes `record` as found, before it's revived, with the values its
    /// terminal is given, to mask.
    pub(crate) fn agent(
        &mut self,
        config: &ReviveConfig,
        record: &AgentRecord,
        tab: Option<&TabRecord>,
    ) {
        let launch = &record.launch;
        let log = config.logs.join(format!("{}.log", record.tab));
        let holder_pid_saved = match record.state {
            AgentState::Running { holder_pid } => Some(holder_pid),
            _ => tab.map(|tab| tab.holder_pid).filter(|pid| *pid != 0),
        };
        self.entries.push(Entry {
            kind: "agent",
            session: Some(record.session.clone()),
            tab: record.tab.clone(),
            agent: launch.agent.clone(),
            was: was(record.state),
            status: "pending",
            reason: None,
            exit_code: None,
            needs: None,
            step: None,
            error: None,
            conversation_id: Some(launch.conversation_id.clone()),
            holder_pid_saved,
            holder_pid_now: None,
            files: vec![
                Found::look("conversation", &launch.conversation_file),
                Found::look("folder", &launch.folder),
                Found::look("scrollback", &humata2_holder::saved_scrollback_path(&launch.holder_socket)),
                Found::look("holder_log", &log),
            ],
            socket: launch.holder_socket.clone(),
            log,
        });
    }

    pub(crate) fn shell(
        &mut self,
        config: &ReviveConfig,
        tab: &TabRecord,
    ) {
        let log = config.logs.join(format!("{}.log", tab.id));
        self.entries.push(Entry {
            kind: "shell",
            session: tab.session.clone(),
            tab: tab.id.clone(),
            agent: "shell".to_owned(),
            was: "shell",
            status: "pending",
            reason: None,
            exit_code: None,
            needs: None,
            step: None,
            error: None,
            conversation_id: None,
            holder_pid_saved: Some(tab.holder_pid).filter(|pid| *pid != 0),
            holder_pid_now: None,
            files: vec![
                Found::look("scrollback", &humata2_holder::saved_scrollback_path(&tab.holder_socket)),
                Found::look("holder_log", &log),
            ],
            socket: tab.holder_socket.clone(),
            log,
        });
    }

    /// Keeps `variables` out of the report.
    pub(crate) fn mask(&mut self, variables: &[(std::ffi::OsString, std::ffi::OsString)]) {
        for (name, value) in variables {
            let value = value.to_string_lossy().into_owned();
            if value.len() >= SHORTEST_SECRET && !self.secrets.iter().any(|(_, known)| *known == value) {
                self.secrets.push((name.to_string_lossy().into_owned(), value));
            }
        }
    }

    /// Notes what became of the agent noted last.
    pub(crate) fn agent_outcome(&mut self, outcome: &Result<Revived, ReviveError>) {
        let Some(entry) = self.entries.last_mut() else { return };
        match outcome {
            Ok(revived) => {
                entry.status = match revived {
                    Revived::Reattached => "reattached",
                    Revived::Exited { code } => {
                        entry.exit_code = Some(*code);
                        "exited"
                    }
                    Revived::Relaunched => "relaunched",
                    Revived::Resumed { .. } => "resumed",
                    Revived::Left => "left",
                    Revived::ResumeFailed { reason, exit_code } => {
                        let (name, step, needs) = failure(*reason);
                        entry.reason = Some(name);
                        entry.exit_code = *exit_code;
                        entry.step = Some(step.to_owned());
                        entry.needs = Some(needs);
                        "recoverable"
                    }
                };
            }
            Err(error) => failed(entry, error),
        }
        self.finish_entry();
    }

    /// Notes what became of the shell noted last.
    pub(crate) fn shell_outcome(&mut self, outcome: &Result<ShellRevived, ReviveError>) {
        let Some(entry) = self.entries.last_mut() else { return };
        match outcome {
            Ok(ShellRevived::Reattached) => entry.status = "reattached",
            Ok(ShellRevived::Restarted { .. }) => entry.status = "restarted",
            Err(error) => failed(entry, error),
        }
        self.finish_entry();
    }

    fn finish_entry(&mut self) {
        let at = self.at_ms();
        let machine = self.recording.context().daemons.first().map(|daemon| daemon.machine.clone()).unwrap_or_default();
        let Some(entry) = self.entries.last_mut() else { return };
        if entry.brought_back() {
            entry.holder_pid_now = humata2_holder::holder_pid(&entry.socket);
        }
        let action = format!("{} {}: {}", entry.kind, entry.session.as_deref().unwrap_or(&entry.tab), entry.status);
        if !entry.brought_back() {
            let message = match (&entry.error, entry.needs) {
                (Some(error), _) => error.clone(),
                (None, Some(needs)) => needs.to_owned(),
                (None, None) => String::new(),
            };
            let error = DaemonError {
                at_ms: at,
                machine: machine.clone(),
                method: entry.step.clone(),
                code: entry.reason.unwrap_or(entry.status).to_owned(),
                message,
            };
            let lines = log_tail(&entry.log);
            self.recording.daemon_error(error);
            for line in lines {
                self.recording.log(LogLine {
                    at_ms: at,
                    machine: machine.clone(),
                    line,
                });
            }
        }
        self.step(&action);
    }

    /// Writes the report into `config.reports.folder` and returns its
    /// folder. Blocks, so run it off the async runtime.
    pub(crate) fn write(mut self, config: &ReviveConfig) -> Result<PathBuf, ReviveError> {
        let reports = &config.reports;
        let back = self.entries.iter().filter(|entry| entry.brought_back()).count();
        let not = self.entries.len() - back;
        let seconds = self.at_unix_ms.max(0) / 1000 % 86_400;
        let form = Form {
            happened: format!(
                "Restart at {:02}:{:02}:{:02} UTC: {back} of {} sessions and shells back, {not} not",
                seconds / 3600,
                seconds / 60 % 60,
                seconds % 60,
                self.entries.len()
            ),
            ..Form::default()
        };
        self.step("done");
        let at = self.at_ms();
        let mut secrets = Secrets::new();
        for (name, value) in &self.secrets {
            secrets = secrets.secret(name, value);
        }
        let captured = self.recording.stop(at);
        let scrubbed = humata2_report::scrub(captured, form, &secrets)
            .map_err(|source| ReviveError::ScrubReport { source })?;
        let folder = humata2_report::write(&reports.folder, &scrubbed).map_err(|source| {
            ReviveError::WriteReport {
                folder: reports.folder.clone(),
                source,
            }
        })?;
        let json = RestartJson {
            format: 1,
            at_unix_ms: self.at_unix_ms,
            machine: &reports.machine,
            version: &reports.version,
            os: &reports.os,
            store_schema_version: SCHEMA_VERSION,
            restored: self.restored,
            sessions: &self.entries,
            daemon_log: &self.daemon_log,
        };
        let mut text = serde_json::to_string_pretty(&json).map_err(|source| ReviveError::RestartJson {
            path: folder.join("restart.json"),
            source: source.into(),
        })?;
        for (name, value) in &self.secrets {
            if let (Ok(quoted), Ok(marker)) = (serde_json::to_string(value), serde_json::to_string(&format!("‹{name}›"))) {
                // Inside a JSON string, the value as JSON escapes it.
                let (inner, marker) = (&quoted[1..quoted.len() - 1], &marker[1..marker.len() - 1]);
                text = text.replace(inner, marker);
            }
        }
        write_beside(&folder, "restart.json", text.as_bytes())?;
        Ok(folder)
    }
}

fn was(state: AgentState) -> &'static str {
    match state {
        AgentState::Launching => "launching",
        AgentState::Running { .. } => "running",
        AgentState::Resuming => "resuming",
        AgentState::Exited { .. } => "exited",
        AgentState::Stopped => "stopped",
        AgentState::ResumeFailed { .. } => "resume_failed",
    }
}

/// A failed resume's name, the step it failed at, and what bringing it
/// back takes.
fn failure(reason: ResumeFailure) -> (&'static str, &'static str, &'static str) {
    const CHECK: &str = "check it can be resumed";
    const VERIFY: &str = "check the resumed agent took up its conversation";
    match reason {
        ResumeFailure::ConversationFileMissing => (
            "conversation_file_missing",
            CHECK,
            "its conversation file is gone: put it back at the path listed, then resume. Starting a new session instead loses the conversation",
        ),
        ResumeFailure::FolderMissing => (
            "folder_missing",
            CHECK,
            "its folder is gone: restore it at the path listed, then resume",
        ),
        ResumeFailure::AgentNotInstalled => (
            "agent_not_installed",
            CHECK,
            "its agent's command isn't installed on this machine: install it, then resume",
        ),
        ResumeFailure::AgentExited => (
            "agent_exited",
            VERIFY,
            "its agent exited when resumed: read its terminal's output and the log lines here for why, fix that, then resume",
        ),
        ResumeFailure::AgentStartedAnotherConversation => (
            "agent_started_another_conversation",
            VERIFY,
            "its agent didn't take up the conversation it was resumed with: resume again, or relaunch it with the conversation id listed. Don't start it fresh in its place, which leaves the conversation behind",
        ),
    }
}

fn failed(entry: &mut Entry, error: &ReviveError) {
    entry.status = "failed";
    entry.step = Some(
        match error {
            ReviveError::Store { doing, .. } => doing,
            ReviveError::Holder { .. } => "start its terminal",
            ReviveError::NoCommand { .. } => "find the command to run",
            ReviveError::Background { .. } => "run a background task",
            ReviveError::ScrubReport { .. } | ReviveError::WriteReport { .. } | ReviveError::RestartJson { .. } => {
                "write the restart report"
            }
        }
        .to_owned(),
    );
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        let _ = write!(text, ": {cause}");
        source = cause.source();
    }
    entry.error = Some(text);
}

/// The last lines of the log at `path`, or none when there's no log.
fn log_tail(path: &Path) -> Vec<String> {
    let read = || -> std::io::Result<String> {
        let mut file = fs::File::open(path)?;
        let len = file.metadata()?.len();
        let from = len.saturating_sub(LOG_TAIL_BYTES);
        file.seek(SeekFrom::Start(from))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    };
    let Ok(text) = read() else { return Vec::new() };
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    let skip = lines.len().saturating_sub(LOG_LINES);
    lines[skip..].iter().map(|line| (*line).to_owned()).collect()
}

/// Writes `name` into the report `folder`: to a temp file, synced, then
/// renamed into place and the folder synced, readable only by this user.
fn write_beside(folder: &Path, name: &str, bytes: &[u8]) -> Result<(), ReviveError> {
    let path = folder.join(name);
    let temp = folder.join(format!(".{name}.{}.partial", std::process::id()));
    let written = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, &path)?;
        fs::File::open(folder)?.sync_all()
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written.map_err(|source| ReviveError::RestartJson { path, source })
}
