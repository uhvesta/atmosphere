//! Upgrading this machine's daemon in place (UP-1, D281 f, g).
//!
//! The app (or the host over the link) has put a release's files in `bin`
//! as `.<name>.next` and `.bundle.json.next`. `Runner::upgrade` checks
//! them against the bundle id it was asked for, records the upgrade
//! (`record.rs`), keeps the old daemon as `humatad2.previous`, renames the
//! staged files over the installed ones (the bundle last), answers, and
//! then has the daemon start itself again from the new file: same process
//! id, and holders and sessions untouched (PER-5). The new daemon settles
//! the record when it is ready.

use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use humata2_access::Refusal;
use humata2_daemon_ports::Caller;
use humata2_daemon_runtime::fault;
use humata2_grpc::humata::v1 as pb;
use humata2_machines::InstalledFile;
use humata2_rpc_kit::helpers as status;
use sha2::{Digest, Sha256};
use tokio::sync::Notify;
use tonic::{Code, Request, Response, Status};

use super::build::{self, Release};
use super::holders::Holders;
use super::items::{Carry, Migrations, Progress};
use super::record::{self, EndKind, Record, RecordError, UpgradeFolder};
use super::rollback;
use super::tools;
use crate::ports;

/// The installed bundle manifest, and the staged one beside it.
const BUNDLE: &str = "bundle.json";
const STAGED_BUNDLE: &str = ".bundle.json.next";

/// How much of a staged file is read at a time while it is hashed.
const CHUNK_BYTES: usize = 64 << 10;

/// Why an upgrade or a Migrate couldn't be carried out. Each message says
/// what failed and what to do; the cause is its source.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(
        "the staged file {} is missing or can't be read, so nothing was changed. Click Migrate again so the app stages this release again; don't copy files into ~/.humata2/bin by hand",
        .path.display()
    )]
    NotStaged {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the staged {} is release {staged}, not the {wanted} that was asked for, so nothing was changed. Click Migrate again so the app stages the right release",
        .path.display()
    )]
    WrongRelease {
        path: PathBuf,
        staged: String,
        wanted: String,
    },

    #[error(
        "the staged {} isn't a Humata bundle manifest, so nothing was changed. Click Migrate again so the app stages it again",
        .path.display()
    )]
    BundleInvalid {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "the staged {} names no humatad2, humata2_holder, humata, stack and humata2_fusefs for this platform, so nothing was changed. This release can't be installed here; update the Humata app to one that ships this platform",
        .path.display()
    )]
    Incomplete { path: PathBuf },

    #[error(
        "the staged file {} isn't the one the staged bundle.json names for it, so nothing was changed. Click Migrate again so the app stages it again; don't replace it by hand",
        .path.display()
    )]
    FileDiffers { path: PathBuf },

    #[error(
        "the installed {names} on this machine aren't the ones release {release} names, and no verified copy of them is staged, so nothing was changed. Open Humata on this Mac so it stages its release, then click Migrate again; for another machine, Migrate stages it over SSH first"
    )]
    ToolsNotStaged {
        names: String,
        release: String,
        #[source]
        source: Option<std::io::Error>,
    },

    #[error(
        "upgrade {upgrade_id} was started on this machine and hasn't finished, so another can't start. Restart humatad2 on this machine so it can finish or undo that one, then migrate again"
    )]
    UnderWay { upgrade_id: String },

    #[error(transparent)]
    Record(#[from] RecordError),

    #[error(
        "couldn't keep the installed file as {}, so nothing was swapped. Check that your user can write in that folder, then migrate again",
        .path.display()
    )]
    Link {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't put the staged {} in place of {}, so this release is only partly installed. Restart humatad2 on this machine to finish or undo the upgrade, and if that fails install Humata again; don't start another Migrate first",
        .from.display(),
        .to.display()
    )]
    Swap {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("the upgrade's background task didn't finish; nothing was reported as swapped, so check ~/.humata2/bin before migrating again")]
    Task {
        #[source]
        source: tokio::task::JoinError,
    },

    #[error(
        "{machine} isn't the machine this daemon runs on, so it can't migrate it from here yet. Migrate it from its own link once that is built, or install Humata on it again"
    )]
    NotHere { machine: String },

    /// Migrating another machine over SSH and its link (`remote`).
    #[error(transparent)]
    Remote(#[from] super::remote::RemoteError),
}

impl RunError {
    /// The status a refused or failed call ends with. What the person can
    /// fix by staging again is a failed precondition.
    pub fn status(&self) -> Status {
        let message = status::chain(self);
        match self {
            Self::NotStaged { .. }
            | Self::WrongRelease { .. }
            | Self::BundleInvalid { .. }
            | Self::Incomplete { .. }
            | Self::FileDiffers { .. }
            | Self::ToolsNotStaged { .. }
            | Self::UnderWay { .. }
            | Self::NotHere { .. }
            | Self::Remote(_) => status::failure(
                Code::FailedPrecondition,
                pb::Error {
                    message,
                    fix: String::new(),
                    retry: None,
                    kind: None,
                },
            ),
            Self::Record(_) | Self::Link { .. } | Self::Swap { .. } | Self::Task { .. } => {
                status::internal(message, "")
            }
        }
    }
}

