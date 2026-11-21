//! Opt-in cached bare git repositories and linked `git worktree` checkouts.
//!
//! When [`USE_WORKTREES_KEY`] is enabled (`"true"` in the store's `settings`
//! table), Humata keeps one shared bare repository per `(owner, name)` under
//! `<channels_root>/.repo-cache/<owner>/<name>.git` and creates channel main
//! checkouts, session checkouts, forks, and PR reviews as linked worktrees.
//!
//! Every bare-cache and worktree mutation is serialized with a two-level lock:
//! an in-process per-path mutex and an OS `flock(2)` on
//! `<channels_root>/.repo-cache/<owner>/<name>.lock`.

use std::collections::{BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use humata2_store::Store;
use rustix::fs::{FlockOperation, flock};

use crate::error::GitError;
use crate::git::{git, git_with_progress, in_partial, io as git_io, settle, text};
use crate::layout::{make_dirs, partial};
use crate::restore::RestoredFrom;
use crate::tree_copy::{Dirs, copy_tree_excluding_git};

/// SQLite `settings` key for opt-in cached bare checkouts + `git worktree` mode.
pub const USE_WORKTREES_KEY: &str = "use_worktrees";

/// Returns whether cached bare checkouts + `git worktree` mode is enabled in
/// `store` (`true` by default; `false` only when explicitly turned off).
#[must_use]
pub fn use_worktrees_enabled(store: &Store) -> bool {
    store
        .setting(USE_WORKTREES_KEY)
        .ok()
        .flatten()
        .map_or(true, |saved| saved.value != "false")
}

/// The git admin directory and shared bare common directory of a linked worktree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeDirs {
    /// `<common_git_dir>/worktrees/<id>`
    pub worktree_git_dir: PathBuf,
    /// `<channels_root>/.repo-cache/<owner>/<name>.git`
    pub common_git_dir: PathBuf,
}

fn process_locks() -> &'static Mutex<HashMap<PathBuf, Arc<Mutex<()>>>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Runs `f` while holding both an in-process mutex and an exclusive OS `flock`
/// on `lock_path`.
pub fn with_repo_cache_lock<T>(
    lock_path: &Path,
    f: impl FnOnce() -> Result<T, GitError>,
) -> Result<T, GitError> {
    if let Some(parent) = lock_path.parent() {
        make_dirs(parent, 0o700).map_err(git_io(parent))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(git_io(lock_path))?;
    let key = lock_path
        .canonicalize()
        .unwrap_or_else(|_| lock_path.to_path_buf());
    let mem_lock = {
        let mut map = process_locks()
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Arc::clone(map.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))))
    };
    let _mem_guard = mem_lock.lock().unwrap_or_else(PoisonError::into_inner);
    flock(&file, FlockOperation::LockExclusive)
        .map_err(|err| git_io(lock_path)(std::io::Error::from(err)))?;
    let result = f();
    let _ = flock(&file, FlockOperation::Unlock);
    result
}

/// Derives the per-repo `.lock` path for a bare repository cache path
/// (`<root>/.repo-cache/<owner>/<name>.git` -> `<root>/.repo-cache/<owner>/<name>.lock`).
#[must_use]
pub fn lock_path_for_bare(bare: &Path) -> PathBuf {
    if bare.extension().is_some_and(|ext| ext == "git") {
        bare.with_extension("lock")
    } else {
        let mut os = bare.as_os_str().to_os_string();
        os.push(".lock");
        PathBuf::from(os)
    }
}

/// Returns `true` when `repo_dir/.git` is a regular file (a linked worktree).
#[must_use]
pub fn is_linked_worktree(repo_dir: &Path) -> bool {
    repo_dir.join(".git").is_file()
}

/// Parses the `gitdir:` pointer in a linked worktree's `.git` file.
pub fn read_worktree_gitdir(repo_dir: &Path) -> Option<PathBuf> {
    let dot_git = repo_dir.join(".git");
    if !dot_git.is_file() {
        return None;
    }
    let raw = fs::read_to_string(&dot_git).ok()?;
    let pointer = raw
        .lines()
        .find_map(|line| line.trim().strip_prefix("gitdir:"))?
        .trim();
    if pointer.is_empty() {
        return None;
    }
    let path = Path::new(pointer);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_dir.join(path)
    };
    Some(resolved)
}

/// Resolves a linked worktree's admin directory (`worktrees/<id>`) and its
/// shared bare common directory. Works even if `worktrees/<id>` was pruned
/// (deriving the common dir from `.../worktrees/<id>`).
#[must_use]
pub fn resolve_worktree_dirs(repo_dir: &Path) -> Option<WorktreeDirs> {
    let worktree_git_dir = read_worktree_gitdir(repo_dir)?;
    let commondir_file = worktree_git_dir.join("commondir");
    let common_git_dir = if let Ok(raw) = fs::read_to_string(&commondir_file) {
        let rel = raw.trim();
        let path = Path::new(rel);
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            worktree_git_dir.join(path)
        };
        joined.canonicalize().unwrap_or(joined)
    } else if worktree_git_dir
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|name| name == "worktrees")
    {
        let parent = worktree_git_dir.parent()?.parent()?;
        parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf())
    } else {
        return None;
    };
    let worktree_git_dir = worktree_git_dir
        .canonicalize()
        .unwrap_or(worktree_git_dir);
    Some(WorktreeDirs {
        worktree_git_dir,
        common_git_dir,
    })
}

