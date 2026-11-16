//! `humata doctor restarts` (PER-30, D242): reads a restart report, says
//! what wasn't brought back and what is stuck, repairs what can't lose
//! anything, offers what could, and writes `diagnosis.md` into the report's
//! folder.
//!
//! The report is read from disk with this file's own types, so `--explain`
//! works with the daemon down. Safe repairs are `DaemonService.RepairRestart`
//! calls; lossy ones run only when named with `--allow <id>`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use humata2_proto::humata::v1 as pb;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::args::RestartsArgs;
use crate::as_json;
use crate::call::{Ctx, Done};
use crate::console::Console;
use crate::daemon::{Daemon, DaemonError, Replies, ReportReply, ReportRequest, Rpc, rpc};
use crate::error::CliError;
use crate::exit;
use crate::principal::Environment;

/// The report's summary for programs, written by the daemon's restart.
const RESTART_JSON: &str = "restart.json";
/// What interrupted operations did, one JSON object per line.
const OPERATIONS: &str = "operations.jsonl";
/// What this command writes beside them.
const DIAGNOSIS: &str = "diagnosis.md";
/// The daemon's own log lines shown in a diagnosis.
const LOG_LINES: usize = 20;

/// What stopped `humata doctor restarts`, and how to go on.
#[derive(Debug, thiserror::Error)]
pub enum DoctorError {
    #[error("there are no restart reports in {}", .folder.display())]
    NoReports { folder: PathBuf },
    #[error("there is no restart report named {name} in {}", .folder.display())]
    UnknownReport { name: String, folder: PathBuf },
    #[error("can't tell where humatad keeps its state; looked in {}", list(.tried))]
    NoState { tried: Vec<PathBuf> },
    #[error("couldn't read {}", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{} isn't a restart report humata can read", .path.display())]
    Unreadable {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("couldn't write {}", .path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("couldn't read the diagnosis from {from}")]
    ReadDiagnosis {
        from: String,
        #[source]
        source: io::Error,
    },
    #[error("the diagnosis from {from} isn't UTF-8 text")]
    DiagnosisNotText { from: String },
    #[error("--allow {id} isn't a repair this report offers; it offers {}", offered_list(.offered))]
    NotOffered { id: String, offered: Vec<String> },
}

fn list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn offered_list(ids: &[String]) -> String {
    if ids.is_empty() {
        "no lossy repairs".to_owned()
    } else {
        ids.join(", ")
    }
}

impl DoctorError {
    /// The error's kind, exit code and what to do next.
    pub fn how(&self) -> (&'static str, i32, String) {
        match self {
            Self::NoReports { .. } => (
                "not_found",
                exit::NOT_FOUND,
                "A report is written each time humatad starts; if it never started on this machine there is nothing to read. Pass --state with the folder humatad2 was started with.".into(),
            ),
            Self::UnknownReport { folder, .. } => (
                "not_found",
                exit::NOT_FOUND,
                format!("List {} for the reports there, or leave the name out for the newest.", folder.display()),
            ),
            Self::NoState { .. } => (
                "invalid_field",
                exit::USAGE,
                "Pass --state with the state folder humatad2 serves from (its --state, ~/.humata2/state by default).".into(),
            ),
            Self::Read { .. } | Self::ReadDiagnosis { .. } => (
                "invalid_field",
                exit::USAGE,
                "Check the path and that you can read it.".into(),
            ),
            Self::Unreadable { .. } => (
                "internal",
                exit::INTERNAL,
                "The file may be cut short or from another version of Humata. Read report.md beside it, and file this as a bug if the file looks whole.".into(),
            ),
            Self::Write { .. } => (
                "internal",
                exit::INTERNAL,
                "Make the report's folder writable by you, then run the command again. Nothing else changed because of this.".into(),
            ),
            Self::DiagnosisNotText { .. } => (
                "invalid_field",
                exit::USAGE,
                "Pass the diagnosis as UTF-8 text.".into(),
            ),
            Self::NotOffered { .. } => (
                "invalid_field",
                exit::USAGE,
                "Run `humata doctor restarts --explain` for the repairs and their ids, then allow one of those.".into(),
            ),
        }
    }
}

/// `restart.json`: what the restart found, session by session. Only what
/// the doctor reads; anything else in it is ignored.
#[derive(Debug, Default, Deserialize)]
struct RestartFile {
    #[serde(default)]
    machine: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    at_unix_ms: i64,
    #[serde(default)]
    sessions: Vec<Entry>,
    #[serde(default)]
    restored: Option<Restored>,
    #[serde(default)]
    daemon_log: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    kind: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    tab: String,
    #[serde(default)]
    agent: String,
    status: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    needs: Option<String>,
    #[serde(default)]
    step: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    files: Vec<Found>,
}

#[derive(Debug, Deserialize)]
struct Found {
    role: String,
    path: String,
    #[serde(default)]
    bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct Restored {
    #[serde(default)]
    copy_taken_unix_ms: i64,
    #[serde(default)]
    damage: String,
}

impl Entry {
    fn not_back(&self) -> bool {
        matches!(self.status.as_str(), "recoverable" | "failed")
    }

