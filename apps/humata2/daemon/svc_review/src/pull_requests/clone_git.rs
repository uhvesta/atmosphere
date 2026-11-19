//! PR-4, PR-21: a review started on another machine keeps its clones there.
//! The host keeps the review's records and asks GitHub through its own `gh`,
//! and looks at the clones through the link: each `git` it would run in a
//! clone folder is run on the machine instead
//! (`SessionCarryService.RunReviewGit`), and the agent is told through the
//! machine's terminal.
//!
//! A clone's path in the host's records is the one the host placed the
//! session at; it says which session and which clone, the machine's own
//! folder is the machine's. The check is the session's record: a review whose
//! session lives on a machine other than the host's is remote.
//!
//! The callers are blocking, so a call here waits on the link with the
//! runtime's handle, never on an async thread.

use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{ExitStatus, Output};
use std::sync::{Arc, OnceLock};

use humata2_grpc::humata::v1 as pb;
use humata2_review_diff::bracketed_paste;
use humata2_store::Store;
use tokio::runtime::Handle;
use tonic::Status;

use crate::ports::store_error;

/// Port for reaching a review's clones and terminal on another machine.
#[tonic::async_trait]
pub trait ReviewRemote: Send + Sync {
    async fn run_review_git(
        &self,
        machine: &str,
        request: pb::RunReviewGitRequest,
    ) -> Result<pb::RunReviewGitResponse, Status>;

    async fn type_terminal(
        &self,
        machine: &str,
        request: pb::TypeRequest,
    ) -> Result<(), Status>;

    async fn get_diff(
        &self,
        machine: &str,
        request: pb::GetDiffRequest,
    ) -> Result<pb::GetDiffResponse, Status>;

    async fn get_file_diff(
        &self,
        machine: &str,
        request: pb::GetFileDiffRequest,
    ) -> Result<pb::FileDiff, Status>;
}

/// What reaches a review's clones on other machines, set once when the
/// review service is given the remote link.
struct Routes {
    store: Arc<Store>,
    /// This daemon's own machine id.
    host: String,
    remote: Arc<dyn ReviewRemote>,
    runtime: Handle,
}

static ROUTES: OnceLock<Routes> = OnceLock::new();

/// Lets reviews on other machines be reached. Once; a later call changes
/// nothing. Does nothing outside a running runtime.
pub(super) fn install(store: Arc<Store>, host: String, remote: Arc<dyn ReviewRemote>) {
    if let Ok(runtime) = Handle::try_current() {
        let _ = ROUTES.set(Routes {
            store,
            host,
            remote,
            runtime,
        });
    }
}

/// Why a review's clone on another machine couldn't be reached.
#[derive(Debug, thiserror::Error)]
#[error("couldn't run git in the review's clone {repo} on {machine}")]
pub(super) struct RemoteGitError {
    machine: String,
    repo: String,
    /// What the machine or the link said, as the person should read it.
    #[source]
    source: Status,
}

impl RemoteGitError {
    pub(super) fn status(&self) -> Status {
        self.source.clone()
    }
}

/// Where a clone is when it isn't on this machine.
struct Where {
    machine: String,
    session: String,
    repo: String,
}

impl Routes {
    /// The machine, session and clone name of `clone` when the session it
    /// is in lives on another machine; `None` when it is here. A store that
    /// can't be read is an error, never a guess that the clone is here.
    fn locate(&self, clone: &Path) -> Result<Option<Where>, RemoteGitError> {
        let (Some(folder), Some(repo)) = (clone.parent(), clone.file_name().and_then(|name| name.to_str())) else {
            return Ok(None);
        };
        let sessions = self.store.sessions().map_err(|error| RemoteGitError {
            machine: self.host.clone(),
            repo: repo.to_owned(),
            source: store_error(error, &self.host, "find which machine holds the review's clone"),
        })?;
        Ok(sessions
            .into_iter()
            .find(|session| session.folder == folder && !session.machine.is_empty() && session.machine != self.host)
            .map(|session| Where {
                machine: session.machine,
                session: session.id,
                repo: repo.to_owned(),
            }))
    }

    /// The machine session `session` lives on, when it isn't this one.
    fn machine_of(&self, session: &str) -> Option<String> {
        let record = self.store.session(session).ok()??;
        (!record.machine.is_empty() && record.machine != self.host).then_some(record.machine)
    }

    async fn run(&self, at: &Where, args: &[&str], stdin: Option<&[u8]>) -> Result<Output, RemoteGitError> {
        let response = self
            .remote
            .run_review_git(
                &at.machine,
                pb::RunReviewGitRequest {
                    session: Some(pb::SessionId {
                        value: at.session.clone(),
                    }),
                    repo: at.repo.clone(),
                    args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                    stdin: stdin.map(<[u8]>::to_vec).unwrap_or_default(),
                },
            )
            .await
            .map_err(|source| RemoteGitError {
                machine: at.machine.clone(),
                repo: at.repo.clone(),
                source,
            })?;
        // A raw wait status: the exit code is the second byte, a signal the first.
        let raw = if response.exit_code >= 0 { response.exit_code << 8 } else { 9 };
        Ok(Output {
            status: ExitStatus::from_raw(raw),
            stdout: response.stdout,
            stderr: response.stderr,
        })
    }
}

/// Runs `git args` (with `stdin`) in `clone` on the machine it is on, when
/// that isn't this one: `None` when it is here. A git that fails is an
/// answer; an error is the machine not being reached. Blocking.
pub(super) fn remote(clone: &Path, args: &[&str], stdin: Option<&[u8]>) -> Option<Result<Output, RemoteGitError>> {
    let routes = ROUTES.get()?;
    let at = match routes.locate(clone) {
        Ok(Some(at)) => at,
        Ok(None) => return None,
        Err(error) => return Some(Err(error)),
    };
    // Every caller runs inside `spawn_blocking` (heads, submit, commits);
    // there waiting on the runtime's handle is allowed.
    Some(routes.runtime.block_on(routes.run(&at, args, stdin)))
}

/// The machine session `session` lives on, when it isn't this one.
pub(super) fn machine_of(session: &str) -> Option<String> {
    ROUTES.get()?.machine_of(session)
}

/// The machine and remote client for `session`, when it lives on another machine.
pub(crate) fn remote_for(session: &str) -> Option<(String, Arc<dyn ReviewRemote>)> {
    let routes = ROUTES.get()?;
    let machine = routes.machine_of(session)?;
    Some((machine, Arc::clone(&routes.remote)))
}

/// Types `text` to the agent of review `session` on the machine it runs on
/// as one paste and a return, once for `key`: `None` when the session is on
/// this machine.
pub(super) async fn tell_remote(session: &str, text: &str, key: &str) -> Option<Result<(), Status>> {
    let routes = ROUTES.get()?;
    let machine = routes.machine_of(session)?;
    let paste = String::from_utf8_lossy(&bracketed_paste(text)).into_owned();
    // A trailing newline is the submit (D671): the agent's daemon
    // presses return once the agent has shown the paste (BUG-T1).
    let request = pb::TypeRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
        session: Some(pb::SessionId {
            value: session.to_owned(),
        }),
        tab: None,
        text: format!("{paste}\n"),
        when: pb::type_request::When::Now as i32,
    };
    Some(routes.remote.type_terminal(&machine, request).await)
}
