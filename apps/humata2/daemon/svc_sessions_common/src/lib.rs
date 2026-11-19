//! Shared state, helpers, and port traits for the 3-way `SessionService` split
//! (`svc_sessions_local`, `svc_sessions_transfer`, `svc_sessions_archive`;
//! SPLIT_PHASE3_DAEMON §2.3).

#[allow(unused_imports, ambiguous_glob_reexports)]
pub mod pb {
    pub use humata2_grpc_core::humata::v1::*;
    pub use humata2_grpc_inbox::humata::v1::*;
    pub use humata2_grpc_peer::humata::v1::*;
    pub use humata2_grpc_sessions::humata::v1::*;
}

pub mod archive;
pub mod archive_remote;
pub mod fork;
pub mod push;
pub mod remote;
pub mod start_fresh;
pub mod watch;

use std::sync::{Arc, Mutex, OnceLock};

use humata2_daemon_ports::Caller;
use humata2_rpc_kit::{chain, failure, invalid_field, missing, not_found};
use humata2_sessions::{StartError, Starter};
use humata2_store::{ResumeFailure, SessionDetail, SessionRecord, Store};
use pb::invalid_field::Problem;
use tonic::{Code, Status};

pub use humata2_daemon_ports::{SessionArchivePort, SessionTransferPort};
pub use humata2_rpc_kit::now_ms;

/// The longest session title taken.
pub const TITLE_MAX: usize = 200;

/// Callback invoked whenever a session is archived or its folder is freed.
pub type EndedHook = Arc<dyn Fn(&str) + Send + Sync>;

/// Narrow port for publishing session changes onto the daemon's event feed.
pub trait SessionFeedPort: Send + Sync {
    fn publish_session(&self, channel_id: &str, session_id: &str, change: pb::Change, session: pb::Session);
}

/// Narrow port for looking up remote session state reported by a peer machine.
pub trait RemoteSessionPort: Send + Sync {
    fn is_local(&self, machine: &str) -> bool;
}

/// Puts a session as the store holds it now on the event feed.
#[derive(Clone)]
pub struct Tell {
    store: Arc<Store>,
    machine: String,
    feed: Option<Arc<dyn SessionFeedPort>>,
    endings: Arc<Mutex<Vec<EndedHook>>>,
}

impl Tell {
    pub fn new(store: Arc<Store>, machine: String) -> Self {
        Self {
            store,
            machine,
            feed: None,
            endings: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn with_feed(mut self, feed: Arc<dyn SessionFeedPort>) -> Self {
        self.feed = Some(feed);
        self
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn machine(&self) -> &str {
        &self.machine
    }

    /// Registers a hook called whenever a session is archived or freed.
    pub fn on_ended(&self, hook: impl Fn(&str) + Send + Sync + 'static) {
        let mut guard = self.endings.lock().unwrap_or_else(|e| e.into_inner());
        guard.push(Arc::new(hook));
    }

    /// Notifies all registered ending hooks that `session` ended.
    pub fn ended(&self, session: &str) {
        let hooks: Vec<EndedHook> = self
            .endings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for hook in hooks {
            hook(session);
        }
    }

    pub fn publish(&self, channel_id: &str, session_id: &str, change: pb::Change, session: pb::Session) {
        if let Some(feed) = &self.feed {
            feed.publish_session(channel_id, session_id, change, session);
        }
    }
}

/// A loaded session record and its detail, ready for handlers.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub record: SessionRecord,
    pub detail: SessionDetail,
}

/// Parent/fork lineage metadata for a session family.
#[derive(Clone, Debug, Default)]
pub struct Lineage {
    pub parent: Option<String>,
    pub forks: Vec<String>,
}

/// Publishes updates for every session in `family` via `tell`.
pub fn publish_family(
    tell: &Tell,
    family: &[(String, String, pb::Session)],
    change: pb::Change,
) {
    for (channel_id, session_id, session) in family {
        tell.publish(channel_id, session_id, change, session.clone());
    }
}

/// Shared core state held by `svc_sessions_local`, `svc_sessions_transfer`,
/// and `svc_sessions_archive`.
#[derive(Clone)]
pub struct SessionCore {
    pub store: Arc<Store>,
    pub starter: Starter,
    pub machine: String,
    pub tell: Tell,
    pub transfer: Arc<OnceLock<Arc<dyn SessionTransferPort>>>,
    pub archive: Arc<OnceLock<Arc<dyn SessionArchivePort>>>,
}

impl SessionCore {
    pub fn new(store: Arc<Store>, starter: Starter, machine: String, tell: Tell) -> Self {
        Self {
            store,
            starter,
            machine,
            tell,
            transfer: Arc::new(OnceLock::new()),
            archive: Arc::new(OnceLock::new()),
        }
    }
}

/// Validates and trims a session title.
pub fn clean_title(raw: &str) -> Result<String, Status> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(missing("title"));
    }
    if trimmed.chars().count() > TITLE_MAX {
        return Err(invalid_field(
            "title",
            Problem::OutOfRange,
            format!("title is longer than {TITLE_MAX} characters"),
            "shorten the session title",
        ));
    }
    Ok(trimmed.to_owned())
}

