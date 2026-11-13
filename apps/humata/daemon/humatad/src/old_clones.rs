//! Moving the clones an older daemon kept in the cache folder to the state
//! folder, where a cache cleaner won't delete the branches and snapshots in
//! them.

use std::path::{Path, PathBuf};

use crate::error::MoveCloneError;
use crate::store::Store;

/// Moves each clone in `old` to `new`, then points the projects in `store`,
/// the session records under `host`, and the clone's worktrees at it. One
/// that can't be moved stays where it is and is still used there; each
/// failure is returned. A move cut short is finished the next time.
pub(crate) fn move_old_clones(
    old: &Path,
    new: &Path,
    store: &Store,
    host: &Path,
) -> Vec<MoveCloneError> {
    let mut names: Vec<String> = Vec::new();
    for dir in [old, new] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let name = if dir == new {
                match name.strip_prefix(MARKER) {
                    Some(name) => name.to_owned(),
                    None => continue,
                }
            } else if entry.path().join(".git").exists() {
                name
            } else {
                continue;
            };
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
        .into_iter()
        .filter_map(|name| move_clone(&old.join(&name), &new.join(&name), store, host).err())
        .collect()
}

/// Beside the moved clone in the new folder until everything names its new
/// path.
const MARKER: &str = ".moving-";

fn move_clone(from: &Path, to: &Path, store: &Store, host: &Path) -> Result<(), MoveCloneError> {
    let name = to
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let marker = to.with_file_name(format!("{MARKER}{name}"));
    if from.join(".git").exists() {
        if to.exists() {
            // A clone of the same repository was made since; the old one
            // keeps being used where it is.
            return Ok(());
        }
        let worktrees = worktrees(from);
        let parent = to.parent().unwrap_or(to);
        std::fs::create_dir_all(parent).map_err(|source| MoveCloneError::Write {
            path: parent.to_owned(),
            source,
        })?;
        write_marker(&marker, &worktrees)?;
        if let Err(source) = std::fs::rename(from, to) {
            let _ = std::fs::remove_file(&marker);
            return Err(MoveCloneError::Rename {
                from: from.to_owned(),
                to: to.to_owned(),
                source,
            });
        }
    } else if !marker.exists() {
        return Ok(());
    }
    // Moved, now or before a stop; what still names the old path is
    // pointed at the new one.
    let worktrees = read_marker(&marker);
    repair_worktrees(to, &worktrees)?;
    let (from_text, to_text) = (from.to_string_lossy(), to.to_string_lossy());
    store
        .move_paths(&from_text, &to_text)
        .map_err(|source| MoveCloneError::Store {
            source: Box::new(source),
        })?;
    for record in session_records(host) {
        rewrite_record(&record, &from_text, &to_text)?;
    }
    std::fs::remove_file(&marker).map_err(|source| MoveCloneError::Write {
        path: marker,
        source,
    })
}

/// The worktrees of the clone at `clone`, as git records them.
fn worktrees(clone: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(clone.join(".git").join("worktrees")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("gitdir")).ok())
        .filter_map(|gitdir| Path::new(gitdir.trim()).parent().map(Path::to_path_buf))
        .collect()
}

fn write_marker(marker: &Path, worktrees: &[PathBuf]) -> Result<(), MoveCloneError> {
    let lines: String = worktrees
        .iter()
        .map(|path| format!("{}\n", path.display()))
        .collect();
    std::fs::write(marker, lines).map_err(|source| MoveCloneError::Write {
        path: marker.to_owned(),
        source,
    })
}

fn read_marker(marker: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(marker)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Points each of `worktrees` that's still there at the clone at `clone`,
/// and the clone at them.
fn repair_worktrees(clone: &Path, worktrees: &[PathBuf]) -> Result<(), MoveCloneError> {
    let present: Vec<&PathBuf> = worktrees.iter().filter(|path| path.exists()).collect();
    if present.is_empty() {
        return Ok(());
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(["worktree", "repair"])
        .args(present)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|source| MoveCloneError::RunGit { source })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(MoveCloneError::Repair {
            clone: clone.to_owned(),
            output: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// Each `<backend>/sessions/<id>/session.json` under `host`.
pub(crate) fn session_records(host: &Path) -> Vec<PathBuf> {
    let read = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default()
    };
    read(host)
        .iter()
        .flat_map(|backend| read(&backend.join("sessions")))
        .map(|session| session.join("session.json"))
        .filter(|record| record.is_file())
        .collect()
}

/// Rewrites the paths at or under `from` in the record at `path` to be under
/// `to`, replacing the file whole so it's never left half written.
fn rewrite_record(path: &Path, from: &str, to: &str) -> Result<(), MoveCloneError> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    if !text.contains(from) {
        return Ok(());
    }
    // A damaged record is left for the host to report.
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(());
    };
    if !move_strings(&mut value, from, to) {
        return Ok(());
    }
    let written = serde_json::to_vec_pretty(&value).unwrap_or_default();
    let partial = path.with_extension("json.moving");
    std::fs::write(&partial, written)
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|source| MoveCloneError::Write {
            path: path.to_owned(),
            source,
        })
}