    fn what(&self) -> String {
        match (&self.session, self.kind.as_str()) {
            (Some(session), "agent") => format!("session {session}"),
            _ => format!("shell tab {}", self.tab),
        }
    }

    fn problem(&self) -> String {
        match (&self.error, &self.needs) {
            (Some(error), _) => error.clone(),
            (None, Some(needs)) => needs.clone(),
            (None, None) => self.reason.clone().unwrap_or_default(),
        }
    }

    fn json(&self) -> Json {
        json!({
            "kind": self.kind,
            "session": self.session,
            "tab": self.tab,
            "agent": self.agent,
            "status": self.status,
            "reason": self.reason,
            "step": self.step,
            "problem": self.problem(),
            "conversation_id": self.conversation_id,
            "files": self.files.iter().map(|file| json!({
                "role": file.role,
                "path": file.path,
                "bytes": file.bytes,
                "present": file.bytes.is_some(),
            })).collect::<Vec<_>>(),
        })
    }
}

/// One line of `operations.jsonl`.
#[derive(Debug, Deserialize)]
struct OperationLine {
    id: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    step: String,
    #[serde(default)]
    state: Json,
    #[serde(default)]
    reason: Option<String>,
}

/// An operation as it last stood.
#[derive(Debug)]
struct Operation {
    id: String,
    kind: String,
    step: String,
    /// pending, finished, undone or stuck.
    state: String,
    /// Why it is stuck or was undone, or what it waits for.
    detail: Option<String>,
}

impl Operation {
    fn of(line: OperationLine) -> Self {
        let (state, inner) = match &line.state {
            Json::String(name) => (name.clone(), None),
            // `{"stuck": {"reason": ".."}}`, as the daemon's enum is written.
            Json::Object(map) => match map.iter().next() {
                Some((name, body)) => {
                    let inner = ["reason", "waits_for"]
                        .iter()
                        .find_map(|key| body.get(*key).and_then(Json::as_str))
                        .map(str::to_owned);
                    (name.clone(), inner)
                }
                None => ("unknown".to_owned(), None),
            },
            _ => ("unknown".to_owned(), None),
        };
        Self {
            id: line.id,
            kind: line.kind,
            step: line.step,
            state,
            detail: line.reason.or(inner),
        }
    }

    fn stuck(&self) -> bool {
        self.state == "stuck"
    }