/// Resolves a path inside a repository's git directory (e.g. `FETCH_HEAD`),
/// handling both standalone `.git/` directories and linked worktree `.git` files.
#[must_use]
pub fn git_path(repo_dir: &Path, rel: &str) -> PathBuf {
    if let Some(gitdir) = read_worktree_gitdir(repo_dir) {
        gitdir.join(rel)
    } else {
        repo_dir.join(".git").join(rel)
    }
}

/// Returns branch names currently checked out across all worktrees of `bare`.
pub fn worktree_branches(bare: &Path) -> Result<Vec<String>, GitError> {
    if !bare.join("HEAD").exists() {
        return Ok(Vec::new());
    }
    let out = git(Some(bare), &["worktree", "list", "--porcelain"])?;
    let mut branches = Vec::new();
    for line in out.lines() {
        if let Some(branch) = line.trim().strip_prefix("branch refs/heads/") {
            branches.push(branch.to_owned());
        }
    }
    Ok(branches)
}

/// Returns `true` if `branch` is currently checked out in any worktree of `bare`.
#[must_use]
pub fn is_branch_checked_out(bare: &Path, branch: &str) -> bool {
    worktree_branches(bare)
        .map(|list| list.iter().any(|b| b == branch))
        .unwrap_or(false)
}

/// Returns `true` if `repo` is currently on a detached `HEAD`.
#[must_use]
pub fn is_detached_head(repo: &Path) -> bool {
    git(Some(repo), &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|out| out.trim() == "HEAD")
        .unwrap_or(false)
}

/// Returns `true` if `git_ref` in `bare` contains a `.gitmodules` file.
#[must_use]
pub fn has_gitmodules(bare: &Path, git_ref: &str) -> bool {
    git(Some(bare), &["cat-file", "-e", &format!("{git_ref}:.gitmodules")]).is_ok()
}

