//! Sessions a backend already has on a host, started outside Humata, read
//! into ours: Claude Code's transcripts and Copilot's session folders. An
//! imported session resumes the backend's own, so its next turn carries on
//! where it left off.

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::error::AgentError;
use crate::event::{Event, EventId, EventKind, SessionId};
use crate::links::Links;

/// A backend's session on this host that can be imported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Importable {
    /// The backend's id for it.
    pub id: String,
    /// The folder it worked in.
    pub working_directory: PathBuf,
    /// Its first message, or the backend's summary of it.
    pub summary: String,
    /// When it was last changed, in milliseconds since 1970 in UTC.
    pub updated_unix_ms: i64,
}

/// A backend's session, read as our history.
#[derive(Debug)]
pub struct Imported {
    pub working_directory: PathBuf,
    pub summary: String,
    pub events: Vec<Event>,
    /// Which of our events are which of the backend's.
    pub links: Vec<(EventId, String)>,
}

/// The longest summary kept, in characters.
const SUMMARY: usize = 80;

/// `text` on one line and at most [`SUMMARY`] characters.
pub(crate) fn summary(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= SUMMARY {
        return line;
    }
    let mut short: String = line.chars().take(SUMMARY - 1).collect();
    short.push('…');
    short
}

/// The lines of the JSON lines file at `path` that parse; a line cut off by
/// a crash, or one the backend wrote in a newer shape, is left out.
pub(crate) fn json_lines(path: &Path) -> Result<Vec<Value>, AgentError> {
    let text = std::fs::read_to_string(path).map_err(|source| AgentError::Read {
        path: path.to_owned(),
        source,
    })?;
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

/// The lines of the JSON lines file at `path` that parse, as
/// [`json_lines`] gives them but read one at a time, so stopping early
/// reads no further. A read that fails part way ends them.
fn each_json_line(path: &Path) -> std::io::Result<impl Iterator<Item = Value> + use<>> {
    let mut reader = BufReader::new(std::fs::File::open(path)?);
    let mut line = Vec::new();
    Ok(std::iter::from_fn(move || {
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {
                    if let Ok(value) = serde_json::from_slice(&line) {
                        return Some(value);
                    }
                }
            }
        }
    }))
}

/// When `path` was last changed, in milliseconds since 1970; 0 if unknown.
pub(crate) fn modified_unix_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// An RFC 3339 time in UTC, such as `2026-11-03T09:41:07.250Z`, in
/// milliseconds since 1970; None for any other form.
pub(crate) fn unix_ms(time: &str) -> Option<i64> {
    let time = time.strip_suffix('Z')?;
    let (date, clock) = time.split_once('T')?;
    let mut date = date.splitn(3, '-').map(str::parse::<i64>);
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let (clock, fraction) = clock.split_once('.').unwrap_or((clock, "0"));
    let mut clock = clock.splitn(3, ':').map(str::parse::<i64>);
    let (hour, minute, second) = (
        clock.next()?.ok()?,
        clock.next()?.ok()?,
        clock.next()?.ok()?,
    );
    let millis: i64 = format!("{fraction:0<3}").get(..3)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days from 1970-01-01 to the date, in the proleptic Gregorian calendar.
    let (year, month) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * month + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    let days = era * 146_097 + of_cycle - 719_468;
    Some((((days * 24 + hour) * 60 + minute) * 60 + second) * 1000 + millis)
}

/// Builds an imported history: each message starts a turn, which ends at
/// the next message or at the end.
pub(crate) struct History {
    events: Vec<Event>,
    links: Vec<(EventId, String)>,
    open: bool,
    /// The time of the event before, for one the backend gave none.
    last: i64,
}

impl History {
    pub(crate) fn new(start: i64) -> Self {
        Self {
            events: Vec::new(),
            links: Vec::new(),
            open: false,
            last: start,
        }
    }

    /// Adds `kind` at `time`, or when the event before was; linked to the
    /// backend's event `theirs`, if given.
    pub(crate) fn push(&mut self, kind: EventKind, time: Option<i64>, theirs: Option<&str>) {
        if let Some(time) = time {
            self.last = time;
        }
        let id = EventId::random();
        if let Some(theirs) = theirs {
            self.links.push((id.clone(), theirs.to_owned()));
        }
        self.events.push(Event {
            id,
            time_unix_ms: self.last,
            kind,
            parent_call_id: None,
        });
    }

