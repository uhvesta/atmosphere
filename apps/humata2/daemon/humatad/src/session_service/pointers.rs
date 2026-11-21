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
use std::path::{Path, PathBuf};
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

/// The most paths the watcher keeps for one session between two looks;
/// past it the clone is listed again instead.
const TOUCHED_MAX: usize = 20_000;

/// What the watcher saw in a session's clones since the loop last looked.
#[derive(Default)]
struct Dirty {
    /// The absolute paths notify named.
    paths: HashSet<PathBuf>,
    /// Events were lost (overflow, rescan or an error): list the clones again.
    rescan: bool,
}

/// The last pointers of each session on this machine.
#[derive(Default)]
pub(super) struct Cache {
    shown: Mutex<HashMap<String, Vec<pb::Pointer>>>,
    /// Sessions whose status or files changed since the loop last looked.
    pending: Mutex<HashMap<String, Dirty>>,
    wake: Notify,
    /// What watches each live session's folder and clones, keyed by session,
    /// along with the exact set of paths being watched so adding a repo
    /// refreshes the watchers.
    watching: Mutex<HashMap<String, (HashSet<PathBuf>, Vec<notify::RecommendedWatcher>)>>,
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
        lock(&self.pending).entry(session.to_owned()).or_default();
        self.wake.notify_one();
    }

    /// Drops cached pointers, pending events, and watchers for an ended session.
    pub(super) fn forget(&self, session: &str) {
        lock(&self.shown).remove(session);
        lock(&self.pending).remove(session);
        lock(&self.watching).remove(session);
    }

    /// Notes what a watcher said, and wakes a look unless it was a read
    /// or only touched noisy subdirs (or clone subdirs when watching a session
    /// folder).
    fn touch(
        &self,
        session: &str,
        root: &Path,
        canon: Option<&Path>,
        skip_dirs: &HashSet<String>,
        event: notify::Result<notify::Event>,
    ) {
        {
            let mut pending = lock(&self.pending);
            match event {
                // Reading a clone is not a change to it; git reads it to look.
                Ok(event) if matches!(event.kind, EventKind::Access(_)) => return,
                Ok(event) if !event.need_rescan() => {
                    let keep_paths: Vec<PathBuf> = if event.paths.is_empty() {
                        Vec::new()
                    } else {
                        let kept: Vec<PathBuf> = event
                            .paths
                            .into_iter()
                            .filter(|path| !is_noisy_path(root, canon, path, skip_dirs))
                            .collect();
                        if kept.is_empty() {
                            return;
                        }
                        kept
                    };
                    let dirty = pending.entry(session.to_owned()).or_default();
                    if !dirty.rescan {
                        dirty.paths.extend(keep_paths);
                        if dirty.paths.len() > TOUCHED_MAX {
                            dirty.paths.clear();
                            dirty.rescan = true;
                        }
                    }
                }
                Ok(_) | Err(_) => {
                    let dirty = pending.entry(session.to_owned()).or_default();
                    dirty.paths.clear();
                    dirty.rescan = true;
                }
            }
        }
        self.wake.notify_one();
    }

    /// Watches the session's folder, clones, and agent artifact directories;
    /// refreshes when the set of watched paths changes (e.g. a repo is added
    /// or the conversation id rotates).
    fn watch(
        self: &Arc<Self>,
        session: &str,
        folder: Option<&Path>,
        repos: &[SessionRepo],
        skip_dirs: &HashSet<String>,
        agent_dirs: &[PathBuf],
    ) {
        let mut wanted_paths: HashSet<PathBuf> = repos.iter().map(|r| r.path.clone()).collect();
        if let Some(folder) = folder.filter(|p| !p.as_os_str().is_empty()) {
            wanted_paths.insert(folder.to_owned());
        }
        for dir in agent_dirs {
            if !dir.as_os_str().is_empty() && dir.exists() {
                wanted_paths.insert(dir.clone());
            }
        }
        let mut watching = lock(&self.watching);
        if watching
            .get(session)
            .is_some_and(|(paths, _)| *paths == wanted_paths)
        {
            return;
        }
        let mut watchers = Vec::with_capacity(wanted_paths.len());
        if let Some(folder) = folder.filter(|p| !p.as_os_str().is_empty()) {
            let cache = Arc::clone(self);
            let id = session.to_owned();
            let root = folder.to_owned();
            let canon = folder.canonicalize().ok();
            let skip = skip_dirs.clone();
            if let Ok(mut watcher) =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    cache.touch(&id, &root, canon.as_deref(), &skip, event);
                })
                && watcher.watch(folder, RecursiveMode::Recursive).is_ok()
            {
                watchers.push(watcher);
            }
        }
        let empty_skip = HashSet::new();
        for repo in repos {
            let cache = Arc::clone(self);
            let id = session.to_owned();
            let root = repo.path.clone();
            let canon = repo.path.canonicalize().ok();
            let skip = empty_skip.clone();
            if let Ok(mut watcher) =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    cache.touch(&id, &root, canon.as_deref(), &skip, event);
                })
                && watcher.watch(&repo.path, RecursiveMode::Recursive).is_ok()
            {
                watchers.push(watcher);
            }
        }
        for dir in agent_dirs {
            if dir.as_os_str().is_empty() || !dir.exists() {
                continue;
            }
            let cache = Arc::clone(self);
            let id = session.to_owned();
            let root = dir.clone();
            let canon = dir.canonicalize().ok();
            let skip = empty_skip.clone();
            if let Ok(mut watcher) =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    cache.touch(&id, &root, canon.as_deref(), &skip, event);
                })
                && watcher.watch(dir, RecursiveMode::Recursive).is_ok()
            {
                watchers.push(watcher);
            }
        }
        watching.insert(session.to_owned(), (wanted_paths, watchers));
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

