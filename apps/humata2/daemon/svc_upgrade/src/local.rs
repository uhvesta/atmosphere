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
use humata2_rpc_kit::helpers as status;
use notify::{EventKind, RecursiveMode, Watcher};

use super::build::Release;
use super::items::{MigrateError, Migrations};
use super::{run, tools};

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
pub struct Local {
    inner: Arc<Inner>,
}

impl Local {
    /// The bin folder `bin` of the daemon of `machine`, which runs `release`.
    pub fn new(migrations: Migrations, release: Release, machine: &str, bin: PathBuf) -> Self {
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
    pub fn wake_after_looks(&self, wake: Arc<tokio::sync::Notify>) {
        let _ = self.inner.after_look.set(wake);
    }

    /// Starts watching the bin folder, then looks once and returns when what
    /// was found is in the Inbox; every later change is looked at in the
    /// background. The watch comes first, so a change during the first look
    /// is looked at again.
    pub async fn start(&self) {
        if self.inner.release.bundle().is_empty() {
            return;
        }
        let wake = Arc::new(tokio::sync::Notify::new());
        let watcher = match self.watcher(wake.clone()) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                humata2_rpc_kit::log::say!("humatad2: warning: {}", status::chain(&error));
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
                humata2_rpc_kit::log::say!(
                    "humatad2: warning: looking at the bin folder failed ({error}), so a staged release or a changed CLI isn't shown until the next change there"
                );
                return;
            }
        };
        if let Err(error) = self.show(looked).await {
            humata2_rpc_kit::log::say!("humatad2: warning: {}", status::chain(&error));
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
                humata2_rpc_kit::log::say!(
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
                let to_bundle = to.bundle.clone();
                migrations
                    .observe(pb::MigrateThing::Daemon, machine, own, to)
                    .await?;
                migrations
                    .auto_migrate(pb::MigrateThing::Daemon, machine, &to_bundle)
                    .await;
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
                let target = tools::release_only(&own);
                migrations
                    .observe_tool(thing, machine, tools::from_installed(&own), target.clone())
                    .await?;
                migrations.auto_migrate(thing, machine, &target.bundle).await;
            } else {
                migrations.resolved(thing, machine).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use humata2_machines::Platform;
    use sha2::{Digest, Sha256};
    use tonic::Status;

    use super::*;
    use crate::items::{Approver, Carry, Progress};
    use crate::ports::{MigrateEdit, MigrateInboxPort};
    use crate::run::{RunError, hex};

    struct FakeInbox {
        requests: Mutex<HashMap<String, pb::InboxRequest>>,
    }

    impl FakeInbox {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(HashMap::new()),
            })
        }
    }

    #[tonic::async_trait]
    impl MigrateInboxPort for FakeInbox {
        fn kept_migrates(&self) -> Vec<pb::InboxRequest> {
            Vec::new()
        }
        fn migrating(&self, _approver: Approver) {}
        async fn post_migrate(&self, request: pb::InboxRequest) -> Result<(), Status> {
            let id = request.id.as_ref().map(|i| i.value.clone()).unwrap_or_default();
            self.requests.lock().unwrap().insert(id, request);
            Ok(())
        }
        async fn update_migrate(
            &self,
            id: &str,
            change: MigrateEdit,
        ) -> Result<Option<pb::InboxRequest>, Status> {
            let mut map = self.requests.lock().unwrap();
            let Some(req) = map.get_mut(id) else {
                return Ok(None);
            };
            change(req);
            Ok(Some(req.clone()))
        }
        async fn settle_migrate(&self, id: &str, _done: String) -> Result<bool, Status> {
            Ok(self.requests.lock().unwrap().remove(id).is_some())
        }
        async fn withdraw_migrate(
            &self,
            id: &str,
            _reason: pb::WithdrawReason,
        ) -> Result<bool, Status> {
            Ok(self.requests.lock().unwrap().remove(id).is_some())
        }
    }

    #[derive(Default)]
    struct CountingCarry {
        calls: AtomicUsize,
    }

    #[tonic::async_trait]
    impl Carry for CountingCarry {
        async fn migrate(
            &self,
            _thing: pb::MigrateThing,
            _machine: &pb::MachineId,
            _to: &pb::BuildInfo,
            _progress: &Progress,
        ) -> Result<(), RunError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn manifest(files: &[(&str, &[u8])]) -> Vec<u8> {
        let entries: serde_json::Map<String, serde_json::Value> = files
            .iter()
            .map(|(name, bytes)| {
                (
                    (*name).to_owned(),
                    serde_json::json!({"path": name, "sha256": hex(Sha256::digest(bytes).as_slice())}),
                )
            })
            .collect();
        let platform = Platform::host().unwrap().bundle_name();
        serde_json::to_vec(&serde_json::json!({"platforms": {platform: entries}})).unwrap()
    }

    fn write_release(bin: &Path, suffix: &str, files: &[(&str, &[u8])]) -> String {
        for (name, bytes) in files {
            let file_name = if suffix.is_empty() {
                (*name).to_owned()
            } else {
                format!(".{name}{suffix}")
            };
            std::fs::write(bin.join(file_name), bytes).unwrap();
        }
        let bundle = manifest(files);
        let manifest_name = if suffix.is_empty() {
            "bundle.json".to_owned()
        } else {
            format!(".bundle.json{suffix}")
        };
        std::fs::write(bin.join(manifest_name), &bundle).unwrap();
        hex(Sha256::digest(&bundle).as_slice())
    }

    #[tokio::test]
    async fn local_show_auto_migrates_staged_next_and_respects_failed_breaker() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let state = dir.path().join("state");
        std::fs::create_dir_all(&bin).unwrap();
        let old_files: [(&str, &[u8]); 4] = [
            ("humatad2", b"daemon v1"),
            ("humata2_holder", b"holder v1"),
            ("humata", b"cli v1"),
            ("stack", b"stack v1"),
        ];
        let new_files: [(&str, &[u8]); 4] = [
            ("humatad2", b"daemon v2"),
            ("humata2_holder", b"holder v2"),
            ("humata", b"cli v2"),
            ("stack", b"stack v2"),
        ];
        write_release(&bin, "", &old_files);
        let to_bundle = write_release(&bin, ".next", &new_files);

        let release = Release::read(&bin, &bin.join("humatad2"), "2.0.4", 3);
        let carry = Arc::new(CountingCarry::default());
        let migrations = Migrations::new_with_state(FakeInbox::new(), carry.clone(), &state);
        let local = Local::new(migrations.clone(), release.clone(), "ma_host", bin.clone());

        // First look sees .next and auto-migrates!
        local.check().await;
        assert_eq!(carry.calls.load(Ordering::SeqCst), 1, "staged .next should auto-migrate");

        // Simulate rollback -> trips failed.json for (ma_host, to_bundle).
        migrations
            .failed(
                "ma_host",
                &to_bundle,
                pb::Error {
                    message: "rolled back".to_owned(),
                    ..Default::default()
                },
            )
            .await;

        // A second look at the same staged .next must NOT retry-loop!
        local.check().await;
        assert_eq!(
            carry.calls.load(Ordering::SeqCst),
            1,
            "tripped failed.json must block local auto-migrate retry loop"
        );
    }
}
