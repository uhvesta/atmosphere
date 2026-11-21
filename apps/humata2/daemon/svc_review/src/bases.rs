//! ListBases (B3): the branches one repository's changes can be compared
//! with, read from the clone's `refs/remotes/origin/*` as of its last
//! fetch. Nothing is fetched here.

use std::path::Path;
use std::process::Command;

use humata2_access::SessionRef;
use humata2_daemon_ports::Caller;
use crate::pb;
use humata2_review_diff::wire;
use humata2_store::SessionRecord;
use tonic::{Code, Status};

use super::{Review, blocking, folder_name, repo_keys};
use crate::ports::{error, failure, internal, invalid_field};

/// Where a remote branch's ref starts.
const REMOTE_PREFIX: &str = "refs/remotes/origin/";

/// Variables that would point git at another repository or index.
pub(super) const SCRUBBED: [&str; 7] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_COMMON_DIR",
];

/// The branches of `repo`'s clone in the session, default branch first.
pub(super) async fn list(
    review: &Review,
    caller: &Caller,
    record: &SessionRecord,
    session: &SessionRef,
    repo: &pb::RepoRef,
) -> Result<pb::ListBasesResponse, Status> {
    let key = wire::repo_key(repo);
    let asked = repo_keys(&review.inner, record, std::slice::from_ref(repo)).await?;
    review.require_read(caller, session, &asked)?;
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
    blocking(move || remote_branches(&clone, &key)).await?
}

fn remote_branches(clone: &Path, key: &str) -> Result<pb::ListBasesResponse, Status> {
    let mut command = Command::new("git");
    command.current_dir(clone);
    for name in SCRUBBED {
        command.env_remove(name);
    }
    let output = command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .args(["for-each-ref", "--format=%(refname) %(symref)", "refs/remotes/origin"])
        .output()
        .map_err(|err| internal(format!("couldn't run git in {}: {err}", clone.display()), ""))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(failure(
            Code::FailedPrecondition,
            error(
                format!("git couldn't list the remote branches of {key}"),
                format!(
                    "check that {} is an intact git clone, then ask again",
                    clone.display()
                ),
                pb::error::Kind::GitFailed(pb::GitFailed {
                    stderr_tail: stderr,
                    ..pb::GitFailed::default()
                }),
            ),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut branches = Vec::new();
    let mut default_branch = String::new();
    for line in text.lines() {
        let (refname, symref) = line.split_once(' ').unwrap_or((line, ""));
        let Some(branch) = refname.strip_prefix(REMOTE_PREFIX) else {
            continue;
        };
        if branch == "HEAD" {
            default_branch = symref.strip_prefix(REMOTE_PREFIX).unwrap_or("").to_owned();
        } else {
            branches.push(branch.to_owned());
        }
    }
    match branches.iter().position(|branch| *branch == default_branch) {
        Some(index) => {
            let default = branches.remove(index);
            branches.insert(0, default);
        }
        None => default_branch.clear(),
    }
    Ok(pb::ListBasesResponse {
        branches,
        default_branch,
    })
}