    fn json(&self) -> Json {
        json!({
            "id": self.id,
            "kind": self.kind,
            "step": self.step,
            "state": self.state,
            "detail": self.detail,
        })
    }
}

/// A restart report as read from its folder.
struct Report {
    folder: PathBuf,
    file: RestartFile,
    operations: Vec<Operation>,
}

impl Report {
    fn read(folder: &Path) -> Result<Self, DoctorError> {
        let path = folder.join(RESTART_JSON);
        let text = std::fs::read_to_string(&path).map_err(|source| DoctorError::Read {
            path: path.clone(),
            source,
        })?;
        let file = serde_json::from_str(&text)
            .map_err(|source| DoctorError::Unreadable { path, source })?;
        let operations = read_operations(&folder.join(OPERATIONS))?;
        Ok(Self {
            folder: folder.to_owned(),
            file,
            operations,
        })
    }
}

/// The operations the file records, the latest line of each id winning, in
/// the order their ids first appeared. A missing file has none; a line cut
/// short by a power cut, or any line that isn't an operation, is skipped.
fn read_operations(path: &Path) -> Result<Vec<Operation>, DoctorError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(DoctorError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    let mut order: Vec<String> = Vec::new();
    let mut latest: HashMap<String, Operation> = HashMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(line) = serde_json::from_str::<OperationLine>(line) else {
            continue;
        };
        let operation = Operation::of(line);
        if !latest.contains_key(&operation.id) {
            order.push(operation.id.clone());
        }
        latest.insert(operation.id.clone(), operation);
    }
    Ok(order
        .into_iter()
        .filter_map(|id| latest.remove(&id))
        .collect())
}

/// What the doctor can do about one thing.
#[derive(Clone, Debug)]
enum Call {
    Session(String),
    Tab(String),
    Store,
    StartFresh(String),
}

/// A repair, by its stable id.
#[derive(Clone, Debug)]
struct Repair {
    id: String,
    /// Can't lose anything: run without asking.
    safe: bool,
    what: String,
    call: Call,
}

impl Repair {
    fn json(&self) -> Json {
        json!({ "id": self.id, "safe": self.safe, "what": self.what })
    }
}

fn repairs(file: &RestartFile) -> Vec<Repair> {
    let mut found: Vec<Repair> = Vec::new();
    let mut store_trouble = false;
    for entry in file.sessions.iter().filter(|entry| entry.not_back()) {
        match (entry.kind.as_str(), entry.session.as_deref()) {
            ("agent", Some(session)) => {
                found.push(Repair {
                    id: format!("resume:{session}"),
                    safe: true,
                    what: format!("resume session {session} again with its saved conversation"),
                    call: Call::Session(session.to_owned()),
                });
                found.push(Repair {
                    id: format!("start-fresh:{session}"),
                    safe: false,
                    what: format!(
                        "start session {session} fresh with a summary: its old conversation stays on disk but its agent no longer continues it"
                    ),
                    call: Call::StartFresh(session.to_owned()),
                });
            }
            _ => found.push(Repair {
                id: format!("restart-tab:{}", entry.tab),
                safe: true,
                what: format!(
                    "start shell tab {} again in its folder with the output it saved",
                    entry.tab
                ),
                call: Call::Tab(entry.tab.clone()),
            }),
        }
        let said = entry.error.as_deref().unwrap_or_default().to_lowercase();
        if entry.status == "failed"
            && ["store", "database", "sqlite"].iter().any(|word| said.contains(word))
        {
            store_trouble = true;
        }
    }
    if store_trouble && file.restored.is_none() {
        found.push(Repair {
            id: "store".to_owned(),
            safe: true,
            what: "check the store; if it is damaged, humatad leaves so its next start restores the last good copy".to_owned(),
            call: Call::Store,
        });
    }
    found
}

/// What running a repair came to.
struct Applied {
    id: String,
    ok: bool,
    said: String,
}

impl Applied {
    fn json(&self) -> Json {
        json!({ "id": self.id, "ok": self.ok, "said": self.said })
    }
}

/// The state folder: `--state`, else the one beside a socket humata would
/// look for, that has restart reports in it.
fn state_folder(environment: &Environment, given: Option<&Path>) -> Result<PathBuf, DoctorError> {
    if let Some(given) = given {
        return Ok(given.to_owned());
    }
    let tried: Vec<PathBuf> = crate::socket::candidates(environment)
        .map(|sockets| {
            sockets
                .iter()
                .filter_map(|socket| socket.parent().map(|parent| parent.join("state")))
                .collect()
        })
        .unwrap_or_default();
    tried
        .iter()
        .find(|state| state.join("restarts").is_dir())
        .cloned()
        .ok_or(DoctorError::NoState { tried })
}

/// The report to read: the one named, else the newest. A name is a folder
/// path, or a folder's name in `restarts`.
fn choose(restarts: &Path, named: Option<&str>) -> Result<PathBuf, DoctorError> {
    if let Some(name) = named {
        let direct = Path::new(name);
        if direct.join(RESTART_JSON).is_file() {
            return Ok(direct.to_owned());
        }
        let beside = restarts.join(name);
        if beside.join(RESTART_JSON).is_file() {
            return Ok(beside);
        }
        return Err(DoctorError::UnknownReport {
            name: name.to_owned(),
            folder: restarts.to_owned(),
        });
    }
    let listing = match std::fs::read_dir(restarts) {
        Ok(listing) => listing,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(DoctorError::NoReports {
                folder: restarts.to_owned(),
            });
        }
        Err(source) => {
            return Err(DoctorError::Read {
                path: restarts.to_owned(),
                source,
            });
        }
    };
    // Folders start with their date, so the last name is the newest. One
    // still being written starts with a dot and has no restart.json yet.
    listing
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path())
        .filter(|path| path.join(RESTART_JSON).is_file())
        .max()
        .ok_or(DoctorError::NoReports {
            folder: restarts.to_owned(),
        })
}