/// Whether `path` lies inside a noisy subdir of the clone (`target`,
/// `node_modules`, or non-state `.git/` internals like `.git/objects`), or
/// inside a top-level skipped clone directory when watching a session folder.
/// Strips the watched root first so path components above the root never
/// match; if stripping fails, returns `false` (fail-open).
fn is_noisy_path(
    root: &Path,
    canon: Option<&Path>,
    path: &Path,
    skip_dirs: &HashSet<String>,
) -> bool {
    let Some(rel) = path
        .strip_prefix(root)
        .ok()
        .or_else(|| canon.and_then(|canon| path.strip_prefix(canon).ok()))
    else {
        return false;
    };
    let mut components = rel.components().filter_map(|c| match c {
        std::path::Component::Normal(name) => name.to_str(),
        _ => None,
    });
    let Some(first) = components.next() else {
        return false;
    };
    if skip_dirs.contains(first) {
        return true;
    }
    if first == ".git" {
        let Some(second) = components.next() else {
            return false;
        };
        let keep = matches!(
            second,
            "HEAD"
                | "index"
                | "refs"
                | "packed-refs"
                | "MERGE_HEAD"
                | "REBASE_HEAD"
                | "CHERRY_PICK_HEAD"
                | "ORIG_HEAD"
                | "FETCH_HEAD"
        ) || second.starts_with("rebase-");
        return !keep;
    }
    if matches!(first, "target" | "node_modules") {
        return true;
    }
    components.any(|part| matches!(part, "target" | "node_modules"))
}