/// Whether any string in `value` was a path at or under `from`, each now
/// under `to`.
fn move_strings(value: &mut serde_json::Value, from: &str, to: &str) -> bool {
    match value {
        serde_json::Value::String(text) => match moved(text, from, to) {
            Some(new) => {
                *text = new;
                true
            }
            None => false,
        },
        serde_json::Value::Array(values) => values.iter_mut().fold(false, |changed, value| {
            move_strings(value, from, to) || changed
        }),
        serde_json::Value::Object(fields) => fields.values_mut().fold(false, |changed, value| {
            move_strings(value, from, to) || changed
        }),
        _ => false,
    }
}

fn moved(path: &str, from: &str, to: &str) -> Option<String> {
    let rest = path.strip_prefix(from)?;
    (rest.is_empty() || rest.starts_with('/')).then(|| format!("{to}{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, arguments: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(arguments)
            .status()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(status.success(), "git {arguments:?}");
    }

    #[test]
    fn an_old_clone_moves_with_its_worktrees_projects_and_sessions() {
        let root = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (old, new, host) = (
            root.path().join("cache/repositories"),
            root.path().join("state/repositories"),
            root.path().join("state/host"),
        );
        let source = crate::tests::git_repository();
        std::fs::create_dir_all(&old).unwrap_or_else(|error| panic!("{error}"));
        let clone = old.join("project-abc");
        let status = std::process::Command::new("git")
            .args(["clone", "--quiet"])
            .arg(source.path())
            .arg(&clone)
            .status()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(status.success());
        let worktree = root.path().join("cache/worktrees/s1/project");
        git(
            &clone,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "work",
                &worktree.to_string_lossy(),
            ],
        );
        let store = Store::open(&root.path().join("state/humata.sqlite3"))
            .unwrap_or_else(|error| panic!("{error}"));
        let project = store
            .add_project(
                "project".into(),
                vec![humata_agent::Repository {
                    name: "project".into(),
                    path: clone.clone(),
                }],
                std::collections::BTreeMap::new(),
                String::new(),
                Vec::new(),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        let record = host.join("claude/sessions/s1/session.json");
        std::fs::create_dir_all(record.parent().unwrap_or(&host))
            .unwrap_or_else(|error| panic!("{error}"));
        let written = serde_json::json!({
            "working_directory": worktree,
            "repositories": [{ "name": "project", "path": clone }],
            "note": format!("{}-not-this", clone.display()),
        });
        std::fs::write(&record, written.to_string()).unwrap_or_else(|error| panic!("{error}"));

        let failed = move_old_clones(&old, &new, &store, &host);

        assert!(failed.is_empty(), "{failed:?}");
        let moved_clone = new.join("project-abc");
        assert!(!clone.exists());
        let paths: Vec<PathBuf> = store
            .project(&project.id)
            .unwrap_or_else(|error| panic!("{error}"))
            .repositories
            .into_iter()
            .map(|repository| repository.path)
            .collect();
        assert_eq!(paths, [moved_clone.clone()]);
        let read: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&record).unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            read["repositories"][0]["path"],
            moved_clone.to_string_lossy().as_ref()
        );
        assert_eq!(
            read["working_directory"],
            worktree.to_string_lossy().as_ref()
        );
        assert_eq!(read["note"], format!("{}-not-this", clone.display()));
        // The worktree still works against the moved clone.
        git(&worktree, &["status", "--porcelain"]);
        assert!(!new.join(format!("{MARKER}project-abc")).exists());
    }

    #[test]
    fn a_clone_made_since_in_the_new_folder_leaves_the_old_one_in_use() {
        let root = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (old, new) = (root.path().join("old"), root.path().join("new"));
        for dir in [old.join("same/.git"), new.join("same/.git")] {
            std::fs::create_dir_all(dir).unwrap_or_else(|error| panic!("{error}"));
        }
        let store = Store::open(&root.path().join("store.sqlite3"))
            .unwrap_or_else(|error| panic!("{error}"));

        let failed = move_old_clones(&old, &new, &store, &root.path().join("host"));

        assert!(failed.is_empty(), "{failed:?}");
        assert!(old.join("same/.git").exists());
    }
}