fn read_diagnosis<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    from: &str,
) -> Result<String, DoctorError> {
    let bytes = if from == "-" {
        ctx.console.stdin()
    } else {
        std::fs::read(from)
    }
    .map_err(|source| DoctorError::ReadDiagnosis {
        from: from.to_owned(),
        source,
    })?;
    String::from_utf8(bytes).map_err(|_| DoctorError::DiagnosisNotText {
        from: from.to_owned(),
    })
}

pub(super) async fn restarts<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    args: RestartsArgs,
) -> Result<Done, CliError> {
    let state = state_folder(ctx.environment, args.state.as_deref())?;
    let folder = choose(&state.join("restarts"), args.report.as_deref())?;
    let report = Report::read(&folder)?;
    let all = repairs(&report.file);
    let written = match &args.diagnosis {
        Some(from) => Some(read_diagnosis(ctx, from)?),
        None => None,
    };
    let lossy: Vec<String> = all
        .iter()
        .filter(|repair| !repair.safe)
        .map(|repair| repair.id.clone())
        .collect();
    for id in &args.allow {
        if !lossy.contains(id) {
            return Err(DoctorError::NotOffered {
                id: id.clone(),
                offered: lossy,
            }
            .into());
        }
    }

    let mut applied: Vec<Applied> = Vec::new();
    if !args.explain {
        // The store last: a damaged one makes humatad leave.
        let (store, rest): (Vec<&Repair>, Vec<&Repair>) = all
            .iter()
            .filter(|repair| repair.safe || args.allow.contains(&repair.id))
            .partition(|repair| matches!(repair.call, Call::Store));
        for repair in rest.into_iter().chain(store) {
            applied.push(apply(ctx, repair).await);
        }
    }

    let diagnosis = render(&report, &all, written.as_deref(), &applied, args.explain);
    let path = report.folder.join(DIAGNOSIS);
    write_atomically(&path, diagnosis.as_bytes())?;

    let brought_back = report
        .file
        .sessions
        .iter()
        .filter(|entry| !entry.not_back())
        .count();
    let not_back: Vec<Json> = report
        .file
        .sessions
        .iter()
        .filter(|entry| entry.not_back())
        .map(Entry::json)
        .collect();
    let stuck: Vec<Json> = report
        .operations
        .iter()
        .filter(|operation| operation.stuck())
        .map(Operation::json)
        .collect();
    Ok(Done::reply(json!({
        "report": report.folder.display().to_string(),
        "machine": report.file.machine,
        "brought_back": brought_back,
        "not_brought_back": not_back,
        "operations": report.operations.iter().map(Operation::json).collect::<Vec<_>>(),
        "stuck": stuck,
        "repairs": all.iter().map(Repair::json).collect::<Vec<_>>(),
        "applied": applied.iter().map(Applied::json).collect::<Vec<_>>(),
        "diagnosis": path.display().to_string(),
    })))
}

