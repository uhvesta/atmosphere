//! Bringing a repository up to date with its remote before a session starts
//! from it, on this Mac or on a host, and what the session is told when it
//! can't be.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::process::Command;
use tokio::sync::OwnedMutexGuard;

/// How a git command in a repository failed.
#[derive(Debug, thiserror::Error)]
enum GitError {
    #[error("couldn't run git: {source}")]
    Run { source: std::io::Error },
    #[error("{output}")]
    Failed { output: String },
    #[error("git didn't answer within {seconds} seconds and was stopped")]
    TimedOut { seconds: u64 },
}

/// How long a fetch may take before the session starts without it: a
/// remote that doesn't answer mustn't hold up the session.
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// A lock for each repository being cloned or brought up to date, so two
/// sessions starting from one at once take turns rather than run git in it
/// together, which fails on git's own locks or leaves a half-made clone.
static PREPARING: LazyLock<Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(Mutex::default);

/// Waits until no one else is cloning or updating the repository at `path`,
/// and keeps others out while the guard is held.
pub(crate) async fn preparing(path: &Path) -> OwnedMutexGuard<()> {
    let lock = {
        let mut locks = PREPARING.lock().unwrap_or_else(PoisonError::into_inner);
        // Those no one holds or waits for are let go, so the map stays small.
        locks.retain(|_, lock| lock.strong_count() > 0);
        match locks.get(path).and_then(Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(path.to_owned(), Arc::downgrade(&lock));
                lock
            }
        }
    };
    lock.lock_owned().await
}

/// What a session's repository is, which decides what fetching does to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Humata's own clone: its checkout is moved to the remote's default
    /// branch, so the session starts from the newest code whatever the
    /// clone's branch did.
    Clone,
    /// The person's folder: fetched, its checkout left as it is.
    Folder,
}

/// Fetches `path`'s `origin` with git commands from `git`, and for a clone
/// checks out the remote's default branch. `name` is the repository's name
/// and `place` where it is, such as "on this Mac", for what the session is
/// told: a notice when the fetch failed, or didn't finish in
/// [`FETCH_TIMEOUT`], or a folder is behind its remote.
pub(crate) async fn update(
    git: &(dyn Fn() -> Command + Sync),
    path: &Path,
    name: &str,
    kind: Kind,
    place: &str,
) -> Option<String> {
    update_within(git, path, name, kind, place, FETCH_TIMEOUT).await
}

/// As [`update`], giving the fetch `patience` to finish.
async fn update_within(
    git: &(dyn Fn() -> Command + Sync),
    path: &Path,
    name: &str,
    kind: Kind,
    place: &str,
    patience: Duration,
) -> Option<String> {
    let in_repo = |args: &'static [&'static str]| run(git, path, args);
    if kind == Kind::Folder && in_repo(&["remote", "get-url", "origin"]).await.is_err() {
        return None;
    }
    // The person's folder keeps the remote branches it has: pruning them is
    // theirs to do.
    let fetch: &'static [&'static str] = match kind {
        Kind::Clone => &["fetch", "--quiet", "--prune", "origin"],
        Kind::Folder => &["fetch", "--quiet", "origin"],
    };
    // Dropping the command when time is up stops git.
    let fetched = match tokio::time::timeout(patience, in_repo(fetch)).await {
        Ok(fetched) => fetched,
        Err(_) => Err(GitError::TimedOut {
            seconds: patience.as_secs(),
        }),
    };
    if let Err(problem) = fetched {
        let age = age(git, path).await;
        return Some(format!(
            "Couldn't fetch {name} {place}, so this session starts from the code as last \
             fetched, from a commit made {age}: {problem}. If you're offline, reconnect; if \
             the sign-in has expired, run `gh auth login` {place}. Then start a new session \
             to work from the newest code."
        ));
    }
    if in_repo(&[
        "rev-parse",
        "--verify",
        "--quiet",
        "refs/remotes/origin/HEAD",
    ])
    .await
    .is_err()
    {
        // Without it, the session starts from the clone's own branch.
        if in_repo(&["remote", "set-head", "origin", "--auto"])
            .await
            .is_err()
        {
            return None;
        }
    }
    match kind {
        Kind::Clone => {
            let moved = in_repo(&["checkout", "--quiet", "--detach", "origin/HEAD"]).await;
            let problem = moved.err()?;
            let age = age(git, path).await;
            Some(format!(
                "Couldn't move Humata's clone of {name} {place} to the newest code, so this \
                 session starts from a commit made {age}: {problem}. Start a new session to \
                 try again."
            ))
        }
        Kind::Folder => {
            let behind = in_repo(&["rev-list", "--count", "HEAD..origin/HEAD"])
                .await
                .ok()?;
            let behind: u64 = behind.trim().parse().ok().filter(|count| *count > 0)?;
            let commits = if behind == 1 { "commit" } else { "commits" };
            // "origin/main"; pulling brings it in only on that branch.
            let default = in_repo(&[
                "symbolic-ref",
                "--quiet",
                "--short",
                "refs/remotes/origin/HEAD",
            ])
            .await
            .map_or_else(|_| "origin/HEAD".to_owned(), |name| name.trim().to_owned());
            let branch = in_repo(&["symbolic-ref", "--quiet", "--short", "HEAD"])
                .await
                .ok()
                .map(|branch| branch.trim().to_owned());
            let fix = match branch {
                Some(branch) if default.strip_prefix("origin/") == Some(branch.as_str()) => {
                    "Pull in the folder".to_owned()
                }
                Some(branch) => format!(
                    "The folder is on the branch {branch}, which pulling may not bring up to \
                     date: merge {default} into {branch} or rebase {branch} onto {default} in \
                     the folder"
                ),
                None => format!(
                    "The folder has a commit checked out, not a branch: check out a branch \
                     and merge {default} into it or rebase it onto {default}"
                ),
            };
            Some(format!(
                "{name} is {behind} {commits} behind its remote's default branch, and this \
                 session starts from the folder as it's checked out. {fix}, then start a new \
                 session, to work from the newest code."
            ))
        }
    }
}