    /// Starts a turn with the person's message.
    pub(crate) fn message(&mut self, text: String, time: Option<i64>, theirs: Option<&str>) {
        self.end_turn();
        self.push(
            EventKind::UserMessage {
                text,
                images: Vec::new(),
            },
            time,
            theirs,
        );
        self.push(EventKind::TurnStarted, None, None);
        self.open = true;
    }

    fn end_turn(&mut self) {
        if std::mem::take(&mut self.open) {
            self.push(EventKind::TurnEnded { aborted: false }, None, None);
        }
    }

    /// Whether the person sent anything.
    pub(crate) fn is_empty(&self) -> bool {
        !self
            .events
            .iter()
            .any(|event| matches!(event.kind, EventKind::UserMessage { .. }))
    }

    pub(crate) fn finish(mut self, working_directory: PathBuf, summary: String) -> Imported {
        self.end_turn();
        if !self.events.is_empty() {
            self.push(EventKind::Idle, None, None);
        }
        Imported {
            working_directory,
            summary,
            events: self.events,
            links: self.links,
        }
    }
}

/// The text of a message's content: a string, or its text blocks joined.
pub(crate) fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The backend's sessions a session under `state` carries on already, so
/// they aren't offered again.
fn imported(state: &Path) -> Result<HashSet<String>, AgentError> {
    Ok(Links::linked(state)?
        .into_iter()
        .map(|(theirs, _)| theirs)
        .collect())
}

/// The backend's session `external`, as `read` reads it, linked to our new
/// `session` under `state`; refused if a session there carries it on
/// already, or its folder is gone.
pub(crate) fn import_into(
    state: &Path,
    external: &str,
    session: &SessionId,
    read: impl FnOnce() -> Result<Imported, AgentError>,
) -> Result<Imported, AgentError> {
    if let Some((_, ours)) = Links::linked(state)?
        .into_iter()
        .find(|(theirs, _)| theirs == external)
    {
        return Err(AgentError::AlreadyImported {
            id: external.to_owned(),
            session: ours,
        });
    }
    let imported = read()?;
    if !imported.working_directory.is_dir() {
        return Err(AgentError::ImportedFolderMissing {
            id: external.to_owned(),
            path: imported.working_directory,
        });
    }
    Links {
        session: Some(external.to_owned()),
        fork: None,
        events: imported.links.clone(),
        imported: Some(external.to_owned()),
    }
    .save(&Links::path(state, session.as_str()))?;
    Ok(imported)
}

/// Whether `id` can be a backend's session id: letters, digits and dashes,
/// so it names a file or folder and nothing outside it.
pub(crate) fn is_session_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Where a backend keeps its own state on this host: `variable` if set, for
/// its programs or for humatad, else `default` under the home folder.
pub(crate) fn backend_home(env: &crate::Env, variable: &str, default: &str) -> Option<PathBuf> {
    let set = |name: &str| {
        env.get(name)
            .or_else(|| std::env::var(name).ok())
            .filter(|value| !value.is_empty())
    };
    set(variable)
        .map(PathBuf::from)
        .or_else(|| set("HOME").map(|home| Path::new(&home).join(default)))
}

/// The entries of `dir`; none if it doesn't exist.
fn entries(dir: &Path) -> Result<Vec<PathBuf>, AgentError> {
    match std::fs::read_dir(dir) {
        Ok(entries) => Ok(entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(AgentError::ReadDirectory {
            path: dir.to_owned(),
            source,
        }),
    }
}

/// Whether a message is one the CLI wrote for itself, such as a slash
/// command's echo, rather than one the person typed.
fn is_cli_note(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with("<command-") || text.starts_with("<local-command-")
}

/// What the person typed in a transcript line; None for any other line: a
/// reply, a tool's result, or a message the CLI wrote for itself.
fn claude_typed(line: &Value) -> Option<String> {
    if line["type"] != "user" || line["isMeta"] == true {
        return None;
    }
    let content = &line["message"]["content"];
    let result = content
        .as_array()
        .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "tool_result"));
    if result {
        return None;
    }
    let text = content_text(content);
    (!text.trim().is_empty() && !is_cli_note(&text)).then_some(text)
}

