//! DV-12 and the live half of DV-5: when a Desk folder's repository gains a
//! commit, from the user's Save version or the agent's plain `git commit`,
//! Humata records, for each file the commit changed, the session's comments
//! on that file, and tells the app the session changed so its strip and
//! version list update without a reload.
//!
//! Nothing here polls. Each open Desk session's `.git` folder is watched for
//! the files a commit rewrites (`HEAD`, `packed-refs`, the branch refs, all
//! written through a lock file and a rename, so folders are watched, not
//! files), the set of watched folders follows the session events, and a
//! burst of changes is looked at once, 100 ms after it starts. A folder
//! starts being watched by recording every commit that has no record yet
//! (newest 256), so commits made while the daemon was down are recorded too,
//! with the comments as they are now (DV-6 shows only the ones with a
//! record as having comments).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::doc_repo::{self, DocGit, DocRepoError};
use humata2_store::{SessionRecord, Store, StoreError};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use prost::Message;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::mpsc;
use tonic::Status;

use super::{DeskSvc, now_ms};
use crate::ports::blocking;
use crate::ports::Feed;

/// How long a burst of changes is let to finish before it is looked at.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// What went wrong recording a version's comments. The version itself is
/// safe in the folder's repository; only its comments are missing.
#[derive(Debug, thiserror::Error)]
pub(super) enum RecordError {
    #[error(
        "couldn't {doing} for Desk session {session}, so its newest versions show their comments as not recorded; check that Humata's state folder is writable, then save another version: {source}"
    )]
    Store {
        session: String,
        doing: &'static str,
        #[source]
        source: StoreError,
    },
    #[error(
        "couldn't read the versions in {}, so their comments were not recorded; check that git is installed and the folder is there and writable: {source}",
        folder.display()
    )]
    Repo {
        folder: PathBuf,
        #[source]
        source: DocRepoError,
    },
}

/// Why a folder can't be watched for new versions.
#[derive(Debug, thiserror::Error)]
enum WatchError {
    #[error(
        "couldn't watch {} for new versions, so the comments of a version an agent makes there are not recorded until this daemon restarts or the session changes again; check that the folder's .git is readable and the machine's file-watch limit isn't used up: {source}",
        folder.display()
    )]
    Watch {
        folder: PathBuf,
        #[source]
        source: notify::Error,
    },
}

/// A look the watch has been asked for.
#[derive(Default)]
struct Pending {
    /// The open sessions changed: start and stop watching folders.
    sync: bool,
    /// Something was missed: look at every folder.
    all: bool,
    /// Paths that moved a head.
    paths: HashSet<PathBuf>,
}

/// What a file-system callback tells the watch.
enum Touch {
    Path(PathBuf),
    /// Events were lost.
    Lost,
}

/// One watched folder.
struct Folder {
    folder: PathBuf,
    /// The folder as the file-system reports it, which differs on a
    /// symlinked path.
    root: PathBuf,
    /// The newest commit as last seen; `None` before the first, or before
    /// the folder has been caught up.
    head: Option<String>,
    caught_up: bool,
}

/// The watcher and the folders it watches.
struct Watching {
    watcher: RecommendedWatcher,
    folders: HashMap<String, Folder>,
}

