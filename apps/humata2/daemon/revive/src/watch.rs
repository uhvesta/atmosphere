//! Waiting on one file to change, from the system's file events (FSEvents
//! on macOS, inotify on Linux), instead of looking again and again.
//!
//! Only the file that matters is watched (TR-12): its folder, not
//! recursively, with every event about another file dropped. The folder
//! rather than the file, so a file replaced by a rename is still seen.

use std::path::{Path, PathBuf};

use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc;

/// Changes to one file.
pub(crate) struct FileWatch {
    /// None when the file couldn't be watched: then nothing ever changes,
    /// and the caller looks once more when it gives up.
    watching: Option<(notify::RecommendedWatcher, mpsc::Receiver<()>)>,
}

impl FileWatch {
    pub(crate) fn new(file: &Path) -> Self {
        let (changed, changes) = mpsc::channel(1);
        let wanted: PathBuf = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_owned());
        let name = wanted.file_name().map(ToOwned::to_owned);
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            // Compared by name too: the events' paths may be spelled
            // through another link to the folder.
            let ours = event
                .paths
                .iter()
                .any(|path| *path == wanted || (name.is_some() && path.file_name() == name.as_deref()));
            if ours {
                // One waiting is enough: the caller looks at the file.
                let _ = changed.try_send(());
            }
        });
        let folder = match file.parent() {
            Some(folder) if !folder.as_os_str().is_empty() => folder,
            _ => Path::new("."),
        };
        let watching = watcher.and_then(|mut watcher| {
            watcher.watch(folder, RecursiveMode::NonRecursive)?;
            Ok(watcher)
        });
        match watching {
            Ok(watcher) => Self {
                watching: Some((watcher, changes)),
            },
            Err(error) => {
                eprintln!(
                    "humata2_revive: couldn't watch {} for changes, so a resume with it is only checked when it times out: {error}",
                    file.display()
                );
                Self { watching: None }
            }
        }
    }

    /// Returns once the file has changed since the last call, or never.
    pub(crate) async fn changed(&mut self) {
        if let Some((_, changes)) = &mut self.watching
            && changes.recv().await.is_some()
        {
            return;
        }
        std::future::pending().await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::FileWatch;

    /// Whether the watch wakes within a second.
    async fn woke(watch: &mut FileWatch) -> bool {
        tokio::time::timeout(Duration::from_secs(1), watch.changed()).await.is_ok()
    }

    #[tokio::test]
    async fn only_the_watched_file_wakes_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("mine.jsonl");
        std::fs::write(&file, "a").unwrap();
        let mut watch = FileWatch::new(&file);
        // FSEvents may report what happened just before the watch began.
        let _ = woke(&mut watch).await;

        std::fs::write(dir.path().join("other.jsonl"), "b").unwrap();
        std::fs::create_dir(dir.path().join("deeper")).unwrap();
        std::fs::write(dir.path().join("deeper/mine.jsonl"), "c").unwrap();
        assert!(!woke(&mut watch).await, "another file woke the watch");

        std::fs::write(&file, "ab").unwrap();
        assert!(woke(&mut watch).await, "the watched file's change was missed");
    }
}
