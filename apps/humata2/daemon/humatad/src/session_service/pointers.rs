//! A session's thin pointers (SL-3): for each repo of each live session on
//! this machine, the clone's path, branch, HEAD, upstream, whether HEAD is
//! on the upstream, the commits the upstream lacks and the dirty files.
//!
//! A look runs when a session's status or turn changes (every such change
//! is a session event, which pokes the loop) and every few seconds for the
//! live sessions only, since a person's plain file edits tell no one. Git
//! runs off the async threads and never writes the clone: no fetch (it
//! writes FETCH_HEAD) and no index refresh. A session event is published
//! only when the pointers differ from the last look, so an idle machine
//! sends nothing; on a box that event is what the peer link forwards to the
//! host, which keeps the last report.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{AgentState, SessionRepo};
use notify::{EventKind, RecursiveMode, Watcher};
use tokio::sync::Notify;
use tokio::task::spawn_blocking;

use super::{Tell, now_ms, remote};
use crate::channel_service::timestamp_ms;

/// How long events are gathered after the first one before a look: a
/// commit or a checkout is many file events. It starts at an event; nothing
/// ticks while the machine is idle.
const QUIET: Duration = Duration::from_millis(200);

/// The last pointers of each session on this machine.
#[derive(Default)]
pub(super) struct Cache {
    shown: Mutex<HashMap<String, Vec<pb::Pointer>>>,
    /// Sessions whose status changed since the loop last looked.
    pending: Mutex<HashSet<String>>,
    wake: Notify,
    /// What watches each live session's clones, so a commit, a branch move,
    /// a push or a file edit wakes a look at that session alone.
    watching: Mutex<HashMap<String, Vec<notify::RecommendedWatcher>>>,
}

/// The cache every `Tell` of this process shares: the host builds more than
/// one, and a look made for one must show on all.
pub(super) fn shared() -> Arc<Cache> {
    static CACHE: OnceLock<Arc<Cache>> = OnceLock::new();
    CACHE.get_or_init(Arc::default).clone()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Cache {
    /// Puts the last pointers of `session` on `shown`.
    pub(super) fn fill(&self, session: &str, shown: &mut pb::Session) {
        if let Some(pointers) = lock(&self.shown).get(session) {
            shown.pointers.clone_from(pointers);
        }
    }

    /// Asks for a look at `session` now.
    pub(super) fn poke(&self, session: &str) {
        lock(&self.pending).insert(session.to_owned());
        self.wake.notify_one();
    }

    /// Watches the clones of `session`, once; an event in one wakes a look.
    /// A clone that can't be watched is still looked at on every status
    /// change.
    fn watch(self: &Arc<Self>, session: &str, repos: &[SessionRepo]) {
        let mut watching = lock(&self.watching);
        if watching.contains_key(session) {
            return;
        }
        let watchers = repos
            .iter()
            .filter_map(|repo| {
                let cache = Arc::clone(self);
                let id = session.to_owned();
                let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    // Reading a clone is not a change to it; git reads it to look.
                    if event.is_ok_and(|event| !matches!(event.kind, EventKind::Access(_))) {
                        cache.poke(&id);
                    }
                })
                .ok()?;
                watcher.watch(&repo.path, RecursiveMode::Recursive).ok()?;
                Some(watcher)
            })
            .collect();
        watching.insert(session.to_owned(), watchers);
    }

    /// Stores `now` for `session`; whether it differs from what was kept.
    fn keep(&self, session: &str, now: Vec<pb::Pointer>) -> bool {
        let mut shown = lock(&self.shown);
        let unchanged = shown
            .get(session)
            .is_some_and(|before| same(before, &now));
        if unchanged {
            return false;
        }
        let at = timestamp_ms(now_ms());
        let stamped = now
            .into_iter()
            .map(|pointer| pb::Pointer {
                reported_at: Some(at.clone()),
                ..pointer
            })
            .collect();
        shown.insert(session.to_owned(), stamped);
        true
    }
}

/// Whether two looks found the same, leaving out when they were made.
fn same(before: &[pb::Pointer], now: &[pb::Pointer]) -> bool {
    before.len() == now.len()
        && before.iter().zip(now).all(|(before, now)| {
            let mut before = before.clone();
            before.reported_at = now.reported_at.clone();
            before == *now
        })
}

/// Starts the loop that looks at this machine's live sessions: once for
/// all of them, then only when a session's status changes or one of its
/// clones does. It waits on nothing else.
pub(super) fn spawn(tell: Tell) {
    tokio::spawn(async move {
        look_at(&tell, None).await;
        loop {
            tell.pointers.wake.notified().await;
            tokio::time::sleep(QUIET).await;
            let only = std::mem::take(&mut *lock(&tell.pointers.pending));
            if !only.is_empty() {
                look_at(&tell, Some(only)).await;
            }
        }
    });
}

