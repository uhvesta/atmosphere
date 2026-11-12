//! Moving sessions' snapshots between machines as git bundles: from a host
//! into a mirror on the Mac after each turn, and from the mirror into a
//! fresh clone on the host a session is restored onto.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::WorkspaceError;
use crate::{git, git_with_input};

/// Names each bundle's temporary file, so two at once don't share one.
static BUNDLES: AtomicU64 = AtomicU64::new(0);

/// The refs a session's snapshots are kept by.
fn snapshot_refs(session: &str) -> String {
    format!("refs/humata/{session}/")
}

/// The commits every ref in `repository` points at, for the other side of a
/// bundle to leave out what is reachable from them.
pub async fn tips(repository: &Path) -> Result<Vec<String>, WorkspaceError> {
    let listed = git(repository, &["for-each-ref", "--format=%(objectname)"]).await?;
    let mut tips: Vec<String> = listed.lines().map(str::to_owned).collect();
    tips.sort();
    tips.dedup();
    Ok(tips)
}

/// A bundle of `session`'s snapshot refs in `repository`, without what is
/// reachable from those of `haves` it knows; None if it has nothing more.
pub async fn create(
    repository: &Path,
    session: &str,
    haves: &[String],
) -> Result<Option<Vec<u8>>, WorkspaceError> {
    let refs = snapshot_refs(session);
    let listed = git(repository, &["for-each-ref", "--format=%(refname)", &refs]).await?;
    if listed.trim().is_empty() {
        return Ok(None);
    }
    let known = known_commits(repository, haves).await?;
    let glob = format!("--glob={refs}*");
    let mut range = vec![glob.as_str()];
    if !known.is_empty() {
        range.push("--not");
        range.extend(known.iter().map(String::as_str));
    }
    let mut count = vec!["rev-list", "--count"];
    count.extend(&range);
    if git(repository, &count).await?.trim() == "0" {
        return Ok(None);
    }
    let file = scratch_file();
    let path = file.to_string_lossy().into_owned();
    let mut create = vec!["bundle", "create", "--quiet", path.as_str()];
    create.extend(&range);
    let made = async {
        git(repository, &create).await?;
        tokio::fs::read(&file)
            .await
            .map_err(|source| WorkspaceError::Write {
                path: file.clone(),
                source,
            })
    }
    .await;
    let _ = tokio::fs::remove_file(&file).await;
    made.map(Some)
}

/// Takes every snapshot ref in `bundle` into `repository`.
pub async fn apply(repository: &Path, bundle: &[u8]) -> Result<(), WorkspaceError> {
    let file = scratch_file();
    tokio::fs::write(&file, bundle)
        .await
        .map_err(|source| WorkspaceError::Write {
            path: file.clone(),
            source,
        })?;
    let path = file.to_string_lossy().into_owned();
    let fetched = git(
        repository,
        &[
            "fetch",
            "--quiet",
            "--no-write-fetch-head",
            &path,
            "refs/humata/*:refs/humata/*",
        ],
    )
    .await;
    let _ = tokio::fs::remove_file(&file).await;
    fetched.map(drop)
}

/// Makes a bare repository at `path` to hold snapshots, if there is none.
pub async fn mirror(path: &Path) -> Result<(), WorkspaceError> {
    if path.join("HEAD").is_file() {
        return Ok(());
    }
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|source| WorkspaceError::CreateDirectory {
            path: path.to_owned(),
            source,
        })?;
    git(path, &["init", "--quiet", "--bare"]).await.map(drop)
}

/// Those of `commits` that `repository` has.
async fn known_commits(
    repository: &Path,
    commits: &[String],
) -> Result<Vec<String>, WorkspaceError> {
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let mut input = commits.join("\n");
    input.push('\n');
    let checked = git_with_input(repository, &["cat-file", "--batch-check"], &input).await?;
    Ok(checked
        .lines()
        .filter_map(|line| {
            let (commit, kind) = line.split_once(' ')?;
            (!kind.starts_with("missing")).then(|| commit.to_owned())
        })
        .collect())
}

fn scratch_file() -> PathBuf {
    std::env::temp_dir().join(format!(
        "humata-{}-{}.bundle",
        std::process::id(),
        BUNDLES.fetch_add(1, Ordering::Relaxed)
    ))
}
