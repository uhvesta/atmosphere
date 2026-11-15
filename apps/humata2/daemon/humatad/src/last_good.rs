//! Keeps the store's last good copy fresh (PER-31): once writes to the
//! database go quiet, a copy is saved off the runtime. The store says when
//! it commits a write (`Store::on_commit`), so an idle daemon runs no
//! timer and nothing watches files; the quiet wait runs only while writes
//! are pending (TR-7, D54, D77).

use std::sync::Arc;
use std::time::Duration;

use humata2_store::{LastGood, Store};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::status;

/// When to save a copy.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Quiet {
    /// How long writes stay quiet before a copy is saved.
    pub(crate) after: Duration,
    /// The longest a stream of writes that never goes quiet waits for one.
    pub(crate) at_most: Duration,
}

impl Default for Quiet {
    fn default() -> Self {
        Self {
            after: Duration::from_secs(30),
            at_most: Duration::from_secs(10 * 60),
        }
    }
}

/// Starts keeping `store`'s last good copy. The receiver has the latest
/// copy saved since; dropping it doesn't stop the keeping.
pub(crate) fn keep(store: Arc<Store>, quiet: Quiet) -> watch::Receiver<Option<LastGood>> {
    let (saved, latest) = watch::channel(None);
    let (wrote, writes) = mpsc::channel(1);
    store.on_commit(move || {
        // One pending is enough: the keeper only needs to know there was one.
        let _ = wrote.try_send(());
    });
    tokio::spawn(run(store, quiet, writes, saved));
    latest
}

async fn run(
    store: Arc<Store>,
    quiet: Quiet,
    mut writes: mpsc::Receiver<()>,
    saved: watch::Sender<Option<LastGood>>,
) {
    while writes.recv().await.is_some() {
        let first = Instant::now();
        loop {
            let deadline = (Instant::now() + quiet.after).min(first + quiet.at_most);
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => break,
                more = writes.recv() => {
                    if more.is_none() {
                        return;
                    }
                }
            }
        }
        let saving = store.clone();
        match tokio::task::spawn_blocking(move || saving.save_last_good()).await {
            Ok(Ok(copy)) => {
                saved.send_replace(Some(copy));
            }
            Ok(Err(error)) => eprintln!(
                "humatad2: couldn't save a last good copy of the state in {}; the one before is kept. Check the disk isn't full: {}",
                store.root().display(),
                status::chain(&error)
            ),
            Err(error) => eprintln!("humatad2: saving the last good copy stopped: {error}"),
        }
        // A write committed while the copy was taken waited for it, so it
        // is pending here and gets a copy of its own.
    }
}

/// Where the copy is, for tests.
#[cfg(test)]
fn copy_in(root: &std::path::Path) -> std::path::PathBuf {
    root.join("last-good.sqlite")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use humata2_store::{SessionRecord, Store};

    use super::{Quiet, copy_in, keep};

    fn session(id: &str, folder: PathBuf) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            channel: "c_1".to_owned(),
            machine: "m_1".to_owned(),
            title: "work".to_owned(),
            agent: "zsh".to_owned(),
            folder,
            created_unix_ms: 1,
        }
    }

    const QUICK: Quiet = Quiet {
        after: Duration::from_millis(200),
        at_most: Duration::from_secs(5),
    };

    #[tokio::test]
    async fn a_copy_is_saved_once_writes_go_quiet() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path().join("store")).unwrap());
        let mut latest = keep(store.clone(), QUICK);

        store.put_session(&session("s_1", dir.path().to_owned())).unwrap();
        tokio::time::timeout(Duration::from_secs(20), latest.changed()).await.unwrap().unwrap();

        let copy = latest.borrow().clone().unwrap();
        assert!(copy.path.is_file());
        assert_eq!(copy.path, copy_in(store.root()));
        let reopened = Store::open_or_restore(dir.path().join("store")).unwrap().0;
        assert!(reopened.session("s_1").unwrap().is_some());
    }

    #[tokio::test]
    async fn an_idle_store_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path().join("store")).unwrap());
        let mut latest = keep(store.clone(), QUICK);

        let saved = tokio::time::timeout(Duration::from_secs(2), latest.changed()).await;

        assert!(saved.is_err(), "saved a copy with nothing written");
        assert!(!copy_in(store.root()).exists());
    }
}
