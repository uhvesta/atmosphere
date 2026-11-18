use std::fs;
use std::path::Path;
use std::process::Command;

use crate::error::GitError;
use crate::layout::{make_dirs, partial};

/// The production repository address template.
pub const DEFAULT_REMOTE_TEMPLATE: &str = "https://github.com/{owner}/{name}.git";

/// Fills `{owner}` and `{name}` in the template.
pub fn remote_url(template: &str, owner: &str, name: &str) -> Result<String, GitError> {
    if !template.contains("{owner}") || !template.contains("{name}") {
        return Err(GitError::BadTemplate {
            template: template.to_string(),
        });
    }
    Ok(template.replace("{owner}", owner).replace("{name}", name))
}

pub(crate) fn git(dir: Option<&Path>, args: &[&str]) -> Result<String, GitError> {
    let mut cmd = Command::new("git");
    // No background gc/maintenance: a detached one outlives the command and
    // deletes temp files while sync_tree walks the clone (ENOENT).
    cmd.args(["-c", "gc.auto=0", "-c", "maintenance.auto=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let shown = args.join(" ");
    let out = cmd.output().map_err(|source| GitError::Spawn {
        args: shown.clone(),
        source,
    })?;
    if !out.status.success() {
        return Err(GitError::Failed {
            args: shown,
            status: out.status.to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub(crate) fn io(path: &Path) -> impl FnOnce(std::io::Error) -> GitError + '_ {
    move |source| GitError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn clear(path: &Path) -> Result<(), GitError> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(io(path))?;
    }
    Ok(())
}

/// Runs `build` in a fresh `<dest>.partial`, then renames it to `dest`; a
/// failure leaves nothing behind.
pub(crate) fn in_partial(
    dest: &Path,
    build: impl FnOnce(&Path) -> Result<(), GitError>,
) -> Result<(), GitError> {
    let tmp = partial(dest);
    clear(&tmp)?;
    if let Some(parent) = dest.parent() {
        make_dirs(parent, 0o700).map_err(io(parent))?;
    }
    let done = build(&tmp).and_then(|()| settle(&tmp, dest));
    if done.is_err() {
        // Best effort: the original error is the one worth reporting.
        let _ = fs::remove_dir_all(&tmp);
    }
    done
}

/// Puts everything under `tmp` on disk, then renames it to `dest` and puts
/// the rename on disk. Git writes the checkout without fsync, so without
/// this a power cut could leave `dest` named but its files empty.
fn settle(tmp: &Path, dest: &Path) -> Result<(), GitError> {
    sync_tree(tmp).map_err(io(tmp))?;
    humata2_durable::rename(tmp, dest).map_err(io(dest))?;
    if let Some(parent) = dest.parent() {
        humata2_durable::sync_dir(parent).map_err(io(parent))?;
    }
    Ok(())
}

/// Fsyncs every file and folder under `path`, and `path` itself. Links
/// aren't followed.
pub(crate) fn sync_tree(path: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            sync_tree(&entry.path())?;
        } else if kind.is_file() {
            fs::File::open(entry.path())?.sync_all()?;
        }
    }
    humata2_durable::sync_dir(path)
}

pub(crate) fn text(path: &Path) -> Result<&str, GitError> {
    path.to_str().ok_or_else(|| GitError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "path is not UTF-8"),
    })
}

/// Clones `remote_url` into `dest` through `<dest>.partial`.
pub fn clone_main(remote_url: &str, dest: &Path) -> Result<(), GitError> {
    in_partial(dest, |tmp| {
        git(None, &["clone", "--", remote_url, text(tmp)?]).map(|_| ())
    })
}

/// Clones the main clone into `dest`, points origin at the remote, and
/// checks out a new `branch` from the main clone's default branch (FC-3).
pub fn clone_session(
    main_clone: &Path,
    remote_url: &str,
    dest: &Path,
    branch: &str,
) -> Result<(), GitError> {
    let base = default_branch(main_clone)?;
    in_partial(dest, |tmp| {
        git(None, &["clone", "--", text(main_clone)?, text(tmp)?])?;
        git(Some(tmp), &["remote", "set-url", "origin", remote_url])?;
        copy_remote_branches(main_clone, tmp)?;
        // No upstream: the new branch is pushed to the real remote, not the base.
        git(
            Some(tmp),
            &[
                "checkout",
                "--no-track",
                "-b",
                branch,
                &format!("origin/{base}"),
            ],
        )?;
        Ok(())
    })
}

/// Clones the main clone into `dest`, points origin at the remote, fetches the
/// head GitHub keeps for pull request `number` (`refs/pull/<n>/head`, so a
/// fork's own repository is never asked) and checks out `branch` at it.
pub fn clone_pull_head(
    main_clone: &Path,
    remote_url: &str,
    dest: &Path,
    branch: &str,
    number: u32,
) -> Result<(), GitError> {
    in_partial(dest, |tmp| {
        git(None, &["clone", "--", text(main_clone)?, text(tmp)?])?;
        git(Some(tmp), &["remote", "set-url", "origin", remote_url])?;
        copy_remote_branches(main_clone, tmp)?;
        git(Some(tmp), &["fetch", "origin", &format!("refs/pull/{number}/head")])?;
        git(Some(tmp), &["checkout", "--no-track", "-B", branch, "FETCH_HEAD"])?;
        Ok(())
    })
}

