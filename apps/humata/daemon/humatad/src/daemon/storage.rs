//! What humatad's files take on the Mac, and cleaning them up: by hand
//! from Settings › Storage, or once a day when the person turns that on.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use humata_wire::pb;

use super::Daemon;
use crate::error::DaemonError;
use crate::store::now_unix_ms;

/// Where humatad keeps its files on the Mac.
pub struct Places {
    /// Kept: the store, histories, clones and recovery copies.
    pub state: PathBuf,
    /// Made again if lost: the worktrees.
    pub cache: PathBuf,
}

/// The cleanup going on, the last one's outcome, and the last measure.
#[derive(Default)]
pub(super) struct Cleaning {
    job: Option<pb::StorageJob>,
    last: Option<Cleaned>,
    measured: Option<(Instant, Measured)>,
}

/// What the files take, and what the cleanups by hand would free.
#[derive(Clone, Default)]
struct Measured {
    uses: Vec<pb::StorageUse>,
    archived_worktrees: u64,
    unused_clones: u64,
}

struct Cleaned {
    skipped: Vec<pb::StorageSkip>,
    deleted: u64,
    finished_unix_ms: i64,
    failure: String,
}

/// How long a measure is shown before the folders are walked again.
const MEASURE_FOR: Duration = Duration::from_secs(60);
/// A clone touched this recently stays, as it may be being cloned now.
const CLONE_SETTLES: Duration = Duration::from_secs(60 * 60);
/// How long the auto-delete waits between looks.
const LOOK_EVERY: Duration = Duration::from_secs(60 * 60);
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// The board's history kept per project; older changes can't be undone.
const BOARD_CHANGES_KEPT: u32 = 2_000;

const AUTO_DELETE: &str = "storage.auto_delete";
const DAYS: &str = "storage.days";
const AUTO_CLEANED: &str = "storage.auto_cleaned_unix_ms";
/// When old histories were last sealed, while nothing is auto-deleted.
const AUTO_SEALED: &str = "storage.auto_sealed_unix_ms";
/// How long a session is idle before its history is sealed.
const SEAL_IDLE_DAYS: i64 = 3;
/// Days an archived session is kept, once auto-delete is on.
const DEFAULT_DAYS: u32 = 45;

impl Daemon {
    pub(super) async fn storage(&self) -> Result<pb::Storage, DaemonError> {
        let fresh = self
            .lock_cleaning()
            .measured
            .as_ref()
            .filter(|(at, _)| at.elapsed() < MEASURE_FOR)
            .map(|(_, measured)| measured.clone());
        let measured = match fresh {
            Some(measured) => measured,
            None => {
                let archived: Vec<String> = self
                    .list("")
                    .await?
                    .sessions
                    .into_iter()
                    .filter(|session| session.archived)
                    .map(|session| session.id)
                    .collect();
                let used = self.used_clones()?;
                let (state, cache) = (self.places.state.clone(), self.places.cache.clone());
                let store = Arc::clone(&self.store);
                let measured = tokio::task::spawn_blocking(move || {
                    let search = store.search_bytes().unwrap_or(0);
                    let mut measured = Measured {
                        uses: measure(&state, &cache, search),
                        ..Measured::default()
                    };
                    measured.archived_worktrees = archived
                        .iter()
                        .map(|id| size(&cache.join("worktrees").join(id)))
                        .sum();
                    measured.unused_clones = unused_clones(
                        &state,
                        &cache,
                        &used,
                        std::time::SystemTime::now() - CLONE_SETTLES,
                    )
                    .iter()
                    .map(|clone| size(clone))
                    .sum();
                    measured
                })
                .await
                .unwrap_or_default();
                self.lock_cleaning().measured = Some((Instant::now(), measured.clone()));
                measured
            }
        };
        let settings = self.storage_settings()?;
        let cleaning = self.lock_cleaning();
        let last = cleaning.last.as_ref();
        Ok(pb::Storage {
            uses: measured.uses,
            archived_worktrees_bytes: measured.archived_worktrees,
            unused_clones_bytes: measured.unused_clones,
            settings: Some(settings),
            job: cleaning.job.clone(),
            skipped: last.map(|last| last.skipped.clone()).unwrap_or_default(),
            deleted_sessions: last.map_or(0, |last| last.deleted),
            cleaned_unix_ms: last.map_or(0, |last| last.finished_unix_ms),
            failure: last.map(|last| last.failure.clone()).unwrap_or_default(),
        })
    }

