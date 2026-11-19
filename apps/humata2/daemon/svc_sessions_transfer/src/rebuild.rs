//! RebuildSession (SL-4, LD-11, flow 26.23 steps 4 and 5; D195): a session
//! whose machine was forgotten comes back on another machine in its channel.
//! Each repo is cloned again at its branch, at the last pushed sha (main if
//! it was never pushed); the conversation resumes from the host's mirror; the
//! agent is told what was lost.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_daemon_ports::Caller;
use humata2_sessions::{AgentError, GitError};
use humata2_store::{StartStep, Store, StoreError};
use humata2_svc_sessions_common::pb;
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use crate::SessionTransferSvc;
use crate::ports::{failed, invalid_field, missing, require_user};

pub const REBUILD_SESSION: &str = "humata.v1.SessionService.RebuildSession";

/// The longest list of names or commits the note to the agent spells out.
pub const NAMED: usize = 10;

/// The file a rebuild's conversation is staged as, in `.carry/<name>`.
pub const STAGED_FILE: &str = "conversation.jsonl";

/// Where a channel keeps the markers of the rebuilds under way on this
/// machine, one file per session.
pub const MARKERS: &str = ".rebuilding";

/// How long a rebuilt session's note waits for the agent's input.
pub const NOTE_WAIT: Duration = Duration::from_secs(600);