/// One pass over the live sessions, or over those of `only`.
async fn look_at(tell: &Tell, only: Option<HashSet<String>>) {
    let store = tell.store.clone();
    let machine = tell.machine.clone();
    let cache = tell.pointers.clone();
    let looked = spawn_blocking(move || -> Vec<(String, Vec<pb::Pointer>)> {
        let (Ok(agents), Ok(details), Ok(records)) =
            (store.agents(), store.session_details(), store.sessions())
        else {
            return Vec::new();
        };
        let live: HashSet<String> = agents
            .into_iter()
            .filter(|agent| {
                matches!(
                    agent.state,
                    AgentState::Launching | AgentState::Running { .. } | AgentState::Resuming
                )
            })
            .map(|agent| agent.session)
            .collect();
        let here: HashSet<String> = records
            .iter()
            .filter(|record| !remote::elsewhere(record, &machine))
            .map(|record| record.id.clone())
            .collect();
        let wanted = |session: &str| only.as_ref().is_none_or(|only| only.contains(session));
        let mut found = Vec::new();
        let mut watched = HashSet::new();
        for detail in details {
            if !(live.contains(&detail.session) && here.contains(&detail.session)) || !wanted(&detail.session) {
                continue;
            }
            cache.watch(&detail.session, &detail.repos);
            watched.insert(detail.session.clone());
            let pointers: Vec<pb::Pointer> = detail.repos.iter().filter_map(pointer).collect();
            found.push((detail.session, pointers));
        }
        // A session that stopped, was archived or left is no longer watched.
        let mut watching = lock(&cache.watching);
        let stale: Vec<String> = watching
            .keys()
            .filter(|session| wanted(session) && !watched.contains(*session))
            .cloned()
            .collect();
        for session in stale {
            watching.remove(&session);
        }
        found
    })
    .await
    .unwrap_or_default();
    for (session, pointers) in looked {
        if tell.pointers.keep(&session, pointers) {
            tell.session(&session, pb::Change::Updated, None).await;
            // The event just told is this change, not a status to look at.
            lock(&tell.pointers.pending).remove(&session);
        }
    }
}

/// The pointer of one repo's clone; none when the clone can't be read.
fn pointer(repo: &SessionRepo) -> Option<pb::Pointer> {
    read(&repo.path).map(|clone| pb::Pointer {
        repo: Some(pb::RepoRef {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
        }),
        path: repo.path.display().to_string(),
        branch: if clone.branch.is_empty() {
            repo.branch.clone()
        } else {
            clone.branch
        },
        head_sha: clone.head,
        upstream: clone.upstream,
        pushed: clone.pushed,
        unpushed_commits: clone.unpushed,
        dirty_files: clone.dirty,
        reported_at: None,
        dirty_paths: clone.dirty_paths,
        unpushed_subjects: clone.subjects,
    })
}

/// How many dirty paths and unpushed subjects a look keeps by name.
const NAMED: usize = 50;

/// What a clone says of itself, from what is on disk.
#[derive(Debug, PartialEq, Eq)]
struct Looked {
    branch: String,
    head: String,
    upstream: String,
    pushed: bool,
    unpushed: u32,
    dirty: u32,
    /// The first dirty files' paths, as git lists them.
    dirty_paths: Vec<String>,
    /// The first unpushed commits' subjects, newest first.
    subjects: Vec<String>,
}