/// Copies the main clone's view of the remote (`refs/remotes/origin/*`) into
/// the clone at `clone`, from disk. A clone of the main clone only gets the
/// main clone's local branches as its `origin/*`, so without this a branch
/// the remote has, such as a release base (D510), would be missing.
fn copy_remote_branches(main_clone: &Path, clone: &Path) -> Result<(), GitError> {
    git(
        Some(clone),
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            "--",
            text(main_clone)?,
            "+refs/remotes/origin/*:refs/remotes/origin/*",
        ],
    )
    .map(|_| ())
}

/// Local branch names of the repository at `repo`.
pub fn local_branches(repo: &Path) -> Result<Vec<String>, GitError> {
    let out = git(
        Some(repo),
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )?;
    Ok(lines(&out))
}

/// Branch names on the remote, from `git ls-remote --heads`.
pub fn remote_branches(remote_url: &str) -> Result<Vec<String>, GitError> {
    let out = git(None, &["ls-remote", "--heads", "--", remote_url])?;
    Ok(out
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter_map(|(_, r)| r.strip_prefix("refs/heads/"))
        .map(str::to_string)
        .collect())
}

/// The branch the clone's origin points at (its default branch).
pub fn default_branch(repo: &Path) -> Result<String, GitError> {
    if let Some(head) = humata2_git_head::origin_head(repo)
        && let Some(name) = head.strip_prefix("origin/")
    {
        return Ok(name.to_string());
    }
    // No origin/HEAD (e.g. the repo is not a clone): use the checked out branch.
    let out = git(Some(repo), &["symbolic-ref", "--short", "HEAD"])?;
    let name = out.trim();
    if name.is_empty() {
        return Err(GitError::NoDefaultBranch {
            path: repo.to_path_buf(),
        });
    }
    Ok(name.to_string())
}

fn lines(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: [&str; 4] = ["-c", "user.name=t", "-c", "user.email=t@t"];

    fn run(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?}");
    }

    /// A repo with one commit on `main`, plus a branch `other`.
    fn origin(root: &Path) -> std::path::PathBuf {
        let repo = root.join("origin");
        fs::create_dir_all(&repo).unwrap();
        run(&repo, &["init", "-b", "main"]);
        fs::write(repo.join("f"), "x").unwrap();
        run(&repo, &["add", "f"]);
        let mut args = ID.to_vec();
        args.extend(["commit", "-m", "one"]);
        run(&repo, &args);
        run(&repo, &["branch", "other"]);
        repo
    }

    fn url(p: &Path) -> String {
        format!("file://{}", p.display())
    }

    #[test]
    fn clone_main_makes_repo_without_partial() {
        let t = tempfile::tempdir().unwrap();
        let o = origin(t.path());
        let dest = t.path().join("ch/main/api");
        fs::create_dir_all(partial(&dest)).unwrap(); // stale
        clone_main(&url(&o), &dest).unwrap();
        assert!(dest.join("f").is_file());
        assert!(!partial(&dest).exists());
        assert_eq!(default_branch(&dest).unwrap(), "main");
    }

    #[test]
    fn clone_session_is_on_branch_with_remote_origin() {
        let t = tempfile::tempdir().unwrap();
        let o = origin(t.path());
        let main = t.path().join("main/api");
        clone_main(&url(&o), &main).unwrap();
        let dest = t.path().join("sessions/s/api");
        clone_session(&main, &url(&o), &dest, "me/fix").unwrap();
        assert!(dest.join("f").is_file());
        assert!(!partial(&dest).exists());
        let head = git(Some(&dest), &["symbolic-ref", "--short", "HEAD"]).unwrap();
        assert_eq!(head.trim(), "me/fix");
        let origin_url = git(Some(&dest), &["remote", "get-url", "origin"]).unwrap();
        assert_eq!(origin_url.trim(), url(&o));
        assert!(
            local_branches(&dest)
                .unwrap()
                .contains(&"me/fix".to_string())
        );
    }

    #[test]
    fn a_session_clone_sees_every_branch_the_main_clone_knows_on_the_remote() {
        let t = tempfile::tempdir().unwrap();
        let o = origin(t.path());
        let main = t.path().join("main/api");
        clone_main(&url(&o), &main).unwrap();
        // `other` is only a remote branch in the main clone, not a local one.
        assert!(!local_branches(&main).unwrap().contains(&"other".to_string()));
        let dest = t.path().join("sessions/s/api");
        clone_session(&main, &url(&o), &dest, "me/fix").unwrap();
        assert!(git(Some(&dest), &["rev-parse", "--verify", "--quiet", "refs/remotes/origin/other"]).is_ok());
    }

    #[test]
    fn bad_remote_leaves_nothing() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("main/api");
        let bad = url(&t.path().join("missing"));
        assert!(clone_main(&bad, &dest).is_err());
        assert!(!dest.exists());
        assert!(!partial(&dest).exists());
    }

    #[test]
    fn lists_branches() {
        let t = tempfile::tempdir().unwrap();
        let o = origin(t.path());
        let mut local = local_branches(&o).unwrap();
        local.sort();
        assert_eq!(local, ["main", "other"]);
        let mut remote = remote_branches(&url(&o)).unwrap();
        remote.sort();
        assert_eq!(remote, ["main", "other"]);
    }

    #[test]
    fn remote_url_fills_template() {
        assert_eq!(
            remote_url(DEFAULT_REMOTE_TEMPLATE, "acme", "api").unwrap(),
            "https://github.com/acme/api.git"
        );
        assert!(remote_url("https://x/{owner}", "a", "b").is_err());
    }
}
