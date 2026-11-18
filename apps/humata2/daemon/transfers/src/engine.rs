//! Copies, moves and uploads (FM-6, FM-8, FM-14, PER-27): every step is
//! journaled, so a crash at any point finishes or undoes the transfer.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use humata2_store::fault::point;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::end::{End, Entry, Kind};
use crate::error::{EndError, TransferError};
use crate::record::{PlanFile, Phase, Record, Slot, Top, Want};

/// The most bytes moved in one piece (TR-11).
pub const CHUNK: usize = 1 << 20;

const KEEP_FINISHED: usize = 200;
const FILES_PER_JOURNAL: u32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Copy,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnExisting {
    Fail,
    /// The replaced one goes to the trash.
    Replace,
    /// `name-2`, `name-3`... before the extension (FC-14).
    KeepBoth,
}

/// Where one source ended up in the destination folder (PL-13).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Landing {
    /// Under its own name, or not placed yet.
    #[default]
    AsAsked,
    /// The name was taken, so it was kept beside it under this name (FC-14).
    SavedAs(String),
    /// A file with the same bytes was already there under that name, so
    /// nothing new was written. A folder counts only when every file in it
    /// matches.
    AlreadyThere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    Paused,
    Verifying,
    Done,
    Failed,
    Cancelled,
}

/// Why a transfer stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Problem {
    DiskFull {
        folder: String,
        needed: u64,
        free: u64,
    },
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    Exists {
        path: String,
    },
    SourceGone {
        path: String,
    },
    Unsupported {
        path: String,
    },
    /// An end's machine can't be reached (its link is down). The transfer
    /// is Failed in the engine, shown as waiting, and resumed when the
    /// link is back.
    Offline {
        machine: String,
    },
    /// An upload's stream ended; resume it.
    Interrupted,
    Io {
        doing: String,
        path: String,
        message: String,
    },
}

/// One path part a file can land under: not empty, `.` or `..`, and
/// without `/` or NUL.
pub(crate) fn plain_name(name: &str) -> bool {
    !(name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0'))
}

#[derive(Debug, Clone)]
pub struct Item {
    /// Absolute; the caller already checked it is allowed.
    pub path: PathBuf,
    pub shown: String,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub id: String,
    pub sources: Vec<Item>,
    /// A folder.
    pub destination: Item,
    pub mode: Mode,
    pub on_existing: OnExisting,
    pub caller_data: String,
    pub now_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub id: String,
    pub sources: Vec<String>,
    pub destination: String,
    pub mode: Mode,
    pub state: State,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub done_files: u32,
    pub total_files: u32,
    pub problem: Option<Problem>,
    pub started_unix_ms: i64,
    pub version: u64,
    pub caller_data: String,
    pub upload: bool,
    /// One per entry of `sources`: where it landed once the transfer is done.
    pub landings: Vec<Landing>,
}

pub type Listener = Arc<dyn Fn(&Snapshot) + Send + Sync>;

pub struct Config {
    /// A folder, such as `<state>/transfers`.
    pub journal: PathBuf,
    pub source: Arc<dyn End>,
    pub destination: Arc<dyn End>,
    /// LD-19: refuse a copy that would leave less than this free.
    pub free_floor_bytes: u64,
    /// Bytes written between syncs and journal writes (8 MiB is usual).
    pub sync_every_bytes: u64,
    pub listener: Listener,
}

#[derive(Debug, Clone)]
pub struct UploadRequest {
    pub id: String,
    pub resume: Option<String>,
    pub destination: Item,
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub on_existing: OnExisting,
    pub caller_data: String,
    pub now_unix_ms: i64,
}

pub(crate) struct Shared {
    pub slots: HashMap<String, Slot>,
}

pub struct Transfers {
    pub(crate) config: Config,
    shared: Mutex<Shared>,
    changed: Condvar,
}

/// Why a worker stopped before the end.
pub(crate) enum Halt {
    Pause,
    Cancel,
    Fail(Problem),
}

impl From<TransferError> for Halt {
    fn from(error: TransferError) -> Self {
        Halt::Fail(Problem::Io {
            doing: "keep the transfer's journal".to_owned(),
            path: String::new(),
            message: chain(&error),
        })
    }
}

fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut next = error.source();
    while let Some(cause) = next {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        next = cause.source();
    }
    text
}

fn io_problem(error: &EndError) -> Problem {
    if let EndError::Offline { machine } = error {
        return Problem::Offline {
            machine: machine.clone(),
        };
    }
    Problem::Io {
        doing: error.doing().to_owned(),
        path: error.path().display().to_string(),
        message: chain(error),
    }
}

/// An error at the source end.
fn src(error: EndError) -> Halt {
    Halt::Fail(match &error {
        EndError::NotFound { path, .. } => Problem::SourceGone {
            path: path.display().to_string(),
        },
        EndError::Unsupported { path, .. } => Problem::Unsupported {
            path: path.display().to_string(),
        },
        _ => io_problem(&error),
    })
}