/// Starts the watch.
pub(super) fn spawn_watch(svc: DeskSvc, feed: Arc<Feed>) {
    tokio::spawn(async move {
        // Subscribed before the first look, so nothing in between is missed.
        let (_, mut events) = feed.subscribe(None);
        let (touch, mut touched) = mpsc::unbounded_channel::<Touch>();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            // Reading the repository is not a change to it; git reads it to look.
            Ok(event) if matches!(event.kind, EventKind::Access(_)) => {}
            Ok(event) if event.need_rescan() => {
                let _ = touch.send(Touch::Lost);
            }
            Ok(event) => {
                for path in event.paths.into_iter().filter(|path| moves_a_head(path)) {
                    let _ = touch.send(Touch::Path(path));
                }
            }
            Err(_) => {
                let _ = touch.send(Touch::Lost);
            }
        });
        let watcher = match watcher {
            Ok(watcher) => watcher,
            Err(error) => {
                humata2_rpc_kit::log::say!(
                    "humatad2: couldn't start watching Desk folders for new versions, so the comments of versions agents make are not recorded until this daemon restarts; check the machine's file-watch limit: {error}"
                );
                return;
            }
        };
        let watching = Arc::new(Mutex::new(Watching {
            watcher,
            folders: HashMap::new(),
        }));
        let mut pending = Pending {
            sync: true,
            ..Pending::default()
        };
        loop {
            let (store, machine, has_remote) = (svc.starter.store.clone(), svc.machine.clone(), svc.remote.is_some());
            let (held, due) = (watching.clone(), std::mem::take(&mut pending));
            if let Ok(moved) = blocking(move || {
                let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
                look(&store, &machine, has_remote, &mut held, due)
            })
            .await
            {
                for session in moved {
                    svc.tell.session(&session, pb::Change::Updated, None).await;
                }
            }
            // Nothing runs until a folder's refs move or the sessions change.
            loop {
                tokio::select! {
                    touch = touched.recv() => match touch {
                        Some(touch) => { pending.add(touch); break; }
                        None => return,
                    },
                    event = events.recv() => match event {
                        Ok(event) if changes_session(&event) => { pending.sync = true; break; }
                        Ok(_) => {}
                        // Missed some: look at everything again.
                        Err(RecvError::Lagged(_)) => { pending.sync = true; pending.all = true; break; }
                        Err(RecvError::Closed) => return,
                    },
                }
            }
            // Let the burst finish, then look once.
            tokio::time::sleep(DEBOUNCE).await;
            while let Ok(touch) = touched.try_recv() {
                pending.add(touch);
            }
            loop {
                match events.try_recv() {
                    Ok(event) => pending.sync |= changes_session(&event),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Lagged(_)) => {
                        pending.sync = true;
                        pending.all = true;
                    }
                    Err(TryRecvError::Closed) => return,
                }
            }
        }
    });
}

impl Pending {
    fn add(&mut self, touch: Touch) {
        match touch {
            Touch::Path(path) => {
                self.paths.insert(path);
            }
            Touch::Lost => self.all = true,
        }
    }
}

/// True for an event that tells of a session, which is what opens and closes
/// a Desk folder.
fn changes_session(event: &pb::Event) -> bool {
    matches!(event.payload, Some(pb::event::Payload::Session(_)))
}

/// True for a path a commit rewrites: `HEAD`, `packed-refs` or a branch ref,
/// not the lock file it is written through.
fn moves_a_head(path: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
    !name.ends_with(".lock")
        && (name == "HEAD" || name == "packed-refs" || path.components().any(|part| part.as_os_str() == "heads"))
}

/// One look: brings the watched folders in line with the open sessions when
/// asked, then records what the folders that moved gained. Answers the
/// sessions that gained recorded commits.
fn look(store: &Store, machine: &str, has_remote: bool, watching: &mut Watching, due: Pending) -> Vec<String> {
    let git = DocGit::inherit();
    let mut check: HashSet<String> = HashSet::new();
    if due.sync {
        match open_folders(store, machine, has_remote) {
            Ok(open) => {
                let gone: Vec<String> = watching
                    .folders
                    .iter()
                    .filter(|(id, watched)| !open.iter().any(|(now, folder)| now == *id && *folder == watched.folder))
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in gone {
                    if let Some(watched) = watching.folders.remove(&id) {
                        unwatch(&mut watching.watcher, &watched.folder);
                    }
                }
                for (id, folder) in open {
                    if watching.folders.contains_key(&id) {
                        continue;
                    }
                    // Watched before its head is read, so a commit in between is seen.
                    match watch(&mut watching.watcher, &folder) {
                        Ok(()) => {
                            let root = folder.canonicalize().unwrap_or_else(|_| folder.clone());
                            watching.folders.insert(
                                id.clone(),
                                Folder {
                                    folder,
                                    root,
                                    head: None,
                                    caught_up: false,
                                },
                            );
                            check.insert(id);
                        }
                        Err(error) => humata2_rpc_kit::log::say!("humatad2: {error}"),
                    }
                }
            }
            Err(error) => humata2_rpc_kit::log::say!(
                "humatad2: couldn't list the Desk sessions to watch for new versions, so none are recorded until a session changes again: {error}"
            ),
        }
    }
    for (id, watched) in &watching.folders {
        if due.all
            || due
                .paths
                .iter()
                .any(|path| path.starts_with(&watched.root) || path.starts_with(&watched.folder))
        {
            check.insert(id.clone());
        }
    }
    let mut moved = Vec::new();
    for id in check {
        let Some(watched) = watching.folders.get_mut(&id) else {
            continue;
        };
        if catch_up(store, &git, &id, watched) {
            moved.push(id);
        }
    }
    moved
}

