//! SessionCarryService.RunReviewGit (PR-4, PR-21): git in one clone of a pull
//! request review that runs on this machine, for the host. The host keeps the
//! review's records and asks GitHub through its own `gh`; what it needs to
//! know of the clones (their heads, what changed between two of them, the
//! lines of a diff) it asks here, and git runs where the clones are.
//!
//! Only the host calls this (`require_host`), and only for a clone of a review
//! session this machine holds, named by the session and the clone's folder
//! name, so a path of the caller's is never used.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use humata2_grpc::humata::v1 as pb;
use humata2_store::{Store, StoreError};
use tonic::{Code, Status};

use super::SessionCarrySvc;
use crate::channel_service::failed;
use crate::trash_service::blocking;

/// The most arguments one git call carries; the host's calls use a few.
const ARGS_MAX: usize = 64;

#[derive(Debug, thiserror::Error)]
enum ReviewGitError {
    #[error("session {session} is not a pull request review on this machine")]
    NotReview { session: String },
    #[error("the review {session} has no clone {repo:?} on this machine")]
    NoClone { session: String, repo: String },
    #[error("no git command was asked for")]
    NoArgs,
    #[error("{count} arguments are more than one call to a review's clone carries")]
    TooMany { count: usize },
    #[error("couldn't read the records of session {session}")]
    Store {
        session: String,
        #[source]
        source: StoreError,
    },
    #[error("couldn't run git in {path}")]
    Spawn {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl ReviewGitError {
    fn status(&self) -> Status {
        match self {
            Self::NotReview { .. } | Self::NoClone { .. } => failed(
                Code::NotFound,
                self,
                "the review's session isn't on this machine any more: look at it from the host's session list, or start the review again",
                None,
            ),
            Self::NoArgs | Self::TooMany { .. } => failed(
                Code::InvalidArgument,
                self,
                "the host asks for one git command at a time; update the Humata tools on both machines from the machine's page",
                None,
            ),
            Self::Store { .. } => failed(
                Code::Internal,
                self,
                "check that this machine's Humata state folder is readable and has room, then try again",
                None,
            ),
            Self::Spawn { .. } => failed(
                Code::FailedPrecondition,
                self,
                "install git on this machine and try again",
                None,
            ),
        }
    }
}

/// Runs `git args` in the clone `repo` of review `session`, feeding it
/// `stdin`. A git that fails is an answer, not an error. Blocking.
fn run(
    store: &Store,
    session: &str,
    repo: &str,
    args: &[String],
    stdin: &[u8],
) -> Result<pb::RunReviewGitResponse, ReviewGitError> {
    if args.is_empty() {
        return Err(ReviewGitError::NoArgs);
    }
    if args.len() > ARGS_MAX {
        return Err(ReviewGitError::TooMany { count: args.len() });
    }
    let store_failed = |source| ReviewGitError::Store {
        session: session.to_owned(),
        source,
    };
    let review = store
        .review_session(session)
        .map_err(store_failed)?
        .ok_or_else(|| ReviewGitError::NotReview {
            session: session.to_owned(),
        })?;
    let record = store
        .session(session)
        .map_err(store_failed)?
        .ok_or_else(|| ReviewGitError::NotReview {
            session: session.to_owned(),
        })?;
    if !review.prs.iter().any(|pr| pr.name == repo) {
        return Err(ReviewGitError::NoClone {
            session: session.to_owned(),
            repo: repo.to_owned(),
        });
    }
    let clone = record.folder.join(repo);
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&clone)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| ReviewGitError::Spawn {
            path: clone.clone(),
            source,
        })?;
    // Written on a thread so git's output is read meanwhile; a write that
    // fails shows as git's own failure.
    let writer = child.stdin.take().map(|mut input| {
        let bytes = stdin.to_vec();
        std::thread::spawn(move || input.write_all(&bytes))
    });
    let output = child
        .wait_with_output()
        .map_err(|source| ReviewGitError::Spawn { path: clone, source })?;
    if let Some(writer) = writer {
        drop(writer.join());
    }
    Ok(pb::RunReviewGitResponse {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

impl SessionCarrySvc {
    pub(super) async fn run_review_git(
        &self,
        session: String,
        repo: String,
        args: Vec<String>,
        stdin: Vec<u8>,
    ) -> Result<pb::RunReviewGitResponse, Status> {
        let store = self.starter.store.clone();
        blocking(move || run(&store, &session, &repo, &args, &stdin))
            .await?
            .map_err(|error| error.status())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call with no arguments, or with a crowd of them, is refused before
    /// the store is read.
    #[test]
    fn a_call_without_a_command_or_with_too_many_is_refused() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let store = Store::open(dir.path().join("store")).expect("a store");
        let none = run(&store, "se_1", "api", &[], &[]).expect_err("no command");
        assert!(matches!(none, ReviewGitError::NoArgs));
        let many = vec!["status".to_owned(); ARGS_MAX + 1];
        let crowd = run(&store, "se_1", "api", &many, &[]).expect_err("too many");
        assert!(matches!(crowd, ReviewGitError::TooMany { .. }));
    }

    /// A session that is no review has no clone to run git in.
    #[test]
    fn a_session_that_is_not_a_review_has_no_clone() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let store = Store::open(dir.path().join("store")).expect("a store");
        let args = vec!["status".to_owned()];
        let refused = run(&store, "se_1", "api", &args, &[]).expect_err("not a review");
        assert!(matches!(refused, ReviewGitError::NotReview { .. }));
    }
}