/// Claude Code's transcripts under `projects` (its config folder's
/// `projects`), one JSON lines file per session, in a folder per project,
/// but those already imported into a session under `state`.
pub(crate) fn claude_importable(
    projects: &Path,
    state: &Path,
) -> Result<Vec<Importable>, AgentError> {
    let imported = imported(state)?;
    let mut found = Vec::new();
    for project in entries(projects)?.into_iter().filter(|path| path.is_dir()) {
        for path in entries(&project)? {
            let Some(id) = claude_session_id(&path).filter(|id| !imported.contains(id)) else {
                continue;
            };
            let Some((working_directory, summary)) = claude_listed(&path) else {
                continue;
            };
            found.push(Importable {
                id,
                working_directory,
                summary,
                updated_unix_ms: modified_unix_ms(&path),
            });
        }
    }
    Ok(found)
}

/// Where a Claude Code transcript worked and what it's about, by the rules
/// of [`claude_read`] but reading only as far as the first message the
/// person typed; None for a transcript `claude_read` refuses. A summary
/// line after that message isn't looked for: the message stands in for it.
fn claude_listed(path: &Path) -> Option<(PathBuf, String)> {
    let mut working_directory = None;
    let mut summary_line = None;
    let mut first = None;
    for line in each_json_line(path).ok()? {
        if line["isSidechain"] == true {
            continue;
        }
        if working_directory.is_none() {
            working_directory = line["cwd"].as_str().map(PathBuf::from);
        }
        if line["type"] == "summary" {
            summary_line = line["summary"].as_str().map(summary);
        } else if first.is_none() {
            first = claude_typed(&line).map(|text| summary(&text));
        }
        if working_directory.is_some() && first.is_some() {
            break;
        }
    }
    let first = first?;
    Some((working_directory?, summary_line.unwrap_or(first)))
}

/// The session a transcript file is of; None for a file that isn't one,
/// such as a subagent's.
fn claude_session_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    (path.extension()? == "jsonl" && !stem.starts_with("agent-")).then(|| stem.to_owned())
}

