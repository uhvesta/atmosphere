//! Following the conversation files of this machine's running agents (N1).
//!
//! A Session event catches a session up, but an agent may write the end of
//! a turn after the last event of its session: Copilot 1.0.113 writes its
//! reply to `events.jsonl` after its turn-ended hook has run, and nothing
//! happens to the session after that hook. So while a session's agent is
//! launching, running or resuming, the folder its conversation file is in
//! is watched (not polled), and a change to the file catches the session up.
//! Folders are watched rather than files, so a file the agent creates later
//! or writes through a rename is seen too; changes to other files in the
//! folder are dropped. A stopped agent's file is no longer watched.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

/// What the file system told about the watched folders.
pub(super) enum Touch {
    /// This path changed.
    Path(PathBuf),
    /// Changes were lost: every followed session may have gained lines.
    Lost,
}

/// Why a running agent's conversation file can't be followed.
#[derive(Debug, thiserror::Error)]
pub(super) enum FollowError {
    #[error(
        "couldn't start watching agents' conversation files, so what an agent writes after its session last changed is not searchable or readable until the session changes again (a new prompt, a stop or a rename); check the machine's file-watch limit, then restart the daemon"
    )]
    Start {
        #[source]
        source: notify::Error,
    },
    #[error(
        "couldn't watch {} for session {session}'s conversation, so what its agent writes after the session last changed is not searchable or readable until the session changes again; check that the folder is readable and the machine's file-watch limit isn't used up",
        folder.display()
    )]
    Watch {
        session: String,
        folder: PathBuf,
        #[source]
        source: notify::Error,
    },
}

/// The sessions whose conversation files are followed.
pub(super) struct Follow {
    touch: mpsc::UnboundedSender<Touch>,
    watching: Mutex<Watching>,
}

#[derive(Default)]
struct Watching {
    /// Started with the first session followed.
    watcher: Option<RecommendedWatcher>,
    /// Each followed session's file, as named and as the file system names
    /// it (they differ on a symlinked path such as macOS's `/tmp`).
    sessions: HashMap<String, Followed>,
    /// How many followed sessions each watched folder holds a file of.
    folders: HashMap<PathBuf, usize>,
}

struct Followed {
    file: PathBuf,
    folder: PathBuf,
    names: Vec<PathBuf>,
}

impl Follow {
    /// A change to a followed file is told on `touch`.
    pub(super) fn new(touch: mpsc::UnboundedSender<Touch>) -> Self {
        Self {
            touch,
            watching: Mutex::new(Watching::default()),
        }
    }

    /// Follows `file` as session `session`'s conversation, in place of any
    /// file followed for it before. A file whose folder isn't there yet is
    /// not followed; the session's next event tries again.
    pub(super) fn follow(&self, session: &str, file: &Path) -> Result<(), FollowError> {
        let mut watching = self.lock();
        if watching.sessions.get(session).is_some_and(|followed| followed.file == file) {
            return Ok(());
        }
        watching.drop_session(session);
        let Some(folder) = file.parent().filter(|folder| folder.is_dir()) else {
            return Ok(());
        };
        let folder = folder.to_path_buf();
        if watching.watcher.is_none() {
            watching.watcher = Some(self.start()?);
        }
        if !watching.folders.contains_key(&folder)
            && let Some(watcher) = watching.watcher.as_mut()
        {
            watcher
                .watch(&folder, RecursiveMode::NonRecursive)
                .map_err(|source| FollowError::Watch {
                    session: session.to_owned(),
                    folder: folder.clone(),
                    source,
                })?;
        }
        *watching.folders.entry(folder.clone()).or_default() += 1;
        let mut names = vec![file.to_path_buf()];
        if let (Ok(real), Some(name)) = (folder.canonicalize(), file.file_name()) {
            let real = real.join(name);
            if real != file {
                names.push(real);
            }
        }
        watching.sessions.insert(
            session.to_owned(),
            Followed {
                file: file.to_path_buf(),
                folder,
                names,
            },
        );
        Ok(())
    }

    /// Stops following session `session`'s file, if it was.
    pub(super) fn unfollow(&self, session: &str) {
        self.lock().drop_session(session);
    }

    /// The followed sessions `touch` may have changed the file of.
    pub(super) fn touched(&self, touch: &Touch) -> Vec<String> {
        let watching = self.lock();
        watching
            .sessions
            .iter()
            .filter(|(_, followed)| match touch {
                Touch::Path(path) => followed.names.iter().any(|name| name == path),
                Touch::Lost => true,
            })
            .map(|(session, _)| session.clone())
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Watching> {
        self.watching.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn start(&self) -> Result<RecommendedWatcher, FollowError> {
        let touch = self.touch.clone();
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            // The mirror reading a file is not a change to it.
            Ok(event) if matches!(event.kind, EventKind::Access(_)) => {}
            Ok(event) if event.need_rescan() => {
                let _ = touch.send(Touch::Lost);
            }
            Ok(event) => {
                for path in event.paths {
                    let _ = touch.send(Touch::Path(path));
                }
            }
            Err(_) => {
                let _ = touch.send(Touch::Lost);
            }
        })
        .map_err(|source| FollowError::Start { source })
    }
}

impl Watching {
    fn drop_session(&mut self, session: &str) {
        let Some(followed) = self.sessions.remove(session) else {
            return;
        };
        let Some(count) = self.folders.get_mut(&followed.folder) else {
            return;
        };
        *count -= 1;
        if *count == 0 {
            self.folders.remove(&followed.folder);
            if let Some(watcher) = self.watcher.as_mut() {
                // A folder that is gone has nothing left to unwatch.
                let _ = watcher.unwatch(&followed.folder);
            }
        }
    }
}