/// How the serve loop is asked to leave and start the new file: the stop
/// signal's other way in, beside a stop's relaunch.
#[derive(Clone)]
pub struct Restarting {
    leave: Arc<Notify>,
    /// The file to start, set once an upgrade has swapped it in.
    path: Arc<Mutex<Option<PathBuf>>>,
}

impl Restarting {
    pub fn new(leave: Arc<Notify>) -> Self {
        Self {
            leave,
            path: Arc::default(),
        }
    }

    /// Asks the daemon to start `path` once it has answered what is in
    /// flight.
    fn request(&self, path: PathBuf) {
        *self.path.lock().unwrap_or_else(PoisonError::into_inner) = Some(path);
        self.leave.notify_one();
    }

    /// The file to start, if an upgrade asked for one.
    pub fn take(&self) -> Option<PathBuf> {
        self.path.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

/// Runs upgrades of this machine's daemon.
#[derive(Clone)]
pub struct Runner {
    machine: String,
    bin: PathBuf,
    folder: UpgradeFolder,
    release: Release,
    restarting: Restarting,
    /// Held while an upgrade checks and swaps, so two never interleave.
    one_at_a_time: Arc<tokio::sync::Mutex<()>>,
    /// This machine's holders, which an Upgrade of the holders starts (D378).
    holders: Arc<OnceLock<Holders>>,
}

impl Runner {
    pub fn new(
        machine: &str,
        bin: PathBuf,
        folder: UpgradeFolder,
        release: Release,
        restarting: Restarting,
    ) -> Self {
        Self {
            machine: machine.to_owned(),
            bin,
            folder,
            release,
            restarting,
            one_at_a_time: Arc::default(),
            holders: Arc::default(),
        }
    }

    /// An Upgrade of the holders is carried out by the holders in `slot`,
    /// once it is set.
    pub fn with_holders(mut self, slot: Arc<OnceLock<Holders>>) -> Self {
        self.holders = slot;
        self
    }

    /// `DaemonService.Upgrade`: the user's, or the host's link's.
    pub async fn upgrade(
        &self,
        request: Request<pb::UpgradeRequest>,
    ) -> Result<Response<pb::UpgradeResponse>, Status> {
        admit(&request)?;
        let wanted = request.into_inner();
        if wanted.thing() == pb::MigrateThing::Holders {
            // D378: no files change; the agents on old holders restart onto
            // the current one as each is idle. Asking again changes nothing.
            return match self.holders.get() {
                Some(holders) => {
                    holders.begin();
                    Ok(Response::new(pb::UpgradeResponse {
                        from: Some(self.release.info()),
                    }))
                }
                None => Err(RunError::NotHere {
                    machine: self.machine.clone(),
                }
                .status()),
            };
        }
        if wanted.to_bundle.is_empty() {
            return Err(status::missing("to_bundle"));
        }
        let upgrade_id = if wanted.upgrade_id.is_empty() {
            new_id()
        } else {
            wanted.upgrade_id
        };
        let from = self
            .begin(&wanted.to_bundle, &upgrade_id, None)
            .await
            .map_err(|error| error.status())?;
        Ok(Response::new(pb::UpgradeResponse { from: Some(from) }))
    }

    /// Swaps in the staged release `to_bundle` and asks the daemon to start
    /// it; returns the build this daemon was. Already at that release:
    /// nothing is done.
    async fn begin(
        &self,
        to_bundle: &str,
        upgrade_id: &str,
        progress: Option<&Progress>,
    ) -> Result<pb::BuildInfo, RunError> {
        let _one = self.one_at_a_time.lock().await;
        // The same upgrade asked for again is the one under way: it
        // answers as the first did, and a daemon that never got to restart
        // is asked to.
        if let Some(under_way) = self.folder.read()?
            && under_way.upgrade_id == upgrade_id
        {
            if self.release.bundle() != under_way.to_bundle {
                self.restarting.request(self.bin.join(InstalledFile::Humatad.file_name()));
            }
            return Ok(pb::BuildInfo {
                version: under_way.from_version,
                bundle: under_way.from_bundle,
                ..Default::default()
            });
        }
        let from = self.release.info();
        if self.release.bundle() == to_bundle {
            // Already at this release: only a CLI or holder that differs
            // from it is put in place, with no record, swap or restart.
            let me = self.clone();
            tokio::task::spawn_blocking(move || me.put_tools())
                .await
                .map_err(|source| RunError::Task { source })??;
            return Ok(from);
        }
        let (me, to) = (self.clone(), to_bundle.to_owned());
        let staged = tokio::task::spawn_blocking(move || me.check(&to))
            .await
            .map_err(|source| RunError::Task { source })??;
        // D379: the files check out; the upgrade is recorded (the new daemon
        // copies the store as it starts), then the files are swapped in.
        if let Some(progress) = progress {
            progress.at(1).await;
        }
        let (me, to, id) = (self.clone(), to_bundle.to_owned(), upgrade_id.to_owned());
        tokio::task::spawn_blocking(move || me.record(&to, &id))
            .await
            .map_err(|source| RunError::Task { source })??;
        if let Some(progress) = progress {
            progress.at(2).await;
        }
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.put_files(&staged))
            .await
            .map_err(|source| RunError::Task { source })??;
        if let Some(progress) = progress {
            progress.at(3).await;
        }
        self.restarting.request(self.bin.join(InstalledFile::Humatad.file_name()));
        Ok(from)
    }

    /// The CLI, holder and stack that aren't the ones this release names (D371):
    /// each is renamed over the installed file from its verified staged copy.
    /// Running holders and CLIs keep their inode, so this is safe while they
    /// run. Nothing differing changes nothing. Blocks.
    fn put_tools(&self) -> Result<(), RunError> {
        let bundle = self.release.bundle();
        let differing = tools::differing(&self.release.info().files, &tools::installed(&self.bin));
        if differing.is_empty() {
            return Ok(());
        }
        let names = differing
            .iter()
            .map(|file| file.file_name())
            .collect::<Vec<_>>()
            .join(" and ");
        let not_staged = |source: std::io::Error| RunError::ToolsNotStaged {
            names: names.clone(),
            release: build::short(bundle).to_owned(),
            source: Some(source),
        };
        let manifest = self.bin.join(STAGED_BUNDLE);
        let bytes = std::fs::read(&manifest).map_err(not_staged)?;
        let staged_id = hex(Sha256::digest(&bytes).as_slice());
        if !staged_id.eq_ignore_ascii_case(bundle) {
            return Err(RunError::WrongRelease {
                path: manifest,
                staged: build::short(&staged_id).to_owned(),
                wanted: build::short(bundle).to_owned(),
            });
        }
        // Every file is checked before the first is renamed.
        let mut checked = Vec::with_capacity(differing.len());
        for file in differing {
            let next = self.bin.join(format!(".{}.next", file.file_name()));
            let got = hash_file(&next).map_err(not_staged)?;
            let named = build::names_file(&bytes, file, &got).map_err(|source| RunError::BundleInvalid {
                path: manifest.clone(),
                source,
            })?;
            if !named {
                return Err(RunError::FileDiffers { path: next });
            }
            checked.push((next, self.bin.join(file.file_name())));
        }
        for (next, to) in &checked {
            put_in_place(next, to)?;
        }
        humata2_durable::sync_dir(&self.bin).map_err(|source| RunError::Swap {
            from: self.bin.join(STAGED_BUNDLE),
            to: self.bin.join(BUNDLE),
            source,
        })?;
        self.release.rescan_installed(&self.bin);
        Ok(())
    }

    /// The blocking part that changes nothing: checks the staged files.
    fn check(&self, to_bundle: &str) -> Result<Vec<Staged>, RunError> {
        if let Some(under_way) = self.folder.read()? {
            return Err(RunError::UnderWay {
                upgrade_id: under_way.upgrade_id,
            });
        }
        let staged = verify(&self.bin, to_bundle)?;
        fault::point("upgrade.files.staged");
        Ok(staged)
    }

    /// The blocking part that keeps the installed files and records the
    /// upgrade, so a cut from here on is undone at the next start.
    fn record(&self, to_bundle: &str, upgrade_id: &str) -> Result<(), RunError> {
        // Every installed file is kept before the record says an upgrade is
        // under way, so a record always has this upgrade's files to go back
        // to, and files from an older upgrade are never taken for them.
        link_previous(&self.bin)?;
        fault::point("upgrade.previous.linked");
        let from = self.release.info();
        self.folder.write_started(&Record {
            upgrade_id: upgrade_id.to_owned(),
            from_version: from.version,
            from_bundle: from.bundle,
            to_bundle: to_bundle.to_owned(),
            started_unix_ms: record::now_unix_ms(),
            boots: 0,
        })?;
        Ok(())
    }

    /// All three blocking parts in a row.
    #[cfg(test)]
    fn swap(&self, to_bundle: &str, upgrade_id: &str) -> Result<(), RunError> {
        let staged = self.check(to_bundle)?;
        self.record(to_bundle, upgrade_id)?;
        self.put_files(&staged)
    }

    /// The blocking part that swaps the checked files in.
    fn put_files(&self, staged: &[Staged]) -> Result<(), RunError> {
        // The daemon last, then the bundle that names it: a cut between
        // leaves a daemon its bundle doesn't name, which reports no release.
        for file in staged.iter().rev() {
            put_in_place(&file.next, &self.bin.join(file.name))?;
        }
        fault::point("upgrade.binary.swapped");
        put_in_place(&self.bin.join(STAGED_BUNDLE), &self.bin.join(BUNDLE))?;
        humata2_durable::sync_dir(&self.bin).map_err(|source| RunError::Swap {
            from: self.bin.join(STAGED_BUNDLE),
            to: self.bin.join(BUNDLE),
            source,
        })
    }

    /// This daemon is ready: the upgrade it came from is settled and its
    /// store copy dropped, a Migrate item waiting for this release is
    /// finished, and one whose upgrade was undone is shown as failed so a
    /// click retries.
    pub async fn ready(&self, migrations: &Migrations) {
        let (folder, release, bin) = (self.folder.clone(), self.release.clone(), self.bin.clone());
        let settled = tokio::task::spawn_blocking(move || -> Result<Option<(Record, EndKind)>, rollback::RollbackError> {
            if let Some(record) = folder.settle(&release)? {
                rollback::drop_copy(&folder, &record.upgrade_id)?;
                rollback::remove_previous(&bin)?;
            }
            Ok(folder.unreported_end()?)
        })
        .await;
        let undone = match settled {
            Ok(Ok(undone)) => undone,
            Ok(Err(error)) => {
                humata2_rpc_kit::log::say!("humatad2: warning: {}", status::chain(&error));
                None
            }
            Err(error) => {
                humata2_rpc_kit::log::say!("humatad2: warning: settling the upgrade record failed: {error}");
                None
            }
        };
        migrations.arrived(&self.machine, self.release.bundle()).await;
        if let Some((record, kind)) = undone {
            migrations
                .failed(&self.machine, &record.to_bundle, undone_problem(&record, kind))
                .await;
            let folder = self.folder.clone();
            if let Ok(Err(error)) = tokio::task::spawn_blocking(move || folder.mark_reported()).await {
                humata2_rpc_kit::log::say!("humatad2: warning: {}", status::chain(&error));
            }
        }
    }
}

/// What a Migrate item shows when its upgrade was undone.
fn undone_problem(record: &Record, kind: EndKind) -> pb::Error {
    let message = match kind {
        EndKind::RolledBack => format!(
            "The new release didn't finish starting on this machine, so Humata went back to {} and put the state back as it was.",
            record.from_version
        ),
        EndKind::Abandoned => format!(
            "The upgrade was cut before the new release was in place, so Humata kept {}.",
            record.from_version
        ),
    };
    pb::Error {
        message,
        fix: "Click Migrate to try again. If it fails the same way, look in the machine's humatad2 log for why the new release didn't start.".to_owned(),
        retry: None,
        kind: None,
    }
}

#[tonic::async_trait]
impl Carry for Runner {
    async fn migrate(
        &self,
        _thing: pb::MigrateThing,
        machine: &pb::MachineId,
        to: &pb::BuildInfo,
        progress: &Progress,
    ) -> Result<(), RunError> {
        if machine.value != self.machine {
            return Err(RunError::NotHere {
                machine: machine.value.clone(),
            });
        }
        self.begin(&to.bundle, &new_id(), Some(progress)).await.map(|_| ())
    }
}

pub(crate) fn new_id() -> String {
    format!("up_{}", uuid::Uuid::new_v4().simple())
}

/// Who may upgrade this machine's daemon: the user always; a person typing
/// `humata` on the machine with no credential, outside every session, as
/// for a stop (D263); never an agent or a script.
fn admit<T>(request: &Request<T>) -> Result<(), Status> {
    match ports::caller_if_any(request)? {
        Some(Caller::User(_)) => Ok(()),
        None if ports::outside_every_session(request) => Ok(()),
        None => Err(Status::unauthenticated("authentication required")),
        Some(Caller::Peer) => Err(ports::refused(&Refusal::PeerCannotCall)),
        Some(Caller::Agent(_) | Caller::Script(_)) => {
            Err(ports::refused(&Refusal::UserOnly { capability: None }))
        }
    }
}

/// A staged file that passed its check.
struct Staged {
    /// Its installed name.
    name: &'static str,
    /// Where it waits.
    next: PathBuf,
}

/// Checks the staged release against the bundle id `to_bundle`: the staged
/// `bundle.json` must hash to it and name all five files for this
/// platform, and the staged files must be what it names for this platform,
/// or one other platform's whole entry (staging over SSH picks the files by
/// the machine's `uname`). In `InstalledFile::ALL` order.
fn verify(bin: &Path, to_bundle: &str) -> Result<Vec<Staged>, RunError> {
    let manifest = bin.join(STAGED_BUNDLE);
    let bytes = std::fs::read(&manifest).map_err(|source| RunError::NotStaged {
        path: manifest.clone(),
        source,
    })?;
    let staged_id = hex(Sha256::digest(&bytes).as_slice());
    if !staged_id.eq_ignore_ascii_case(to_bundle) {
        return Err(RunError::WrongRelease {
            path: manifest,
            staged: build::short(&staged_id).to_owned(),
            wanted: build::short(to_bundle).to_owned(),
        });
    }
    let named = build::host_files(&bytes)
        .map_err(|source| RunError::BundleInvalid {
            path: manifest.clone(),
            source,
        })?
        .ok_or_else(|| RunError::Incomplete {
            path: manifest.clone(),
        })?;
    let staged = InstalledFile::ALL
        .into_iter()
        .map(|file| -> Result<(InstalledFile, PathBuf, String), RunError> {
            if !named.iter().any(|(candidate, _)| *candidate == file) {
                return Err(RunError::Incomplete {
                    path: manifest.clone(),
                });
            }
            let next = bin.join(format!(".{}.next", file.file_name()));
            let got = hash_file(&next).map_err(|source| RunError::NotStaged {
                path: next.clone(),
                source,
            })?;
            Ok((file, next, got))
        })
        .collect::<Result<Vec<_>, RunError>>()?;
    let differs = staged
        .iter()
        .find(|(file, _, got)| named.iter().any(|(candidate, wanted)| candidate == file && wanted != got));
    if let Some((_, next, _)) = differs {
        let hashes: Vec<(InstalledFile, String)> = staged.iter().map(|(file, _, got)| (*file, got.clone())).collect();
        let whole = build::names_whole_entry(&bytes, &hashes).map_err(|source| RunError::BundleInvalid {
            path: manifest.clone(),
            source,
        })?;
        if !whole {
            return Err(RunError::FileDiffers { path: next.clone() });
        }
    }
    Ok(staged
        .into_iter()
        .map(|(file, next, _)| Staged {
            name: file.file_name(),
            next,
        })
        .collect())
}

/// The release staged in `bin` as a build: its bundle id and this
/// platform's files. `None` when nothing is staged. The staged files are
/// checked unless `verified` is already that bundle's id.
pub(crate) fn staged(bin: &Path, verified: Option<&str>) -> Result<Option<pb::BuildInfo>, RunError> {
    let manifest = bin.join(STAGED_BUNDLE);
    let bytes = match std::fs::read(&manifest) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(RunError::NotStaged { path: manifest, source }),
    };
    let id = hex(Sha256::digest(&bytes).as_slice());
    if verified != Some(id.as_str()) {
        verify(bin, &id)?;
    }
    let named = build::host_files(&bytes)
        .map_err(|source| RunError::BundleInvalid {
            path: manifest.clone(),
            source,
        })?
        .ok_or(RunError::Incomplete { path: manifest })?;
    Ok(Some(pb::BuildInfo {
        bundle: id,
        files: named
            .into_iter()
            .map(|(file, sha256)| pb::BuildFile {
                name: file.file_name().to_owned(),
                sha256,
            })
            .collect(),
        ..Default::default()
    }))
}

