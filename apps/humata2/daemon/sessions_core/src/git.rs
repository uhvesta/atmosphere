use std::fs;
use std::path::Path;
use std::io::Read;
use std::process::{Command, Stdio};

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

pub fn git(dir: Option<&Path>, args: &[&str]) -> Result<String, GitError> {
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

/// Where a clone is, 0 to 1000, from one line of git's progress on stderr:
/// receiving objects fills 0..=750, resolving deltas 750..=900, and checkout
/// ("Updating files") 900..=990; only final completion reports 1000. Any other
/// line is None.
pub(crate) fn clone_permille(line: &str) -> Option<u64> {
    let line = line.trim();
    let (phase, rest) = line.split_once(':')?;
    let percent = rest.trim_start().split_once('%')?.0.trim();
    let percent: u64 = percent.parse().ok().filter(|p| *p <= 100)?;
    match phase {
        "Receiving objects" => Some(percent * 750 / 100),
        "Resolving deltas" => Some(750 + percent * 150 / 100),
        "Updating files" => Some(900 + percent * 90 / 100),
        _ => None,
    }
}

/// Runs a clone-like git command, reporting how far it is through `on_units`
/// as git prints progress. Only values higher than the last are reported. The
/// lines that aren't progress are what a failure says.
pub(crate) fn git_with_progress(
    dir: Option<&Path>,
    args: &[&str],
    on_units: &mut dyn FnMut(u64),
) -> Result<(), GitError> {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "gc.auto=0", "-c", "maintenance.auto=false"]);
    match args.split_first() {
        Some((first, rest)) if *first == "clone" => cmd.arg("clone").arg("--progress").args(rest),
        Some((first, rest)) if *first == "fetch" => cmd.arg("fetch").arg("--progress").args(rest),
        _ => cmd.args(args),
    };
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let shown = args.join(" ");
    let mut child = cmd.spawn().map_err(|source| GitError::Spawn {
        args: shown.clone(),
        source,
    })?;
    let mut kept = String::new();
    let mut last = 0;
    if let Some(mut stderr) = child.stderr.take() {
        let mut pending = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut line = |bytes: &[u8], kept: &mut String| {
            let text = String::from_utf8_lossy(bytes);
            let text = text.trim();
            if text.is_empty() {
                return;
            }
            match clone_permille(text) {
                Some(at) if at > last => {
                    last = at;
                    on_units(at);
                }
                Some(_) => {}
                None => {
                    kept.push_str(text);
                    kept.push('\n');
                }
            }
        };
        // A read that fails ends the reading; the wait below still decides.
        while let Ok(n) = stderr.read(&mut chunk) {
            if n == 0 {
                break;
            }
            for &byte in &chunk[..n] {
                if byte == b'\r' || byte == b'\n' {
                    line(&pending, &mut kept);
                    pending.clear();
                } else {
                    pending.push(byte);
                }
            }
        }
        line(&pending, &mut kept);
    }
    let status = child.wait().map_err(|source| GitError::Spawn {
        args: shown.clone(),
        source,
    })?;
    if !status.success() {
        return Err(GitError::Failed {
            args: shown,
            status: status.to_string(),
            stderr: kept.trim().to_string(),
        });
    }
    Ok(())
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
pub(crate) fn settle(tmp: &Path, dest: &Path) -> Result<(), GitError> {
    sync_tree(tmp).map_err(io(tmp))?;
    humata2_durable::rename(tmp, dest).map_err(io(dest))?;
    if let Some(parent) = dest.parent() {
        humata2_durable::sync_dir(parent).map_err(io(parent))?;
    }
    Ok(())
}

/// Fsyncs every file and folder under `path`, and `path` itself. Links
/// aren't followed. An entry that is gone by the time it is reached (a
/// temp file git removed after it returned) has nothing left to put on
/// disk and is skipped; `path` itself must exist.
pub fn sync_tree(path: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let synced = match entry.file_type() {
            Ok(kind) if kind.is_dir() => sync_tree(&entry.path()),
            Ok(kind) if kind.is_file() => fs::File::open(entry.path()).and_then(|file| file.sync_all()),
            Ok(_) => Ok(()),
            Err(error) => Err(error),
        };
        match synced {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            other => other?,
        }
    }
    humata2_durable::sync_dir(path)
}

pub fn text(path: &Path) -> Result<&str, GitError> {
    path.to_str().ok_or_else(|| GitError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "path is not UTF-8"),
    })
}

/// Clones `remote_url` into `dest` through `<dest>.partial`.
pub fn clone_main(remote_url: &str, dest: &Path) -> Result<(), GitError> {
    clone_main_with(remote_url, dest, &mut |_| {})
}