/// An error at the destination end.
fn dst(error: EndError) -> Halt {
    Halt::Fail(match &error {
        EndError::Exists { path } => Problem::Exists {
            path: path.display().to_string(),
        },
        EndError::Unsupported { path, .. } => Problem::Unsupported {
            path: path.display().to_string(),
        },
        _ => io_problem(&error),
    })
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn digest_of(hasher: Sha256) -> [u8; 32] {
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    for (o, b) in out.iter_mut().zip(digest.iter()) {
        *o = *b;
    }
    out
}

pub(crate) fn staging_path(destination: &Path, id: &str) -> PathBuf {
    destination.join(format!(".humata-transfer-{id}"))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn with_rel(base: &Path, rel: &str) -> PathBuf {
    if rel.is_empty() {
        base.to_owned()
    } else {
        base.join(rel)
    }
}

fn numbered(name: &str, n: u32) -> String {
    let path = Path::new(name);
    match (path.file_stem(), path.extension()) {
        (Some(stem), Some(ext)) => {
            format!("{}-{n}.{}", stem.to_string_lossy(), ext.to_string_lossy())
        }
        _ => format!("{name}-{n}"),
    }
}

impl Transfers {
    pub(crate) fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn journal_path(&self, id: &str) -> PathBuf {
        self.config.journal.join(format!("{id}.json"))
    }

    pub(crate) fn write_journal(&self, rec: &Record) -> Result<(), TransferError> {
        let path = self.journal_path(&rec.id);
        let bytes = serde_json::to_vec(rec).map_err(|source| TransferError::JournalFormat {
            path: path.clone(),
            source,
        })?;
        humata2_store::write_atomic(&path, &bytes)
            .map_err(|source| TransferError::Journal { path, source })
    }

    /// Changes a transfer, journals it if asked, and tells the listener.
    pub(crate) fn update(
        &self,
        id: &str,
        journal: bool,
        change: impl FnOnce(&mut Slot),
    ) -> Result<Snapshot, TransferError> {
        let snapshot = {
            let mut shared = self.lock();
            let slot = shared
                .slots
                .get_mut(id)
                .ok_or_else(|| TransferError::Unknown { id: id.to_owned() })?;
            change(slot);
            slot.rec.version += 1;
            let written = if journal {
                self.write_journal(&slot.rec)
            } else {
                Ok(())
            };
            let snapshot = slot.rec.snapshot();
            self.changed.notify_all();
            written?;
            snapshot
        };
        (self.config.listener)(&snapshot);
        Ok(snapshot)
    }

    /// As `update`, journaled, but tells no listener: for a step that is
    /// only on the way to the state watchers should hear about.
    fn update_unheard(
        &self,
        id: &str,
        change: impl FnOnce(&mut Slot),
    ) -> Result<(), TransferError> {
        let mut shared = self.lock();
        let slot = shared
            .slots
            .get_mut(id)
            .ok_or_else(|| TransferError::Unknown { id: id.to_owned() })?;
        change(slot);
        slot.rec.version += 1;
        let written = self.write_journal(&slot.rec);
        self.changed.notify_all();
        written
    }

    /// A final or resting state: the worker is done with the transfer.
    pub(crate) fn settle(&self, id: &str, change: impl FnOnce(&mut Record)) {
        let _ = self.update(id, true, |slot| {
            change(&mut slot.rec);
            slot.running = false;
            slot.want = Want::Run;
        });
    }

    fn read<T>(&self, id: &str, f: impl FnOnce(&Record) -> T) -> Result<T, TransferError> {
        let shared = self.lock();
        shared
            .slots
            .get(id)
            .map(|slot| f(&slot.rec))
            .ok_or_else(|| TransferError::Unknown { id: id.to_owned() })
    }

    fn want(&self, id: &str) -> Result<Want, TransferError> {
        let shared = self.lock();
        shared
            .slots
            .get(id)
            .map(|slot| slot.want)
            .ok_or_else(|| TransferError::Unknown { id: id.to_owned() })
    }

    fn check_want(&self, id: &str) -> Result<(), Halt> {
        match self.want(id)? {
            Want::Run => Ok(()),
            Want::Pause => Err(Halt::Pause),
            Want::Cancel => Err(Halt::Cancel),
        }
    }

    /// Reads the journals and recovers what a crash cut short.
    pub fn open(config: Config) -> Result<Arc<Self>, TransferError> {
        let folder = config.journal.clone();
        // Every folder this makes, not only the journal folder, stays only
        // once its own parent is synced: a power cut takes an unsynced
        // `carry/` back, and with it the journals other parts keep beside
        // `carry/transfers` (a move's or a fork's), so nothing is recovered.
        let missing: Vec<PathBuf> = folder
            .ancestors()
            .take_while(|found| !found.as_os_str().is_empty() && std::fs::symlink_metadata(found).is_err())
            .map(Path::to_owned)
            .collect();
        humata2_durable::create_dir_all(&folder, 0o700).map_err(|source| {
            TransferError::JournalRead {
                path: folder.clone(),
                source,
            }
        })?;
        for made in &missing {
            if let Some(parent) = made.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                humata2_durable::sync_dir(parent).map_err(|source| TransferError::JournalRead {
                    path: made.clone(),
                    source,
                })?;
            }
        }
        let read_failed = |source| TransferError::JournalRead {
            path: folder.clone(),
            source,
        };
        let mut slots = HashMap::new();
        let mut to_run = Vec::new();
        for entry in std::fs::read_dir(&folder).map_err(read_failed)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !name.ends_with(".json") {
                continue;
            }
            let path = entry.path();
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let Ok(mut rec) = serde_json::from_slice::<Record>(&bytes) else {
                let _ = std::fs::rename(&path, path.with_extension("json.corrupt"));
                continue;
            };
            let mut run = false;
            match rec.phase {
                Phase::Running | Phase::Verifying => {
                    if let Some(info) = &rec.upload {
                        let staged = staging_path(&rec.destination, &rec.id).join(&info.name);
                        if let Ok(mut sink) = config.destination.open_write(&staged, rec.offset) {
                            let _ = sink.sync();
                        }
                        rec.phase = Phase::Paused;
                        rec.problem = Some(Problem::Interrupted);
                        rec.version += 1;
                        let _ = humata2_store::write_atomic(
                            &path,
                            &serde_json::to_vec(&rec).unwrap_or(bytes),
                        );
                    } else {
                        run = true;
                    }
                }
                Phase::Committing | Phase::Cancelling => run = true,
                _ => {}
            }
            if run {
                to_run.push(rec.id.clone());
            }
            slots.insert(
                rec.id.clone(),
                Slot {
                    rec,
                    want: Want::Run,
                    running: run,
                },
            );
        }
        let this = Arc::new(Self {
            config,
            shared: Mutex::new(Shared { slots }),
            changed: Condvar::new(),
        });
        this.prune();
        for id in to_run {
            this.spawn(id)?;
        }
        Ok(this)
    }

    /// Forgets the oldest finished transfers beyond the newest 200, except
    /// those that may still own staging on their destination (see
    /// [`expired`]). Looks at nothing on either end: those may be offline.
    fn prune(&self) {
        let mut shared = self.lock();
        let gone = expired(&shared.slots);
        if gone.is_empty() {
            return;
        }
        for id in gone {
            shared.slots.remove(&id);
            let _ = humata2_durable::remove_file(&self.journal_path(&id));
        }
        let _ = humata2_durable::sync_dir(&self.config.journal);
    }

    pub(crate) fn spawn(self: &Arc<Self>, id: String) -> Result<(), TransferError> {
        let this = self.clone();
        let name = format!("transfer-{id}");
        let worker_id = id.clone();
        let spawned = std::thread::Builder::new()
            .name(name)
            .spawn(move || this.run(&worker_id));
        if let Err(source) = spawned {
            self.settle(&id, |rec| rec.phase = Phase::Paused);
            return Err(TransferError::Spawn { id, source });
        }
        Ok(())
    }

    /// Starts a transfer; the same id again returns what exists.
    pub fn start(self: &Arc<Self>, request: Request) -> Result<Snapshot, TransferError> {
        self.start_as(request, None)
    }

    /// Starts a transfer whose one source lands under `name` in the
    /// destination, as `cp a b` makes a new file b (D769). The caller
    /// checked that `name` is one path part. With more than one source the
    /// name is refused, since they can't all take it.
    pub fn start_as(self: &Arc<Self>, request: Request, name: Option<String>) -> Result<Snapshot, TransferError> {
        if !valid_id(&request.id) {
            return Err(TransferError::BadId { id: request.id });
        }
        if request.sources.is_empty() {
            return Err(TransferError::NoSources);
        }
        let mut tops = Vec::new();
        for item in &request.sources {
            let Some(name) = item.path.file_name() else {
                return Err(TransferError::BadSource {
                    path: item.path.display().to_string(),
                });
            };
            tops.push(Top {
                name: name.to_string_lossy().into_owned(),
                source: Some(item.path.clone()),
                shown: item.shown.clone(),
                landed: Landing::AsAsked,
            });
        }
        if let Some(name) = name {
            if !plain_name(&name) {
                return Err(TransferError::BadName { name });
            }
            let count = tops.len();
            let [top] = tops.as_mut_slice() else {
                return Err(TransferError::NameNeedsOneSource { count });
            };
            top.name = name;
        }
        let rec = Record {
            id: request.id.clone(),
            tops,
            destination: request.destination.path,
            destination_shown: request.destination.shown,
            mode: request.mode,
            on_existing: request.on_existing,
            caller_data: request.caller_data,
            now_unix_ms: request.now_unix_ms,
            upload: None,
            phase: Phase::Running,
            problem: None,
            planned: false,
            direct: false,
            files: Vec::new(),
            file_index: 0,
            offset: 0,
            confirmed: 0,
            done_bytes: 0,
            total_bytes: 0,
            done_files: 0,
            total_files: 0,
            version: 1,
        };
        let (snapshot, fresh) = self.insert(rec, true)?;
        if fresh {
            self.prune();
            self.spawn(request.id)?;
        }
        Ok(snapshot)
    }

    /// Adds a transfer and journals it; false if the id was taken.
    pub(crate) fn insert(&self, rec: Record, running: bool) -> Result<(Snapshot, bool), TransferError> {
        let (snapshot, fresh) = {
            let mut shared = self.lock();
            if let Some(slot) = shared.slots.get(&rec.id) {
                return Ok((slot.rec.snapshot(), false));
            }
            self.write_journal(&rec)?;
            let snapshot = rec.snapshot();
            shared.slots.insert(
                rec.id.clone(),
                Slot {
                    rec,
                    want: Want::Run,
                    running,
                },
            );
            self.changed.notify_all();
            (snapshot, true)
        };
        point("transfer.journaled");
        (self.config.listener)(&snapshot);
        Ok((snapshot, fresh))
    }

    pub fn pause(&self, id: &str) -> Result<Snapshot, TransferError> {
        let mut shared = self.lock();
        let slot = shared
            .slots
            .get_mut(id)
            .ok_or_else(|| TransferError::Unknown { id: id.to_owned() })?;
        if slot.running && matches!(slot.rec.phase, Phase::Running | Phase::Verifying) {
            slot.want = Want::Pause;
            if slot.rec.upload.is_none() {
                shared = self
                    .changed
                    .wait_while(shared, |sh| sh.slots.get(id).is_some_and(|s| s.running))
                    .unwrap_or_else(|p| p.into_inner());
            }
        }
        shared
            .slots
            .get(id)
            .map(|s| s.rec.snapshot())
            .ok_or_else(|| TransferError::Unknown { id: id.to_owned() })
    }

    /// Continues a paused or failed transfer from its journaled offset.
    pub fn resume(self: &Arc<Self>, id: &str) -> Result<Snapshot, TransferError> {
        if self.read(id, |r| r.upload.is_some())? {
            return Err(TransferError::IsUpload { id: id.to_owned() });
        }
        let mut start = false;
        let snapshot = self.update(id, true, |slot| {
            if !slot.running && matches!(slot.rec.phase, Phase::Paused | Phase::Failed) {
                slot.rec.phase = Phase::Running;
                slot.rec.problem = None;
                slot.want = Want::Run;
                slot.running = true;
                start = true;
            }
        })?;
        if start {
            self.spawn(id.to_owned())?;
        }
        Ok(snapshot)
    }

    /// Removes what reached the destination; the sources are untouched.
    pub fn cancel(self: &Arc<Self>, id: &str) -> Result<Snapshot, TransferError> {
        loop {
            let mut shared = self.lock();
            let slot = shared
                .slots
                .get_mut(id)
                .ok_or_else(|| TransferError::Unknown { id: id.to_owned() })?;
            match slot.rec.phase {
                Phase::Done | Phase::Committing => {
                    return Err(TransferError::TooLate { id: id.to_owned() });
                }
                Phase::Cancelled => return Ok(slot.rec.snapshot()),
                _ => {}
            }
            slot.want = Want::Cancel;
            if slot.running && slot.rec.upload.is_none() {
                let _unused = self
                    .changed
                    .wait_while(shared, |sh| sh.slots.get(id).is_some_and(|s| s.running))
                    .unwrap_or_else(|p| p.into_inner());
                continue;
            }
            if !slot.running {
                slot.running = true;
            }
            drop(shared);
            self.cancel_now(id);
            return self
                .get(id)
                .ok_or_else(|| TransferError::Unknown { id: id.to_owned() });
        }
    }

    pub fn get(&self, id: &str) -> Option<Snapshot> {
        self.lock().slots.get(id).map(|s| s.rec.snapshot())
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Snapshot> {
        let shared = self.lock();
        let mut all: Vec<Snapshot> = shared.slots.values().map(|s| s.rec.snapshot()).collect();
        all.sort_by(|a, b| (b.started_unix_ms, &b.id).cmp(&(a.started_unix_ms, &a.id)));
        all
    }

    /// Blocks until Done, Failed, Cancelled or Paused.
    pub fn wait(&self, id: &str) -> Option<Snapshot> {
        let mut shared = self.lock();
        loop {
            let slot = shared.slots.get(id)?;
            if !slot.running
                && matches!(
                    slot.rec.phase,
                    Phase::Paused | Phase::Done | Phase::Failed | Phase::Cancelled
                )
            {
                return Some(slot.rec.snapshot());
            }
            shared = self.changed.wait(shared).unwrap_or_else(|p| p.into_inner());
        }
    }

    fn run(&self, id: &str) {
        let mut buf = vec![0u8; CHUNK];
        match self.work(id, &mut buf) {
            Ok(()) => {}
            Err(Halt::Pause) => self.settle(id, |r| {
                r.phase = Phase::Paused;
                r.problem = None;
            }),
            Err(Halt::Cancel) => self.cancel_now(id),
            Err(Halt::Fail(problem)) => self.settle(id, |r| {
                r.phase = Phase::Failed;
                r.problem = Some(problem);
            }),
        }
    }

    /// Removes the staging folder and ends in Cancelled.
    pub(crate) fn cancel_now(&self, id: &str) {
        let Ok(destination) = self.read(id, |r| r.destination.clone()) else { return };
        // Journaled so a crash finishes the cancel, but not announced: a
        // watcher hears of the cancel once, when what arrived is gone.
        let _ = self.update_unheard(id, |s| s.rec.phase = Phase::Cancelling);
        point("transfer.cancelling");
        let staging = staging_path(&destination, id);
        let end = &self.config.destination;
        match end.remove_tree(&staging) {
            Ok(()) => {
                let _ = end.sync_folder(&destination);
                self.settle(id, |r| {
                    r.phase = Phase::Cancelled;
                    r.problem = None;
                });
            }
            Err(error) => {
                let problem = io_problem(&error);
                self.settle(id, |r| {
                    r.phase = Phase::Failed;
                    r.problem = Some(problem);
                });
            }
        }
    }

    fn work(&self, id: &str, buf: &mut [u8]) -> Result<(), Halt> {
        let (phase, planned, direct) = self.read(id, |r| (r.phase, r.planned, r.direct))?;
        if phase == Phase::Cancelling {
            return Err(Halt::Cancel);
        }
        if phase == Phase::Verifying {
            self.update(id, false, |s| s.rec.phase = Phase::Running)?;
        }
        if phase != Phase::Committing {
            if !planned {
                self.plan(id)?;
            }
            self.check_want(id)?;
            let direct = direct || self.read(id, |r| r.direct)?;
            if !direct {
                self.copy_all(id, buf)?;
            }
        }
        self.commit(id)
    }

    fn plan(&self, id: &str) -> Result<(), Halt> {
        let rec = self.read(id, |r| r.clone())?;
        let source = &self.config.source;
        let destination = &self.config.destination;
        let folder = rec.destination.display().to_string();
        if !destination.exists(&rec.destination).map_err(dst)? {
            return Err(Halt::Fail(Problem::Io {
                doing: "find the destination folder".to_owned(),
                path: folder,
                message: "it doesn't exist; create it or pick another folder, then resume"
                    .to_owned(),
            }));
        }
        let mut seen = HashSet::new();
        for top in &rec.tops {
            let target = rec.destination.join(&top.name);
            let taken = !seen.insert(top.name.clone())
                || (rec.on_existing == OnExisting::Fail
                    && destination.exists(&target).map_err(dst)?);
            if taken {
                return Err(Halt::Fail(Problem::Exists {
                    path: target.display().to_string(),
                }));
            }
        }
        let direct = rec.mode == Mode::Move
            && rec.tops.iter().all(|t| {
                t.source
                    .as_deref()
                    .is_some_and(|p| source.same_disk(&**destination, p, &rec.destination))
            });
        if direct {
            self.update(id, true, |s| {
                s.rec.planned = true;
                s.rec.direct = true;
            })?;
            return Ok(());
        }
        let mut files = Vec::new();
        for (index, top) in rec.tops.iter().enumerate() {
            let Some(path) = &top.source else { continue };
            for entry in source.plan(path).map_err(src)? {
                files.push(PlanFile { top: index, entry });
            }
        }
        let total_bytes: u64 = files
            .iter()
            .filter(|f| f.entry.kind == Kind::File)
            .map(|f| f.entry.size)
            .sum();
        let total_files = files.iter().filter(|f| f.entry.kind == Kind::File).count();
        let needed = total_bytes.saturating_add(self.config.free_floor_bytes);
        let free = destination.free_bytes(&rec.destination).map_err(dst)?;
        if needed > free {
            return Err(Halt::Fail(Problem::DiskFull {
                folder: rec.destination_shown,
                needed,
                free,
            }));
        }
        self.update(id, true, |s| {
            s.rec.files = files;
            s.rec.total_bytes = total_bytes;
            s.rec.total_files = u32::try_from(total_files).unwrap_or(u32::MAX);
            s.rec.planned = true;
        })?;
        Ok(())
    }

    fn copy_all(&self, id: &str, buf: &mut [u8]) -> Result<(), Halt> {
        let (files, tops, destination, start, offset) = self.read(id, |r| {
            (
                r.files.clone(),
                r.tops.clone(),
                r.destination.clone(),
                r.file_index.min(r.files.len()),
                r.offset,
            )
        })?;
        let end = &self.config.destination;
        // Counters follow from the journaled position.
        let before: u64 = files[..start]
            .iter()
            .filter(|f| f.entry.kind == Kind::File)
            .map(|f| f.entry.size)
            .sum();
        let done_files = files[..start]
            .iter()
            .filter(|f| f.entry.kind == Kind::File)
            .count();
        self.update(id, false, |s| {
            s.rec.done_bytes = before + offset;
            s.rec.done_files = u32::try_from(done_files).unwrap_or(u32::MAX);
        })?;
        let staging = staging_path(&destination, id);
        if !end.exists(&staging).map_err(dst)? {
            end.make_folder(&staging, 0o700).map_err(dst)?;
            end.sync_folder(&destination).map_err(dst)?;
        }
        let mut unjournaled = 0u32;
        for (index, file) in files.iter().enumerate().skip(start) {
            self.check_want(id)?;
            let top = &tops[file.top];
            let Some(origin) = &top.source else { continue };
            let from = with_rel(origin, &file.entry.rel);
            let staged = with_rel(&staging.join(&top.name), &file.entry.rel);
            let entry = &file.entry;
            match entry.kind {
                Kind::Folder => end.make_folder(&staged, entry.mode).map_err(dst)?,
                Kind::Link => {
                    if let Some(target) = &entry.link_target {
                        end.make_link(&staged, target).map_err(dst)?;
                    }
                }
                Kind::File => self.copy_file(id, index, entry, &from, &staged, buf)?,
            }
            unjournaled += 1;
            let journal = unjournaled >= FILES_PER_JOURNAL;
            if journal {
                unjournaled = 0;
            }
            self.update(id, journal, |s| {
                s.rec.file_index = index + 1;
                s.rec.offset = 0;
                s.rec.confirmed = 0;
                if entry.kind == Kind::File {
                    s.rec.done_files += 1;
                }
            })?;
            if journal {
                point("transfer.progress.journaled");
            }
        }
        Ok(())
    }

    fn progress(
        &self,
        id: &str,
        index: usize,
        before: u64,
        offset: u64,
        confirmed: u64,
        journal: bool,
    ) -> Result<(), Halt> {
        self.update(id, journal, |s| {
            s.rec.file_index = index;
            s.rec.offset = offset;
            s.rec.confirmed = confirmed;
            s.rec.done_bytes = before + offset;
        })?;
        Ok(())
    }

    /// Whether the staged prefix equals the source's, with the source's
    /// prefix fed to `hasher`.
    fn prefix_matches(
        &self,
        from: &Path,
        staged: &Path,
        offset: u64,
        hasher: &mut Sha256,
        buf: &mut [u8],
    ) -> bool {
        let Ok(reader) = self.config.source.open_read(from, 0) else {
            return false;
        };
        let mut reader = reader.take(offset);
        let mut seen = 0u64;
        loop {
            match reader.read(buf) {
                Ok(0) => break,
                Ok(n) => {
                    hasher.update(&buf[..n]);
                    seen += n as u64;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
        }
        if seen != offset {
            return false;
        }
        match self.config.destination.hash(staged, offset) {
            Ok(found) => found == digest_of(hasher.clone()),
            Err(_) => false,
        }
    }

    fn copy_file(
        &self,
        id: &str,
        index: usize,
        entry: &Entry,
        from: &Path,
        staged: &Path,
        buf: &mut [u8],
    ) -> Result<(), Halt> {
        let source = &self.config.source;
        let end = &self.config.destination;
        let size = entry.size;
        let (mut offset, mut confirmed, current) =
            self.read(id, |r| (r.offset, r.confirmed, r.done_bytes))?;
        let before = current.saturating_sub(offset);
        if offset > size {
            offset = 0;
        }
        if confirmed > offset {
            confirmed = 0;
        }
        let mut retried = false;
        loop {
            let mut hasher = Sha256::new();
            if offset > 0 && !self.prefix_matches(from, staged, offset, &mut hasher, buf) {
                // The destination may hold only what it confirmed.
                hasher = Sha256::new();
                if confirmed > 0
                    && confirmed < offset
                    && self.prefix_matches(from, staged, confirmed, &mut hasher, buf)
                {
                    offset = confirmed;
                } else {
                    offset = 0;
                    hasher = Sha256::new();
                }
            }
            // Whatever prefix matched is on the destination's disk.
            confirmed = offset;
            self.progress(id, index, before, offset, confirmed, false)?;
            let mut sink = end.open_write(staged, offset).map_err(dst)?;
            let mut reader = source.open_read(from, offset).map_err(src)?.take(size - offset);
            let mut since_sync = 0u64;
            loop {
                match self.want(id)? {
                    Want::Run => {}
                    other => {
                        sink.sync().map_err(dst)?;
                        self.progress(id, index, before, offset, offset, false)?;
                        return Err(if other == Want::Pause {
                            Halt::Pause
                        } else {
                            Halt::Cancel
                        });
                    }
                }
                let n = match reader.read(buf) {
                    Ok(n) => n,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        if let Some(EndError::Offline { machine }) =
                            error.get_ref().and_then(|inner| inner.downcast_ref::<EndError>())
                        {
                            return Err(src(EndError::Offline {
                                machine: machine.clone(),
                            }));
                        }
                        return Err(src(EndError::Io {
                            doing: "read",
                            path: from.to_owned(),
                            source: error,
                        }));
                    }
                };
                if n == 0 {
                    break;
                }
                sink.write(&buf[..n]).map_err(dst)?;
                hasher.update(&buf[..n]);
                offset += n as u64;
                since_sync += n as u64;
                point("transfer.chunk.written");
                if since_sync >= self.config.sync_every_bytes {
                    sink.sync().map_err(dst)?;
                    point("transfer.chunk.synced");
                    confirmed = offset;
                    self.progress(id, index, before, offset, confirmed, true)?;
                    point("transfer.progress.journaled");
                    since_sync = 0;
                } else {
                    self.progress(id, index, before, offset, confirmed, false)?;
                }
            }
            if offset != size {
                return Err(Halt::Fail(Problem::SourceGone {
                    path: from.display().to_string(),
                }));
            }
            sink.sync().map_err(dst)?;
            drop(sink);
            self.update(id, false, |s| s.rec.phase = Phase::Verifying)?;
            let expected = digest_of(hasher);
            let actual = end.hash(staged, size).map_err(dst)?;
            if actual == expected {
                end.finish_file(staged, entry.mode, entry.modified_unix_ms)
                    .map_err(dst)?;
                self.update(id, false, |s| s.rec.phase = Phase::Running)?;
                point("transfer.file.verified");
                return Ok(());
            }
            if retried {
                return Err(Halt::Fail(Problem::ChecksumMismatch {
                    path: from.display().to_string(),
                    expected: hex(&expected),
                    actual: hex(&actual),
                }));
            }
            retried = true;
            end.remove_tree(staged).map_err(dst)?;
            offset = 0;
            confirmed = 0;
            self.update(id, false, |s| s.rec.phase = Phase::Running)?;
        }
    }

    /// Puts the staged names in place, trashes moved sources, and ends in
    /// Done. Each step can be repeated after a crash.
    pub(crate) fn commit(&self, id: &str) -> Result<(), Halt> {
        let phase = self.read(id, |r| r.phase)?;
        if phase != Phase::Committing {
            let mut refused = None;
            self.update(id, true, |s| {
                if s.want == Want::Run {
                    s.rec.phase = Phase::Committing;
                } else {
                    refused = Some(s.want);
                }
            })?;
            match refused {
                Some(Want::Pause) => return Err(Halt::Pause),
                Some(Want::Cancel) => return Err(Halt::Cancel),
                _ => {}
            }
        }
        point("transfer.committing");
        let rec = self.read(id, |r| r.clone())?;
        let end = &self.config.destination;
        let staging = staging_path(&rec.destination, id);
        if !rec.direct {
            self.finish_folders(&rec, &staging)?;
        }
        for (index, top) in rec.tops.iter().enumerate() {
            let from = match (&top.source, rec.direct) {
                (Some(path), true) => path.clone(),
                _ => staging.join(&top.name),
            };
            if !end.exists(&from).map_err(dst)? {
                continue;
            }
            let (target, landing) = self.choose_target(&rec, &top.name, &from)?;
            if landing != top.landed {
                // Journaled before the rename, so a crash after it still
                // remembers where the name landed.
                self.update(id, true, |s| {
                    if let Some(top) = s.rec.tops.get_mut(index) {
                        top.landed = landing.clone();
                    }
                })?;
            }
            if landing == Landing::AlreadyThere {
                // Nothing new is written. A one-disk move still takes the
                // source away, into the trash like any other moved source.
                if rec.direct {
                    end.trash(&from, "already at the destination", rec.now_unix_ms)
                        .map_err(dst)?;
                }
                continue;
            }
            end.rename(&from, &target).map_err(dst)?;
            // A top folder gets its mode once in place; a crash in between
            // leaves it writable by its owner.
            let folder = rec
                .files
                .iter()
                .find(|f| f.top == index && f.entry.rel.is_empty() && f.entry.kind == Kind::Folder);
            if let Some(folder) = folder {
                end.finish_file(&target, folder.entry.mode, folder.entry.modified_unix_ms)
                    .map_err(dst)?;
            }
            point("transfer.commit.renamed");
        }
        end.sync_folder(&rec.destination).map_err(dst)?;
        // A rename is on disk once both folders it touched are saved: the
        // one a name left as well as the one it came to.
        if rec.direct {
            for top in &rec.tops {
                let Some(parent) = top.source.as_deref().and_then(Path::parent) else {
                    continue;
                };
                if end.exists(parent).map_err(dst)? {
                    end.sync_folder(parent).map_err(dst)?;
                }
            }
        } else if end.exists(&staging).map_err(dst)? {
            end.sync_folder(&staging).map_err(dst)?;
        }
        if !rec.direct {
            end.remove_tree(&staging).map_err(dst)?;
            end.sync_folder(&rec.destination).map_err(dst)?;
        }
        if rec.mode == Mode::Move && !rec.direct {
            let source = &self.config.source;
            for top in &rec.tops {
                let Some(path) = &top.source else { continue };
                if source.exists(path).map_err(src)? {
                    let note = format!("moved to {}", rec.destination_shown);
                    source.trash(path, &note, rec.now_unix_ms).map_err(src)?;
                    point("transfer.move.trashed");
                }
            }
        }
        self.settle(id, |r| {
            r.phase = Phase::Done;
            r.problem = None;
            r.offset = 0;
            r.confirmed = 0;
            r.done_bytes = r.total_bytes;
            r.done_files = r.total_files;
        });
        Ok(())
    }

    /// Gives every staged folder below a top its source's exact mode and
    /// time, deepest first, so a read-only folder is set only once all it
    /// holds is written (VF-59). Repeating it after a crash is harmless.
    /// The tops are set as they are put in place.
    fn finish_folders(&self, rec: &Record, staging: &Path) -> Result<(), Halt> {
        let end = &self.config.destination;
        let mut staged = Vec::with_capacity(rec.tops.len());
        for top in &rec.tops {
            staged.push(end.exists(&staging.join(&top.name)).map_err(dst)?);
        }
        // The plan lists a folder before what it holds, so backwards is
        // deepest first.
        for file in rec.files.iter().rev() {
            let entry = &file.entry;
            if entry.kind != Kind::Folder || entry.rel.is_empty() {
                continue;
            }
            let (Some(top), Some(true)) = (rec.tops.get(file.top), staged.get(file.top).copied()) else {
                continue;
            };
            let path = with_rel(&staging.join(&top.name), &entry.rel);
            end.finish_file(&path, entry.mode, entry.modified_unix_ms)
                .map_err(dst)?;
        }
        Ok(())
    }

    /// Where `name` goes in the destination folder, and how it got there.
    /// `from` is the finished copy, read by KeepBoth to tell a name taken by
    /// other bytes (kept under `-2`) from one taken by the same bytes.
    fn choose_target(&self, rec: &Record, name: &str, from: &Path) -> Result<(PathBuf, Landing), Halt> {
        let end = &self.config.destination;
        let target = rec.destination.join(name);
        match rec.on_existing {
            OnExisting::Fail => {
                if end.exists(&target).map_err(dst)? {
                    return Err(Halt::Fail(Problem::Exists {
                        path: target.display().to_string(),
                    }));
                }
                Ok((target, Landing::AsAsked))
            }
            OnExisting::Replace => {
                if end.exists(&target).map_err(dst)? {
                    end.trash(&target, "replaced by a transfer", rec.now_unix_ms)
                        .map_err(dst)?;
                }
                Ok((target, Landing::AsAsked))
            }
            OnExisting::KeepBoth => {
                if !end.exists(&target).map_err(dst)? {
                    return Ok((target, Landing::AsAsked));
                }
                if self.same_bytes(from, &target)? {
                    return Ok((target, Landing::AlreadyThere));
                }
                for n in 2u32.. {
                    let kept = numbered(name, n);
                    let candidate = rec.destination.join(&kept);
                    if !end.exists(&candidate).map_err(dst)? {
                        return Ok((candidate, Landing::SavedAs(kept)));
                    }
                }
                Ok((target, Landing::AsAsked))
            }
        }
    }

    /// Whether the trees at `a` and `b` on the destination end hold the same
    /// names and bytes: same entries, sizes and link targets, and for every
    /// file the same SHA-256 (size first, so unequal files are never read). A tree
    /// with no file or link in it is never the same.
    fn same_bytes(&self, a: &Path, b: &Path) -> Result<bool, Halt> {
        let end = &self.config.destination;
        let mut left = end.plan(a).map_err(dst)?;
        let mut right = end.plan(b).map_err(dst)?;
        if left.len() != right.len() {
            return Ok(false);
        }
        left.sort_by(|x, y| x.rel.cmp(&y.rel));
        right.sort_by(|x, y| x.rel.cmp(&y.rel));
        // A folder with nothing but folders in it has no bytes to be the
        // same, so it is kept beside the other rather than called a copy.
        let mut compared = false;
        for (l, r) in left.iter().zip(&right) {
            if l.rel != r.rel || l.kind != r.kind || l.size != r.size || l.link_target != r.link_target {
                return Ok(false);
            }
            compared |= l.kind != Kind::Folder;
            if l.kind == Kind::File {
                let one = end.hash(&with_rel(a, &l.rel), l.size).map_err(dst)?;
                let two = end.hash(&with_rel(b, &r.rel), r.size).map_err(dst)?;
                if one != two {
                    return Ok(false);
                }
            }
        }
        Ok(compared)
    }
}
/// Whether a failed transfer may still own staging on its destination: one
/// that planned a copy or took an upload made a staging folder there, and a
/// failure (its machine offline, a name in the way) leaves it for the
/// resume or the cancel that removes it.
fn owns_staging(rec: &Record) -> bool {
    rec.upload.is_some() || (rec.planned && !rec.direct)
}

/// The finished transfers to forget: the oldest beyond the newest 200 of
/// those whose journal is the last trace of them. A failed transfer that
/// may still own staging is kept, so its staging is never orphaned; it goes
/// once it is cancelled, which removes the staging first.
fn expired(slots: &HashMap<String, Slot>) -> Vec<String> {
    let mut finished: Vec<(i64, String)> = slots
        .values()
        .filter(|s| {
            !s.running
                && match s.rec.phase {
                    Phase::Done | Phase::Cancelled => true,
                    Phase::Failed => !owns_staging(&s.rec),
                    _ => false,
                }
        })
        .map(|s| (s.rec.now_unix_ms, s.rec.id.clone()))
        .collect();
    if finished.len() <= KEEP_FINISHED {
        return Vec::new();
    }
    finished.sort_by(|a, b| b.cmp(a));
    finished.split_off(KEEP_FINISHED).into_iter().map(|(_, id)| id).collect()
}

#[cfg(test)]
mod prune_tests {
    use super::*;

    fn record(id: &str, phase: Phase, problem: Option<Problem>, planned: bool, at: i64) -> Record {
        Record {
            id: id.to_owned(),
            tops: Vec::new(),
            destination: PathBuf::from("/dest"),
            destination_shown: "dest".to_owned(),
            mode: Mode::Copy,
            on_existing: OnExisting::Fail,
            caller_data: String::new(),
            now_unix_ms: at,
            upload: None,
            phase,
            problem,
            planned,
            direct: false,
            files: Vec::new(),
            file_index: 0,
            offset: 0,
            confirmed: 0,
            done_bytes: 0,
            total_bytes: 0,
            done_files: 0,
            total_files: 0,
            version: 1,
        }
    }

    fn slots(records: Vec<Record>) -> HashMap<String, Slot> {
        records
            .into_iter()
            .map(|rec| {
                (
                    rec.id.clone(),
                    Slot {
                        rec,
                        want: Want::Run,
                        running: false,
                    },
                )
            })
            .collect()
    }

    fn offline() -> Option<Problem> {
        Some(Problem::Offline {
            machine: "devbox".to_owned(),
        })
    }

    /// Past the newest 200, the oldest finished transfers go.
    #[test]
    fn the_oldest_finished_transfers_past_the_newest_200_are_forgotten() {
        let all = (0..205)
            .map(|n| record(&format!("t{n}"), Phase::Done, None, true, n))
            .collect();
        let mut gone = expired(&slots(all));
        gone.sort();
        let mut want: Vec<String> = (0..5).map(|n| format!("t{n}")).collect();
        want.sort();
        assert_eq!(gone, want);
    }

    /// A failed transfer that waits for its machine still owns its staging
    /// there: it is kept however old it is.
    #[test]
    fn a_transfer_waiting_for_its_machine_is_never_forgotten() {
        let mut all: Vec<Record> = (0..205)
            .map(|n| record(&format!("t{}", n + 1), Phase::Done, None, true, i64::from(n) + 1))
            .collect();
        all.push(record("waiting", Phase::Failed, offline(), true, 0));
        let gone = expired(&slots(all));
        assert!(!gone.contains(&"waiting".to_owned()));
        assert_eq!(gone.len(), 5);
    }

    /// A failed transfer whose staging was made is kept for whatever it
    /// failed on; one that never planned made none, and goes like a done one.
    #[test]
    fn a_failed_transfer_is_forgotten_only_when_it_made_no_staging() {
        let mut all: Vec<Record> = (0..200)
            .map(|n| record(&format!("t{}", n + 2), Phase::Done, None, true, i64::from(n) + 2))
            .collect();
        all.push(record("staged", Phase::Failed, None, true, 0));
        all.push(record("unplanned", Phase::Failed, None, false, 1));
        let gone = expired(&slots(all));
        assert_eq!(gone, vec!["unplanned".to_owned()]);
    }
}

