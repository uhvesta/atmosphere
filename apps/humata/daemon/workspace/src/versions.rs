//! A file's versions across a session's turns, from its snapshots.

use std::path::Path;

use crate::{Repository, Snapshot, WorkspaceError, git, git_with_input};

/// The checkpoints, by turn, where `path` in `repository` is there and
/// differs from the version before it, oldest first. Turn 0 is the code
/// before the first turn. Checkpoints without a snapshot of the repository
/// are skipped.
pub async fn versions(
    repository: &Repository,
    checkpoints: &[Vec<Snapshot>],
    path: &str,
) -> Result<Vec<usize>, WorkspaceError> {
    let turns: Vec<(usize, &Snapshot)> = checkpoints
        .iter()
        .enumerate()
        .filter_map(|(turn, snapshots)| {
            snapshots
                .iter()
                .find(|snapshot| snapshot.name == repository.name)
                .map(|snapshot| (turn, snapshot))
        })
        .collect();
    if turns.is_empty() {
        return Ok(Vec::new());
    }
    // One git for every checkpoint: each line names the blob or says it's
    // missing.
    let input: String = turns
        .iter()
        .map(|(_, snapshot)| format!("{}:{path}\n", snapshot.commit))
        .collect();
    let output = git_with_input(&repository.path, &["cat-file", "--batch-check"], &input).await?;
    let mut versions = Vec::new();
    let mut last: Option<&str> = None;
    for ((turn, _), line) in turns.iter().zip(output.lines()) {
        let mut fields = line.split(' ');
        let (Some(blob), Some("blob")) = (fields.next(), fields.next()) else {
            continue;
        };
        if last != Some(blob) {
            versions.push(*turn);
            last = Some(blob);
        }
    }
    Ok(versions)
}

/// `path` in `repository` as checkpoint `turn` has it.
pub async fn file_at(
    repository: &Repository,
    checkpoints: &[Vec<Snapshot>],
    turn: usize,
    path: &str,
) -> Result<String, WorkspaceError> {
    let snapshot = checkpoints
        .get(turn)
        .ok_or_else(|| WorkspaceError::UnknownTurn {
            turn,
            turns: checkpoints.len().saturating_sub(1),
        })?
        .iter()
        .find(|snapshot| snapshot.name == repository.name)
        .ok_or_else(|| WorkspaceError::NoSuchFileThen {
            path: path.to_owned(),
            repository: repository.name.clone(),
            turn,
        })?;
    let object = format!("{}:{path}", snapshot.commit);
    let directory: &Path = &repository.path;
    let missing = git(directory, &["cat-file", "-e", &object]).await.is_err();
    if missing {
        return Err(WorkspaceError::NoSuchFileThen {
            path: path.to_owned(),
            repository: repository.name.clone(),
            turn,
        });
    }
    git(directory, &["cat-file", "blob", &object]).await
}
