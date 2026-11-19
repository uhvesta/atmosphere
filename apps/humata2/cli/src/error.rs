//! What can go wrong in a run, and how each failure is shown: its kind, exit
//! code, message and fix (CLI.md §7).

use std::io;
use std::path::PathBuf;

use humata2_proto::humata::v1 as pb;
use serde_json::{Value, json};

use crate::as_json;
use crate::daemon::DaemonError;
use crate::exit;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error(transparent)]
    Daemon(#[from] DaemonError),
    #[error("{what} \"{name}\" matches more than one: {}", .matches.join(", "))]
    Ambiguous {
        what: &'static str,
        name: String,
        matches: Vec<String>,
    },
    #[error("no {what} is named \"{name}\"")]
    NoMatch { what: &'static str, name: String },
    #[error("{field} can't be \"{value}\": expected {expected}")]
    BadValue {
        field: &'static str,
        value: String,
        expected: &'static str,
    },
    #[error("this command needs {flag}")]
    Missing { flag: &'static str },
    /// A bare path inside a session names the agent's own clone, and its
    /// session has more than one repository, so which clone isn't known.
    #[error("\"{path}\" is a path in your own clone, and your session has {} repositories: {}", .repos.len(), .repos.join(", "))]
    WhichClone { path: String, repos: Vec<String> },
    /// An agent's last word of `cp|mv` names no machine it can see and has
    /// no `.`, so it is neither a machine nor clearly a file (D756).
    #[error("\"{word}\" is not a machine of this channel, so nothing was copied")]
    NotAMachineHere { word: String },
    /// An agent's bare `shared…` location was read as the channel's shared
    /// folder on its machine (D777) and the daemon didn't find it.
    #[error("\"{typed}\" was looked up in the channel's shared folder on your machine and not found")]
    SharedNotFound {
        typed: String,
        #[source]
        source: DaemonError,
    },
    /// `session fork --channel` named a channel the parent isn't in: a fork
    /// always stays in its parent's channel (CV-8).
    #[error("a fork stays in its parent's channel, so session {session} can't be forked into channel \"{channel}\"")]
    ForkElsewhere { session: String, channel: String },
    /// `attach` without `--tab` found no agent tab and not exactly one tab.
    #[error("session {session} has no agent tab to attach to; its tabs are {}", tab_list(.tabs))]
    NoAgentTab { session: String, tabs: Vec<String> },
    #[error("this changes or deletes things, and there's no terminal to confirm it in")]
    NeedsConfirmation,
    #[error("stopped before changing anything")]
    NotConfirmed,
    #[error("the daemon needs you to confirm it's you, and humata can't ask here")]
    NoPresence {
        accepted: Vec<&'static str>,
        in_session: bool,
    },
    #[error("couldn't use the terminal")]
    Console {
        #[source]
        source: io::Error,
    },
    #[error("couldn't read {}", .path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("still waiting for an answer (request {request})")]
    Pending { request: String },
    /// `humata login` keeps its login under HOME, per terminal session.
    #[error("there's nowhere to keep this terminal's login: HOME isn't set")]
    LoginNeedsHome,
    #[error("there's nowhere to keep this terminal's login: humata isn't in a terminal session")]
    LoginNeedsTerminal,
    #[error("the daemon's login reply has no token humata can keep")]
    NoLoginToken,
    #[error("couldn't save this terminal's login to {}", .path.display())]
    SaveLogin {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("couldn't remove this terminal's login at {}", .path.display())]
    RemoveLogin {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("stopped waiting")]
    Interrupted,
    /// `--dry-run` reached a call that is not read-only: the guard refused it
    /// before anything was sent (CLI.md §4).
    #[error(
        "this is a bug in humata: --dry-run was about to send {method} ({request}) and nothing was sent"
    )]
    DryRunWouldSend { method: String, request: String },
    /// `attach --dry-run`: a live terminal only reads and types, so there is
    /// no plan to print. Refused before anything is sent.
    #[error("attach can't be dry-run; it only reads and types live")]
    AttachDryRun,
    /// `hook --dry-run`: a hook answers the agent live, so there is no plan
    /// to print. Only a person types this; real hook calls keep exiting 0.
    #[error("hooks can't be dry-run; they answer the agent live")]
    HookDryRun,
    #[error("the chunk of the file at byte {offset} didn't decode")]
    BadChunk {
        offset: u64,
        #[source]
        source: humata2_chunk::ChunkError,
    },
    #[error("the terminal's output at byte {offset} didn't decode")]
    BadTerminalChunk {
        offset: u64,
        #[source]
        source: humata2_chunk::ChunkError,
    },
    #[error("humata stopped while decoding the chunk of the file at byte {offset}")]
    DecoderStopped { offset: u64 },
    /// `humata opencode` couldn't run OpenCode; the message says how to fix it.
    #[error(transparent)]
    OpenCode(#[from] humata2_opencode::WrapperError),
    /// `humata codex` couldn't run Codex; the message says how to fix it.
    #[error(transparent)]
    Codex(#[from] humata2_codex::WrapperError),
    /// `humata machine script check` found the rules file or a script name
    /// unusable (MACHINESCRIPTS, D687).
    #[error("the machine scripts rules have {} problem(s):\n{}", .problems.len(), .problems.join("\n"))]
    ScriptsInvalid {
        problems: Vec<String>,
        warnings: Vec<String>,
    },
    /// `humata doctor restarts` couldn't read or write a restart report.
    #[error(transparent)]
    Doctor(#[from] crate::commands::DoctorError),
}

/// A failure as printed: `error` in `--json`, message and fix on stderr.
pub struct Failure {
    pub kind: String,
    pub code: i32,
    pub message: String,
    pub fix: String,
    pub details: Value,
    /// When to try again, and whether with the same `--key`.
    pub retry: Option<(std::time::Duration, bool)>,
}

impl CliError {
    pub fn failure(&self) -> Failure {
        let plain = |kind: &str, code: i32, fix: String| Failure {
            kind: kind.to_owned(),
            code,
            message: self.to_string(),
            fix,
            details: json!({}),
            retry: None,
        };
        match self {
            Self::Daemon(error) => daemon_failure(error),
            Self::Ambiguous { what, matches, .. } => Failure {
                details: json!({ "matches": matches }),
                ..plain(
                    "invalid_field",
                    exit::USAGE,
                    format!("Use the {what}'s id, or a name that matches only one."),
                )
            },
            Self::NoMatch { what, .. } => plain(
                "not_found",
                exit::NOT_FOUND,
                format!("Check the name, or list them to find the {what}'s id."),
            ),
            Self::AttachDryRun => plain(
                "invalid_field",
                exit::USAGE,
                "Run attach without --dry-run. To watch without typing, add --read-only.".into(),
            ),
            Self::HookDryRun => plain(
                "invalid_field",
                exit::USAGE,
                "Run the hook without --dry-run, or let the agent call it.".into(),
            ),
            Self::BadValue { .. } => plain(
                "invalid_field",
                exit::USAGE,
                "Fix the value and run it again; --help lists what each command takes.".into(),
            ),
            Self::Missing { flag } => plain(
                "invalid_field",
                exit::USAGE,
                format!("Add {flag}."),
            ),
            Self::WhichClone { path, repos } => Failure {
                details: json!({ "repos": repos }),
                ..plain(
                    "invalid_field",
                    exit::USAGE,
                    format!(
                        "Name the clone: write it as this-mac:<your session>/{}/{path}, with the repository you mean. \
                         To send to a machine, name one of your channel's machines; `humata host ls` lists them.",
                        repos.first().map_or("<owner>/<repo>", String::as_str)
                    ),
                )
            },
            Self::NotAMachineHere { word } => plain(
                "invalid_field",
                exit::USAGE,
                format!(
                    "To send to a machine, use one that `humata host ls` lists. If you meant a file in \
                     your own clone, write ./{word}; for a folder, write {word}/."
                ),
            ),
            Self::SharedNotFound { typed, .. } => plain(
                "not_found",
                exit::NOT_FOUND,
                format!(
                    "A bare shared/ is the channel's shared folder on your machine. If you meant a folder \
                     named shared in your own clone, write ./{typed}."
                ),
            ),
            Self::ForkElsewhere { session, channel } => plain(
                "invalid_field",
                exit::USAGE,
                format!(
                    "To carry the session into that channel, run `humata session continue {session} \
                     --channel {channel}`. To fork it in its own channel, leave out --channel."
                ),
            ),
            Self::NoAgentTab { session, tabs } => Failure {
                details: json!({ "tabs": tabs }),
                ..plain(
                    "no_agent_tab",
                    exit::PRECONDITION,
                    if tabs.is_empty() {
                        format!("Check the session with `humata session tabs {session}`, and attach once it has a tab.")
                    } else {
                        format!(
                            "Pick one of those tabs with `humata attach {session} --tab <id>`; `humata session tabs {session}` shows which is which."
                        )
                    },
                )
            },
            Self::NeedsConfirmation => plain(
                "invalid_field",
                exit::USAGE,
                "Run it with --dry-run to see the plan, then with --plan <token> to do exactly that, or add --yes to skip the check."
                    .into(),
            ),
            Self::NotConfirmed => plain(
                "declined",
                exit::DECLINED,
                "Nothing was changed. Run it again and answer y to go ahead, or add --yes to skip the question.".into(),
            ),
            Self::NoPresence { accepted, in_session } => Failure {
                details: json!({ "accepted": accepted, "caller_in_session_terminal": in_session }),
                ..plain(
                    "presence_required",
                    exit::NEEDS_USER,
                    if *in_session {
                        "Run it from a terminal outside Humata's sessions: anything typed in a session's terminal may come from its agent.".into()
                    } else if accepted.contains(&"approval_passphrase") {
                        "Run `humata login` in this terminal and type the approval passphrase when it asks, then run this again.".into()
                    } else {
                        "Run `humata login` in a terminal, or confirm it in the Humata app, then run it again.".into()
                    },
                )
            },
            Self::Console { .. } => plain(
                "internal",
                exit::INTERNAL,
                "Run it from a terminal humata can read from and write to.".into(),
            ),
            Self::ReadFile { .. } => plain(
                "invalid_field",
                exit::USAGE,
                "Check the path and that you can read it.".into(),
            ),
            Self::Pending { request } => pending_failure(request, self.to_string()),
            Self::Interrupted => plain("interrupted", exit::INTERRUPTED, String::new()),
            Self::DryRunWouldSend { .. } => plain(
                "internal",
                exit::INTERNAL,
                "Run the command without --dry-run to make the change, and report this bug in humata with the command you ran.".into(),
            ),
            Self::DecoderStopped { .. } => plain(
                "internal",
                exit::INTERNAL,
                "This is a bug in humata; run the command again, and report it if it happens again.".into(),
            ),
            Self::BadTerminalChunk { .. } => plain(
                "invalid_field",
                exit::INTERNAL,
                "Attach again: the terminal resumes from where it stopped. If it fails the same way, set compression to Off in the daemon's compression.toml and report it.".into(),
            ),
            Self::BadChunk { .. } => plain(
                "invalid_field",
                exit::INTERNAL,
                "Run the command again. If it fails the same way, set compression to Off in the daemon's compression.toml and report it.".into(),
            ),
            Self::ScriptsInvalid { problems, warnings } => Failure {
                details: json!({ "problems": problems, "warnings": warnings }),
                ..plain(
                    "invalid_field",
                    exit::USAGE,
                    "Fix each problem in ~/.humata2/config/machine-scripts.toml or the scripts folder, then run `humata machine script check` again.".into(),
                )
            },
            Self::OpenCode(_) => plain("tool_failed", exit::TOOL_FAILED, String::new()),
            Self::Codex(_) => plain("tool_failed", exit::TOOL_FAILED, String::new()),
            Self::Doctor(error) => {
                let (kind, code, fix) = error.how();
                plain(kind, code, fix)
            }
            Self::LoginNeedsHome => plain(
                "invalid_field",
                exit::USAGE,
                "Set HOME, then run `humata login` again.".into(),
            ),
            Self::LoginNeedsTerminal => plain(
                "invalid_field",
                exit::USAGE,
                "Run `humata login` in the terminal you'll use it from.".into(),
            ),
            Self::NoLoginToken => plain(
                "internal",
                exit::INTERNAL,
                "This humata and humatad may be different versions; update Humata on this machine, then run `humata login` again.".into(),
            ),
            Self::SaveLogin { path, .. } => plain(
                "internal",
                exit::INTERNAL,
                format!(
                    "Make {} writable by you alone, then run `humata login` again. The login you confirmed stays in humatad until it expires; nothing kept its token, so nothing can use it, and you can end it sooner in the app.",
                    path.parent().unwrap_or(path).display()
                ),
            ),
            Self::RemoveLogin { path, .. } => plain(
                "internal",
                exit::INTERNAL,
                format!(
                    "Delete {} yourself. The login is already ended in humatad unless this run reported otherwise; don't copy the file anywhere, since anyone who can read it holds the token until it expires.",
                    path.display()
                ),
            ),
        }
    }
}

/// The tabs named in an error, or `none` when there are none.
fn tab_list(tabs: &[String]) -> String {
    if tabs.is_empty() {
        "none".to_owned()
    } else {
        tabs.join(", ")
    }
}

/// A call still waiting for the user (exit 12), as the request that holds it.
fn pending_failure(request: &str, message: String) -> Failure {
    Failure {
        kind: "pending".to_owned(),
        code: exit::PENDING,
        message,
        fix: format!(
            "`humata inbox show {request}` shows the answer when it comes. Run the command with --wait <seconds> to wait for it in the same run."
        ),
        details: json!({ "request": request }),
        retry: None,
    }
}

fn daemon_failure(error: &DaemonError) -> Failure {
    let plain = |kind: &str, code: i32, fix: &str| Failure {
        kind: kind.to_owned(),
        code,
        message: error.to_string(),
        fix: fix.to_owned(),
        details: json!({}),
        retry: None,
    };
    match error {
        DaemonError::Refused {
            error: refusal,
            code,
            ..
        } => refusal_failure(refusal, *code),
        DaemonError::Status { code, .. } => {
            let (kind, exit) = exit::of_status(*code);
            plain(
                kind,
                exit,
                if kind == "unimplemented" {
                    // D768: a bare Unimplemented has no route at all, so this CLI and
                    // humatad come from different releases; a built-but-unready
                    // method carries its own humata.v1.Error and fix instead.
                    "This humatad has no such method, so this humata CLI and humatad come from different releases. Migrate this machine from the Inbox on the host Mac so both match, or do it from the Humata app until then."
                } else {
                    ""
                },
            )
        }
        DaemonError::Connect { .. } => plain(
            "daemon_not_running",
            exit::PRECONDITION,
            "Check that humatad is running (open the Humata app on the Mac), or set HUMATA_SOCKET to its socket.",
        ),
        DaemonError::NotRunning { .. } => plain(
            "daemon_not_running",
            exit::PRECONDITION,
            "Start Humata (on the Mac, open the app), or set HUMATA_SOCKET to the daemon's socket.",
        ),
        DaemonError::NoHome => plain(
            "daemon_not_running",
            exit::PRECONDITION,
            "Set HOME, or set HUMATA_SOCKET to the daemon's socket.",
        ),
        DaemonError::BadToken { .. } => plain(
            "invalid_field",
            exit::USAGE,
            "Set it to the token exactly as `humata token create` printed it.",
        ),
        DaemonError::Decode { .. } | DaemonError::Ended { .. } => plain(
            "internal",
            exit::INTERNAL,
            "This humata and humatad may be different versions; update Humata on this machine.",
        ),
        DaemonError::NoReportService => plain(
            "unimplemented",
            exit::PRECONDITION,
            "Bug reports arrive in a later humatad; until then, describe the problem in an issue.",
        ),
        DaemonError::App { source } => match source {
            humata2_report::remote::RemoteError::NotRunning { .. } => plain(
                "app_not_running",
                exit::PRECONDITION,
                "Open the Humata 2 app, then run the command again. If it is open, it couldn't take the report socket: check its log, or set HUMATA_APP_SOCKET to the same path for both.",
            ),
            _ => plain(
                "app_unreachable",
                exit::PRECONDITION,
                "Quit and reopen the Humata 2 app, then run the command again.",
            ),
        },
        DaemonError::AppRefused { refusal } => match refusal.kind {
            humata2_report::remote::RefusalKind::AlreadyRecording => plain(
                "already_recording",
                exit::PRECONDITION,
                "A recording is already running; `humata report stop` saves it.",
            ),
            humata2_report::remote::RefusalKind::NotRecording => plain(
                "not_recording",
                exit::PRECONDITION,
                "Nothing is being recorded; start with `humata report record`.",
            ),
            humata2_report::remote::RefusalKind::Failed => plain(
                "report_failed",
                exit::INTERNAL,
                "The report wasn't saved; the message says why. Fix that and run the command again.",
            ),
        },
    }
}

fn refusal_failure(error: &pb::Error, code: i32) -> Failure {
    if let Some(pb::error::Kind::ApprovalPending(pending)) = &error.kind {
        let request = pending
            .request
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        return pending_failure(&request, error.message.clone());
    }
    let (kind, exit) = match &error.kind {
        Some(kind) => exit::of_kind(kind),
        None => exit::of_status(code),
    };
    let details = as_json::typed("humata.v1.Error", error)
        .get(kind)
        .cloned()
        .unwrap_or_else(|| json!({}));
    let retry = error.retry.as_ref().map(|retry| {
        let after = retry
            .after
            .as_ref()
            .map_or(std::time::Duration::ZERO, |after| {
                std::time::Duration::new(
                    u64::try_from(after.seconds).unwrap_or(0),
                    u32::try_from(after.nanos).unwrap_or(0),
                )
            });
        (after, retry.same_key)
    });
    Failure {
        kind: kind.to_owned(),
        code: exit,
        message: error.message.clone(),
        fix: error.fix.clone(),
        details,
        retry,
    }
}
