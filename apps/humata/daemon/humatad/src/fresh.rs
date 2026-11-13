//! Bringing a repository up to date with its remote before a session starts
//! from it, on this Mac or on a host, and what the session is told when it
//! can't be.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::process::Command;

/// How a git command in a repository failed.
#[derive(Debug, thiserror::Error)]
enum GitError {
    #[error("couldn't run git: {source}")]
    Run { source: std::io::Error },
    #[error("{output}")]
    Failed { output: String },
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
/// told: a notice when the fetch failed or a folder is behind its remote.
pub(crate) async fn update(
    git: &(dyn Fn() -> Command + Sync),
    path: &Path,
    name: &str,
    kind: Kind,
    place: &str,
) -> Option<String> {
    let in_repo = |args: &'static [&'static str]| run(git, path, args);
    if kind == Kind::Folder && in_repo(&["remote", "get-url", "origin"]).await.is_err() {
        return None;
    }
    if let Err(problem) = in_repo(&["fetch", "--quiet", "--prune", "origin"]).await {
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
            Some(format!(
                "{name} is {behind} {commits} behind its remote's default branch, and this \
                 session starts from the folder as it's checked out. Pull in the folder, then \
                 start a new session, to work from the newest code."
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
