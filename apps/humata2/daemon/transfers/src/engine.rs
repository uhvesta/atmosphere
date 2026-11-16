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
    /// "name 2", "name 3"... before the extension.
    KeepBoth,
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
    /// An upload's stream ended; resume it.
    Interrupted,
    Io {
        doing: String,
        path: String,
        message: String,
    },
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
            format!("{} {n}.{}", stem.to_string_lossy(), ext.to_string_lossy())
        }
        _ => format!("{name} {n}"),
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
        humata2_durable::create_dir_all(&folder, 0o700).map_err(|source| {
            TransferError::JournalRead {
                path: folder.clone(),
                source,
            }
        })?;
        if let Some(parent) = folder.parent() {
            let _ = humata2_durable::sync_dir(parent);
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

    /// Forgets the oldest finished transfers beyond the newest 200.
    fn prune(&self) {
        let mut shared = self.lock();
        let mut finished: Vec<(i64, String)> = shared
            .slots
            .values()
            .filter(|s| {
                !s.running
                    && matches!(s.rec.phase, Phase::Done | Phase::Failed | Phase::Cancelled)
            })
            .map(|s| (s.rec.now_unix_ms, s.rec.id.clone()))
            .collect();
        if finished.len() <= KEEP_FINISHED {
            return;
        }
        finished.sort_by(|a, b| b.cmp(a));
        for (_, id) in finished.split_off(KEEP_FINISHED) {
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
            });
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
        journal: bool,
    ) -> Result<(), Halt> {
        self.update(id, journal, |s| {
            s.rec.file_index = index;
            s.rec.offset = offset;
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
        let (mut offset, current) = self.read(id, |r| (r.offset, r.done_bytes))?;
        let before = current.saturating_sub(offset);
        if offset > size {
            offset = 0;
        }
        let mut retried = false;
        loop {
            let mut hasher = Sha256::new();
            if offset > 0 && !self.prefix_matches(from, staged, offset, &mut hasher, buf) {
                offset = 0;
                hasher = Sha256::new();
            }
            self.progress(id, index, before, offset, false)?;
            let mut sink = end.open_write(staged, offset).map_err(dst)?;
            let mut reader = source.open_read(from, offset).map_err(src)?.take(size - offset);
            let mut since_sync = 0u64;
            loop {
                match self.want(id)? {
                    Want::Run => {}
                    other => {
                        sink.sync().map_err(dst)?;
                        self.progress(id, index, before, offset, false)?;
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
                    self.progress(id, index, before, offset, true)?;
                    point("transfer.progress.journaled");
                    since_sync = 0;
                } else {
                    self.progress(id, index, before, offset, false)?;
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
        for top in &rec.tops {
            let from = match (&top.source, rec.direct) {
                (Some(path), true) => path.clone(),
                _ => staging.join(&top.name),
            };
            if !end.exists(&from).map_err(dst)? {
                continue;
            }
            let target = self.choose_target(&rec, &top.name)?;
            end.rename(&from, &target).map_err(dst)?;
            point("transfer.commit.renamed");
        }
        end.sync_folder(&rec.destination).map_err(dst)?;
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
            r.done_bytes = r.total_bytes;
            r.done_files = r.total_files;
        });
        Ok(())
    }

    fn choose_target(&self, rec: &Record, name: &str) -> Result<PathBuf, Halt> {
        let end = &self.config.destination;
        let target = rec.destination.join(name);
        match rec.on_existing {
            OnExisting::Fail => {
                if end.exists(&target).map_err(dst)? {
                    return Err(Halt::Fail(Problem::Exists {
                        path: target.display().to_string(),
                    }));
                }
                Ok(target)
            }
            OnExisting::Replace => {
                if end.exists(&target).map_err(dst)? {
                    end.trash(&target, "replaced by a transfer", rec.now_unix_ms)
                        .map_err(dst)?;
                }
                Ok(target)
            }
            OnExisting::KeepBoth => {
                if !end.exists(&target).map_err(dst)? {
                    return Ok(target);
                }
                for n in 2u32.. {
                    let candidate = rec.destination.join(numbered(name, n));
                    if !end.exists(&candidate).map_err(dst)? {
                        return Ok(candidate);
                    }
                }
                Ok(target)
            }
        }
    }
}