/// Starts the loop that looks at this machine's live sessions: once for
/// all of them, then only when a session's status changes or one of its
/// clones does. It waits on nothing else.
pub(super) fn spawn(tell: Tell) {
    let pointers = Arc::clone(&tell.pointers);
    let file_changes = Arc::clone(&tell.file_changes);
    tell.on_ended(move |session| {
        pointers.forget(session);
        file_changes.forget(session);
    });
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

/// `path` as a clone's relative `/`-separated path; none for the clone
/// itself or a path outside it. `canon` is the clone's real path, which is
/// how some watchers name what they saw.
fn inside(root: &Path, canon: Option<&Path>, path: &Path) -> Option<String> {
    let rel = path
        .strip_prefix(root)
        .ok()
        .or_else(|| canon.and_then(|canon| path.strip_prefix(canon).ok()))?;
    let parts: Option<Vec<&str>> = rel.components().map(|part| part.as_os_str().to_str()).collect();
    let joined = parts?.join("/");
    (!joined.is_empty()).then_some(joined)
}

/// One pass over the live sessions, or over those of `only`.
async fn look_at(tell: &Tell, only: Option<HashMap<String, Dirty>>) {
    let store = tell.store.clone();
    let machine = tell.machine.clone();
    let cache = tell.pointers.clone();
    let file_changes = tell.file_changes.clone();
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
        let here: HashMap<String, PathBuf> = records
            .iter()
            .filter(|record| !remote::elsewhere(record, &machine))
            .map(|record| (record.id.clone(), record.folder.clone()))
            .collect();
        let wanted = |session: &str| only.as_ref().is_none_or(|only| only.contains_key(session));
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let mut found = Vec::new();
        let mut watched = HashSet::new();
        let empty_skip = HashSet::new();
        for detail in details {
            let Some(folder) = here.get(&detail.session) else {
                continue;
            };
            if !live.contains(&detail.session) || !wanted(&detail.session) {
                continue;
            }
            let skip_dirs = clone_dir_names(folder, &detail.repos);
            let session_folder = (!folder.as_os_str().is_empty()).then_some(folder.as_path());
            let agent_dirs = crate::files::session_agent_watch_dirs(&store, &home, &detail.session);
            cache.watch(
                &detail.session,
                session_folder,
                &detail.repos,
                &skip_dirs,
                &agent_dirs,
            );
            watched.insert(detail.session.clone());
            // Bug 23: the watcher wakes this look on every file change in the
            // session folder or clone, an agent's edits included. Only the paths
            // it named are looked at; the root is listed again only when events
            // were lost or it has no list yet (the first pass, which names none).
            let dirty = only.as_ref().and_then(|only| only.get(&detail.session));
            let rescan = dirty.is_some_and(|dirty| dirty.rescan);
            if let Some(folder) = session_folder {
                let canon = dirty
                    .filter(|dirty| !dirty.paths.is_empty())
                    .and_then(|_| folder.canonicalize().ok());
                let touched: Vec<String> = dirty
                    .map(|dirty| {
                        dirty
                            .paths
                            .iter()
                            .filter_map(|path| inside(folder, canon.as_deref(), path))
                            .filter(|rel| {
                                rel.split('/')
                                    .next()
                                    .is_none_or(|first| !skip_dirs.contains(first))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                file_changes.look(&detail.session, None, folder, &touched, rescan, &skip_dirs);
            }
            for repo in &detail.repos {
                let named = pb::RepoRef {
                    owner: repo.owner.clone(),
                    name: repo.name.clone(),
                };
                let canon = dirty
                    .filter(|dirty| !dirty.paths.is_empty())
                    .and_then(|_| repo.path.canonicalize().ok());
                let touched: Vec<String> = dirty
                    .map(|dirty| {
                        dirty
                            .paths
                            .iter()
                            .filter_map(|path| inside(&repo.path, canon.as_deref(), path))
                            .collect()
                    })
                    .unwrap_or_default();
                file_changes.look(
                    &detail.session,
                    Some(&named),
                    &repo.path,
                    &touched,
                    rescan,
                    &empty_skip,
                );
            }
            if let Some(agent_root) = agent_dirs.first().filter(|p| p.exists()) {
                let agent_repo = pb::RepoRef {
                    owner: "@agent-state".to_owned(),
                    name: String::new(),
                };
                let canon = dirty
                    .filter(|dirty| !dirty.paths.is_empty())
                    .and_then(|_| agent_root.canonicalize().ok());
                let touched: Vec<String> = dirty
                    .map(|dirty| {
                        dirty
                            .paths
                            .iter()
                            .filter_map(|path| inside(agent_root, canon.as_deref(), path))
                            .collect()
                    })
                    .unwrap_or_default();
                file_changes.look(
                    &detail.session,
                    Some(&agent_repo),
                    agent_root,
                    &touched,
                    rescan,
                    &empty_skip,
                );
            }
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
            // The event just told is this change, not a status to look at; paths the
            // watcher named since stay for the next look.
            let mut pending = lock(&tell.pointers.pending);
            if pending.get(&session).is_some_and(|dirty| dirty.paths.is_empty() && !dirty.rescan) {
                pending.remove(&session);
            }
        }
    }
}

/// Top-level directory names under `folder` belonging to `repos`.
fn clone_dir_names(folder: &Path, repos: &[SessionRepo]) -> HashSet<String> {
    let mut names = HashSet::with_capacity(repos.len());
    for repo in repos {
        if !repo.name.is_empty() {
            names.insert(repo.name.clone());
        }
        if let Ok(rel) = repo.path.strip_prefix(folder)
            && let Some(first) = rel.components().next().and_then(|c| c.as_os_str().to_str())
        {
            names.insert(first.to_owned());
        } else if let Some(name) = repo.path.file_name().and_then(|n| n.to_str()) {
            names.insert(name.to_owned());
        }
    }
    names
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
    git_bytes(clone, args).map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
}

fn git_bytes(clone: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(clone)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

#[derive(Default)]
struct StatusV2 {
    branch: String,
    head: String,
    upstream: Option<String>,
    ahead_behind: Option<(u32, u32)>,
    dirty: usize,
    dirty_paths: Vec<String>,
}

/// Parses `git status --porcelain=v2 --branch -z --untracked-files=all`.
/// Handles NUL-delimited `2 ` rename/copy entries which emit TWO NUL-terminated
/// paths (destination followed by original source).
fn parse_status_v2_z(bytes: &[u8]) -> StatusV2 {
    let mut parsed = StatusV2::default();
    let mut entries = bytes.split(|&b| b == 0);
    while let Some(raw) = entries.next() {
        if raw.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(raw);
        if let Some(rest) = line.strip_prefix("# ") {
            if let Some(oid) = rest.strip_prefix("branch.oid ") {
                if oid != "(initial)" {
                    parsed.head = oid.to_owned();
                }
            } else if let Some(head) = rest.strip_prefix("branch.head ") {
                if head != "(detached)" {
                    parsed.branch = head.to_owned();
                }
            } else if let Some(up) = rest.strip_prefix("branch.upstream ") {
                if !up.is_empty() {
                    parsed.upstream = Some(up.to_owned());
                }
            } else if let Some(ab) = rest.strip_prefix("branch.ab ") {
                let mut parts = ab.split_whitespace();
                if let (Some(ahead), Some(behind)) = (parts.next(), parts.next())
                    && let (Ok(ahead), Ok(behind)) = (
                        ahead.trim_start_matches('+').parse::<u32>(),
                        behind.trim_start_matches('-').parse::<u32>(),
                    )
                {
                    parsed.ahead_behind = Some((ahead, behind));
                }
            }
            continue;
        }
        let path = if line.starts_with("1 ") {
            line.splitn(9, ' ').nth(8)
        } else if line.starts_with("2 ") {
            // Rename/copy: destination is in this record, source is the next NUL-delimited record.
            let _orig = entries.next();
            line.splitn(10, ' ').nth(9)
        } else if line.starts_with("u ") {
            line.splitn(11, ' ').nth(10)
        } else if let Some(untracked) = line.strip_prefix("? ") {
            Some(untracked)
        } else {
            None
        };
        if let Some(path) = path {
            parsed.dirty += 1;
            if parsed.dirty_paths.len() < NAMED {
                parsed.dirty_paths.push(path.to_owned());
            }
        }
    }
    parsed
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
    let status = git_bytes(
        path,
        &["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"],
    )
    .map(|bytes| parse_status_v2_z(&bytes))
    .unwrap_or_default();
    let StatusV2 {
        branch,
        head,
        upstream: tracked,
        ahead_behind,
        dirty,
        dirty_paths,
    } = status;
    // Keep the `origin/{branch}` fallback when no upstream is configured.
    let upstream = tracked.or_else(|| {
        let name = format!("origin/{branch}");
        (!branch.is_empty()
            && git(path, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{name}")]).is_some())
        .then_some(name)
    });
    let (pushed, unpushed) = match (&upstream, ahead_behind) {
        (Some(_), Some((ahead, _))) => (!head.is_empty() && ahead == 0, ahead),
        (Some(upstream), None) => (
            !head.is_empty() && git(path, &["merge-base", "--is-ancestor", "HEAD", upstream]).is_some(),
            count(path, &format!("{upstream}..HEAD")),
        ),
        (None, _) => (false, count(path, &format!("{}..HEAD", default_base(path)))),
    };
    let subjects = if unpushed == 0 {
        Vec::new()
    } else {
        let range = match &upstream {
            Some(upstream) => format!("{upstream}..HEAD"),
            None => format!("{}..HEAD", default_base(path)),
        };
        git(path, &["log", "--format=%s", "-n", &NAMED.to_string(), &range])
            .map(|log| log.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    };
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
    humata2_review_diff::origin_head(path)
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

    #[test]
    fn noisy_path_strips_clone_root_and_keeps_git_state_files() {
        let root = Path::new("/tmp/target/node_modules/repo");
        let empty = HashSet::new();
        // Above-root `target`/`node_modules` must NOT match.
        assert!(!is_noisy_path(root, None, &root.join("src/lib.rs"), &empty));
        // Inside-clone noisy subdirs ARE filtered.
        assert!(is_noisy_path(root, None, &root.join("target/debug/app"), &empty));
        assert!(is_noisy_path(root, None, &root.join("web/node_modules/pkg/index.js"), &empty));
        assert!(is_noisy_path(root, None, &root.join(".git/objects/ab/1234"), &empty));
        // Top-level skipped clone directories ARE filtered when watching a session folder.
        let skip: HashSet<String> = ["api".to_owned()].into_iter().collect();
        assert!(is_noisy_path(root, None, &root.join("api/src/lib.rs"), &skip));
        assert!(!is_noisy_path(root, None, &root.join("notes.md"), &skip));
        // Essential git state files must NOT be ignored.
        for keep in [
            ".git/HEAD",
            ".git/index",
            ".git/refs/heads/main",
            ".git/packed-refs",
            ".git/MERGE_HEAD",
            ".git/REBASE_HEAD",
            ".git/CHERRY_PICK_HEAD",
            ".git/ORIG_HEAD",
            ".git/rebase-merge/head-name",
            ".git/rebase-apply/next",
            ".git/FETCH_HEAD",
        ] {
            assert!(!is_noisy_path(root, None, &root.join(keep), &empty), "should keep {keep}");
        }
    }

    #[test]
    fn status_v2_z_parses_branch_ab_and_two_nul_rename_entries() {
        let raw = concat!(
            "# branch.oid 0123456789abcdef0123456789abcdef01234567\0",
            "# branch.head main\0",
            "# branch.upstream origin/main\0",
            "# branch.ab +2 -1\0",
            "2 R. N... 100644 100644 100644 aaa bbb R100 new name.rs\0old name.rs\0",
            "1 .M N... 100644 100644 100644 aaa aaa src/lib.rs\0",
            "? untracked.txt\0",
        );
        let parsed = parse_status_v2_z(raw.as_bytes());
        assert_eq!(parsed.branch, "main");
        assert_eq!(parsed.upstream.as_deref(), Some("origin/main"));
        assert_eq!(parsed.ahead_behind, Some((2, 1)));
        assert_eq!(parsed.dirty, 3);
        assert_eq!(
            parsed.dirty_paths,
            vec!["new name.rs", "src/lib.rs", "untracked.txt"]
        );
    }

    #[test]
    fn cache_watch_rotates_agent_dirs_on_conversation_change() {
        let root = tempfile::tempdir().unwrap();
        let old_dir = root.path().join("conv-old");
        let new_dir = root.path().join("conv-new");
        std::fs::create_dir_all(&old_dir).unwrap();
        std::fs::create_dir_all(&new_dir).unwrap();

        let cache = Arc::new(Cache::default());
        let empty_skip = HashSet::new();
        cache.watch("s1", None, &[], &empty_skip, std::slice::from_ref(&old_dir));
        {
            let watching = lock(&cache.watching);
            let (paths, _) = watching.get("s1").unwrap();
            assert!(paths.contains(&old_dir));
            assert!(!paths.contains(&new_dir));
        }

        cache.watch("s1", None, &[], &empty_skip, std::slice::from_ref(&new_dir));
        {
            let watching = lock(&cache.watching);
            let (paths, _) = watching.get("s1").unwrap();
            assert!(!paths.contains(&old_dir));
            assert!(paths.contains(&new_dir));
        }
    }
}
