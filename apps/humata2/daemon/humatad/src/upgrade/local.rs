//! This Mac's own bin folder, beside the running host daemon (D370, D371).
//!
//! The daemon looks at `<bin>` when it is ready and again on every change
//! there (a push from `notify`, never a poll):
//!
//! - A release the app staged as `.<name>.next` and `.bundle.json.next` is
//!   one VIP "Migrate the daemon" item for this machine; taking it away
//!   withdraws the item. The staged files are checked first, so an item is
//!   only ever for a release that can be swapped in.
//! - With no other release staged, a `humata` or `humata2_holder` that is
//!   there and isn't the one the release names is a CLI or host tools item,
//!   and putting the release's back withdraws it.
//!
//! Only a host daemon looks: a machine's own tools are checked by the host
//! from what the machine reports (`items::Watch`). A daemon that names no
//! release posts nothing (D290 l).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use humata2_grpc::humata::v1 as pb;
use notify::{EventKind, RecursiveMode, Watcher};

use super::build::Release;
use super::items::{MigrateError, Migrations};
use super::{run, tools};
use crate::status;

/// Why the bin folder couldn't be watched.
#[derive(Debug, thiserror::Error)]
enum WatchError {
    #[error(
        "couldn't watch {} for a staged release or a changed CLI, so they are only noticed when this daemon starts. Restart humatad2 after the app stages a release, or check that your user can read that folder",
        .path.display()
    )]
    Watch {
        path: PathBuf,
        #[source]
        source: notify::Error,
    },
}

/// What one look at the bin folder found of a staged release.
enum Staged {
    /// None, or one that is the release already running.
    Nothing,
    /// A release other than the running one, its files checked.
    Release(pb::BuildInfo),
    /// A staged release that couldn't be checked: what is shown stays.
    Unknown,
}

struct Looked {
    own: pb::BuildInfo,
    staged: Staged,
}

struct Inner {
    migrations: Migrations,
    release: Release,
    machine: pb::MachineId,
    bin: PathBuf,
    /// The staged bundle whose files last checked out, so a look at the same
    /// staged release doesn't hash its files again.
    verified: Mutex<Option<String>>,
    /// Held while one look is made and shown.
    one_look: tokio::sync::Mutex<()>,
    /// Woken after each look: the holder file may have changed, and with it
    /// which sessions are on an old holder (D373).
    after_look: OnceLock<Arc<tokio::sync::Notify>>,
}

/// The looks at this host's bin folder.
#[derive(Clone)]
pub(crate) struct Local {
    inner: Arc<Inner>,
}

impl Local {
    /// The bin folder `bin` of the daemon of `machine`, which runs `release`.
    pub(crate) fn new(migrations: Migrations, release: Release, machine: &str, bin: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                migrations,
                release,
                machine: pb::MachineId {
                    value: machine.to_owned(),
                },
                bin,
                verified: Mutex::new(None),
                one_look: tokio::sync::Mutex::new(()),
                after_look: OnceLock::new(),
            }),
        }
    }

    /// `wake` is woken after every look at the bin folder.
    pub(crate) fn wake_after_looks(&self, wake: Arc<tokio::sync::Notify>) {
        let _ = self.inner.after_look.set(wake);
    }

    /// Starts watching the bin folder, then looks once and returns when what
    /// was found is in the Inbox; every later change is looked at in the
    /// background. The watch comes first, so a change during the first look
    /// is looked at again.
    pub(crate) async fn start(&self) {
        if self.inner.release.bundle().is_empty() {
            return;
        }
        let wake = Arc::new(tokio::sync::Notify::new());
        let watcher = match self.watcher(wake.clone()) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                crate::log::say!("humatad2: warning: {}", status::chain(&error));
                None
            }
        };
        self.check().await;
        if let Some(watcher) = watcher {
            let me = self.clone();
            tokio::spawn(async move {
                let _watching = watcher;
                loop {
                    wake.notified().await;
                    me.check().await;
                }
            });
        }
    }

    fn watcher(&self, wake: Arc<tokio::sync::Notify>) -> Result<notify::RecommendedWatcher, WatchError> {
        let path = &self.inner.bin;
        let failed = |source: notify::Error| WatchError::Watch {
            path: path.clone(),
            source,
        };
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            // Reading a file is not a change to it, and this daemon reads
            // these to hash them; an error means events were lost.
            if !matches!(&event, Ok(event) if matches!(event.kind, EventKind::Access(_))) {
                wake.notify_one();
            }
        })
        .map_err(failed)?;
        watcher.watch(path, RecursiveMode::NonRecursive).map_err(failed)?;
        Ok(watcher)
    }

    /// Looks at the bin folder and shows what is found.
    async fn check(&self) {
        let _one = self.inner.one_look.lock().await;
        let me = self.clone();
        let looked = match tokio::task::spawn_blocking(move || me.look()).await {
            Ok(looked) => looked,
            Err(error) => {
                crate::log::say!(
                    "humatad2: warning: looking at the bin folder failed ({error}), so a staged release or a changed CLI isn't shown until the next change there"
                );
                return;
            }
        };
        if let Err(error) = self.show(looked).await {
            crate::log::say!("humatad2: warning: {}", status::chain(&error));
        }
        if let Some(wake) = self.inner.after_look.get() {
            wake.notify_one();
        }
    }

    /// The blocking part: what is installed now, and what is staged.
    fn look(&self) -> Looked {
        let inner = &self.inner;
        inner.release.rescan_installed(&inner.bin);
        let own = inner.release.info();
        let verified = inner.verified.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let staged = match run::staged(&inner.bin, verified.as_deref()) {
            Ok(None) => Staged::Nothing,
            Ok(Some(to)) if to.bundle == own.bundle => Staged::Nothing,
            Ok(Some(to)) => {
                *inner.verified.lock().unwrap_or_else(PoisonError::into_inner) = Some(to.bundle.clone());
                Staged::Release(to)
            }
            Err(error) => {
                crate::log::say!(
                    "humatad2: warning: the release staged in {} can't be offered for Migrate: {}",
                    inner.bin.display(),
                    status::chain(&error)
                );
                Staged::Unknown
            }
        };
        Looked { own, staged }
    }

    async fn show(&self, looked: Looked) -> Result<(), MigrateError> {
        let (migrations, machine) = (&self.inner.migrations, &self.inner.machine);
        let own = looked.own;
        if own.bundle.is_empty() {
            return Ok(());
        }
        match looked.staged {
            Staged::Release(to) => {
                // The daemon item says what differs among the tools too.
                migrations
                    .observe(pb::MigrateThing::Daemon, machine, own, to)
                    .await?;
                return Ok(());
            }
            Staged::Nothing => {
                migrations
                    .resolved_if_waiting(pb::MigrateThing::Daemon, machine)
                    .await?;
            }
            Staged::Unknown => {}
        }
        let differing = tools::differing(&own.files, &own.installed);
        for thing in tools::THINGS {
            if tools::wants(&differing, thing) {
                migrations
                    .observe_tool(thing, machine, tools::from_installed(&own), tools::release_only(&own))
                    .await?;
            } else {
                migrations.resolved(thing, machine).await?;
            }
        }
        Ok(())
    }
}