/// [`clone_main`] telling `on_units` how far the clone is, 0 to 1000; the last
/// report of a clone that worked is 1000.
pub fn clone_main_with(
    remote_url: &str,
    dest: &Path,
    on_units: &mut dyn FnMut(u64),
) -> Result<(), GitError> {
    in_partial(dest, |tmp| {
        git_with_progress(None, &["clone", "--", remote_url, text(tmp)?], on_units)?;
        on_units(1000);
        Ok(())
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
    clone_session_with(main_clone, remote_url, dest, branch, &mut |_| {})
}

/// [`clone_session`] telling `on_units` how far the clone is, 0 to 1000; the
/// last report of a clone that worked is 1000.
pub fn clone_session_with(
    main_clone: &Path,
    remote_url: &str,
    dest: &Path,
    branch: &str,
    on_units: &mut dyn FnMut(u64),
) -> Result<(), GitError> {
    let base = default_branch(main_clone)?;
    in_partial(dest, |tmp| {
        git_with_progress(None, &["clone", "--", text(main_clone)?, text(tmp)?], on_units)?;
        git(Some(tmp), &["remote", "set-url", "origin", remote_url])?;
        copy_remote_branches(main_clone, tmp)?;
        // No upstream: the new branch is pushed to the real remote, not the base.
        // `-B` resets the branch in this fresh standalone clone even if cloning
        // from a worktree main whose shared bare repo already had a branch of the same name.
        git(
            Some(tmp),
            &[
                "checkout",
                "--no-track",
                "-B",
                branch,
                &format!("origin/{base}"),
            ],
        )?;
        on_units(1000);
        Ok(())
    })
}

/// Clones the main clone into `dest`, points origin at the remote, fetches the
/// head GitHub keeps for pull request `number` (`refs/pull/<n>/head`, so a
/// fork's own repository is never asked) and checks out `branch` at it,
/// telling `on_units` how far the clone is, 0 to 1000; the last report of a
/// clone that worked is 1000.
pub fn clone_pull_head_with(
    main_clone: &Path,
    remote_url: &str,
    dest: &Path,
    branch: &str,
    number: u32,
    on_units: &mut dyn FnMut(u64),
) -> Result<(), GitError> {
    in_partial(dest, |tmp| {
        git_with_progress(None, &["clone", "--", text(main_clone)?, text(tmp)?], on_units)?;
        git(Some(tmp), &["remote", "set-url", "origin", remote_url])?;
        copy_remote_branches(main_clone, tmp)?;
        git(Some(tmp), &["fetch", "origin", &format!("refs/pull/{number}/head")])?;
        git(Some(tmp), &["checkout", "--no-track", "-B", branch, "FETCH_HEAD"])?;
        on_units(1000);
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
    fn parses_git_progress_lines() {
        let blob = "Cloning into 'x'...\rReceiving objects:   0% (0/10)\rReceiving objects:  42% (4/10), 1 MiB | 2 MiB/s\rReceiving objects: 100% (10/10), done.\nResolving deltas:   0% (0/5)\rResolving deltas: 100% (5/5), done.\nUpdating files:   0% (0/10)\rUpdating files:  50% (5/10)\rUpdating files: 100% (10/10), done.\n";
        let seen: Vec<u64> = blob
            .split(['\r', '\n'])
            .filter_map(clone_permille)
            .collect();
        assert_eq!(seen, [0, 315, 750, 750, 900, 900, 945, 990]);
        assert!(seen.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(clone_permille("Updating files:  50% (5/10)"), Some(945));
        assert_eq!(clone_permille("Updating files: 100% (10/10), done."), Some(990));
    }

    #[test]
    fn other_lines_are_not_progress() {
        for line in [
            "remote: Counting objects: 100% (5/5), done.",
            "Counting objects: 100% (5/5), done.",
            "Compressing objects:  50% (1/2)",
            "Receiving objects: lots",
            "Receiving objects: 400% (1/2)",
            "Empfange Objekte:  42% (4/10)",
            "\u{0}\u{1} garbled % %",
            "fatal: repository 'x' not found",
            "",
        ] {
            assert_eq!(clone_permille(line), None, "{line:?}");
        }
    }

    #[test]
    fn a_local_clone_reports_rising_units_ending_at_1000() {
        let t = tempfile::tempdir().unwrap();
        let o = origin(t.path());
        let mut seen = Vec::new();
        clone_main_with(&url(&o), &t.path().join("main/api"), &mut |at| seen.push(at)).unwrap();
        assert_eq!(seen.last(), Some(&1000));
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    }

    #[test]
    fn a_failed_clone_with_progress_says_why_and_leaves_nothing() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("main/api");
        let bad = url(&t.path().join("missing"));
        let mut seen = Vec::new();
        let failed = clone_main_with(&bad, &dest, &mut |at| seen.push(at));
        assert!(matches!(failed, Err(GitError::Failed { .. })), "{failed:?}");
        assert!(!seen.contains(&1000));
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
