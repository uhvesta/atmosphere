//! What can go wrong in a run, and how each failure is shown: its kind, exit
//! code, message and fix (CLI.md §7).

use std::io;
use std::path::PathBuf;

use humata2_proto::humata::v1 as pb;
use serde_json::{Value, json};

use crate::daemon::DaemonError;
use crate::exit;
use crate::as_json;

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
    #[error("this changes or deletes things, and there's no terminal to confirm it in")]
    NeedsConfirmation,
    #[error("stopped before changing anything")]
    NotConfirmed,
    #[error("the daemon needs you to confirm it's you, and humata can't ask here")]
    NoPresence { accepted: Vec<&'static str>, in_session: bool },
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
            Self::NeedsConfirmation => plain(
                "invalid_field",
                exit::USAGE,
                "Run it with --dry-run to see the plan, then with --plan <token> to do exactly that, or add --yes to skip the check."
                    .into(),
            ),
            Self::NotConfirmed => plain("interrupted", exit::INTERRUPTED, String::new()),
            Self::NoPresence { accepted, in_session } => Failure {
                details: json!({ "accepted": accepted, "caller_in_session_terminal": in_session }),
                ..plain(
                    "presence_required",
                    exit::NEEDS_USER,
                    if *in_session {
                        "Run it from a terminal outside Humata's sessions: anything typed in a session's terminal may come from its agent.".into()
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
            Self::Pending { request } => Failure {
                details: json!({ "request": request }),
                ..plain(
                    "pending",
                    exit::PENDING,
                    format!("`humata inbox show {request}` shows the answer when it comes."),
                )
            },
            Self::Interrupted => plain("interrupted", exit::INTERRUPTED, String::new()),
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
        DaemonError::Refused { error: refusal, code, .. } => refusal_failure(refusal, *code),
        DaemonError::Status { code, .. } => {
            let (kind, exit) = exit::of_status(*code);
            plain(
                kind,
                exit,
                if kind == "unimplemented" {
                    "This humatad doesn't serve that yet; update Humata on this machine."
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
    }
}

fn refusal_failure(error: &pb::Error, code: i32) -> Failure {
    let (kind, exit) = match &error.kind {
        Some(kind) => exit::of_kind(kind),
        None => exit::of_status(code),
    };
    let details = as_json::typed("humata.v1.Error", error)
        .get(kind)
        .cloned()
        .unwrap_or_else(|| json!({}));
    let retry = error.retry.as_ref().map(|retry| {
        let after = retry.after.as_ref().map_or(std::time::Duration::ZERO, |after| {
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