/// Runs one repair; what went wrong with it is its result, so the others
/// still run.
async fn apply<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>, repair: &Repair) -> Applied {
    use pb::repair_restart_request::Target;
    let repair_restart = |target: Target| pb::RepairRestartRequest {
        target: Some(target),
    };
    let outcome = match &repair.call {
        Call::Session(session) => {
            let request = repair_restart(Target::Session(pb::SessionId {
                value: session.clone(),
            }));
            ctx.json(rpc::daemon::REPAIR_RESTART, &request).await
        }
        Call::Tab(tab) => {
            let request = repair_restart(Target::Tab(pb::TabId { value: tab.clone() }));
            ctx.json(rpc::daemon::REPAIR_RESTART, &request).await
        }
        Call::Store => {
            let request = repair_restart(Target::Store(true));
            ctx.json(rpc::daemon::REPAIR_RESTART, &request).await
        }
        Call::StartFresh(session) => {
            // One key for each repair, so a second start-fresh in the same
            // run isn't taken for the first.
            let key = ctx.key().map(|key| pb::IdempotencyKey {
                value: format!("{}-{}", key.value, repair.id),
            });
            let request = pb::ResumeSessionRequest {
                idempotency_key: key,
                id: Some(pb::SessionId {
                    value: session.clone(),
                }),
                mode: pb::resume_session_request::Mode::StartFreshWithSummary as i32,
            };
            ctx.json(rpc::session::RESUME_SESSION, &request).await
        }
    };
    match outcome {
        Ok(reply) => Applied {
            id: repair.id.clone(),
            ok: true,
            said: said(&reply),
        },
        Err(error) => {
            let failure = error.failure();
            Applied {
                id: repair.id.clone(),
                ok: false,
                said: format!("{} {}", failure.message, failure.fix).trim().to_owned(),
            }
        }
    }
}

/// What a repair's reply says happened.
fn said(reply: &Json) -> String {
    if reply.get("exiting").and_then(Json::as_bool) == Some(true) {
        return "the store is damaged; humatad is leaving so its next start restores the last good copy: start it again".to_owned();
    }
    let operation = reply.get("operation").unwrap_or(reply);
    match operation.get("id").and_then(Json::as_str) {
        Some(id) => format!("started operation {id}; `humata ops {id}` shows how it goes"),
        None => "done".to_owned(),
    }
}