#[derive(Debug, thiserror::Error)]
pub enum RebuildError {
    #[error("session {session} was not archived with its machine, so there is nothing to rebuild")]
    NotLost { session: String },
    #[error("{machine} is not one of the machines in channel {channel}")]
    NotInChannel { machine: String, channel: String },
    #[error("session {session} was lost with {machine}; it can't be rebuilt there")]
    LostMachine { session: String, machine: String },
    #[error("only the host places a rebuilt session on a machine")]
    NotTheHost,
    #[error("{folder:?} is not a plain folder name")]
    BadFolder { folder: String },
    #[error("couldn't read or save the records of session {session}")]
    Store {
        session: String,
        #[source]
        source: StoreError,
    },
    #[error("couldn't clone {owner}/{name} at branch {branch}")]
    Clone {
        owner: String,
        name: String,
        branch: String,
        #[source]
        source: GitError,
    },
    #[error("couldn't write the conversation to {path}")]
    Conversation {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("couldn't read the conversation the host sent to {path}")]
    ReadConversation {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{staged:?} is not where a host stages a conversation")]
    BadStaged { staged: String },
    #[error("the agent {agent} can't be started here")]
    Agent {
        agent: String,
        #[source]
        source: AgentError,
    },
    #[error("couldn't write {path}, which a restart reads to take a cut rebuild back")]
    Marker {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("couldn't remove {path}")]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the daemon stopped the rebuild's work early")]
    Task(#[source] tokio::task::JoinError),
}

/// Whether `staged` is `.carry/<name>/conversation.jsonl` with a name of
/// letters, digits and underscores only: the one shape a host sends.
pub fn staged_shape(staged: &str) -> bool {
    let mut parts = staged.split('/');
    let (carry, name, file, rest) = (parts.next(), parts.next(), parts.next(), parts.next());
    carry == Some(humata2_sessions::CARRY_DIR)
        && name.is_some_and(|name| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        && file == Some(STAGED_FILE)
        && rest.is_none()
}

/// What the names of the folders a session's conversation is staged in
/// start with: unique to the session, so a machine can tell the ones an
/// earlier, cut ask left from those of other rebuilds.
pub fn stage_prefix(session: &str) -> String {
    let plain: String = session
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    format!("rb_{plain}_")
}

/// Removes the folders under `.carry` that an earlier ask for `session`
/// staged and nothing took back (a host cut after it sent the conversation
/// and before the machine began), except `keep`. Best effort.
pub fn sweep_stale_stages(channel_dir: &Path, session: &str, keep: &str) {
    let prefix = stage_prefix(session);
    let Ok(entries) = std::fs::read_dir(channel_dir.join(humata2_sessions::CARRY_DIR)) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let stale = name
            .strip_prefix(&prefix)
            .is_some_and(|rest| rest.len() == 32 && rest.chars().all(|c| c.is_ascii_hexdigit()));
        if stale && name != keep {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The folder `staged` (`.carry/<name>/conversation.jsonl`) is in, under
/// the channel's folder `channel_dir`; none when `staged` isn't that shape.
pub fn staged_folder(channel_dir: &Path, staged: &str) -> Option<PathBuf> {
    staged_shape(staged).then(|| {
        channel_dir
            .join(humata2_sessions::CARRY_DIR)
            .join(staged_dir_name(staged))
    })
}

/// The name of the folder under `.carry` that `staged` is in, or empty.
pub fn staged_dir_name(staged: &str) -> String {
    staged.split('/').nth(1).unwrap_or_default().to_owned()
}

/// A plain name for one folder: no path, no dots that climb.
pub fn plain(folder: &str) -> bool {
    !folder.is_empty() && folder != "." && folder != ".." && !folder.contains(['/', '\\', '\0'])
}

/// `names` as a list of at most `NAMED`, with how many more there were.
pub fn listed(names: &[String]) -> String {
    let shown = names
        .iter()
        .take(NAMED)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    match names.len().saturating_sub(NAMED) {
        0 => shown,
        more => format!("{shown} and {more} more"),
    }
}

/// What to tell the agent: where it was rebuilt and what the machine took
/// with it. One line, because it is typed at the agent's prompt.
pub fn note_of(session: &pb::ArchiveSummary, lost_machine: &str, on: &str) -> String {
    let mut parts = vec![format!(
        "Humata rebuilt this session on {on} because {lost_machine} is gone. Each repo is back at the last commit that was pushed to its branch."
    )];
    for repo in &session.repos {
        let Some(named) = &repo.repo else { continue };
        let mut lost = Vec::new();
        if !repo.files.is_empty() {
            let paths: Vec<String> = repo.files.iter().map(|file| file.path.clone()).collect();
            lost.push(format!("uncommitted changes to {}", listed(&paths)));
        }
        if !repo.unpushed_subjects.is_empty() {
            let subjects: Vec<String> = repo
                .unpushed_subjects
                .iter()
                .map(|subject| format!("\"{subject}\""))
                .collect();
            lost.push(format!(
                "commits that were never pushed: {}",
                listed(&subjects)
            ));
        }
        if !lost.is_empty() {
            parts.push(format!(
                "{}/{} lost {}.",
                named.owner,
                named.name,
                lost.join(" and ")
            ));
        }
    }
    if parts.len() == 1 {
        parts.push("Nothing was lost.".to_owned());
    }
    parts.push(
        "A name in ‹›, like ‹NAME›, stands for a host variable whose value was removed."
            .to_owned(),
    );
    parts.join(" ")
}

/// `data` with every `from` replaced by `to`.
pub fn replaced(data: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    if from.is_empty() || from == to {
        return data.to_vec();
    }
    let mut out = Vec::with_capacity(data.len());
    let mut at = 0;
    while at < data.len() {
        if data[at..].starts_with(from) {
            out.extend_from_slice(to);
            at += from.len();
        } else {
            out.push(data[at]);
            at += 1;
        }
    }
    out
}

/// Writes the conversation file's lines, whole or not at all.
pub fn write_lines(path: &Path, lines: &[u8]) -> Result<(), RebuildError> {
    let fail = |source| RebuildError::Conversation {
        path: path.to_owned(),
        source,
    };
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(fail)?;
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, lines).map_err(fail)?;
    std::fs::rename(&partial, path).map_err(fail)
}

pub fn marker_file(channel_dir: &Path, session: &str) -> PathBuf {
    channel_dir.join(MARKERS).join(session)
}

/// A rebuild the last run began on this machine and didn't end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CutRebuild {
    pub marker: PathBuf,
    pub session: String,
    pub folder: PathBuf,
    pub staged: Option<PathBuf>,
}

/// How a cut rebuild ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CutEnd {
    /// Never launched its agent: its folder and records were removed.
    TakenBack,
    /// Its agent was launched: the session stays.
    Kept,
}

/// Takes back a cut rebuild whose agent was never launched.
pub fn undo_cut(store: &Store, rebuild: &CutRebuild) -> Result<CutEnd, RebuildError> {
    let session = &rebuild.session;
    let store_error = |source| RebuildError::Store {
        session: session.clone(),
        source,
    };
    let launched = store.agent(session).map_err(store_error)?.is_some()
        || store
            .session_detail(session)
            .map_err(store_error)?
            .is_some_and(|detail| detail.start == StartStep::Done);
    if !launched {
        remove_dir(&rebuild.folder)?;
        store.remove_session(session).map_err(store_error)?;
    }
    if let Some(staged) = &rebuild.staged {
        remove_dir(staged)?;
    }
    match std::fs::remove_file(&rebuild.marker) {
        Err(source) if source.kind() != io::ErrorKind::NotFound => Err(RebuildError::Remove {
            path: rebuild.marker.clone(),
            source,
        }),
        _ => Ok(if launched {
            CutEnd::Kept
        } else {
            CutEnd::TakenBack
        }),
    }
}

fn remove_dir(path: &Path) -> Result<(), RebuildError> {
    match std::fs::remove_dir_all(path) {
        Err(source) if source.kind() != io::ErrorKind::NotFound => Err(RebuildError::Remove {
            path: path.to_owned(),
            source,
        }),
        _ => Ok(()),
    }
}

impl SessionTransferSvc {
    /// Handles `RebuildSession` for `SessionTransferPort`.
    pub(crate) async fn handle_rebuild_session(
        &self,
        caller: &Caller,
        req: pb::RebuildSessionRequest,
    ) -> Result<pb::Operation, Status> {
        require_user(caller)?;
        let id = req
            .id
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| missing("id"))?;
        let target = req
            .machine
            .map(|machine| machine.value)
            .filter(|machine| !machine.is_empty())
            .ok_or_else(|| missing("machine"))?;

        let loaded = self.env.load_session(&self.core, &id)?;
        if target == loaded.record.machine {
            return Err(failed(
                Code::FailedPrecondition,
                &RebuildError::LostMachine {
                    session: id,
                    machine: target,
                },
                "pick another machine in the channel",
                None,
            ));
        }
        if self.env.is_host(&target) {
            return Err(invalid_field(
                "machine",
                Problem::NotAllowedHere,
                "a rebuild on the host's own machine isn't built yet",
                "pick another machine in the channel",
            ));
        }

        // Mirror/carry/remote rebuild orchestration is wired by the host adapter.
        Err(self.env.not_wired(REBUILD_SESSION))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_staged_conversation_is_in_its_own_folder_under_the_channels_carry() {
        let channel = Path::new("/h/ch/payments");
        assert_eq!(
            staged_folder(channel, ".carry/rb_s1_0f3a/conversation.jsonl"),
            Some(channel.join(".carry").join("rb_s1_0f3a"))
        );
        assert_eq!(staged_folder(channel, ".carry/../conversation.jsonl"), None);
        assert_eq!(staged_folder(channel, ""), None);
    }

    #[test]
    fn only_a_carry_folder_of_plain_name_holding_the_conversation_is_staged() {
        assert!(staged_shape(".carry/rb_0f3a/conversation.jsonl"));
        for bad in [
            "",
            ".carry/conversation.jsonl",
            ".carry//conversation.jsonl",
            ".carry/../conversation.jsonl",
            ".carry/rb_0f3a/other.jsonl",
            ".carry/rb_0f3a/conversation.jsonl/more",
            "sessions/rb_0f3a/conversation.jsonl",
            "/etc/rb_0f3a/conversation.jsonl",
            ".carry/rb-0f3a/conversation.jsonl",
        ] {
            assert!(!staged_shape(bad), "{bad:?} was taken as staged");
        }
    }

    #[test]
    fn plain_rejects_paths_and_dot_segments() {
        assert!(plain("fix-login"));
        for bad in ["", ".", "..", "a/b", "a\\b", "a\0b"] {
            assert!(!plain(bad), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn replaced_rewrites_all_occurrences() {
        assert_eq!(
            replaced(b"/old/a and /old/b", b"/old", b"/new"),
            b"/new/a and /new/b"
        );
        assert_eq!(replaced(b"same", b"", b"x"), b"same");
    }

    #[test]
    fn note_of_reports_nothing_lost_when_clean() {
        let summary = pb::ArchiveSummary::default();
        let note = note_of(&summary, "old-box", "new-box");
        assert!(note.contains("old-box"));
        assert!(note.contains("new-box"));
        assert!(note.contains("Nothing was lost."));
    }
}