/// Maps a [`StartError`] from `humata2_sessions` into a gRPC [`Status`].
pub fn start_error(error: &StartError) -> Status {
    failure(
        Code::FailedPrecondition,
        pb::Error {
            message: chain(error),
            fix: "check the machine, channel repositories, and agent setup, then try again"
                .to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// The reason a failed resume gives, as the API names it.
pub fn resume_reason(reason: ResumeFailure) -> pb::resume_failed::Reason {
    use pb::resume_failed::Reason;
    match reason {
        ResumeFailure::ConversationFileMissing => Reason::ConversationFileMissing,
        ResumeFailure::AgentStartedAnotherConversation => Reason::AgentStartedAnotherConversation,
        ResumeFailure::AgentExited => Reason::AgentExited,
        ResumeFailure::AgentNotInstalled => Reason::AgentNotInstalled,
        ResumeFailure::FolderMissing => Reason::FolderMissing,
        ResumeFailure::ConversationUnknown => Reason::ConversationUnknown,
    }
}

/// Converts a stored [`ResumeFailure`] into its wire [`pb::ResumeFailed`] form.
pub fn resume_failed(
    session: &str,
    reason: ResumeFailure,
    exit_code: Option<i32>,
) -> pb::ResumeFailed {
    pb::ResumeFailed {
        session: Some(pb::SessionId {
            value: session.to_owned(),
        }),
        reason: resume_reason(reason) as i32,
        exit_code: exit_code.unwrap_or(0),
    }
}

/// Helper to check whether `caller` is a user caller.
pub fn require_user(caller: &Caller) -> Result<(), Status> {
    if matches!(caller, Caller::User(_)) {
        return Ok(());
    }
    Err(failure(
        Code::PermissionDenied,
        pb::Error {
            message: "only the user can perform this session action".to_owned(),
            fix: "run this action from the Humata app or CLI as the user".to_owned(),
            retry: None,
            kind: Some(pb::error::Kind::UserOnly(pb::UserOnly::default())),
        },
    ))
}

/// Helper for missing session errors.
pub fn session_not_found(id: &str) -> Status {
    not_found(
        pb::EntityKind::Session,
        id,
        format!("session {id} was not found"),
        "Check the session ID or list sessions to see active ones.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_title_trims_and_validates() {
        assert_eq!(clean_title("  fix bug  ").unwrap(), "fix bug");
        assert_eq!(clean_title("   ").unwrap_err().code(), Code::InvalidArgument);
        let long = "a".repeat(TITLE_MAX + 1);
        assert_eq!(clean_title(&long).unwrap_err().code(), Code::InvalidArgument);
    }

    #[test]
    fn human_bytes_formats_sizes() {
        assert_eq!(fork::human_bytes(512), "512 B");
        assert_eq!(fork::human_bytes(2048), "2.0 KiB");
        assert_eq!(fork::human_bytes(3 * 1024 * 1024), "3.0 MiB");
    }
}