    pub(super) fn storage_settings(&self) -> Result<pb::StorageSettings, DaemonError> {
        let auto_delete = self.store.setting(AUTO_DELETE)?.as_deref() == Some("1");
        let days = self
            .store
            .setting(DAYS)?
            .and_then(|days| days.parse().ok())
            .unwrap_or(DEFAULT_DAYS);
        Ok(pb::StorageSettings { auto_delete, days })
    }

    pub(super) fn set_storage_settings(
        &self,
        settings: &pb::StorageSettings,
    ) -> Result<(), DaemonError> {
        if !(1..=3650).contains(&settings.days) {
            return Err(DaemonError::StorageDays {
                days: settings.days,
            });
        }
        self.store
            .set_setting(AUTO_DELETE, if settings.auto_delete { "1" } else { "0" })?;
        self.store.set_setting(DAYS, &settings.days.to_string())?;
        Ok(())
    }

    /// Starts a cleanup in the background; refused while one runs.
    pub(super) fn start_cleanup(&self, clean: pb::CleanUpStorage) -> Result<(), DaemonError> {
        let Some(me) = self.me.upgrade() else {
            return Ok(());
        };
        {
            let mut cleaning = self.lock_cleaning();
            if cleaning.job.is_some() {
                return Err(DaemonError::CleanupRunning);
            }
            cleaning.job = Some(pb::StorageJob {
                step: "Starting".into(),
                done: 0,
                total: 0,
            });
        }
        tokio::spawn(async move { me.run_cleanup(clean).await });
        Ok(())
    }