/// Keeps each file now installed (the five tools and `bundle.json`) as a
/// second name for its file, so the old release can come back whole. A file
/// not installed yet is nothing to keep.
fn link_previous(bin: &Path) -> Result<(), RunError> {
    for name in rollback::kept_files() {
        let installed = bin.join(name);
        let previous = bin.join(rollback::kept_name(name));
        let failed = |source| RunError::Link {
            path: previous.clone(),
            source,
        };
        match std::fs::remove_file(&previous) {
            Ok(()) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(failed(source)),
        }
        match std::fs::hard_link(&installed, &previous) {
            Ok(()) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(failed(source)),
        }
    }
    Ok(())
}

/// Renames `from` over `to`. A holder or CLI running from `to` keeps its
/// own copy of the file (it is open), so this is safe while they run.
fn put_in_place(from: &Path, to: &Path) -> Result<(), RunError> {
    humata2_durable::rename(from, to).map_err(|source| RunError::Swap {
        from: from.to_owned(),
        to: to.to_owned(),
        source,
    })
}

/// The lowercase hex sha256 of the file at `path`, read in chunks.
pub(crate) fn hash_file(path: &Path) -> Result<String, std::io::Error> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0; CHUNK_BYTES];
    loop {
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => hasher.update(&chunk[..read]),
            Err(source) if source.kind() == ErrorKind::Interrupted => {}
            Err(source) => return Err(source),
        }
    }
    Ok(hex(hasher.finalize().as_slice()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use humata2_machines::Platform;

    use super::*;

    /// The old daemon, kept as a second name for its file.
    const PREVIOUS: &str = "humatad2.previous";

    /// A `bin` folder with a daemon, holder and CLI installed (and the
    /// bundle naming them) and the release `bytes` staged beside them.
    struct Bin {
        dir: tempfile::TempDir,
    }

    const OLD: [(&str, &[u8]); 5] = [
        ("humatad2", b"old daemon"),
        ("humata2_holder", b"old holder"),
        ("humata", b"old cli"),
        ("stack", b"old stack"),
        ("humata2_fusefs", b"old fusefs"),
    ];
    const NEW: [(&str, &[u8]); 5] = [
        ("humatad2", b"new daemon"),
        ("humata2_holder", b"new holder"),
        ("humata", b"new cli"),
        ("stack", b"new stack"),
        ("humata2_fusefs", b"new fusefs"),
    ];

    fn sha(bytes: &[u8]) -> String {
        hex(Sha256::digest(bytes).as_slice())
    }

    /// The bundle manifest naming `files` for this platform.
    fn manifest(files: &[(&str, &[u8])]) -> Vec<u8> {
        let entries: serde_json::Map<String, serde_json::Value> = files
            .iter()
            .map(|(name, bytes)| {
                (
                    (*name).to_owned(),
                    serde_json::json!({"path": name, "sha256": sha(bytes)}),
                )
            })
            .collect();
        let platform = Platform::host().expect("a platform Humata ships").bundle_name();
        serde_json::to_vec(&serde_json::json!({"platforms": {platform: entries}}))
            .expect("a manifest serializes")
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).expect("a file is written");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("made runnable");
    }

    impl Bin {
        /// `OLD` installed.
        fn installed() -> Self {
            let dir = tempfile::tempdir().expect("a temp folder");
            for (name, bytes) in OLD {
                write(&dir.path().join(name), bytes);
            }
            write(&dir.path().join("bundle.json"), &manifest(&OLD));
            Self { dir }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        /// `NEW` staged; returns its bundle id.
        fn stage_new(&self) -> String {
            for (name, bytes) in NEW {
                write(&self.path().join(format!(".{name}.next")), bytes);
            }
            let bundle = manifest(&NEW);
            write(&self.path().join(STAGED_BUNDLE), &bundle);
            sha(&bundle)
        }

        fn read(&self, name: &str) -> Vec<u8> {
            std::fs::read(self.path().join(name)).unwrap_or_default()
        }

        fn runner(&self) -> Runner {
            let exe = self.path().join("humatad2");
            let release = Release::read(self.path(), &exe, "2.0.4", 3);
            let restarting = Restarting::new(Arc::new(Notify::new()));
            Runner::new(
                "ma_here",
                self.path().to_owned(),
                UpgradeFolder::new(&self.path().join("state")),
                release,
                restarting,
            )
        }

        /// Everything installed is what it was.
        fn untouched(&self) -> bool {
            OLD.iter().all(|(name, bytes)| self.read(name) == *bytes)
                && self.read("bundle.json") == manifest(&OLD)
                && !self.path().join(PREVIOUS).exists()
                && !self.path().join("state/upgrade/this.json").exists()
        }
    }

    #[test]
    fn a_staged_release_is_swapped_in_with_the_old_daemon_kept() {
        let bin = Bin::installed();
        let id = bin.stage_new();
        let runner = bin.runner();

        runner.swap(&id, "up_1").expect("the swap succeeds");

        for (name, bytes) in NEW {
            assert_eq!(bin.read(name), bytes, "{name}");
            assert!(!bin.path().join(format!(".{name}.next")).exists(), "{name} is no longer staged");
        }
        assert_eq!(bin.read("bundle.json"), manifest(&NEW));
        assert!(!bin.path().join(STAGED_BUNDLE).exists());
        assert_eq!(bin.read(PREVIOUS), b"old daemon");
        assert_eq!(bin.read("humata2_holder.previous"), b"old holder");
        assert_eq!(bin.read("humata.previous"), b"old cli");
        assert_eq!(bin.read("stack.previous"), b"old stack");
        assert_eq!(bin.read("bundle.json.previous"), manifest(&OLD), "the old release can come back whole");
        let record = runner.folder.read().expect("the record reads").expect("the upgrade is recorded");
        assert_eq!((record.upgrade_id.as_str(), record.to_bundle.as_str()), ("up_1", id.as_str()));
        assert_eq!(record.boots, 0);
    }

    #[test]
    fn a_release_that_is_not_staged_changes_nothing() {
        let bin = Bin::installed();
        let wanted = sha(&manifest(&NEW));

        let refused = bin.runner().swap(&wanted, "up_1");

        assert!(matches!(refused, Err(RunError::NotStaged { .. })), "{refused:?}");
        assert!(bin.untouched());
    }

    #[test]
    fn a_staged_file_that_isnt_the_one_the_bundle_names_changes_nothing() {
        let bin = Bin::installed();
        let id = bin.stage_new();
        write(&bin.path().join(".humata.next"), b"somebody else's cli");

        let refused = bin.runner().swap(&id, "up_1");

        assert!(matches!(refused, Err(RunError::FileDiffers { .. })), "{refused:?}");
        assert!(bin.untouched());
        assert!(bin.path().join(".humatad2.next").exists(), "staged files are left for a retry");
    }

    #[test]
    fn a_staged_bundle_of_another_release_than_asked_for_changes_nothing() {
        let bin = Bin::installed();
        bin.stage_new();

        let refused = bin.runner().swap(&sha(b"another release"), "up_1");

        assert!(matches!(refused, Err(RunError::WrongRelease { .. })), "{refused:?}");
        assert!(bin.untouched());
    }

    #[test]
    fn a_staged_bundle_that_is_not_a_bundle_changes_nothing() {
        let bin = Bin::installed();
        bin.stage_new();
        let junk = b"not a manifest";
        write(&bin.path().join(STAGED_BUNDLE), junk);

        let refused = bin.runner().swap(&sha(junk), "up_1");

        assert!(matches!(refused, Err(RunError::BundleInvalid { .. })), "{refused:?}");
        assert!(bin.untouched());
    }

    #[test]
    fn a_bundle_that_lacks_one_of_the_files_changes_nothing() {
        let bin = Bin::installed();
        bin.stage_new();
        let bundle = manifest(&NEW[..2]);
        write(&bin.path().join(STAGED_BUNDLE), &bundle);

        let refused = bin.runner().swap(&sha(&bundle), "up_1");

        assert!(matches!(refused, Err(RunError::Incomplete { .. })), "{refused:?}");
        assert!(bin.untouched());
    }

    /// A bundle naming `here` for this platform and `there` for another.
    fn two_platforms(here: &[(&str, &[u8])], there: &[(&str, &[u8])]) -> Vec<u8> {
        let entries = |files: &[(&str, &[u8])]| -> serde_json::Map<String, serde_json::Value> {
            files
                .iter()
                .map(|(name, bytes)| {
                    (
                        (*name).to_owned(),
                        serde_json::json!({"path": name, "sha256": sha(bytes)}),
                    )
                })
                .collect()
        };
        let this = Platform::host().expect("a platform Humata ships");
        let other = Platform::SHIPPED
            .into_iter()
            .find(|platform| *platform != this)
            .expect("Humata ships more than one platform");
        serde_json::to_vec(&serde_json::json!({"platforms": {
            this.bundle_name(): entries(here),
            other.bundle_name(): entries(there),
        }}))
        .expect("a manifest serializes")
    }

    fn stage(bin: &Bin, bundle: &[u8]) -> String {
        for (name, bytes) in NEW {
            write(&bin.path().join(format!(".{name}.next")), bytes);
        }
        write(&bin.path().join(STAGED_BUNDLE), bundle);
        sha(bundle)
    }

    #[test]
    fn a_release_staged_for_the_platform_the_machine_named_is_swapped_in() {
        let bin = Bin::installed();
        let id = stage(&bin, &two_platforms(&OLD, &NEW));

        bin.runner().swap(&id, "up_1").expect("the swap succeeds");

        for (name, bytes) in NEW {
            assert_eq!(bin.read(name), bytes, "{name}");
        }
    }

    #[test]
    fn staged_files_that_are_no_one_platform_s_entry_change_nothing() {
        let bin = Bin::installed();
        let there = [NEW[0], NEW[1], ("humata", &b"another cli"[..]), NEW[3], NEW[4]];
        let id = stage(&bin, &two_platforms(&OLD, &there));

        let refused = bin.runner().swap(&id, "up_1");

        assert!(matches!(refused, Err(RunError::FileDiffers { .. })), "{refused:?}");
        assert!(bin.untouched());
    }

    #[test]
    fn an_upgrade_that_has_not_finished_blocks_another() {
        let bin = Bin::installed();
        let id = bin.stage_new();
        let runner = bin.runner();
        runner.swap(&id, "up_1").expect("the first swap succeeds");
        let id = bin.stage_new();

        let refused = runner.swap(&id, "up_2");

        assert!(matches!(&refused, Err(RunError::UnderWay { upgrade_id }) if upgrade_id == "up_1"), "{refused:?}");
    }

    #[tokio::test]
    async fn the_same_upgrade_asked_for_again_answers_as_the_first_did() {
        let bin = Bin::installed();
        let id = bin.stage_new();
        let runner = bin.runner();
        let first = runner.begin(&id, "up_1", None).await.expect("the upgrade begins");
        runner.restarting.take();

        let again = runner.begin(&id, "up_1", None).await.expect("asked again, it answers");

        assert_eq!((again.version, again.bundle), (first.version, first.bundle));
        assert_eq!(bin.read("humatad2"), b"new daemon");
        assert!(runner.restarting.take().is_some(), "a daemon that is still the old one is asked to restart");
    }

    #[tokio::test]
    async fn another_upgrade_while_one_is_under_way_is_refused_even_with_nothing_staged() {
        let bin = Bin::installed();
        let id = bin.stage_new();
        let runner = bin.runner();
        runner.begin(&id, "up_1", None).await.expect("the upgrade begins");
        runner.restarting.take();

        let refused = runner.begin(&sha(b"another release"), "up_2", None).await;

        assert!(matches!(&refused, Err(RunError::UnderWay { upgrade_id }) if upgrade_id == "up_1"), "{refused:?}");
        assert!(runner.restarting.take().is_none());
    }

    #[test]
    fn a_first_install_has_no_old_daemon_to_keep() {
        let bin = Bin::installed();
        let id = bin.stage_new();
        std::fs::remove_file(bin.path().join("humatad2")).expect("the daemon is removed");

        bin.runner().swap(&id, "up_1").expect("the swap succeeds");

        assert_eq!(bin.read("humatad2"), b"new daemon");
        assert!(!bin.path().join(PREVIOUS).exists());
    }

    #[tokio::test]
    async fn a_swap_asks_the_daemon_to_start_the_new_file() {
        let bin = Bin::installed();
        let id = bin.stage_new();
        let runner = bin.runner();

        let from = runner.begin(&id, "up_1", None).await.expect("the upgrade begins");

        assert_eq!(from.version, "2.0.4");
        assert_eq!(runner.restarting.take(), Some(bin.path().join("humatad2")));
        assert_eq!(runner.restarting.take(), None);
    }

    #[tokio::test]
    async fn an_upgrade_to_the_release_already_running_does_nothing() {
        let bin = Bin::installed();
        let runner = bin.runner();
        let running = sha(&manifest(&OLD));
        assert_eq!(runner.release.bundle(), running, "the installed bundle names this daemon");

        let from = runner.begin(&running, "up_1", None).await.expect("nothing to do is not a failure");

        assert_eq!(from.bundle, running);
        assert!(runner.restarting.take().is_none());
        assert!(bin.untouched());
    }

    #[tokio::test]
    async fn a_machine_other_than_this_one_is_not_migrated_here() {
        let bin = Bin::installed();
        let id = bin.stage_new();
        let runner = bin.runner();
        let to = pb::BuildInfo {
            bundle: id,
            ..Default::default()
        };

        let elsewhere = pb::MachineId {
            value: "gpu".to_owned(),
        };
        let refused = runner.migrate(pb::MigrateThing::Daemon, &elsewhere, &to, &Progress::silent()).await;

        assert!(matches!(refused, Err(RunError::NotHere { .. })), "{refused:?}");
        assert!(bin.untouched());
        assert!(runner.restarting.take().is_none());
    }

    #[tokio::test]
    async fn migrating_this_machine_swaps_and_asks_for_the_restart() {
        let bin = Bin::installed();
        let id = bin.stage_new();
        let runner = bin.runner();
        let to = pb::BuildInfo {
            bundle: id,
            ..Default::default()
        };

        runner
            .migrate(
                pb::MigrateThing::Daemon,
                &pb::MachineId {
                    value: "ma_here".to_owned(),
                },
                &to,
                &Progress::silent(),
            )
            .await
            .expect("this machine is migrated");

        assert_eq!(bin.read("humatad2"), b"new daemon");
        assert!(runner.restarting.take().is_some());
    }

    /// `OLD` installed with its CLI replaced by `somebody else's cli`.
    fn with_a_foreign_cli() -> Bin {
        let bin = Bin::installed();
        write(&bin.path().join("humata"), b"somebody else's cli");
        bin
    }

    /// The installed release's own files staged beside what is installed.
    fn stage_old(bin: &Bin) -> String {
        for (name, bytes) in OLD {
            write(&bin.path().join(format!(".{name}.next")), bytes);
        }
        let bundle = manifest(&OLD);
        write(&bin.path().join(STAGED_BUNDLE), &bundle);
        sha(&bundle)
    }

    #[tokio::test]
    async fn a_cli_that_differs_is_put_back_from_the_staged_release_and_nothing_else_changes() {
        let bin = with_a_foreign_cli();
        let id = stage_old(&bin);
        let runner = bin.runner();

        runner.begin(&id, "up_1", None).await.expect("the CLI is put back");

        assert_eq!(bin.read("humata"), b"old cli");
        assert_eq!(bin.read("humatad2"), b"old daemon");
        assert!(!bin.path().join(".humata.next").exists(), "the staged CLI was renamed in");
        assert!(bin.path().join(".humatad2.next").exists(), "files that matched stay staged");
        assert!(!bin.path().join(PREVIOUS).exists(), "no old daemon is set aside");
        assert!(runner.folder.read().expect("the record reads").is_none(), "no upgrade is recorded");
        assert!(runner.restarting.take().is_none(), "the daemon isn't asked to start again");
        let installed = runner.release.info().installed;
        assert!(
            installed.iter().any(|file| file.name == "humata" && file.sha256 == sha(b"old cli")),
            "the release reports the CLI it has now: {installed:?}"
        );
    }

    #[tokio::test]
    async fn a_cli_that_differs_with_nothing_staged_is_refused_with_nothing_changed() {
        let bin = with_a_foreign_cli();
        let runner = bin.runner();
        let id = runner.release.bundle().to_owned();

        let refused = runner.begin(&id, "up_1", None).await;

        assert!(matches!(refused, Err(RunError::ToolsNotStaged { .. })), "{refused:?}");
        assert_eq!(bin.read("humata"), b"somebody else's cli");
    }

    #[tokio::test]
    async fn a_staged_cli_the_bundle_does_not_name_is_refused_with_nothing_changed() {
        let bin = with_a_foreign_cli();
        let id = stage_old(&bin);
        write(&bin.path().join(".humata.next"), b"another foreign cli");

        let refused = bin.runner().begin(&id, "up_1", None).await;

        assert!(matches!(refused, Err(RunError::FileDiffers { .. })), "{refused:?}");
        assert_eq!(bin.read("humata"), b"somebody else's cli");
    }

    #[tokio::test]
    async fn tools_that_match_the_release_need_no_staged_copy_and_change_nothing() {
        let bin = Bin::installed();
        let runner = bin.runner();
        let id = runner.release.bundle().to_owned();

        runner.begin(&id, "up_1", None).await.expect("nothing to do is not a failure");

        assert!(bin.untouched());
    }

    #[test]
    fn what_staging_again_fixes_is_a_failed_precondition_and_the_rest_is_internal() {
        let missing = RunError::NotStaged {
            path: PathBuf::from("/bin/.humata.next"),
            source: std::io::Error::from(ErrorKind::NotFound),
        };
        let cut = RunError::Swap {
            from: PathBuf::from("/a"),
            to: PathBuf::from("/b"),
            source: std::io::Error::from(ErrorKind::PermissionDenied),
        };

        assert_eq!(missing.status().code(), Code::FailedPrecondition);
        assert_eq!(cut.status().code(), Code::Internal);
    }
}