async fn run(
    git: &(dyn Fn() -> Command + Sync),
    path: &Path,
    args: &[&str],
) -> Result<String, GitError> {
    let output = git()
        .arg("-C")
        .arg(path)
        .args(args)
        .stdin(std::process::Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        // One whose caller stopped waiting, as when a fetch takes too
        // long, is stopped too.
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|source| GitError::Run { source })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(GitError::Failed {
            output: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// How long ago `path`'s `HEAD` was committed, in words.
async fn age(git: &(dyn Fn() -> Command + Sync), path: &Path) -> String {
    let committed = run(git, path, &["log", "-1", "--format=%ct", "HEAD"])
        .await
        .ok()
        .and_then(|time| time.trim().parse::<u64>().ok());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|now| now.as_secs())
        .unwrap_or_default();
    match committed {
        Some(time) => ago(now.saturating_sub(time)),
        None => "at an unknown time".to_owned(),
    }
}

/// `seconds` in the past, in words.
fn ago(seconds: u64) -> String {
    let (count, unit) = match seconds {
        0..3_600 => return "within the last hour".to_owned(),
        3_600..86_400 => (seconds / 3_600, "hour"),
        _ => (seconds / 86_400, "day"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git() -> Command {
        Command::new("git")
    }

    async fn sh(dir: &Path, args: &[&str]) -> String {
        run(&git, dir, args).await.expect("git runs")
    }

    /// A remote with one commit, and a clone of it.
    async fn remote_and_clone(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let remote = root.join("remote");
        let clone = root.join("clone");
        std::fs::create_dir_all(&remote).expect("remote folder");
        sh(&remote, &["init", "--quiet", "--initial-branch=main"]).await;
        commit(&remote, "one").await;
        let url = format!("file://{}", remote.display());
        sh(root, &["clone", "--quiet", &url, "clone"]).await;
        (remote, clone)
    }

    async fn commit(dir: &Path, name: &str) -> String {
        std::fs::write(dir.join(name), name).expect("file written");
        sh(dir, &["add", name]).await;
        sh(
            dir,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "--quiet",
                "-m",
                name,
            ],
        )
        .await;
        sh(dir, &["rev-parse", "HEAD"]).await
    }

    #[tokio::test]
    async fn one_repository_is_prepared_by_one_session_at_a_time() {
        let root = tempfile::tempdir().expect("temp dir");
        let (one, other) = (root.path().join("one"), root.path().join("other"));
        let held = preparing(&one).await;
        let waiting = tokio::spawn({
            let one = one.clone();
            async move { drop(preparing(&one).await) }
        });
        let short = Duration::from_millis(50);
        // Another repository isn't held up.
        assert!(tokio::time::timeout(short, preparing(&other)).await.is_ok());
        tokio::time::sleep(short).await;
        assert!(!waiting.is_finished());
        drop(held);
        let done = tokio::time::timeout(Duration::from_secs(5), waiting).await;
        assert!(matches!(done, Ok(Ok(()))));
    }

    #[tokio::test]
    async fn a_clone_starts_from_the_remotes_newest_commit_even_off_its_upstream() {
        let root = tempfile::tempdir().expect("temp dir");
        let (remote, clone) = remote_and_clone(root.path()).await;
        // The clone's branch moves off its upstream, so a fast-forward fails.
        commit(&clone, "local").await;
        let newest = commit(&remote, "two").await;
        let notice = update(&git, &clone, "app", Kind::Clone, "on this Mac").await;
        assert_eq!(notice, None);
        assert_eq!(sh(&clone, &["rev-parse", "HEAD"]).await, newest);
    }

    #[tokio::test]
    async fn a_folder_is_fetched_but_left_where_it_is_and_says_how_far_behind() {
        let root = tempfile::tempdir().expect("temp dir");
        let (remote, folder) = remote_and_clone(root.path()).await;
        let checked_out = sh(&folder, &["rev-parse", "HEAD"]).await;
        commit(&remote, "two").await;
        commit(&remote, "three").await;
        let notice = update(&git, &folder, "app", Kind::Folder, "on this Mac").await;
        assert!(notice.is_some_and(|notice| notice.contains('2')));
        assert_eq!(sh(&folder, &["rev-parse", "HEAD"]).await, checked_out);
        let fetched = sh(&folder, &["rev-parse", "origin/HEAD"]).await;
        assert_eq!(fetched, sh(&remote, &["rev-parse", "HEAD"]).await);
    }

    #[tokio::test]
    async fn a_folder_behind_on_another_branch_is_told_its_branch() {
        let root = tempfile::tempdir().expect("temp dir");
        let (remote, folder) = remote_and_clone(root.path()).await;
        commit(&remote, "two").await;
        let on_default = update(&git, &folder, "app", Kind::Folder, "on this Mac")
            .await
            .expect("behind");
        sh(&folder, &["checkout", "--quiet", "-b", "side-quest"]).await;
        let on_branch = update(&git, &folder, "app", Kind::Folder, "on this Mac")
            .await
            .expect("behind");
        assert!(on_branch.contains("side-quest"), "{on_branch}");
        assert!(!on_default.contains("side-quest"), "{on_default}");
        assert_ne!(on_default, on_branch);
    }

    #[tokio::test]
    async fn a_folder_keeps_remote_branches_its_remote_deleted() {
        let root = tempfile::tempdir().expect("temp dir");
        let (remote, folder) = remote_and_clone(root.path()).await;
        sh(&remote, &["branch", "topic"]).await;
        sh(&folder, &["fetch", "--quiet", "origin"]).await;
        sh(&remote, &["branch", "-D", "topic"]).await;
        let seen = |repository: std::path::PathBuf| async move {
            run(
                &git,
                &repository,
                &["rev-parse", "--verify", "--quiet", "origin/topic"],
            )
            .await
            .is_ok()
        };
        update(&git, &folder, "app", Kind::Folder, "on this Mac").await;
        assert!(seen(folder.clone()).await);
        // Humata's own clone follows its remote.
        update(&git, &folder, "app", Kind::Clone, "on this Mac").await;
        assert!(!seen(folder).await);
    }

    #[tokio::test]
    async fn a_fetch_that_doesnt_finish_is_stopped_and_the_session_told() {
        let root = tempfile::tempdir().expect("temp dir");
        let (remote, clone) = remote_and_clone(root.path()).await;
        let fetched = sh(&clone, &["rev-parse", "HEAD"]).await;
        commit(&remote, "two").await;
        // Git, but for a fetch, which hangs: `-C <path> fetch …`.
        let slow = || {
            let mut git = Command::new("sh");
            git.args([
                "-c",
                r#"case "$3" in fetch) sleep 30 ;; *) exec git "$@" ;; esac"#,
                "sh",
            ]);
            git
        };
        let started = std::time::Instant::now();
        let notice = update_within(
            &slow,
            &clone,
            "app",
            Kind::Clone,
            "on this Mac",
            Duration::from_millis(200),
        )
        .await;
        assert!(notice.is_some());
        assert!(started.elapsed() < Duration::from_secs(20));
        assert_eq!(sh(&clone, &["rev-parse", "HEAD"]).await, fetched);
    }

    #[tokio::test]
    async fn an_up_to_date_folder_says_nothing() {
        let root = tempfile::tempdir().expect("temp dir");
        let (_remote, folder) = remote_and_clone(root.path()).await;
        let notice = update(&git, &folder, "app", Kind::Folder, "on this Mac").await;
        assert_eq!(notice, None);
    }

    #[tokio::test]
    async fn a_remote_gone_starts_from_what_was_fetched_and_says_so() {
        let root = tempfile::tempdir().expect("temp dir");
        let (remote, clone) = remote_and_clone(root.path()).await;
        let fetched = sh(&clone, &["rev-parse", "HEAD"]).await;
        std::fs::remove_dir_all(&remote).expect("remote removed");
        let notice = update(&git, &clone, "app", Kind::Clone, "on this Mac").await;
        assert!(notice.is_some());
        assert_eq!(sh(&clone, &["rev-parse", "HEAD"]).await, fetched);
    }

    #[tokio::test]
    async fn a_folder_without_a_remote_is_left_alone() {
        let root = tempfile::tempdir().expect("temp dir");
        let folder = root.path().join("folder");
        std::fs::create_dir_all(&folder).expect("folder");
        sh(&folder, &["init", "--quiet"]).await;
        commit(&folder, "one").await;
        let notice = update(&git, &folder, "app", Kind::Folder, "on this Mac").await;
        assert_eq!(notice, None);
    }

    #[test]
    fn ages_read_in_hours_and_days() {
        assert_eq!(ago(0), ago(3_599));
        assert_ne!(ago(3_600), ago(86_400));
        assert_ne!(ago(86_400), ago(2 * 86_400));
    }
}