    /// Once an hour, if the person turned auto-delete on and a day has
    /// passed since it last ran, cleans up while no session works.
    pub fn delete_on_schedule(self: &Arc<Self>) {
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(LOOK_EVERY).await;
                let Some(daemon) = me.upgrade() else {
                    return;
                };
                if let Err(error) = daemon.clean_up_if_due().await {
                    eprintln!(
                        "humatad: the daily cleanup didn't run: {}",
                        humata_wire::describe(&error)
                    );
                }
            }
        });
    }

    async fn clean_up_if_due(&self) -> Result<(), DaemonError> {
        let settings = self.storage_settings()?;
        let key = if settings.auto_delete {
            AUTO_CLEANED
        } else {
            // Old histories are sealed every day all the same.
            AUTO_SEALED
        };
        let last: i64 = self
            .store
            .setting(key)?
            .and_then(|last| last.parse().ok())
            .unwrap_or(0);
        if now_unix_ms() - last < DAY_MS {
            return Ok(());
        }
        let working = self.list("").await?.sessions.iter().any(|session| {
            matches!(
                session.state(),
                pb::SessionState::Working | pb::SessionState::WaitingForApproval
            )
        });
        if working {
            return Ok(());
        }
        let clean = if settings.auto_delete {
            pb::CleanUpStorage {
                delete_archived_after_days: settings.days,
                compact: true,
                ..Default::default()
            }
        } else {
            pb::CleanUpStorage::default()
        };
        match self.start_cleanup(clean) {
            // One the person started counts.
            Ok(()) | Err(DaemonError::CleanupRunning) => {}
            Err(error) => return Err(error),
        }
        self.store.set_setting(key, &now_unix_ms().to_string())?;
        Ok(())
    }

    async fn run_cleanup(&self, clean: pb::CleanUpStorage) {
        let mut cleaned = Cleaned {
            skipped: Vec::new(),
            deleted: 0,
            finished_unix_ms: 0,
            failure: String::new(),
        };
        if let Err(error) = self.clean(&clean, &mut cleaned).await {
            cleaned.failure = humata_wire::describe(&error);
        }
        cleaned.finished_unix_ms = now_unix_ms();
        let mut cleaning = self.lock_cleaning();
        cleaning.job = None;
        cleaning.last = Some(cleaned);
        cleaning.measured = None;
    }

    async fn clean(
        &self,
        clean: &pb::CleanUpStorage,
        cleaned: &mut Cleaned,
    ) -> Result<(), DaemonError> {
        let sessions = self.list("").await?.sessions;
        if clean.delete_archived_after_days > 0 {
            let before = now_unix_ms() - i64::from(clean.delete_archived_after_days) * DAY_MS;
            self.delete_archived(&sessions, before, cleaned).await;
        }
        if clean.put_away_worktrees {
            self.put_away_worktrees(&sessions, cleaned).await;
        }
        if clean.remove_unused_clones {
            self.remove_unused_clones().await?;
        }
        self.seal_histories(&sessions).await?;
        if !clean.compact {
            return Ok(());
        }
        let archived: BTreeSet<String> = sessions
            .iter()
            .filter(|session| session.archived)
            .map(|session| session.id.clone())
            .collect();
        let repositories = self.repositories_here()?;
        self.step(
            "Dropping saves from part way through turns",
            0,
            repositories.len(),
        );
        for (done, repository) in repositories.iter().enumerate() {
            drop_saves(repository, &archived).await;
            self.step(
                "Dropping saves from part way through turns",
                done + 1,
                repositories.len(),
            );
        }
        let packed = self.packed_here();
        self.step("Packing repositories", 0, packed.len());
        for (done, repository) in packed.iter().enumerate() {
            // Not pruned at once: a snapshot being taken now has objects
            // no ref names yet.
            let _ = tokio::process::Command::new("git")
                .arg("-C")
                .arg(repository)
                .args(["gc", "--quiet", "--prune=1.day.ago"])
                .stdin(std::process::Stdio::null())
                .output()
                .await;
            self.step("Packing repositories", done + 1, packed.len());
        }
        self.step("Compacting the store", 0, 1);
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || store.compact(BOARD_CHANGES_KEPT))
            .await
            .unwrap_or(Ok(()))?;
        self.step("Compacting the store", 1, 1);
        Ok(())
    }

    /// Deletes the archived sessions not used since `before`, forks before
    /// the sessions they came from, and lists those it leaves.
    async fn delete_archived(
        &self,
        sessions: &[pb::SessionInfo],
        before: i64,
        cleaned: &mut Cleaned,
    ) {
        let parents: BTreeMap<&str, &str> = sessions
            .iter()
            .map(|session| (session.id.as_str(), session.parent_id.as_str()))
            .collect();
        let depth = |id: &str| {
            let mut depth = 0;
            let mut at = id;
            while let Some(parent) = parents.get(at).filter(|parent| !parent.is_empty()) {
                depth += 1;
                at = parent;
                if depth > sessions.len() {
                    break;
                }
            }
            depth
        };
        let mut due: Vec<&pb::SessionInfo> = sessions
            .iter()
            .filter(|session| {
                session.archived
                    && session.last_active_unix_ms > 0
                    && session.last_active_unix_ms < before
            })
            .collect();
        due.sort_by_key(|session| std::cmp::Reverse(depth(&session.id)));
        let mut gone: BTreeSet<String> = BTreeSet::new();
        let step = "Deleting archived sessions";
        self.step(step, 0, due.len());
        for (done, session) in due.iter().enumerate() {
            let fork = sessions
                .iter()
                .find(|fork| fork.parent_id == session.id && !gone.contains(&fork.id));
            let reason = if session.keeper {
                Some("It's its task's keeper; unmark it to let it go.".to_owned())
            } else if let Some(fork) = fork {
                Some(format!("{:?} was forked from it and is kept.", fork.name))
            } else if session.host_offline {
                Some(format!(
                    "Its host {} can't be reached; it's tried again next time.",
                    session.host
                ))
            } else {
                match self.delete_session(&session.id, true).await {
                    Ok(true) => None,
                    Ok(false) => Some(
                        "Its branch has commits nothing else holds; push or merge them, or delete it yourself."
                            .to_owned(),
                    ),
                    Err(error) => Some(humata_wire::describe(&error)),
                }
            };
            match reason {
                None => {
                    gone.insert(session.id.clone());
                    cleaned.deleted += 1;
                }
                Some(reason) => cleaned.skipped.push(pb::StorageSkip {
                    session_id: session.id.clone(),
                    name: session.name.clone(),
                    reason,
                }),
            }
            self.step(step, done + 1, due.len());
        }
    }

    /// Archives again each archived session that still has worktrees,
    /// which puts them away; one deleted just now has none.
    async fn put_away_worktrees(&self, sessions: &[pb::SessionInfo], cleaned: &mut Cleaned) {
        let worktrees = self.places.cache.join("worktrees");
        let left: Vec<&pb::SessionInfo> = sessions
            .iter()
            .filter(|session| session.archived && worktrees.join(&session.id).exists())
            .collect();
        let step = "Removing archived sessions' worktrees";
        self.step(step, 0, left.len());
        for (done, session) in left.iter().enumerate() {
            let archive = pb::request::Kind::UpdateSession(pb::UpdateSession {
                session_id: session.id.clone(),
                archived: Some(true),
                ..Default::default()
            });
            if let Err(error) = self.ask(&session.host, archive).await {
                cleaned.skipped.push(pb::StorageSkip {
                    session_id: session.id.clone(),
                    name: session.name.clone(),
                    reason: humata_wire::describe(&error),
                });
            }
            self.step(step, done + 1, left.len());
        }
    }

    /// Deletes humatad's clones no project or session uses. Stops at the
    /// first that can't go, after trying the rest.
    async fn remove_unused_clones(&self) -> Result<(), DaemonError> {
        let used = self.used_clones()?;
        let (state, cache) = (self.places.state.clone(), self.places.cache.clone());
        let unused = tokio::task::spawn_blocking(move || {
            unused_clones(
                &state,
                &cache,
                &used,
                std::time::SystemTime::now() - CLONE_SETTLES,
            )
        })
        .await
        .unwrap_or_default();
        let step = "Removing clones no project uses";
        let total = unused.len();
        self.step(step, 0, total);
        let mut failed = None;
        for (done, clone) in unused.into_iter().enumerate() {
            let path = clone.clone();
            let removed = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&path)).await;
            if let Ok(Err(source)) = removed {
                failed.get_or_insert(DaemonError::RemoveClone {
                    path: clone,
                    source,
                });
            }
            self.step(step, done + 1, total);
        }
        failed.map_or(Ok(()), Err)
    }

    /// The repositories the projects use.
    fn used_clones(&self) -> Result<BTreeSet<PathBuf>, DaemonError> {
        Ok(self
            .store
            .projects()?
            .into_iter()
            .flat_map(|project| project.repositories)
            .map(|repository| repository.path)
            .collect())
    }

    /// The repositories on the Mac that hold sessions' snapshots: the
    /// projects' and the recovery copies.
    fn repositories_here(&self) -> Result<Vec<PathBuf>, DaemonError> {
        let mut repositories: BTreeSet<PathBuf> = self
            .store
            .projects()?
            .into_iter()
            .flat_map(|project| project.repositories)
            .map(|repository| repository.path)
            .filter(|path| path.exists())
            .collect();
        repositories.extend(self.mirrors_here());
        Ok(repositories.into_iter().collect())
    }

    /// The repositories humatad made, which it may pack: its clones and
    /// the recovery copies. The person's own folders are theirs.
    fn packed_here(&self) -> Vec<PathBuf> {
        let mut packed: Vec<PathBuf> = [
            self.clones.dir.clone(),
            self.places.cache.join("repositories"),
        ]
        .iter()
        .flat_map(|dir| children(dir))
        .filter(|clone| clone.join(".git").exists())
        .collect();
        packed.extend(self.mirrors_here());
        packed
    }

    fn mirrors_here(&self) -> Vec<PathBuf> {
        ["projects", "sessions"]
            .iter()
            .flat_map(|kind| children(&self.recovery.join(kind)))
            .flat_map(|folder| children(&folder))
            .filter(|mirror| {
                mirror
                    .extension()
                    .is_some_and(|extension| extension == "git")
            })
            .collect()
    }

    /// Seals the histories of this Mac's sessions that are archived or idle
    /// for [`SEAL_IDLE_DAYS`], and drops every idle one no one watches from
    /// memory, which reads it again when it's opened.
    async fn seal_histories(&self, sessions: &[pb::SessionInfo]) -> Result<(), DaemonError> {
        let before = now_unix_ms() - SEAL_IDLE_DAYS * DAY_MS;
        let seal: std::collections::HashSet<String> = sessions
            .iter()
            .filter(|session| session.host.is_empty())
            .filter(|session| session.archived || session.last_active_unix_ms < before)
            .map(|session| session.id.clone())
            .collect();
        let step = "Sealing old histories";
        self.step(step, 0, 0);
        let me = self.me.clone();
        let report = Arc::new(move |done: usize, total: usize| {
            if let Some(daemon) = me.upgrade() {
                daemon.step(step, done, total);
            }
        });
        self.local
            .rest(&seal, report)
            .await
            .map_err(|source| DaemonError::Seal {
                source: Box::new(source),
            })?;
        Ok(())
    }

    fn step(&self, step: &str, done: usize, total: usize) {
        let count = |count: usize| u32::try_from(count).unwrap_or(u32::MAX);
        self.lock_cleaning().job = Some(pb::StorageJob {
            step: step.to_owned(),
            done: count(done),
            total: count(total),
        });
    }

    fn lock_cleaning(&self) -> std::sync::MutexGuard<'_, Cleaning> {
        self.cleaning
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Deletes the snapshots `archived` sessions saved part way through turns
/// from the repository at `repository`; each turn's own snapshot stays.
async fn drop_saves(repository: &Path, archived: &BTreeSet<String>) {
    let listed = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["for-each-ref", "--format=%(refname)", "refs/humata/"])
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    let Ok(listed) = listed else {
        return;
    };
    let deletes: String = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter(|name| {
            let mut parts = name.trim_start_matches("refs/humata/").split('/');
            let session = parts.next().unwrap_or_default();
            archived.contains(session) && parts.next() == Some("saved")
        })
        .map(|name| format!("delete {name}\n"))
        .collect();
    if deletes.is_empty() {
        return;
    }
    let child = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["update-ref", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        if let Some(mut input) = child.stdin.take() {
            use tokio::io::AsyncWriteExt as _;
            let _ = input.write_all(deletes.as_bytes()).await;
        }
        let _ = child.wait().await;
    }
}