/// Reads the folder's head and records the commits that have no record:
/// those since the head last seen, or, the first time, the newest ones back
/// to the newest that has a record. Whether any were recorded.
fn catch_up(store: &Store, git: &DocGit, id: &str, watched: &mut Folder) -> bool {
    let new = match doc_repo::head(git, &watched.folder) {
        Ok(head) => head,
        Err(error) => {
            humata2_rpc_kit::log::say!(
                "humatad2: couldn't read the newest version of {}, so a new one is not recorded yet: {error}",
                watched.folder.display()
            );
            return false;
        }
    };
    let Some(new) = new else {
        watched.head = None;
        watched.caught_up = true;
        return false;
    };
    let commits = if !watched.caught_up {
        match unrecorded(store, git, id, &watched.folder, &new) {
            Ok(commits) => commits,
            Err(error) => {
                humata2_rpc_kit::log::say!("humatad2: {error}");
                return false;
            }
        }
    } else if watched.head.as_deref() == Some(new.as_str()) {
        return false;
    } else {
        doc_repo::commits_since(git, &watched.folder, watched.head.as_deref(), &new)
            .unwrap_or_else(|_| vec![new.clone()])
    };
    watched.head = Some(new);
    watched.caught_up = true;
    if let Err(error) = record(store, git, id, &watched.folder, &commits, now_ms()) {
        humata2_rpc_kit::log::say!("humatad2: {error}");
    }
    !commits.is_empty()
}

/// The commits up to `head` that no record names, oldest first: everything
/// after the newest commit that has a record, looking back at most 256.
fn unrecorded(store: &Store, git: &DocGit, id: &str, folder: &Path, head: &str) -> Result<Vec<String>, RecordError> {
    let recorded: HashSet<String> = store
        .recorded_doc_commits(id)
        .map_err(|source| RecordError::Store {
            session: id.to_owned(),
            doing: "read which versions have recorded comments",
            source,
        })?
        .into_iter()
        .collect();
    let recent = doc_repo::commits_since(git, folder, None, head).map_err(|source| RecordError::Repo {
        folder: folder.to_path_buf(),
        source,
    })?;
    let start = recent
        .iter()
        .rposition(|commit| recorded.contains(commit))
        .map_or(0, |at| at + 1);
    Ok(recent[start..].to_vec())
}

/// Watches the folder's `.git` (where `HEAD` and `packed-refs` are rewritten)
/// and its branch refs.
fn watch(watcher: &mut RecommendedWatcher, folder: &Path) -> Result<(), WatchError> {
    let git = folder.join(".git");
    let failed = |source| WatchError::Watch {
        folder: folder.to_path_buf(),
        source,
    };
    watcher.watch(&git, RecursiveMode::NonRecursive).map_err(failed)?;
    let heads = git.join("refs").join("heads");
    if heads.is_dir() {
        watcher.watch(&heads, RecursiveMode::Recursive).map_err(failed)?;
    }
    Ok(())
}

fn unwatch(watcher: &mut RecommendedWatcher, folder: &Path) {
    let git = folder.join(".git");
    // A folder that is gone has nothing left to unwatch.
    let _ = watcher.unwatch(&git.join("refs").join("heads"));
    let _ = watcher.unwatch(&git);
}

/// The open Desk and channel Desk sessions of this machine whose folder is a
/// versions repository, with that folder.
fn open_folders(store: &Store, machine: &str, has_remote: bool) -> Result<Vec<(String, PathBuf)>, StoreError> {
    let mut open: Vec<String> = store
        .desks()?
        .into_iter()
        .filter(|row| row.closed_unix_ms.is_none() && row.deleted_unix_ms.is_none())
        .map(|row| row.session_id)
        .collect();
    open.extend(
        store
            .channel_desks()?
            .into_iter()
            .filter(|row| row.closed_unix_ms.is_none())
            .map(|row| row.session_id),
    );
    if open.is_empty() {
        return Ok(Vec::new());
    }
    let sessions: HashMap<String, SessionRecord> = store
        .sessions()?
        .into_iter()
        .map(|session| (session.id.clone(), session))
        .collect();
    Ok(open
        .into_iter()
        .filter_map(|id| {
            let session = sessions.get(&id)?;
            let here = !has_remote || session.machine.is_empty() || session.machine == machine;
            (here && doc_repo::is_doc_repo(&session.folder)).then(|| (id, session.folder.clone()))
        })
        .collect())
}

