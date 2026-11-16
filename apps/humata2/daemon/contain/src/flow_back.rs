//! What flows back from a contained Claude Code's config copy (D217, the
//! flow-back pick): only the session's own todos and rewind checkpoints,
//! moved into the real `~/.claude/` when the session next starts
//! uncontained. Regular files only, chosen by the Claude Code session id,
//! never through a link, and never over a newer real file.

use std::io;
use std::path::{Path, PathBuf};

use crate::ContainError;

/// What one flow-back did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FlowBack {
    /// Where each file now is in the real config.
    pub moved: Vec<PathBuf>,
    /// Files left in the copy because the real config has a newer file, or
    /// something that isn't a regular file, at their place.
    pub conflicts: Vec<PathBuf>,
}

/// Moves the todos and checkpoints of `conversation` (Claude Code's session
/// id) from the config `copy` into `home`'s real `~/.claude/`. Nothing
/// happens when there's no copy.
pub fn flow_back(home: &Path, copy: &Path, conversation: &str) -> Result<FlowBack, ContainError> {
    let mut report = FlowBack::default();
    if conversation.is_empty() || conversation.contains(['/', '\\']) || conversation.starts_with('.') {
        return Ok(report);
    }
    let real = home.join(".claude");

    let todos = copy.join("todos");
    if is_real_dir(&todos) {
        for entry in read_dir(&todos)? {
            let (path, name) = entry;
            if name.to_string_lossy().contains(conversation) && is_regular(&path) {
                move_file(&path, &real.join("todos").join(&name), &mut report)?;
            }
        }
    }

    let history = copy.join("file-history").join(conversation);
    if is_real_dir(&history) {
        move_tree(&history, &real.join("file-history").join(conversation), &mut report)?;
    }
    Ok(report)
}

fn failed(path: &Path) -> impl FnOnce(io::Error) -> ContainError + '_ {
    move |source| ContainError::ClaudeFlowBack {
        path: path.to_path_buf(),
        source,
    }
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

fn is_regular(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file())
}

fn read_dir(path: &Path) -> Result<Vec<(PathBuf, std::ffi::OsString)>, ContainError> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path).map_err(failed(path))? {
        let entry = entry.map_err(failed(path))?;
        entries.push((entry.path(), entry.file_name()));
    }
    Ok(entries)
}

/// Moves the regular files under `from` to the same places under `to`,
/// skipping links, then removes the folders that were emptied.
fn move_tree(from: &Path, to: &Path, report: &mut FlowBack) -> Result<(), ContainError> {
    for (path, name) in read_dir(from)? {
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.is_dir() {
            move_tree(&path, &to.join(&name), report)?;
        } else if meta.is_file() {
            move_file(&path, &to.join(&name), report)?;
        }
    }
    // Only succeeds on an empty folder: files left in conflict stay.
    let _ = std::fs::remove_dir(from);
    Ok(())
}