/// The transcript of Claude Code's session `id` under `projects`.
pub(crate) fn claude_transcript(projects: &Path, id: &str) -> Result<PathBuf, AgentError> {
    if !is_session_id(id) {
        return Err(AgentError::UnknownImport { id: id.to_owned() });
    }
    let file = format!("{id}.jsonl");
    for project in entries(projects)?.into_iter().filter(|path| path.is_dir()) {
        let path = project.join(&file);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(AgentError::UnknownImport { id: id.to_owned() })
}

/// A Claude Code transcript as our history; refused if the person sent
/// nothing in it or it doesn't say where it worked.
pub(crate) fn claude_read(path: &Path) -> Result<Imported, AgentError> {
    let id = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lines = json_lines(path)?;
    let mut history = History::new(modified_unix_ms(path));
    let mut working_directory = None;
    let mut summary_line = None;
    let mut first = None;
    for line in lines.iter().filter(|line| line["isSidechain"] != true) {
        if working_directory.is_none() {
            working_directory = line["cwd"].as_str().map(PathBuf::from);
        }
        let time = line["timestamp"].as_str().and_then(unix_ms);
        let uuid = line["uuid"].as_str();
        let content = &line["message"]["content"];
        let blocks = content.as_array().map_or(&[][..], Vec::as_slice);
        match line["type"].as_str().unwrap_or_default() {
            "summary" => summary_line = line["summary"].as_str().map(summary),
            "user" if line["isMeta"] == true => {}
            "user" if blocks.iter().any(|block| block["type"] == "tool_result") => {
                for block in blocks.iter().filter(|block| block["type"] == "tool_result") {
                    history.push(
                        EventKind::ToolCompleted {
                            call_id: block["tool_use_id"].as_str().unwrap_or_default().to_owned(),
                            success: block["is_error"] != true,
                            output: tool_output(&block["content"]),
                        },
                        time,
                        uuid,
                    );
                }
            }
            "user" => {
                if let Some(text) = claude_typed(line) {
                    first.get_or_insert_with(|| summary(&text));
                    history.message(text, time, None);
                }
            }
            "assistant" => {
                for block in blocks {
                    let text = |field: &str| block[field].as_str().unwrap_or_default().to_owned();
                    match block["type"].as_str().unwrap_or_default() {
                        "text" if !text("text").is_empty() => {
                            history.push(
                                EventKind::AssistantMessage { text: text("text") },
                                time,
                                uuid,
                            );
                        }
                        "thinking" if !text("thinking").is_empty() => {
                            history.push(
                                EventKind::Reasoning {
                                    text: text("thinking"),
                                },
                                time,
                                None,
                            );
                        }
                        "tool_use" => history.push(
                            EventKind::ToolStarted {
                                call_id: text("id"),
                                tool: text("name"),
                                arguments_json: block["input"].to_string(),
                            },
                            time,
                            None,
                        ),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let (Some(working_directory), Some(first)) = (working_directory, first) else {
        return Err(AgentError::NothingToImport { id });
    };
    if history.is_empty() {
        return Err(AgentError::NothingToImport { id });
    }
    Ok(history.finish(working_directory, summary_line.unwrap_or(first)))
}

/// A tool result's text: a string, or its text parts joined.
fn tool_output(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Copilot's session folders under `sessions` (its home's `session-state`),
/// each with its events as JSON lines, but those already imported into a
/// session under `state`.
pub(crate) fn copilot_importable(
    sessions: &Path,
    state: &Path,
) -> Result<Vec<Importable>, AgentError> {
    let imported = imported(state)?;
    let mut found = Vec::new();
    for dir in entries(sessions)?.into_iter().filter(|path| path.is_dir()) {
        let Some(id) = dir
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|id| !imported.contains(*id))
            .map(str::to_owned)
        else {
            continue;
        };
        let Some((working_directory, summary)) = copilot_listed(&dir) else {
            continue;
        };
        found.push(Importable {
            id,
            working_directory,
            summary,
            updated_unix_ms: modified_unix_ms(&dir.join(COPILOT_EVENTS)),
        });
    }
    Ok(found)
}

const COPILOT_EVENTS: &str = "events.jsonl";

/// What Copilot keeps beside a session's events: where it worked and its
/// summary of it, a `name: value` on a line each.
struct CopilotWorkspace(String);

impl CopilotWorkspace {
    /// The one in the session folder `dir`; empty if it has none.
    fn read(dir: &Path) -> Self {
        Self(std::fs::read_to_string(dir.join("workspace.yaml")).unwrap_or_default())
    }

    fn field(&self, name: &str) -> Option<String> {
        self.0.lines().find_map(|line| {
            line.strip_prefix(name)
                .and_then(|rest| rest.strip_prefix(':'))
                .map(|value| value.trim().trim_matches(['"', '\'']).to_owned())
                .filter(|value| !value.is_empty())
        })
    }
}

/// Where the session a `session.start` event starts works, if it says.
fn copilot_started_in(data: &Value) -> Option<PathBuf> {
    data["context"]["cwd"]
        .as_str()
        .or_else(|| data["cwd"].as_str())
        .map(PathBuf::from)
}

/// What the person typed in one of Copilot's events; None for any other
/// event.
fn copilot_typed(line: &Value) -> Option<&str> {
    line["data"]["content"]
        .as_str()
        .filter(|content| line["type"] == "user.message" && !content.trim().is_empty())
}

/// Where a Copilot session worked and what it's about, by the rules of
/// [`copilot_read`] but reading its events only as far as the first message
/// the person typed; None for a session `copilot_read` refuses.
fn copilot_listed(dir: &Path) -> Option<(PathBuf, String)> {
    let workspace = CopilotWorkspace::read(dir);
    let mut working_directory = workspace.field("cwd").map(PathBuf::from);
    let mut first = None;
    for line in each_json_line(&dir.join(COPILOT_EVENTS)).ok()? {
        if working_directory.is_none() && line["type"] == "session.start" {
            working_directory = copilot_started_in(&line["data"]);
        } else if first.is_none() {
            first = copilot_typed(&line).map(summary);
        }
        if working_directory.is_some() && first.is_some() {
            break;
        }
    }
    let first = first?;
    Some((
        working_directory?,
        workspace
            .field("summary")
            .map_or(first, |text| summary(&text)),
    ))
}

/// The folder of Copilot's session `id` under `sessions`.
pub(crate) fn copilot_folder(sessions: &Path, id: &str) -> Result<PathBuf, AgentError> {
    Some(sessions.join(id))
        .filter(|dir| is_session_id(id) && dir.join(COPILOT_EVENTS).is_file())
        .ok_or_else(|| AgentError::UnknownImport { id: id.to_owned() })
}

/// A Copilot session folder as our history; refused if the person sent
/// nothing in it or it doesn't say where it worked.
pub(crate) fn copilot_read(dir: &Path) -> Result<Imported, AgentError> {
    let id = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let events = dir.join(COPILOT_EVENTS);
    let lines = json_lines(&events)?;
    let workspace = CopilotWorkspace::read(dir);
    let mut working_directory = workspace.field("cwd").map(PathBuf::from);
    let mut first = None;
    let mut history = History::new(modified_unix_ms(&events));
    for line in &lines {
        let data = &line["data"];
        // A subagent's steps showed as the call that started it.
        if line["agentId"].is_string() || data["parentToolCallId"].is_string() {
            continue;
        }
        let text = |field: &str| data[field].as_str().unwrap_or_default().to_owned();
        let time = line["timestamp"].as_str().and_then(unix_ms);
        let theirs = line["id"].as_str();
        let kind = match line["type"].as_str().unwrap_or_default() {
            "session.start" => {
                if working_directory.is_none() {
                    working_directory = copilot_started_in(data);
                }
                continue;
            }
            "user.message" => {
                if let Some(content) = copilot_typed(line) {
                    first.get_or_insert_with(|| summary(content));
                    history.message(content.to_owned(), time, theirs);
                }
                continue;
            }
            "assistant.message" if !text("content").is_empty() => EventKind::AssistantMessage {
                text: text("content"),
            },
            "assistant.reasoning" if !text("content").is_empty() => EventKind::Reasoning {
                text: text("content"),
            },
            "tool.execution_start" => EventKind::ToolStarted {
                call_id: text("toolCallId"),
                tool: text("toolName"),
                arguments_json: data["arguments"].to_string(),
            },
            "tool.execution_complete" => EventKind::ToolCompleted {
                call_id: text("toolCallId"),
                success: data["success"].as_bool().unwrap_or(false),
                output: data["result"]["content"]
                    .as_str()
                    .or_else(|| data["error"]["message"].as_str())
                    .or_else(|| data["error"].as_str())
                    .unwrap_or_default()
                    .to_owned(),
            },
            _ => continue,
        };
        history.push(kind, time, theirs);
    }
    let (Some(working_directory), Some(first)) = (working_directory, first) else {
        return Err(AgentError::NothingToImport { id });
    };
    Ok(history.finish(
        working_directory,
        workspace
            .field("summary")
            .map_or(first, |text| summary(&text)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_utc_time_is_read_to_the_millisecond() {
        assert_eq!(unix_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_ms("2000-03-01T00:00:00.5Z"), Some(951_868_800_500));
        assert_eq!(unix_ms("2026-11-03T09:41:07.250Z"), Some(1_793_698_867_250));
        assert_eq!(unix_ms("2026-11-03 09:41:07"), None);
        assert_eq!(unix_ms("2026-13-03T09:41:07Z"), None);
    }

    #[test]
    fn a_long_message_is_summed_up_on_one_line() {
        assert_eq!(summary("fix\n  the  login"), "fix the login");
        let long = summary(&"word ".repeat(40));
        assert_eq!(long.chars().count(), SUMMARY);
        assert!(long.ends_with('…'));
    }

    use crate::{
        Agent, ClaudeOptions, ClaudeRunner, CopilotOptions, CopilotRunner, Env, LocalAgent,
    };

    fn write_lines(path: &Path, lines: &[serde_json::Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text: Vec<String> = lines.iter().map(ToString::to_string).collect();
        std::fs::write(path, text.join("\n") + "\n").unwrap();
    }

    fn kinds(events: &[Event]) -> Vec<&EventKind> {
        events.iter().map(|event| &event.kind).collect()
    }

    #[tokio::test]
    async fn a_claude_code_transcript_is_imported_once_and_carries_on_its_session() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let cwd = work.to_str().unwrap();
        let config = dir.path().join("claude");
        write_lines(
            &config.join("projects/-work/0a1b-2c3d.jsonl"),
            &[
                serde_json::json!({"type": "user", "uuid": "u1", "cwd": cwd, "isMeta": true,
                    "timestamp": "2026-11-03T09:00:00Z", "message": {"content": "Caveat"}}),
                serde_json::json!({"type": "user", "uuid": "u2", "cwd": cwd,
                    "timestamp": "2026-11-03T09:00:01Z", "message": {"content": "fix the login"}}),
                serde_json::json!({"type": "assistant", "uuid": "a1", "cwd": cwd,
                    "timestamp": "2026-11-03T09:00:02Z", "message": {"content": [
                        {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file": "login.rs"}}]}}),
                serde_json::json!({"type": "user", "uuid": "u3", "cwd": cwd,
                    "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": "fn login()"}]}}),
                serde_json::json!({"type": "assistant", "uuid": "a2", "cwd": cwd, "isSidechain": true,
                    "message": {"content": [{"type": "text", "text": "a subagent's"}]}}),
                serde_json::json!({"type": "assistant", "uuid": "a3", "cwd": cwd,
                    "timestamp": "2026-11-03T09:00:05Z",
                    "message": {"content": [{"type": "text", "text": "Fixed."}]}}),
            ],
        );
        // A subagent's transcript isn't a session.
        write_lines(&config.join("projects/-work/agent-1.jsonl"), &[]);
        let state = dir.path().join("state");
        let runner = ClaudeRunner::new(ClaudeOptions {
            program: "claude".into(),
            state: state.join("backend"),
            env: Env::new([(
                "CLAUDE_CONFIG_DIR".to_owned(),
                config.to_str().unwrap().to_owned(),
            )]),
            mcp: None,
        });
        let agent = LocalAgent::open(state.join("sessions"), runner).unwrap();

        let found = agent.importable().await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "0a1b-2c3d");
        assert_eq!(found[0].working_directory, work);
        assert_eq!(found[0].summary, "fix the login");

        let record = agent.import("0a1b-2c3d", None).await.unwrap();
        assert_eq!(record.name, "fix the login");
        assert_eq!(record.working_directory, work);
        let history = agent.history(&record.id).await.unwrap();
        assert_eq!(
            kinds(&history),
            [
                &EventKind::UserMessage {
                    text: "fix the login".into(),
                    images: Vec::new(),
                },
                &EventKind::TurnStarted,
                &EventKind::ToolStarted {
                    call_id: "t1".into(),
                    tool: "Read".into(),
                    arguments_json: r#"{"file":"login.rs"}"#.into(),
                },
                &EventKind::ToolCompleted {
                    call_id: "t1".into(),
                    success: true,
                    output: "fn login()".into()
                },
                &EventKind::AssistantMessage {
                    text: "Fixed.".into()
                },
                &EventKind::TurnEnded { aborted: false },
                &EventKind::Idle,
            ]
        );
        assert_eq!(
            history[0].time_unix_ms,
            unix_ms("2026-11-03T09:00:01Z").unwrap()
        );
        let links = Links::load(&Links::path(&state.join("backend"), record.id.as_str())).unwrap();
        assert_eq!(links.session.as_deref(), Some("0a1b-2c3d"));
        assert_eq!(links.last(), Some("a3"));

        assert_eq!(agent.importable().await.unwrap(), []);
        let again = agent.import("0a1b-2c3d", None).await;
        assert!(
            matches!(again, Err(AgentError::AlreadyImported { session, .. }) if session == record.id)
        );
        let outside = agent.import("../0a1b-2c3d", None).await;
        assert!(matches!(outside, Err(AgentError::UnknownImport { .. })));
    }

    #[test]
    fn a_session_with_no_message_from_the_person_is_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap();
        let state = dir.path().join("state");
        let projects = dir.path().join("claude/projects");
        write_lines(
            &projects.join("-work/no-message.jsonl"),
            &[
                serde_json::json!({"type": "user", "cwd": cwd, "isMeta": true,
                    "message": {"content": "Caveat"}}),
                serde_json::json!({"type": "user", "cwd": cwd,
                    "message": {"content": "<command-name>/clear</command-name>"}}),
                serde_json::json!({"type": "user", "cwd": cwd, "isSidechain": true,
                    "message": {"content": "a subagent's prompt"}}),
                serde_json::json!({"type": "user", "cwd": cwd,
                    "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]}}),
                serde_json::json!({"type": "assistant", "cwd": cwd,
                    "message": {"content": [{"type": "text", "text": "Done."}]}}),
            ],
        );
        assert_eq!(claude_importable(&projects, &state).unwrap(), []);

        let sessions = dir.path().join("copilot/session-state");
        write_lines(
            &sessions.join("no-message/events.jsonl"),
            &[
                serde_json::json!({"type": "session.start", "data": {"context": {"cwd": cwd}}}),
                serde_json::json!({"type": "user.message", "data": {"content": "  "}}),
                serde_json::json!({"type": "assistant.message", "data": {"content": "Hello."}}),
            ],
        );
        assert_eq!(copilot_importable(&sessions, &state).unwrap(), []);
    }

    #[test]
    fn a_listed_session_has_the_folder_and_summary_its_import_has() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap();
        let state = dir.path().join("state");
        let projects = dir.path().join("claude/projects");
        let transcript = projects.join("-work/0a1b.jsonl");
        write_lines(
            &transcript,
            &[
                serde_json::json!({"type": "summary", "summary": "Login fix", "leafUuid": "a1"}),
                serde_json::json!({"type": "user", "uuid": "u1", "cwd": cwd,
                    "message": {"content": "fix the login"}}),
                serde_json::json!({"type": "assistant", "uuid": "a1", "cwd": cwd,
                    "message": {"content": [{"type": "text", "text": "Fixed."}]}}),
                serde_json::json!({"type": "user", "uuid": "u2", "cwd": "/elsewhere",
                    "message": {"content": "and the logout"}}),
            ],
        );
        let listed = claude_importable(&projects, &state).unwrap();
        let read = claude_read(&transcript).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].working_directory, read.working_directory);
        assert_eq!(listed[0].summary, read.summary);
        assert_eq!(listed[0].summary, "Login fix");

        let sessions = dir.path().join("copilot/session-state");
        let session = sessions.join("s-1");
        write_lines(
            &session.join("events.jsonl"),
            &[
                serde_json::json!({"type": "session.start", "data": {"context": {"cwd": "/elsewhere"}}}),
                serde_json::json!({"type": "user.message", "data": {"content": "add a test"}}),
                serde_json::json!({"type": "assistant.message", "data": {"content": "Added."}}),
            ],
        );
        std::fs::write(
            session.join("workspace.yaml"),
            format!("id: s-1\ncwd: {cwd}\nsummary: \"Tests for login\"\n"),
        )
        .unwrap();
        let listed = copilot_importable(&sessions, &state).unwrap();
        let read = copilot_read(&session).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].working_directory, read.working_directory);
        assert_eq!(listed[0].summary, read.summary);
        assert_eq!(listed[0].summary, "Tests for login");
    }

    #[tokio::test]
    async fn a_copilot_session_whose_folder_is_gone_is_not_imported() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let home = dir.path().join("copilot");
        let events = |cwd: &Path| {
            vec![
                serde_json::json!({"type": "session.start", "id": "c0",
                    "data": {"context": {"cwd": cwd.to_str().unwrap()}}}),
                serde_json::json!({"type": "user.message", "id": "c1", "timestamp": "2026-11-03T10:00:00Z",
                    "data": {"content": "add a test"}}),
                serde_json::json!({"type": "assistant.message", "id": "c2", "data": {"content": "Added."}}),
            ]
        };
        write_lines(
            &home.join("session-state/s-here/events.jsonl"),
            &events(&work),
        );
        write_lines(
            &home.join("session-state/s-gone/events.jsonl"),
            &events(&dir.path().join("gone")),
        );
        let state = dir.path().join("state");
        let runner = CopilotRunner::new(CopilotOptions {
            program: "copilot".into(),
            state: state.join("backend"),
            env: Env::new([("COPILOT_HOME".to_owned(), home.to_str().unwrap().to_owned())]),
            provider: None,
            mcp: None,
        });
        let agent = LocalAgent::open(state.join("sessions"), runner).unwrap();

        let mut found: Vec<_> = agent
            .importable()
            .await
            .unwrap()
            .into_iter()
            .map(|found| found.id)
            .collect();
        found.sort();
        assert_eq!(found, ["s-gone", "s-here"]);

        let gone = agent.import("s-gone", None).await;
        assert!(matches!(
            gone,
            Err(AgentError::ImportedFolderMissing { .. })
        ));
        let record = agent.import("s-here", Some("Tests".into())).await.unwrap();
        assert_eq!(record.name, "Tests");
        let history = agent.history(&record.id).await.unwrap();
        assert_eq!(
            kinds(&history),
            [
                &EventKind::UserMessage {
                    text: "add a test".into(),
                    images: Vec::new(),
                },
                &EventKind::TurnStarted,
                &EventKind::AssistantMessage {
                    text: "Added.".into()
                },
                &EventKind::TurnEnded { aborted: false },
                &EventKind::Idle,
            ]
        );
        let ids: Vec<_> = agent
            .importable()
            .await
            .unwrap()
            .into_iter()
            .map(|found| found.id)
            .collect();
        assert_eq!(ids, ["s-gone"]);
        assert_eq!(agent.sessions().await.unwrap().len(), 1);
    }
}