/// Records, for each of `commits` and each file it changed, the session's
/// current comments on that file: drafts included, archived ones left out.
/// A commit and file that already have a record keep it.
pub(super) fn record(
    store: &Store,
    git: &DocGit,
    session: &str,
    folder: &Path,
    commits: &[String],
    at: i64,
) -> Result<(), RecordError> {
    let store_error = |doing| move |source| RecordError::Store {
        session: session.to_owned(),
        doing,
        source,
    };
    let kept = store
        .comments(session)
        .map_err(store_error("read the session's comments"))?;
    // Each file's comments, one length-delimited `Comment` after another.
    let mut on_file: HashMap<String, Vec<u8>> = HashMap::new();
    for row in kept.comments.iter().filter(|row| !row.archived) {
        let Ok(comment) = pb::Comment::decode(row.comment.as_slice()) else {
            continue;
        };
        if comment.archived_at.is_some() {
            continue;
        }
        if let Some(path) = folder_file(&comment) {
            on_file
                .entry(path)
                .or_default()
                .extend(comment.encode_length_delimited_to_vec());
        }
    }
    for commit in commits {
        let paths = doc_repo::changed_paths(git, folder, commit).map_err(|source| RecordError::Repo {
            folder: folder.to_path_buf(),
            source,
        })?;
        for path in paths {
            let comments = on_file.get(&path).map_or(&[][..], Vec::as_slice);
            store
                .put_doc_comments(session, commit, &path, comments, at)
                .map_err(store_error("record the comments a version had"))?;
        }
    }
    Ok(())
}

/// The path of a comment on a file of the session's folder itself (no
/// repository named), which is what a Desk document's comments are.
fn folder_file(comment: &pb::Comment) -> Option<String> {
    let file = comment.anchor.as_ref()?.file.as_ref()?;
    let outside = file.repo.as_ref().is_some_and(|repo| !repo.name.is_empty());
    (!outside).then(|| file.path.clone())
}

/// The comments of a record, as they were recorded. One that can't be read
/// by this build is left out.
pub(super) fn decode(bytes: &[u8]) -> Vec<pb::Comment> {
    let mut rest = bytes;
    let mut comments = Vec::new();
    while !rest.is_empty() {
        match pb::Comment::decode_length_delimited(&mut rest) {
            Ok(comment) => comments.push(comment),
            Err(_) => break,
        }
    }
    comments
}

impl DeskSvc {
    /// Records the comments of the commits a save or restore just made.
    /// The call that made them has succeeded, so a failure here is logged,
    /// not returned.
    pub(super) async fn record_commits(&self, session: &str, folder: PathBuf, commits: Vec<String>) {
        let (store, id) = (self.starter.store.clone(), session.to_owned());
        let done = blocking(move || record(&store, &DocGit::inherit(), &id, &folder, &commits, now_ms())).await;
        match done {
            Ok(Ok(())) => {}
            Ok(Err(error)) => humata2_rpc_kit::log::say!("humatad2: {error}"),
            Err(error) => humata2_rpc_kit::log::say!(
                "humatad2: recording the comments of a new version didn't finish: {}",
                error.message()
            ),
        }
    }

    /// The commits that have a comment record for `path`.
    pub(super) async fn recorded_commits(&self, session: &str, path: String) -> Result<HashSet<String>, Status> {
        let (store, id) = (self.starter.store.clone(), session.to_owned());
        blocking(move || store.doc_comment_commits(&id, &path))
            .await?
            .map(|commits| commits.into_iter().collect())
            .map_err(|error| crate::ports::store_error(error, &self.machine, "read which versions have recorded comments"))
    }

    /// The comments recorded for `path` at `commit`; `None` when none were.
    pub(super) async fn recorded_comments(
        &self,
        session: &str,
        commit: String,
        path: String,
    ) -> Result<Option<Vec<pb::Comment>>, Status> {
        let (store, id) = (self.starter.store.clone(), session.to_owned());
        blocking(move || store.doc_comments(&id, &commit, &path))
            .await?
            .map(|record| record.map(|bytes| decode(&bytes)))
            .map_err(|error| crate::ports::store_error(error, &self.machine, "read a version's recorded comments"))
    }
}