fn move_file(from: &Path, to: &Path, report: &mut FlowBack) -> Result<(), ContainError> {
    match std::fs::symlink_metadata(to) {
        Ok(existing) => {
            let newer = existing
                .modified()
                .ok()
                .zip(std::fs::metadata(from).and_then(|meta| meta.modified()).ok())
                .is_none_or(|(real, copied)| real > copied);
            if !existing.is_file() || newer {
                report.conflicts.push(from.to_path_buf());
                return Ok(());
            }
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(failed(to)(source)),
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(failed(parent))?;
    }
    if std::fs::rename(from, to).is_err() {
        // Another disk: copied beside the place, renamed into it, then removed.
        let partial = to.with_extension("humata-flow");
        std::fs::copy(from, &partial).map_err(failed(&partial))?;
        std::fs::rename(&partial, to).map_err(failed(to))?;
        std::fs::remove_file(from).map_err(failed(from))?;
    }
    report.moved.push(to.to_path_buf());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    const ID: &str = "11111111-aaaa-bbbb-cccc-222222222222";
    const OTHER: &str = "99999999-aaaa-bbbb-cccc-888888888888";

    struct Setup {
        _dir: tempfile::TempDir,
        home: PathBuf,
        copy: PathBuf,
    }

    fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let copy = dir.path().join("session/.humata/claude");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir_all(copy.join("todos")).unwrap();
        std::fs::create_dir_all(copy.join("file-history").join(ID)).unwrap();
        std::fs::create_dir_all(copy.join("file-history").join(OTHER)).unwrap();
        std::fs::write(copy.join(format!("todos/{ID}-agent-{ID}.json")), "[1]").unwrap();
        std::fs::write(copy.join(format!("todos/{OTHER}-agent-{OTHER}.json")), "[2]").unwrap();
        std::fs::write(copy.join("file-history").join(ID).join("abc@v1"), "one").unwrap();
        std::fs::write(copy.join("file-history").join(OTHER).join("def@v1"), "two").unwrap();
        Setup { _dir: dir, home, copy }
    }

    #[test]
    fn the_sessions_todos_and_checkpoints_move_and_other_sessions_stay() {
        let s = setup();
        let report = flow_back(&s.home, &s.copy, ID).unwrap();

        let real = s.home.join(".claude");
        assert_eq!(std::fs::read_to_string(real.join(format!("todos/{ID}-agent-{ID}.json"))).unwrap(), "[1]");
        assert_eq!(std::fs::read_to_string(real.join("file-history").join(ID).join("abc@v1")).unwrap(), "one");
        assert_eq!(report.moved.len(), 2);
        assert!(report.conflicts.is_empty());
        assert!(!s.copy.join(format!("todos/{ID}-agent-{ID}.json")).exists());
        assert!(s.copy.join(format!("todos/{OTHER}-agent-{OTHER}.json")).is_file());
        assert!(s.copy.join("file-history").join(OTHER).join("def@v1").is_file());
        assert!(!real.join("file-history").join(OTHER).exists());
    }

    #[test]
    fn links_are_not_followed() {
        let s = setup();
        let outside = s.home.join("secret.txt");
        std::fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, s.copy.join(format!("todos/{ID}-link.json"))).unwrap();
        std::os::unix::fs::symlink(&outside, s.copy.join("file-history").join(ID).join("link@v1")).unwrap();

        flow_back(&s.home, &s.copy, ID).unwrap();

        let real = s.home.join(".claude");
        assert!(!real.join(format!("todos/{ID}-link.json")).exists());
        assert!(std::fs::symlink_metadata(real.join("file-history").join(ID).join("link@v1")).is_err());
        assert!(std::fs::symlink_metadata(s.copy.join(format!("todos/{ID}-link.json"))).is_ok());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "secret");
    }

    #[test]
    fn a_linked_checkpoint_folder_is_not_followed() {
        let s = setup();
        let elsewhere = s.home.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("x@v1"), "x").unwrap();
        std::fs::remove_dir_all(s.copy.join("file-history").join(ID)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, s.copy.join("file-history").join(ID)).unwrap();

        flow_back(&s.home, &s.copy, ID).unwrap();

        assert!(elsewhere.join("x@v1").is_file());
        assert!(!s.home.join(".claude/file-history").join(ID).exists());
    }

    #[test]
    fn a_newer_real_file_is_kept_and_reported() {
        let s = setup();
        let real = s.home.join(".claude").join(format!("todos/{ID}-agent-{ID}.json"));
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "newer").unwrap();
        let file = std::fs::File::options().write(true).open(&real).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(3600)).unwrap();

        let report = flow_back(&s.home, &s.copy, ID).unwrap();

        assert_eq!(std::fs::read_to_string(&real).unwrap(), "newer");
        assert_eq!(report.conflicts, vec![s.copy.join(format!("todos/{ID}-agent-{ID}.json"))]);
        assert!(s.copy.join(format!("todos/{ID}-agent-{ID}.json")).is_file());
    }

    #[test]
    fn an_older_real_file_is_replaced() {
        let s = setup();
        let real = s.home.join(".claude").join(format!("todos/{ID}-agent-{ID}.json"));
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "older").unwrap();
        let file = std::fs::File::options().write(true).open(&real).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();

        let report = flow_back(&s.home, &s.copy, ID).unwrap();

        assert_eq!(std::fs::read_to_string(&real).unwrap(), "[1]");
        assert!(report.conflicts.is_empty());
    }

    #[test]
    fn nothing_happens_without_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let report = flow_back(&home, &dir.path().join("session/.humata/claude"), ID).unwrap();

        assert_eq!(report, FlowBack::default());
        assert!(!home.join(".claude").exists());
    }

    #[test]
    fn an_empty_or_path_like_id_moves_nothing() {
        let s = setup();
        assert_eq!(flow_back(&s.home, &s.copy, "").unwrap(), FlowBack::default());
        assert_eq!(flow_back(&s.home, &s.copy, "../x").unwrap(), FlowBack::default());
        assert!(s.copy.join(format!("todos/{ID}-agent-{ID}.json")).is_file());
    }
}
