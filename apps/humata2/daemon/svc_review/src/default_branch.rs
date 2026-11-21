//! FindDefaultBranch (frame 98): when a clone doesn't know its remote's
//! default branch, Humata asks the remote for it in that clone, on the
//! session's machine, instead of telling the person to run git.

use std::path::Path;
use std::process::Command;

use humata2_access::{Capability, SessionRef};
use humata2_daemon_ports::Caller;
use crate::pb;
use humata2_review_diff::{DiffError, wire};
use humata2_store::SessionRecord;
use tonic::{Code, Status};

use super::bases::SCRUBBED;
use super::{Review, blocking, diff_failed, folder_name, repo_keys};
use crate::ports::{error, failure, invalid_field};

/// Where origin/HEAD points once the remote has named its default branch.
const ORIGIN_HEAD: &str = "refs/remotes/origin/HEAD";

/// A failed read of the default bases, as the client sees it.
pub(super) fn default_failed(err: &DiffError, keys: &[String]) -> Status {
    let DiffError::NoDefaultBranch { repo } = err else {
        return diff_failed(err);
    };
    let folder = repo
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let key = keys
        .iter()
        .find(|key| folder_name(key) == folder)
        .cloned()
        .unwrap_or_else(|| folder.to_owned());
    failure(
        Code::FailedPrecondition,
        error(
            format!("{key} has no default branch to compare with"),
            "use Find the default branch, which asks the remote for it, or compare against a branch you pick",
            pb::error::Kind::NoDefaultBranch(pb::NoDefaultBranch {
                repo: Some(wire::repo_ref(&key)),
            }),
        ),
    )
}

/// Asks `repo`'s remote for its default branch in the session's clone.
pub(super) async fn find(
    review: &Review,
    caller: &Caller,
    record: &SessionRecord,
    session: &SessionRef,
    repo: &pb::RepoRef,
) -> Result<pb::FindDefaultBranchResponse, Status> {
    review.require_use(caller, Capability::UserReviewState, session)?;
    let key = wire::repo_key(repo);
    repo_keys(&review.inner, record, std::slice::from_ref(repo)).await?;
    let folder = folder_name(&key).to_owned();
    let repos = repo_keys(&review.inner, record, &[]).await?;
    if !repos.iter().any(|known| folder_name(known) == folder.as_str()) {
        return Err(invalid_field(
            "repo",
            pb::invalid_field::Problem::BadFormat,
            format!("{key} isn't a repository of this session"),
            "name one of the session's repositories as owner/name, like acme/api",
        ));
    }
    let clone = record.folder.join(&folder);
    let default_branch = blocking(move || set_head(&clone, &key)).await??;
    Ok(pb::FindDefaultBranchResponse { default_branch })
}

fn set_head(clone: &Path, key: &str) -> Result<String, Status> {
    git(clone, &["remote", "set-head", "origin", "--auto"]).map_err(|stderr| {
        git_failed(
            format!("couldn't ask {key}'s remote for its default branch"),
            "check that this machine can reach the remote and is signed in to it, then try again; or compare against a branch you pick",
            stderr,
        )
    })?;
    let named = git(clone, &["symbolic-ref", "--quiet", ORIGIN_HEAD]).map_err(|stderr| {
        git_failed(
            format!("{key}'s remote didn't name a default branch"),
            "compare against a branch you pick; once the remote has a default branch, try again",
            stderr,
        )
    })?;
    match named.trim().strip_prefix("refs/remotes/origin/") {
        Some(branch) if !branch.is_empty() => Ok(branch.to_owned()),
        _ => Err(git_failed(
            format!(
                "{key}'s remote named {} as its default branch, which isn't a branch of origin",
                named.trim()
            ),
            "compare against a branch you pick",
            String::new(),
        )),
    }
}

fn git(clone: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("git");
    command.current_dir(clone);
    for name in SCRUBBED {
        command.env_remove(name);
    }
    let output = command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .args(args)
        .output()
        .map_err(|err| format!("couldn't run git in {}: {err}", clone.display()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

fn git_failed(message: String, fix: &str, stderr_tail: String) -> Status {
    failure(
        Code::FailedPrecondition,
        error(
            message,
            fix,
            pb::error::Kind::GitFailed(pb::GitFailed {
                stderr_tail,
                ..pb::GitFailed::default()
            }),
        ),
    )
}