/// `diagnosis.md`: the agent's own words if it gave any, then the facts the
/// report and this run hold, ready to be filed as a bug report.
fn render(
    report: &Report,
    all: &[Repair],
    written: Option<&str>,
    applied: &[Applied],
    explain: bool,
) -> String {
    let mut out = String::from("# Restart diagnosis\n\n");
    if let Some(text) = written {
        out.push_str(text.trim());
        out.push_str("\n\n");
    }
    let file = &report.file;
    out.push_str("## What the restart found\n\n");
    let _ = writeln!(out, "- Report: {}", report.folder.display());
    let _ = writeln!(out, "- Machine: {}", file.machine);
    let _ = writeln!(out, "- humatad version: {}", file.version);
    let _ = writeln!(
        out,
        "- At: {}",
        as_json::rfc3339(file.at_unix_ms.div_euclid(1000), 0)
    );
    let back = file.sessions.iter().filter(|entry| !entry.not_back()).count();
    let _ = writeln!(out, "- {back} of {} sessions and shells came back.", file.sessions.len());
    if let Some(restored) = &file.restored {
        let _ = writeln!(
            out,
            "- The store was damaged ({}) and restored from the copy taken at {}.",
            restored.damage,
            as_json::rfc3339(restored.copy_taken_unix_ms.div_euclid(1000), 0)
        );
    }
    out.push_str("\n## Not brought back\n\n");
    let missing: Vec<&Entry> = file.sessions.iter().filter(|entry| entry.not_back()).collect();
    if missing.is_empty() {
        out.push_str("Nothing.\n");
    }
    for entry in missing {
        let _ = writeln!(
            out,
            "- {}: {} ({}) at \"{}\": {}",
            entry.what(),
            entry.status,
            entry.reason.as_deref().unwrap_or("no reason recorded"),
            entry.step.as_deref().unwrap_or("unknown step"),
            entry.problem()
        );
        for found in &entry.files {
            let size = found
                .bytes
                .map_or_else(|| "missing".to_owned(), |bytes| format!("{bytes} bytes"));
            let _ = writeln!(out, "  - {}: {} ({size})", found.role, found.path);
        }
    }
    out.push_str("\n## Interrupted operations\n\n");
    if report.operations.is_empty() {
        out.push_str("None recorded.\n");
    }
    for operation in &report.operations {
        let detail = operation
            .detail
            .as_deref()
            .map(|detail| format!(": {detail}"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "- {} {} at {}: {}{detail}",
            operation.kind, operation.id, operation.step, operation.state
        );
    }
    out.push_str("\n## Repairs\n\n");
    if all.is_empty() {
        out.push_str("None to offer.\n");
    }
    for repair in all {
        let outcome = match applied.iter().find(|done| done.id == repair.id) {
            Some(done) if done.ok => format!("done: {}", done.said),
            Some(done) => format!("failed: {}", done.said),
            None if explain => "not run (--explain)".to_owned(),
            None if repair.safe => "not run".to_owned(),
            None => "not run: it needs `--allow` because it can lose something".to_owned(),
        };
        let kind = if repair.safe { "safe" } else { "lossy" };
        let _ = writeln!(out, "- {} ({kind}): {}; {outcome}", repair.id, repair.what);
    }
    if !file.daemon_log.is_empty() {
        out.push_str("\n## The daemon's last log lines before this start\n\n```\n");
        let skip = file.daemon_log.len().saturating_sub(LOG_LINES);
        for line in &file.daemon_log[skip..] {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("```\n");
    }
    out
}

/// Writes `bytes` to `path` through a temp file beside it, so a reader never
/// sees half of a diagnosis.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), DoctorError> {
    let fail = |source| DoctorError::Write {
        path: path.to_owned(),
        source,
    };
    let temp = path.with_file_name(format!(".{DIAGNOSIS}.partial"));
    let mut file = std::fs::File::create(&temp).map_err(fail)?;
    file.write_all(bytes).map_err(fail)?;
    file.sync_all().map_err(fail)?;
    drop(file);
    std::fs::rename(&temp, path).map_err(fail)
}

/// Stands in for the daemon when it isn't running: every call says so.
/// What reads only files (`--explain`) never calls it.
pub struct Offline;

impl Offline {
    fn down() -> DaemonError {
        DaemonError::NotRunning { tried: Vec::new() }
    }
}

impl Daemon for Offline {
    async fn unary(&self, _rpc: Rpc, _request: Vec<u8>) -> Result<Vec<u8>, DaemonError> {
        Err(Self::down())
    }

    async fn server_stream(&self, _rpc: Rpc, _request: Vec<u8>) -> Result<Replies, DaemonError> {
        Err(Self::down())
    }

    async fn client_stream(
        &self,
        _rpc: Rpc,
        _requests: tokio::sync::mpsc::Receiver<Vec<u8>>,
    ) -> Result<Vec<u8>, DaemonError> {
        Err(Self::down())
    }

    async fn bidi_stream(
        &self,
        _rpc: Rpc,
        _requests: tokio::sync::mpsc::Receiver<Vec<u8>>,
    ) -> Result<Replies, DaemonError> {
        Err(Self::down())
    }

    async fn report(&self, _request: ReportRequest) -> Result<ReportReply, DaemonError> {
        Err(Self::down())
    }
}