fn git(clone: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(clone)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// What a clone says of itself: the look both of two passes in a row gave, so
/// a clone changing under git's several commands never shows a state it was
/// never in. The third try's is taken if none agreed.
fn read(path: &Path) -> Option<Looked> {
    let mut last = read_once(path)?;
    for _ in 0..3 {
        let again = read_once(path)?;
        if again == last {
            return Some(again);
        }
        last = again;
    }
    Some(last)
}

fn read_once(path: &Path) -> Option<Looked> {
    if !path.join(".git").exists() {
        return None;
    }
    let branch = git(path, &["symbolic-ref", "--short", "HEAD"]).unwrap_or_default();
    let head = git(path, &["rev-parse", "HEAD"]).unwrap_or_default();
    let tracked = git(
        path,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
    )
    .filter(|name| !name.is_empty());
    let upstream = tracked.or_else(|| {
        let name = format!("origin/{branch}");
        (!branch.is_empty()
            && git(path, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{name}")]).is_some())
        .then_some(name)
    });
    let (pushed, unpushed) = match &upstream {
        Some(upstream) => (
            !head.is_empty() && git(path, &["merge-base", "--is-ancestor", "HEAD", upstream]).is_some(),
            count(path, &format!("{upstream}..HEAD")),
        ),
        None => (false, count(path, &format!("{}..HEAD", default_base(path)))),
    };
    let status = git(path, &["status", "--porcelain", "--untracked-files=all"]).unwrap_or_default();
    let changed: Vec<&str> = status.lines().filter(|line| !line.trim().is_empty()).collect();
    let dirty = changed.len();
    let dirty_paths = changed
        .iter()
        .take(NAMED)
        .map(|line| {
            // The command's output is trimmed, so the first line may have lost its
            // leading space: take what follows the status letters.
            let named = line.trim_start().split_once(' ').map_or(*line, |(_, path)| path).trim();
            named.rsplit_once(" -> ").map_or(named, |(_, to)| to).to_owned()
        })
        .collect();
    let range = match &upstream {
        Some(upstream) => format!("{upstream}..HEAD"),
        None => format!("{}..HEAD", default_base(path)),
    };
    let subjects = git(path, &["log", "--format=%s", "-n", &NAMED.to_string(), &range])
        .map(|log| log.lines().map(str::to_owned).collect())
        .unwrap_or_default();
    Some(Looked {
        branch,
        head,
        upstream: upstream.unwrap_or_default(),
        pushed,
        unpushed,
        dirty: u32::try_from(dirty).unwrap_or(u32::MAX),
        dirty_paths,
        subjects,
    })
}

fn count(path: &Path, range: &str) -> u32 {
    git(path, &["rev-list", "--count", range])
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// Where a branch with no upstream is counted from: the remote's default
/// branch as last fetched.
fn default_base(path: &Path) -> String {
    git(path, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .filter(|name| !name.is_empty())
        .or_else(|| {
            ["origin/main", "origin/master"].into_iter().find_map(|name| {
                git(path, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{name}")])
                    .map(|_| name.to_owned())
            })
        })
        .unwrap_or_else(|| "origin/main".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn clone_on(root: &Path, branch: &str) -> std::path::PathBuf {
        let origin = root.join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        run(&origin, &["init", "-b", "main"]);
        std::fs::write(origin.join("f"), "one\n").unwrap();
        run(&origin, &["add", "f"]);
        run(&origin, &["commit", "-m", "one"]);
        let clone = root.join("clone");
        run(root, &["clone", origin.to_str().unwrap(), clone.to_str().unwrap()]);
        run(&clone, &["checkout", "--no-track", "-b", branch, "origin/main"]);
        clone
    }

    #[test]
    fn a_fresh_branch_has_no_work_and_no_upstream() {
        let root = tempfile::tempdir().unwrap();
        let clone = clone_on(root.path(), "me/x");
        let read = read(&clone).unwrap();
        assert_eq!(read.branch, "me/x");
        assert_eq!((read.dirty, read.unpushed, read.pushed), (0, 0, false));
        assert_eq!(read.upstream, "");
        assert_eq!(read.head.len(), 40);
    }

    #[test]
    fn dirty_files_and_unpushed_commits_are_counted() {
        let root = tempfile::tempdir().unwrap();
        let clone = clone_on(root.path(), "me/x");
        std::fs::write(clone.join("f"), "changed\n").unwrap();
        std::fs::write(clone.join("new"), "n\n").unwrap();
        std::fs::write(clone.join("g"), "g\n").unwrap();
        run(&clone, &["add", "g"]);
        run(&clone, &["commit", "-m", "two"]);
        let read = read(&clone).unwrap();
        assert_eq!((read.dirty, read.unpushed, read.pushed), (2, 1, false));
    }

    #[test]
    fn a_pushed_branch_is_pushed_until_it_moves_on() {
        let root = tempfile::tempdir().unwrap();
        let clone = clone_on(root.path(), "me/x");
        run(&clone, &["push", "origin", "me/x"]);
        let pushed = read(&clone).unwrap();
        assert!(pushed.pushed);
        assert_eq!(pushed.upstream, "origin/me/x");
        std::fs::write(clone.join("g"), "g\n").unwrap();
        run(&clone, &["add", "g"]);
        run(&clone, &["commit", "-m", "two"]);
        let ahead = read(&clone).unwrap();
        assert!(!ahead.pushed);
        assert_eq!(ahead.unpushed, 1);
    }

    #[test]
    fn a_folder_that_is_not_a_clone_has_no_pointer() {
        let root = tempfile::tempdir().unwrap();
        assert!(read(root.path()).is_none());
    }

    #[test]
    fn looks_that_differ_only_in_time_are_the_same() {
        let one = pb::Pointer {
            branch: "a".into(),
            reported_at: Some(timestamp_ms(1)),
            ..pb::Pointer::default()
        };
        let two = pb::Pointer {
            reported_at: Some(timestamp_ms(2)),
            ..one.clone()
        };
        assert!(same(&[one.clone()], &[two]));
        assert!(!same(&[one], &[]));
    }
}