fn children(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

/// humatad's clones that no project (`used`) and no session record on this
/// Mac names, left alone since `touched_before`.
fn unused_clones(
    state: &Path,
    cache: &Path,
    used: &BTreeSet<PathBuf>,
    touched_before: std::time::SystemTime,
) -> Vec<PathBuf> {
    let records: Vec<String> = crate::old_clones::session_records(&state.join("host"))
        .iter()
        .filter_map(|record| std::fs::read_to_string(record).ok())
        .collect();
    [state.join("repositories"), cache.join("repositories")]
        .iter()
        .flat_map(|dir| children(dir))
        .filter(|clone| clone.is_dir() && !used.contains(clone))
        .filter(|clone| {
            let path = clone.to_string_lossy();
            !records.iter().any(|record| record.contains(path.as_ref()))
        })
        .filter(|clone| {
            std::fs::symlink_metadata(clone)
                .and_then(|metadata| metadata.modified())
                .is_ok_and(|modified| modified < touched_before)
        })
        .collect()
}

/// What each kind of file takes on disk, walking the folders; `search` of
/// the store is the search index.
fn measure(state: &Path, cache: &Path, search: u64) -> Vec<pb::StorageUse> {
    let store = state.join("humata.sqlite3");
    let with_suffix = |path: &Path, suffix: &str| {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    let log = state.join("host.log");
    let kinds: [(pb::StorageKind, Vec<PathBuf>, PathBuf); 6] = [
        (
            pb::StorageKind::Clones,
            vec![state.join("repositories"), cache.join("repositories")],
            state.join("repositories"),
        ),
        (
            pb::StorageKind::Worktrees,
            vec![cache.join("worktrees")],
            cache.join("worktrees"),
        ),
        (
            pb::StorageKind::Histories,
            vec![state.join("host")],
            state.join("host"),
        ),
        (
            pb::StorageKind::Recovery,
            vec![state.join("recovery")],
            state.join("recovery"),
        ),
        (
            pb::StorageKind::Store,
            vec![
                store.clone(),
                with_suffix(&store, "-wal"),
                with_suffix(&store, "-shm"),
            ],
            store,
        ),
        (
            pb::StorageKind::Logs,
            vec![log.clone(), with_suffix(&log, ".1")],
            log,
        ),
    ];
    let mut uses: Vec<pb::StorageUse> = kinds
        .into_iter()
        .map(|(kind, paths, shown)| pb::StorageUse {
            kind: kind.into(),
            bytes: paths.iter().map(|path| size(path)).sum(),
            path: shown.to_string_lossy().into_owned(),
        })
        .collect();
    if let Some(store) = uses
        .iter_mut()
        .find(|used| used.kind() == pb::StorageKind::Store)
    {
        let search = search.min(store.bytes);
        store.bytes -= search;
        let path = store.path.clone();
        uses.push(pb::StorageUse {
            kind: pb::StorageKind::Search.into(),
            bytes: search,
            path,
        });
    }
    uses
}

/// The disk space the file or folder at `path` takes, links not followed.
fn size(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    let own = metadata.blocks() * 512;
    if !metadata.is_dir() {
        return own;
    }
    own + children(path).iter().map(|child| size(child)).sum::<u64>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_measures_what_its_files_take() {
        let root = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let worktrees = root.path().join("cache/worktrees/s1");
        std::fs::create_dir_all(&worktrees).unwrap_or_else(|error| panic!("{error}"));
        std::fs::write(worktrees.join("big"), vec![1u8; 1 << 20])
            .unwrap_or_else(|error| panic!("{error}"));

        let uses = measure(&root.path().join("state"), &root.path().join("cache"), 0);

        let of = |kind: pb::StorageKind| {
            uses.iter()
                .find(|used| used.kind() == kind)
                .map_or(0, |used| used.bytes)
        };
        assert!(of(pb::StorageKind::Worktrees) >= 1 << 20);
        assert_eq!(of(pb::StorageKind::Clones), 0);
    }

    #[test]
    fn only_clones_nothing_uses_are_unused() {
        let root = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (state, cache) = (root.path().join("state"), root.path().join("cache"));
        let clone = |name: &str| {
            let path = state.join("repositories").join(name);
            std::fs::create_dir_all(path.join(".git")).unwrap_or_else(|error| panic!("{error}"));
            path
        };
        let (project, session, spare) = (clone("project"), clone("session"), clone("spare"));
        let record = state.join("host/claude/sessions/s1");
        std::fs::create_dir_all(&record).unwrap_or_else(|error| panic!("{error}"));
        std::fs::write(
            record.join("session.json"),
            format!("{{\"repositories\": [{:?}]}}", session.to_string_lossy()),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let used = BTreeSet::from([project]);
        let later = std::time::SystemTime::now() + Duration::from_secs(60);

        assert_eq!(unused_clones(&state, &cache, &used, later), vec![spare]);
        // One cloned just now stays.
        let earlier = std::time::SystemTime::now() - Duration::from_secs(60);
        assert!(unused_clones(&state, &cache, &used, earlier).is_empty());
    }
}