/// Ensures the shared bare cache at `bare` exists with the proper refspec
/// (`+refs/heads/*:refs/remotes/origin/*`) and up-to-date remote refs.
///
/// Caller MUST hold [`with_repo_cache_lock`] for `bare`.
pub fn ensure_bare_cache(
    bare: &Path,
    remote_url: &str,
    on_units: &mut dyn FnMut(u64),
) -> Result<(), GitError> {
    if bare.join("HEAD").exists() {
        if git(Some(bare), &["remote", "get-url", "origin"]).is_ok() {
            let _ = git(Some(bare), &["remote", "set-url", "origin", remote_url]);
        } else {
            git(Some(bare), &["remote", "add", "origin", remote_url])?;
        }
        git(
            Some(bare),
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/remotes/origin/*",
            ],
        )?;
        let _ = git(Some(bare), &["worktree", "prune", "--expire=now"]);
        let fetch_res =
            git_with_progress(Some(bare), &["fetch", "--prune", "--tags", "origin"], on_units);
        if let Err(err) = fetch_res {
            // Offline resilience: if cached remote refs already exist, proceed with cache.
            let has_cached = git(
                Some(bare),
                &["for-each-ref", "--count=1", "refs/remotes/origin"],
            )
            .map(|out| !out.trim().is_empty())
            .unwrap_or(false);
            if !has_cached {
                return Err(err);
            }
        }
        let _ = git(Some(bare), &["remote", "set-head", "origin", "-a"]);
        return Ok(());
    }

    in_partial(bare, |tmp| {
        git(None, &["init", "--bare", text(tmp)?])?;
        git(Some(tmp), &["remote", "add", "origin", remote_url])?;
        git(
            Some(tmp),
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/remotes/origin/*",
            ],
        )?;
        git(Some(tmp), &["config", "remote.origin.tagOpt", "--tags"])?;
        git_with_progress(Some(tmp), &["fetch", "--prune", "origin"], on_units)?;
        let _ = git(Some(tmp), &["remote", "set-head", "origin", "-a"]);
        Ok(())
    })
}

/// Resolves the default branch name of `bare` from `refs/remotes/origin/*`.
pub fn bare_default_branch(bare: &Path) -> Result<String, GitError> {
    if let Ok(out) = git(Some(bare), &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        let trimmed = out.trim();
        if let Some(name) = trimmed.strip_prefix("origin/")
            && !name.is_empty()
        {
            return Ok(name.to_owned());
        }
    }
    for candidate in ["main", "master"] {
        if git(
            Some(bare),
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/remotes/origin/{candidate}"),
            ],
        )
        .is_ok()
        {
            return Ok(candidate.to_owned());
        }
    }
    let out = git(
        Some(bare),
        &["for-each-ref", "--format=%(refname:short)", "refs/remotes/origin"],
    )?;
    for line in out.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix("origin/")
            && name != "HEAD"
            && !name.is_empty()
        {
            return Ok(name.to_owned());
        }
    }
    Err(GitError::NoDefaultBranch {
        path: bare.to_path_buf(),
    })
}

/// Builds a linked worktree in `<dest>.partial`, atomically renames it into
/// `dest`, and immediately runs `git -C <bare> worktree repair <dest>` so the
/// bidirectional `gitdir` links point at `dest`.
///
/// Caller MUST hold [`with_repo_cache_lock`] for `bare`.
pub fn in_partial_worktree(
    bare: &Path,
    dest: &Path,
    build: impl FnOnce(&Path) -> Result<(), GitError>,
) -> Result<(), GitError> {
    let tmp = partial(dest);
    if tmp.exists() {
        let _ = fs::remove_dir_all(&tmp);
        let _ = git(Some(bare), &["worktree", "prune", "--expire=now"]);
    }
    if let Some(parent) = dest.parent() {
        make_dirs(parent, 0o700).map_err(git_io(parent))?;
    }
    let done = build(&tmp).and_then(|()| {
        settle(&tmp, dest)?;
        git(Some(bare), &["worktree", "repair", text(dest)?])?;
        if let Some(dirs) = resolve_worktree_dirs(dest) {
            let _ = humata2_durable::sync_dir(&dirs.worktree_git_dir);
        }
        Ok(())
    });
    if done.is_err() {
        let _ = fs::remove_dir_all(&tmp);
        let _ = git(Some(bare), &["worktree", "prune", "--expire=now"]);
    }
    done
}

/// Creates a channel's `<channel>/main/<repo>` as a detached-HEAD worktree from
/// the shared bare cache. Returns `Err` if `.gitmodules` is present or git fails
/// so the caller can fall back to `clone_main_with`.
pub fn worktree_main_with(
    bare: &Path,
    lock: &Path,
    remote_url: &str,
    dest: &Path,
    on_units: &mut dyn FnMut(u64),
) -> Result<(), GitError> {
    with_repo_cache_lock(lock, || {
        ensure_bare_cache(bare, remote_url, on_units)?;
        let base = bare_default_branch(bare)?;
        let target = format!("refs/remotes/origin/{base}");
        if has_gitmodules(bare, &target) {
            return Err(GitError::Failed {
                args: "worktree add (submodules)".to_owned(),
                status: "skipped".to_owned(),
                stderr: "repository uses .gitmodules; falling back to full clone".to_owned(),
            });
        }
        in_partial_worktree(bare, dest, |tmp| {
            git(
                Some(bare),
                &["worktree", "add", "--detach", text(tmp)?, &target],
            )?;
            Ok(())
        })?;
        on_units(1000);
        Ok(())
    })
}

/// Creates a session checkout as a linked worktree on `branch` from the shared
/// bare cache. Returns `Err` if `.gitmodules` is present or git fails so the
/// caller can fall back to `clone_session_with`.
pub fn worktree_session_with(
    bare: &Path,
    lock: &Path,
    remote_url: &str,
    dest: &Path,
    branch: &str,
    on_units: &mut dyn FnMut(u64),
) -> Result<(), GitError> {
    with_repo_cache_lock(lock, || {
        ensure_bare_cache(bare, remote_url, on_units)?;
        let base = bare_default_branch(bare)?;
        let target = format!("refs/remotes/origin/{base}");
        if has_gitmodules(bare, &target) {
            return Err(GitError::Failed {
                args: "worktree add (submodules)".to_owned(),
                status: "skipped".to_owned(),
                stderr: "repository uses .gitmodules; falling back to full clone".to_owned(),
            });
        }
        in_partial_worktree(bare, dest, |tmp| {
            git(
                Some(bare),
                &[
                    "worktree",
                    "add",
                    "--no-track",
                    "-b",
                    branch,
                    text(tmp)?,
                    &target,
                ],
            )?;
            Ok(())
        })?;
        on_units(1000);
        Ok(())
    })
}

/// Creates a PR review checkout as a linked worktree from `refs/pull/<number>/head`.
/// If `branch` is already checked out in another worktree, creates a detached-HEAD
/// worktree at the PR tip so concurrent reviews of the same PR (or PRs whose
/// `head_branch == "main"`) never collide.
pub fn worktree_pull_head_with(
    bare: &Path,
    lock: &Path,
    remote_url: &str,
    dest: &Path,
    branch: &str,
    number: u32,
    on_units: &mut dyn FnMut(u64),
) -> Result<(), GitError> {
    with_repo_cache_lock(lock, || {
        ensure_bare_cache(bare, remote_url, on_units)?;
        git(Some(bare), &["fetch", "origin", &format!("refs/pull/{number}/head")])?;
        let sha = git(Some(bare), &["rev-parse", "FETCH_HEAD"])?
            .trim()
            .to_owned();
        if has_gitmodules(bare, &sha) {
            return Err(GitError::Failed {
                args: "worktree add (submodules)".to_owned(),
                status: "skipped".to_owned(),
                stderr: "repository uses .gitmodules; falling back to full clone".to_owned(),
            });
        }
        let detach = is_branch_checked_out(bare, branch);
        in_partial_worktree(bare, dest, |tmp| {
            if detach {
                git(Some(bare), &["worktree", "add", "--detach", text(tmp)?, &sha])?;
            } else {
                git(
                    Some(bare),
                    &["worktree", "add", "--no-track", "-B", branch, text(tmp)?, &sha],
                )?;
            }
            Ok(())
        })?;
        on_units(1000);
        Ok(())
    })
}

/// Restores a freed session repo as a linked worktree from the shared bare cache.
pub fn worktree_restore_with(
    bare: &Path,
    lock: &Path,
    remote_url: &str,
    dest: &Path,
    branch: &str,
) -> Result<RestoredFrom, GitError> {
    with_repo_cache_lock(lock, || {
        ensure_bare_cache(bare, remote_url, &mut |_| {})?;
        if is_branch_checked_out(bare, branch) {
            return Err(GitError::Failed {
                args: "worktree restore".to_owned(),
                status: "branch checked out".to_owned(),
                stderr: format!("branch {branch} is already checked out in another worktree"),
            });
        }
        let remote_ref = format!("refs/remotes/origin/{branch}");
        let (from, start) = if git(Some(bare), &["rev-parse", "--verify", "--quiet", &remote_ref]).is_ok() {
            (RestoredFrom::Branch, remote_ref)
        } else {
            let base = bare_default_branch(bare)?;
            (RestoredFrom::Main, format!("refs/remotes/origin/{base}"))
        };
        if has_gitmodules(bare, &start) {
            return Err(GitError::Failed {
                args: "worktree restore (submodules)".to_owned(),
                status: "skipped".to_owned(),
                stderr: "repository uses .gitmodules; falling back to full clone".to_owned(),
            });
        }
        in_partial_worktree(bare, dest, |tmp| {
            git(
                Some(bare),
                &["worktree", "add", "--no-track", "-B", branch, text(tmp)?, &start],
            )?;
            Ok(())
        })?;
        Ok(from)
    })
}

/// Creates a new linked worktree at `dest` on `branch` forked from `src`,
/// preserving uncommitted, staged, and untracked working-tree files without
/// sharing `src`'s `worktrees/<id>` admin directory.
pub fn copy_worktree(
    src: &Path,
    bare: &Path,
    lock: &Path,
    dest: &Path,
    branch: &str,
) -> Result<(), GitError> {
    if src.join(".gitmodules").exists() {
        return Err(GitError::Failed {
            args: "copy_worktree (submodules)".to_owned(),
            status: "skipped".to_owned(),
            stderr: "repository uses .gitmodules; falling back to standalone copy".to_owned(),
        });
    }
    let head = git(Some(src), &["rev-parse", "HEAD"])?.trim().to_owned();
    let staged_patch = staged_diff_bytes(src)?;
    let tracked_files = tracked_paths(src)?;
    with_repo_cache_lock(lock, || {
        // If `src` is a standalone clone rather than already backed by `bare`,
        // fetch `src`'s HEAD commit into `bare` so `worktree add` can reach it.
        if git(Some(bare), &["cat-file", "-e", &format!("{head}^{{commit}}")]).is_err() {
            git(Some(bare), &["fetch", "--quiet", "--no-tags", text(src)?, &head])?;
        }
        in_partial_worktree(bare, dest, |tmp| {
            git(
                Some(bare),
                &["worktree", "add", "--no-track", "-b", branch, text(tmp)?, &head],
            )?;
            overlay_working_tree(src, tmp, dest, &tracked_files)?;
            if !staged_patch.is_empty() {
                apply_cached_patch(tmp, &staged_patch)?;
            }
            Ok(())
        })
    })
}

/// Materializes a standalone `.git/` clone at `dest` on `branch` from a linked
/// worktree `src`, carrying uncommitted, staged, and untracked changes. Never
/// copies a raw `.git` pointer file.
pub fn copy_clone_standalone(src: &Path, dest: &Path, branch: &str) -> Result<(), GitError> {
    let dirs = resolve_worktree_dirs(src).ok_or_else(|| GitError::Failed {
        args: "copy_clone_standalone".to_owned(),
        status: "not a worktree".to_owned(),
        stderr: format!("{} is not a linked worktree", src.display()),
    })?;
    let head = git(Some(src), &["rev-parse", "HEAD"])?.trim().to_owned();
    let origin_url = git(Some(src), &["remote", "get-url", "origin"])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let staged_patch = staged_diff_bytes(src)?;
    let tracked_files = tracked_paths(src)?;
    in_partial(dest, |tmp| {
        git(
            None,
            &[
                "clone",
                "--no-hardlinks",
                "--no-checkout",
                "--",
                text(&dirs.common_git_dir)?,
                text(tmp)?,
            ],
        )?;
        if let Some(url) = &origin_url {
            git(Some(tmp), &["remote", "set-url", "origin", url])?;
        }
        let _ = git(
            Some(tmp),
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                "--",
                text(&dirs.common_git_dir)?,
                "+refs/remotes/origin/*:refs/remotes/origin/*",
            ],
        );
        git(Some(tmp), &["checkout", "--no-track", "-b", branch, &head])?;
        overlay_working_tree(src, tmp, dest, &tracked_files)?;
        if !staged_patch.is_empty() {
            apply_cached_patch(tmp, &staged_patch)?;
        }
        Ok(())
    })
}

fn tracked_paths(repo: &Path) -> Result<Vec<String>, GitError> {
    let out = git(Some(repo), &["ls-files"])?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

fn overlay_working_tree(
    src: &Path,
    tmp: &Path,
    lands_at: &Path,
    tracked_in_src: &[String],
) -> Result<(), GitError> {
    // Remove tracked files that were deleted in `src`'s working tree.
    for rel in tracked_in_src {
        let src_path = src.join(rel);
        let dst_path = tmp.join(rel);
        if fs::symlink_metadata(&src_path).is_err() && fs::symlink_metadata(&dst_path).is_ok() {
            let _ = fs::remove_file(&dst_path);
        }
    }
    // Copy every non-`.git` file, directory, and symlink from `src`.
    copy_tree_excluding_git(src, tmp, lands_at, Dirs::KeepModes)
        .map_err(|err| git_io(&err.path)(err.source))?;
    Ok(())
}

fn staged_diff_bytes(repo: &Path) -> Result<Vec<u8>, GitError> {
    let out = Command::new("git")
        .args(["-c", "gc.auto=0", "-c", "maintenance.auto=false", "diff", "--cached", "--binary"])
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|source| GitError::Spawn {
            args: "diff --cached --binary".to_owned(),
            source,
        })?;
    if !out.status.success() {
        return Err(GitError::Failed {
            args: "diff --cached --binary".to_owned(),
            status: out.status.to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        });
    }
    Ok(out.stdout)
}

fn apply_cached_patch(repo: &Path, patch: &[u8]) -> Result<(), GitError> {
    let mut child = Command::new("git")
        .args(["-c", "gc.auto=0", "-c", "maintenance.auto=false", "apply", "--cached", "--allow-empty"])
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| GitError::Spawn {
            args: "apply --cached".to_owned(),
            source,
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(patch);
    }
    let out = child.wait_with_output().map_err(|source| GitError::Spawn {
        args: "apply --cached".to_owned(),
        source,
    })?;
    if !out.status.success() {
        return Err(GitError::Failed {
            args: "apply --cached".to_owned(),
            status: out.status.to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

/// Finds all linked worktrees inside `folder` (checking `folder` itself and its
/// direct subdirectories, plus `<folder>/main/*` and `<folder>/sessions/*/*`
/// when `folder` is a channel directory). Returns `(repo_path, common_git_dir)`.
#[must_use]
pub fn collect_worktrees_in_folder(folder: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut found = Vec::new();
    let mut check_dir = |dir: &Path| {
        if let Some(dirs) = resolve_worktree_dirs(dir) {
            found.push((dir.to_path_buf(), dirs.common_git_dir));
        }
    };
    check_dir(folder);
    let Ok(entries) = fs::read_dir(folder) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if resolve_worktree_dirs(&path).is_some() {
            check_dir(&path);
            continue;
        }
        // Channel folders contain `main/<repo>` and `sessions/<session>/<repo>`.
        let name = entry.file_name();
        if name == "main" {
            if let Ok(mains) = fs::read_dir(&path) {
                for m in mains.flatten() {
                    if m.path().is_dir() {
                        check_dir(&m.path());
                    }
                }
            }
        } else if name == "sessions" {
            if let Ok(sessions) = fs::read_dir(&path) {
                for s in sessions.flatten() {
                    if s.path().is_dir()
                        && let Ok(repos) = fs::read_dir(s.path())
                    {
                        for r in repos.flatten() {
                            if r.path().is_dir() {
                                check_dir(&r.path());
                            }
                        }
                    }
                }
            }
        }
    }
    found
}

/// Removes linked worktrees under `folder` cleanly via `git worktree remove --force`
/// (with `worktree prune --expire=now` fallback), so their branches are freed
/// immediately before the enclosing directory is deleted.
pub fn remove_worktrees_in_folder(folder: &Path) {
    let worktrees = collect_worktrees_in_folder(folder);
    let mut commons = BTreeSet::new();
    for (repo_path, common) in worktrees {
        let lock = lock_path_for_bare(&common);
        let _ = with_repo_cache_lock(&lock, || {
            let removed = git(
                Some(&common),
                &["worktree", "remove", "--force", text(&repo_path)?],
            );
            if removed.is_err() {
                let _ = fs::remove_dir_all(&repo_path);
                let _ = git(Some(&common), &["worktree", "prune", "--expire=now"]);
            }
            Ok(())
        });
        commons.insert(common);
    }
    for common in commons {
        let lock = lock_path_for_bare(&common);
        let _ = with_repo_cache_lock(&lock, || {
            let _ = git(Some(&common), &["worktree", "prune", "--expire=now"]);
            Ok(())
        });
    }
}

/// Runs `git worktree prune --expire=now` under lock on each unique bare repo
/// in `common_dirs`.
pub fn prune_worktrees(common_dirs: &[PathBuf]) {
    let unique: BTreeSet<&PathBuf> = common_dirs.iter().collect();
    for common in unique {
        if !common.join("HEAD").exists() {
            continue;
        }
        let lock = lock_path_for_bare(common);
        let _ = with_repo_cache_lock(&lock, || {
            let _ = git(Some(common), &["worktree", "prune", "--expire=now"]);
            Ok(())
        });
    }
}

/// Runs `git worktree repair` on every linked worktree found under `folder`.
pub fn repair_worktrees_in_folder(folder: &Path) {
    for (repo_path, common) in collect_worktrees_in_folder(folder) {
        if !common.join("HEAD").exists() {
            continue;
        }
        let lock = lock_path_for_bare(&common);
        let _ = with_repo_cache_lock(&lock, || {
            let _ = git(Some(&common), &["worktree", "repair", text(&repo_path)?]);
            Ok(())
        });
    }
}

/// Repairs or re-attaches a linked worktree restored from Trash, preserving any
/// uncommitted/untracked working-tree files even if its `worktrees/<id>` admin
/// directory was pruned when trashed.
pub fn repair_or_reattach_worktree(
    repo_path: &Path,
    fallback_branch: Option<&str>,
) -> Result<(), GitError> {
    let Some(dirs) = resolve_worktree_dirs(repo_path) else {
        return Ok(());
    };
    if !dirs.common_git_dir.join("HEAD").exists() {
        return Ok(());
    }
    let lock = lock_path_for_bare(&dirs.common_git_dir);
    with_repo_cache_lock(&lock, || {
        if dirs.worktree_git_dir.exists() {
            git(
                Some(&dirs.common_git_dir),
                &["worktree", "repair", text(repo_path)?],
            )?;
            return Ok(());
        }
        // Admin dir was pruned when trashed. Re-register via a `--no-checkout`
        // scratch worktree and swap its `.git` pointer without touching working files.
        let attach_branch = fallback_branch.filter(|b| {
            git(
                Some(&dirs.common_git_dir),
                &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")],
            )
            .is_ok()
                && !is_branch_checked_out(&dirs.common_git_dir, b)
        });
        let scratch = partial(repo_path);
        let _ = fs::remove_dir_all(&scratch);
        let add_res = if let Some(branch) = attach_branch {
            git(
                Some(&dirs.common_git_dir),
                &["worktree", "add", "--no-checkout", text(&scratch)?, branch],
            )
        } else {
            let base = bare_default_branch(&dirs.common_git_dir)?;
            let target = fallback_branch
                .map(|b| format!("refs/remotes/origin/{b}"))
                .filter(|r| {
                    git(
                        Some(&dirs.common_git_dir),
                        &["rev-parse", "--verify", "--quiet", r],
                    )
                    .is_ok()
                })
                .unwrap_or_else(|| format!("refs/remotes/origin/{base}"));
            git(
                Some(&dirs.common_git_dir),
                &[
                    "worktree",
                    "add",
                    "--no-checkout",
                    "--detach",
                    text(&scratch)?,
                    &target,
                ],
            )
        };
        if let Err(err) = add_res {
            let _ = fs::remove_dir_all(&scratch);
            let _ = git(Some(&dirs.common_git_dir), &["worktree", "prune", "--expire=now"]);
            return Err(err);
        }
        let dot_git_contents = fs::read_to_string(scratch.join(".git"))
            .map_err(git_io(&scratch.join(".git")))?;
        fs::write(repo_path.join(".git"), &dot_git_contents)
            .map_err(git_io(&repo_path.join(".git")))?;
        let _ = fs::remove_dir_all(&scratch);
        git(
            Some(&dirs.common_git_dir),
            &["worktree", "repair", text(repo_path)?],
        )?;
        let _ = git(Some(repo_path), &["reset", "--mixed", "--quiet", "HEAD"]);
        Ok(())
    })
}

/// If `repo_path/.git` is a copied worktree pointer whose admin `gitdir` still
/// belongs to another directory (e.g., after `land_copy`), allocates a fresh
/// `worktrees/<new_id>` admin directory for `repo_path` at the same `HEAD`
/// without touching its working-tree files.
pub fn detach_copied_worktree(repo_path: &Path) -> Result<(), GitError> {
    let Some(dirs) = resolve_worktree_dirs(repo_path) else {
        return Ok(());
    };
    if !dirs.common_git_dir.join("HEAD").exists() {
        return Ok(());
    }
    let expected = repo_path
        .join(".git")
        .canonicalize()
        .unwrap_or_else(|_| repo_path.join(".git"));
    if let Ok(back) = fs::read_to_string(dirs.worktree_git_dir.join("gitdir")) {
        let back_path = PathBuf::from(back.trim());
        let back_canon = back_path.canonicalize().unwrap_or(back_path);
        if back_canon == expected {
            // Already owns its own admin dir.
            return Ok(());
        }
    }
    let head = git(Some(repo_path), &["rev-parse", "HEAD"])?
        .trim()
        .to_owned();
    let lock = lock_path_for_bare(&dirs.common_git_dir);
    with_repo_cache_lock(&lock, || {
        let scratch = partial(repo_path);
        let _ = fs::remove_dir_all(&scratch);
        let add_res = git(
            Some(&dirs.common_git_dir),
            &[
                "worktree",
                "add",
                "--no-checkout",
                "--detach",
                text(&scratch)?,
                &head,
            ],
        );
        if let Err(err) = add_res {
            let _ = fs::remove_dir_all(&scratch);
            let _ = git(
                Some(&dirs.common_git_dir),
                &["worktree", "prune", "--expire=now"],
            );
            return Err(err);
        }
        let dot_git_contents = fs::read_to_string(scratch.join(".git"))
            .map_err(git_io(&scratch.join(".git")))?;
        fs::write(repo_path.join(".git"), &dot_git_contents)
            .map_err(git_io(&repo_path.join(".git")))?;
        let _ = fs::remove_dir_all(&scratch);
        git(
            Some(&dirs.common_git_dir),
            &["worktree", "repair", text(repo_path)?],
        )?;
        let _ = git(Some(repo_path), &["reset", "--mixed", "--quiet", "HEAD"]);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::local_branches;

    const ID: [&str; 4] = ["-c", "user.name=t", "-c", "user.email=t@t"];

    fn run(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?}");
    }

    fn make_origin(root: &Path) -> PathBuf {
        let repo = root.join("origin");
        fs::create_dir_all(&repo).unwrap();
        run(&repo, &["init", "-b", "main"]);
        fs::write(repo.join("f.txt"), "hello\n").unwrap();
        run(&repo, &["add", "f.txt"]);
        let mut args = ID.to_vec();
        args.extend(["commit", "-m", "initial"]);
        run(&repo, &args);
        run(&repo, &["branch", "other"]);
        repo
    }

    fn url(p: &Path) -> String {
        format!("file://{}", p.display())
    }

    #[test]
    fn bare_cache_refspec_and_in_partial_repair_work() {
        let t = tempfile::tempdir().unwrap();
        let origin = make_origin(t.path());
        let bare = t.path().join(".repo-cache/acme/api.git");
        let lock = t.path().join(".repo-cache/acme/api.lock");

        with_repo_cache_lock(&lock, || ensure_bare_cache(&bare, &url(&origin), &mut |_| {}))
            .unwrap();
        // Remote branches live under refs/remotes/origin/*; refs/heads/* stays clean.
        assert!(local_branches(&bare).unwrap().is_empty());
        assert_eq!(bare_default_branch(&bare).unwrap(), "main");
        assert!(
            git(Some(&bare), &["rev-parse", "--verify", "refs/remotes/origin/other"]).is_ok()
        );

        // Session worktree created via in_partial_worktree has valid bidirectional gitdir links.
        let wt = t.path().join("ch/sessions/s1/api");
        worktree_session_with(&bare, &lock, &url(&origin), &wt, "me/fix", &mut |_| {}).unwrap();
        assert!(wt.join(".git").is_file());
        assert!(!partial(&wt).exists());
        let dirs = resolve_worktree_dirs(&wt).unwrap();
        let back_ptr = fs::read_to_string(dirs.worktree_git_dir.join("gitdir")).unwrap();
        assert_eq!(
            PathBuf::from(back_ptr.trim()).canonicalize().unwrap(),
            wt.join(".git").canonicalize().unwrap()
        );
        assert_eq!(
            git(Some(&wt), &["symbolic-ref", "--short", "HEAD"])
                .unwrap()
                .trim(),
            "me/fix"
        );
    }

    #[test]
    fn concurrent_n_threads_create_worktrees_without_races() {
        let t = tempfile::tempdir().unwrap();
        let origin = make_origin(t.path());
        let bare = t.path().join(".repo-cache/acme/api.git");
        let lock = t.path().join(".repo-cache/acme/api.lock");
        let remote = url(&origin);

        let n = 6;
        let handles: Vec<_> = (0..n)
            .map(|i| {
                let bare = bare.clone();
                let lock = lock.clone();
                let remote = remote.clone();
                let dest = t.path().join(format!("ch/sessions/s{i}/api"));
                let branch = format!("me/task-{i}");
                std::thread::spawn(move || {
                    worktree_session_with(&bare, &lock, &remote, &dest, &branch, &mut |_| {})
                        .unwrap();
                    let head = git(Some(&dest), &["symbolic-ref", "--short", "HEAD"])
                        .unwrap()
                        .trim()
                        .to_owned();
                    assert_eq!(head, branch);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let mut branches = worktree_branches(&bare).unwrap();
        branches.sort();
        assert_eq!(branches.len(), n);
    }

    #[test]
    fn gitmodules_triggers_worktree_fallback_error() {
        let t = tempfile::tempdir().unwrap();
        let origin = make_origin(t.path());
        fs::write(origin.join(".gitmodules"), "[submodule \"sub\"]\n").unwrap();
        run(&origin, &["add", ".gitmodules"]);
        let mut args = ID.to_vec();
        args.extend(["commit", "-m", "add gitmodules"]);
        run(&origin, &args);

        let bare = t.path().join(".repo-cache/acme/api.git");
        let lock = t.path().join(".repo-cache/acme/api.lock");
        let dest = t.path().join("ch/sessions/s1/api");
        let err = worktree_session_with(&bare, &lock, &url(&origin), &dest, "me/sub", &mut |_| {});
        assert!(err.is_err());
        assert!(!dest.exists());
    }

    #[test]
    fn copy_worktree_isolates_admin_dir_and_preserves_dirty_files() {
        let t = tempfile::tempdir().unwrap();
        let origin = make_origin(t.path());
        let bare = t.path().join(".repo-cache/acme/api.git");
        let lock = t.path().join(".repo-cache/acme/api.lock");
        let parent = t.path().join("ch/sessions/parent/api");
        worktree_session_with(&bare, &lock, &url(&origin), &parent, "me/parent", &mut |_| {})
            .unwrap();
        fs::write(parent.join("f.txt"), "dirty\n").unwrap();
        fs::write(parent.join("untracked.txt"), "new\n").unwrap();

        let fork = t.path().join("ch/sessions/fork/api");
        copy_worktree(&parent, &bare, &lock, &fork, "me/fork").unwrap();
        let parent_dirs = resolve_worktree_dirs(&parent).unwrap();
        let fork_dirs = resolve_worktree_dirs(&fork).unwrap();
        assert_ne!(parent_dirs.worktree_git_dir, fork_dirs.worktree_git_dir);
        assert_eq!(fs::read_to_string(fork.join("f.txt")).unwrap(), "dirty\n");
        assert_eq!(fs::read_to_string(fork.join("untracked.txt")).unwrap(), "new\n");

        // Committing in fork does not mutate parent HEAD or branch.
        run(&fork, &["add", "-A"]);
        let mut args = ID.to_vec();
        args.extend(["commit", "-m", "fork commit"]);
        run(&fork, &args);
        assert_ne!(
            git(Some(&parent), &["rev-parse", "HEAD"]).unwrap().trim(),
            git(Some(&fork), &["rev-parse", "HEAD"]).unwrap().trim()
        );
        assert_eq!(
            git(Some(&parent), &["symbolic-ref", "--short", "HEAD"])
                .unwrap()
                .trim(),
            "me/parent"
        );
    }

    #[test]
    fn copy_clone_standalone_materializes_full_git_dir() {
        let t = tempfile::tempdir().unwrap();
        let origin = make_origin(t.path());
        let bare = t.path().join(".repo-cache/acme/api.git");
        let lock = t.path().join(".repo-cache/acme/api.lock");
        let parent = t.path().join("ch/sessions/parent/api");
        worktree_session_with(&bare, &lock, &url(&origin), &parent, "me/parent", &mut |_| {})
            .unwrap();
        fs::write(parent.join("untracked.txt"), "kept\n").unwrap();

        let standalone = t.path().join("standalone/api");
        copy_clone_standalone(&parent, &standalone, "me/standalone").unwrap();
        assert!(standalone.join(".git").is_dir());
        assert_eq!(
            git(Some(&standalone), &["symbolic-ref", "--short", "HEAD"])
                .unwrap()
                .trim(),
            "me/standalone"
        );
        assert_eq!(
            fs::read_to_string(standalone.join("untracked.txt")).unwrap(),
            "kept\n"
        );
    }

    #[test]
    fn concurrent_pr_review_worktrees_fall_back_to_detached_head() {
        let t = tempfile::tempdir().unwrap();
        let origin = make_origin(t.path());
        let tip = git(Some(&origin), &["rev-parse", "HEAD"]).unwrap().trim().to_owned();
        run(&origin, &["update-ref", "refs/pull/42/head", &tip]);

        let bare = t.path().join(".repo-cache/acme/api.git");
        let lock = t.path().join(".repo-cache/acme/api.lock");
        let r1 = t.path().join("ch/sessions/r1/api");
        let r2 = t.path().join("ch/sessions/r2/api");
        worktree_pull_head_with(&bare, &lock, &url(&origin), &r1, "main", 42, &mut |_| {}).unwrap();
        worktree_pull_head_with(&bare, &lock, &url(&origin), &r2, "main", 42, &mut |_| {}).unwrap();

        assert!(!is_detached_head(&r1));
        assert_eq!(
            git(Some(&r1), &["symbolic-ref", "--short", "HEAD"])
                .unwrap()
                .trim(),
            "main"
        );
        assert!(is_detached_head(&r2));
        assert_eq!(git(Some(&r2), &["rev-parse", "HEAD"]).unwrap().trim(), tip);
    }

    #[test]
    fn trash_prune_and_restore_reattaches_worktree_preserving_files() {
        let t = tempfile::tempdir().unwrap();
        let origin = make_origin(t.path());
        let bare = t.path().join(".repo-cache/acme/api.git");
        let lock = t.path().join(".repo-cache/acme/api.lock");
        let wt = t.path().join("ch/sessions/s1/api");
        worktree_session_with(&bare, &lock, &url(&origin), &wt, "me/trash-me", &mut |_| {})
            .unwrap();
        fs::write(wt.join("f.txt"), "unsaved work\n").unwrap();

        // Simulate move to .trash + prune.
        let trash = t.path().join(".trash/s1/api");
        fs::create_dir_all(trash.parent().unwrap()).unwrap();
        let commons: Vec<PathBuf> = collect_worktrees_in_folder(wt.parent().unwrap())
            .into_iter()
            .map(|(_, c)| c)
            .collect();
        fs::rename(&wt, &trash).unwrap();
        prune_worktrees(&commons);
        assert!(!is_branch_checked_out(&bare, "me/trash-me"));

        // Restore from trash and re-attach.
        fs::rename(&trash, &wt).unwrap();
        repair_or_reattach_worktree(&wt, Some("me/trash-me")).unwrap();
        assert_eq!(fs::read_to_string(wt.join("f.txt")).unwrap(), "unsaved work\n");
        assert_eq!(
            git(Some(&wt), &["symbolic-ref", "--short", "HEAD"])
                .unwrap()
                .trim(),
            "me/trash-me"
        );
        assert!(git(Some(&wt), &["status", "--porcelain"]).unwrap().contains("f.txt"));
    }
}
