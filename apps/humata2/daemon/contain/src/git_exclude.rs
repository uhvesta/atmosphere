//! Keeps the session's `.humata/` folder (the contained agent's temp folder
//! and its Claude Code config copy) out of `git status`, through the
//! repository's own `info/exclude`. A tracked `.gitignore` is never changed.

use std::io;
use std::path::{Path, PathBuf};

use crate::ContainError;

/// The folder inside a session that Humata keeps for itself.
const HUMATA_FOLDER: &str = ".humata";

/// Adds the session's `.humata/` folder to the `info/exclude` of the git
/// repository `folder` is in. Nothing happens when `folder` is in no
/// repository, or when the line is already there.
pub fn hide_from_git(folder: &Path) -> Result<(), ContainError> {
    let Some((root, exclude)) = exclude_file(folder)? else {
        return Ok(());
    };
    let relative = folder.strip_prefix(&root).unwrap_or(Path::new(""));
    let mut pattern = String::from("/");
    for part in relative.components() {
        pattern.push_str(&escape(&part.as_os_str().to_string_lossy()));
        pattern.push('/');
    }
    pattern.push_str(HUMATA_FOLDER);
    pattern.push('/');

    let failed = |source| ContainError::GitExclude {
        path: exclude.clone(),
        source,
    };
    let existing = match std::fs::read_to_string(&exclude) {
        Ok(text) => text,
        Err(source) if source.kind() == io::ErrorKind::NotFound => String::new(),
        Err(source) => return Err(failed(source)),
    };
    if existing.lines().any(|line| line.trim() == pattern) {
        return Ok(());
    }
    if let Some(info) = exclude.parent() {
        std::fs::create_dir_all(info).map_err(failed)?;
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&pattern);
    text.push('\n');
    std::fs::write(&exclude, text).map_err(failed)
}

/// The repository root `folder` is in and its `info/exclude` file. A linked
/// worktree's `.git` is a file naming its git folder; the exclude file is in
/// the common folder that one names.
fn exclude_file(folder: &Path) -> Result<Option<(PathBuf, PathBuf)>, ContainError> {
    for root in folder.ancestors() {
        let dot_git = root.join(".git");
        let meta = match std::fs::metadata(&dot_git) {
            Ok(meta) => meta,
            Err(source) if source.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(ContainError::GitExclude {
                    path: dot_git,
                    source,
                });
            }
        };
        let common = if meta.is_dir() {
            dot_git
        } else {
            let unreadable = |source| ContainError::GitExclude {
                path: dot_git.clone(),
                source,
            };
            let text = std::fs::read_to_string(&dot_git).map_err(unreadable)?;
            let Some(named) = text.lines().find_map(|line| line.strip_prefix("gitdir:")) else {
                return Err(unreadable(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the .git file names no gitdir",
                )));
            };
            let git_dir = root.join(named.trim());
            match std::fs::read_to_string(git_dir.join("commondir")) {
                Ok(common) => git_dir.join(common.trim()),
                Err(source) if source.kind() == io::ErrorKind::NotFound => git_dir,
                Err(source) => {
                    return Err(ContainError::GitExclude {
                        path: git_dir.join("commondir"),
                        source,
                    });
                }
            }
        };
        return Ok(Some((root.to_path_buf(), common.join("info").join("exclude"))));
    }
    Ok(None)
}

/// A path part with git's pattern characters escaped, so it matches only itself.
fn escape(part: &str) -> String {
    let mut escaped = String::with_capacity(part.len());
    for ch in part.chars() {
        if matches!(ch, '*' | '?' | '[' | '\\' | '!' | '#') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::hide_from_git;

    fn git(folder: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(folder)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .expect("git runs");
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// A `.humata/` folder with files in it, as a contained session has.
    fn humata_files(folder: &Path) {
        std::fs::create_dir_all(folder.join(".humata/tmp")).expect("a folder");
        std::fs::create_dir_all(folder.join(".humata/claude")).expect("a folder");
        std::fs::write(folder.join(".humata/claude/.claude.json"), "{}\n").expect("a file");
        std::fs::write(folder.join("notes.md"), "mine\n").expect("a file");
    }

    #[test]
    fn a_session_repository_does_not_list_the_humata_folder() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let session = dir.path().join("session");
        std::fs::create_dir_all(&session).expect("a folder");
        git(&session, &["init", "-q"]);
        humata_files(&session);

        hide_from_git(&session).expect("hidden");

        let status = git(&session, &["status", "--porcelain", "--untracked-files=all"]);
        assert!(!status.contains(".humata"), "{status}");
        assert!(status.contains("notes.md"), "{status}");
        assert!(!session.join(".gitignore").exists());
    }

    #[test]
    fn a_session_inside_a_repository_is_excluded_at_its_own_path() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let root = dir.path().join("repo");
        let session = root.join("work/fix login");
        std::fs::create_dir_all(&session).expect("a folder");
        git(&root, &["init", "-q"]);
        humata_files(&session);
        humata_files(&root);

        hide_from_git(&session).expect("hidden");

        let status = git(&root, &["status", "--porcelain", "--untracked-files=all"]);
        assert!(!status.contains("work/fix login/.humata"), "{status}");
        // Only the session's own folder is excluded.
        assert!(status.contains(".humata/claude/.claude.json"), "{status}");
    }

    #[test]
    fn a_linked_worktree_is_excluded_too() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let main = dir.path().join("main");
        std::fs::create_dir_all(&main).expect("a folder");
        git(&main, &["init", "-q"]);
        git(&main, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "start"]);
        let session = dir.path().join("session");
        git(&main, &["worktree", "add", "-q", "-b", "fix", session.to_str().expect("utf-8")]);
        humata_files(&session);

        hide_from_git(&session).expect("hidden");

        let status = git(&session, &["status", "--porcelain", "--untracked-files=all"]);
        assert!(!status.contains(".humata"), "{status}");
    }

    #[test]
    fn hiding_twice_adds_one_line() {
        let dir = tempfile::tempdir().expect("a temp folder");
        git(dir.path(), &["init", "-q"]);

        hide_from_git(dir.path()).expect("hidden");
        hide_from_git(dir.path()).expect("hidden");

        let exclude = std::fs::read_to_string(dir.path().join(".git/info/exclude")).expect("the file");
        assert_eq!(exclude.lines().filter(|line| line.contains(".humata")).count(), 1);
    }

    #[test]
    fn a_folder_in_no_repository_is_left_alone() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let session = dir.path().join("session");
        std::fs::create_dir_all(&session).expect("a folder");

        hide_from_git(&session).expect("nothing to do");
        assert!(!session.join(".git").exists());
    }
}
