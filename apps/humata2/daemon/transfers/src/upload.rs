//! Uploads (FM-4): the bytes come from the caller, in order, and are
//! confirmed at each sync.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use humata2_store::fault::point;
use sha2::{Digest, Sha256};

use crate::end::{Entry, Kind, Sink};
use crate::engine::{
    Halt, Landing, Mode, OnExisting, Problem, Snapshot, Transfers, UploadRequest, digest_of, hex,
    staging_path,
};
use crate::error::{EndError, TransferError};
use crate::record::{PlanFile, Phase, Record, Top, UploadInfo, Want};

/// An upload being streamed in. Dropped without [`Upload::finish`], what
/// arrived is synced and the transfer is Paused with
/// `Problem::Interrupted`, to resume or cancel.
pub struct Upload {
    owner: Arc<Transfers>,
    id: String,
    sink: Option<Box<dyn Sink>>,
    hasher: Sha256,
    offset: u64,
    since_sync: u64,
    settled: bool,
    info: UploadInfo,
    destination: PathBuf,
    now_unix_ms: i64,
}

fn end_error(id: &str) -> impl FnOnce(EndError) -> TransferError {
    let id = id.to_owned();
    move |source| TransferError::End { id, source }
}

impl Transfers {
    /// Starts an upload, or with `resume` continues one from its
    /// confirmed offset.
    pub fn begin_upload(self: &Arc<Self>, request: UploadRequest) -> Result<Upload, TransferError> {
        let name = request.name.clone();
        if !crate::engine::plain_name(&name) {
            return Err(TransferError::BadName { name });
        }
        match request.resume.clone() {
            Some(id) => self.resume_upload(&id, request),
            None => self.new_upload(request),
        }
    }

    fn new_upload(self: &Arc<Self>, request: UploadRequest) -> Result<Upload, TransferError> {
        let id = request.id.clone();
        if self.get(&id).is_some() {
            return Err(TransferError::Duplicate { id });
        }
        let refuse = |problem| TransferError::Refused {
            id: id.clone(),
            problem,
        };
        let end = &self.config.destination;
        let folder = request.destination.path.clone();
        if !end.exists(&folder).map_err(end_error(&id))? {
            return Err(refuse(Problem::Io {
                doing: "find the destination folder".to_owned(),
                path: folder.display().to_string(),
                message: "it doesn't exist; create it or pick another folder".to_owned(),
            }));
        }
        let target = folder.join(&request.name);
        if request.on_existing == OnExisting::Fail && end.exists(&target).map_err(end_error(&id))? {
            return Err(refuse(Problem::Exists {
                path: target.display().to_string(),
            }));
        }
        let needed = request.size.saturating_add(self.config.free_floor_bytes);
        let free = end.free_bytes(&folder).map_err(end_error(&id))?;
        if needed > free {
            return Err(refuse(Problem::DiskFull {
                folder: request.destination.shown.clone(),
                needed,
                free,
            }));
        }
        let info = UploadInfo {
            name: request.name.clone(),
            size: request.size,
            sha256: request.sha256.to_lowercase(),
        };
        let rec = Record {
            id: id.clone(),
            tops: vec![Top {
                name: request.name.clone(),
                source: None,
                shown: request.name.clone(),
                landed: Landing::AsAsked,
            }],
            destination: folder.clone(),
            destination_shown: request.destination.shown.clone(),
            mode: Mode::Copy,
            on_existing: request.on_existing,
            caller_data: request.caller_data.clone(),
            now_unix_ms: request.now_unix_ms,
            upload: Some(info.clone()),
            phase: Phase::Running,
            problem: None,
            planned: true,
            direct: false,
            files: vec![PlanFile {
                top: 0,
                entry: Entry {
                    rel: String::new(),
                    kind: Kind::File,
                    size: request.size,
                    mode: 0o644,
                    modified_unix_ms: request.now_unix_ms,
                    link_target: None,
                },
            }],
            file_index: 0,
            offset: 0,
            confirmed: 0,
            done_bytes: 0,
            total_bytes: request.size,
            done_files: 0,
            total_files: 1,
            version: 1,
        };
        let (_, fresh) = self.insert(rec, true)?;
        if !fresh {
            return Err(TransferError::Duplicate { id });
        }
        // The journal comes first, so a crash from here on is recovered.
        let staging = staging_path(&folder, &id);
        let opened = end
            .make_folder(&staging, 0o700)
            .and_then(|()| end.sync_folder(&folder))
            .and_then(|()| end.open_write(&staging.join(&request.name), 0));
        let sink = match opened {
            Ok(sink) => sink,
            Err(error) => {
                let _ = end.remove_tree(&staging);
                let problem = Problem::Io {
                    doing: "start the upload".to_owned(),
                    path: error.path().display().to_string(),
                    message: error.to_string(),
                };
                self.settle(&id, |r| {
                    r.phase = Phase::Failed;
                    r.problem = Some(problem);
                });
                return Err(TransferError::End { id, source: error });
            }
        };
        Ok(Upload {
            owner: self.clone(),
            id,
            sink: Some(sink),
            hasher: Sha256::new(),
            offset: 0,
            since_sync: 0,
            settled: false,
            info,
            destination: folder,
            now_unix_ms: request.now_unix_ms,
        })
    }

    fn resume_upload(self: &Arc<Self>, id: &str, request: UploadRequest) -> Result<Upload, TransferError> {
        let (info, folder, journaled, now) = {
            let shared = self.lock();
            let slot = shared
                .slots
                .get(id)
                .ok_or_else(|| TransferError::Unknown { id: id.to_owned() })?;
            let Some(info) = slot.rec.upload.clone() else {
                return Err(TransferError::NotResumable { id: id.to_owned() });
            };
            if slot.running {
                return Err(TransferError::AlreadyRunning { id: id.to_owned() });
            }
            if !matches!(slot.rec.phase, Phase::Paused | Phase::Failed) {
                return Err(TransferError::NotResumable { id: id.to_owned() });
            }
            (
                info,
                slot.rec.destination.clone(),
                slot.rec.offset,
                slot.rec.now_unix_ms,
            )
        };
        let end = &self.config.destination;
        let staging = staging_path(&folder, id);
        let staged = staging.join(&info.name);
        end.make_folder(&staging, 0o700).map_err(end_error(id))?;
        // What really reached disk decides, and feeds the hash.
        let mut hasher = Sha256::new();
        let mut have = 0u64;
        if let Ok(reader) = end.open_read(&staged, 0) {
            let mut reader = reader.take(journaled);
            let mut buf = vec![0u8; 1 << 20];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        hasher.update(&buf[..n]);
                        have += n as u64;
                    }
                }
            }
        }
        let sink = end.open_write(&staged, have).map_err(end_error(id))?;
        let mut claimed = false;
        self.update(id, true, |slot| {
            if !slot.running {
                claimed = true;
                slot.running = true;
                slot.want = Want::Run;
                slot.rec.phase = Phase::Running;
                slot.rec.problem = None;
                slot.rec.offset = have;
                slot.rec.done_bytes = have;
            }
        })?;
        if !claimed {
            return Err(TransferError::AlreadyRunning { id: id.to_owned() });
        }
        let _ = request.sha256;
        Ok(Upload {
            owner: self.clone(),
            id: id.to_owned(),
            sink: Some(sink),
            hasher,
            offset: have,
            since_sync: 0,
            settled: false,
            info,
            destination: folder,
            now_unix_ms: now,
        })
    }
}

impl Upload {
    /// Where the next bytes must start.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Writes the next bytes, which must start at [`Upload::offset`].
    pub fn write(&mut self, offset: u64, bytes: &[u8]) -> Result<(), TransferError> {
        if offset != self.offset {
            return Err(TransferError::WrongOffset {
                id: self.id.clone(),
                expected: self.offset,
                got: offset,
            });
        }
        if self.offset + bytes.len() as u64 > self.info.size {
            return Err(TransferError::Overrun {
                id: self.id.clone(),
                size: self.info.size,
            });
        }
        let (phase, want) = {
            let shared = self.owner.lock();
            let slot = shared
                .slots
                .get(&self.id)
                .ok_or_else(|| TransferError::Unknown { id: self.id.clone() })?;
            (slot.rec.phase, slot.want)
        };
        if phase == Phase::Cancelled || phase == Phase::Cancelling || want == Want::Cancel {
            self.settled = true;
            return Err(TransferError::Cancelled { id: self.id.clone() });
        }
        if want == Want::Pause {
            return Err(TransferError::Interrupted { id: self.id.clone() });
        }
        let id = self.id.clone();
        let Some(sink) = self.sink.as_mut() else {
            return Err(TransferError::Interrupted { id });
        };
        sink.write(bytes).map_err(end_error(&id))?;
        self.hasher.update(bytes);
        self.offset += bytes.len() as u64;
        self.since_sync += bytes.len() as u64;
        point("transfer.upload.written");
        let offset = self.offset;
        let journal = self.since_sync >= self.owner.config.sync_every_bytes;
        if journal {
            sink.sync().map_err(end_error(&id))?;
            point("transfer.upload.synced");
            self.since_sync = 0;
        }
        self.owner.update(&id, journal, |slot| {
            slot.rec.offset = offset;
            slot.rec.done_bytes = offset;
        })?;
        Ok(())
    }

    /// Checks the size and SHA-256 and moves the file into place.
    pub fn finish(mut self) -> Result<Snapshot, TransferError> {
        let id = self.id.clone();
        if self.offset != self.info.size {
            return Err(TransferError::Incomplete {
                id,
                offset: self.offset,
                size: self.info.size,
            });
        }
        if let Some(mut sink) = self.sink.take() {
            sink.sync().map_err(end_error(&id))?;
        }
        self.owner.update(&id, false, |s| s.rec.phase = Phase::Verifying)?;
        let end = &self.owner.config.destination;
        let staging = staging_path(&self.destination, &id);
        let staged = staging.join(&self.info.name);
        let computed = digest_of(self.hasher.clone());
        let actual = end.hash(&staged, self.info.size).map_err(end_error(&id))?;
        if hex(&computed) != self.info.sha256 || actual != computed {
            let found = if actual != computed { actual } else { computed };
            let _ = end.remove_tree(&staging);
            let problem = Problem::ChecksumMismatch {
                path: self.info.name.clone(),
                expected: self.info.sha256.clone(),
                actual: hex(&found),
            };
            self.settled = true;
            let kept = problem.clone();
            self.owner.settle(&id, |r| {
                r.phase = Phase::Failed;
                r.problem = Some(kept);
            });
            return Err(TransferError::Refused { id, problem });
        }
        end.finish_file(&staged, 0o644, self.now_unix_ms)
            .map_err(end_error(&id))?;
        point("transfer.file.verified");
        match self.owner.commit(&id) {
            Ok(()) => {
                self.settled = true;
                self.owner
                    .get(&id)
                    .ok_or(TransferError::Unknown { id })
            }
            Err(Halt::Fail(problem)) => {
                self.settled = true;
                let kept = problem.clone();
                self.owner.settle(&id, |r| {
                    r.phase = Phase::Failed;
                    r.problem = Some(kept);
                });
                Err(TransferError::Refused { id, problem })
            }
            Err(Halt::Pause) => Err(TransferError::Interrupted { id }),
            Err(Halt::Cancel) => {
                self.settled = true;
                self.owner.cancel_now(&id);
                Err(TransferError::Cancelled { id })
            }
        }
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        if let Some(mut sink) = self.sink.take() {
            let _ = sink.sync();
        }
        let offset = self.offset;
        let _ = self.owner.update(&self.id, true, |slot| {
            if matches!(slot.rec.phase, Phase::Running | Phase::Verifying) {
                slot.rec.phase = Phase::Paused;
                slot.rec.problem = Some(Problem::Interrupted);
                slot.rec.offset = offset;
                slot.rec.done_bytes = offset;
                slot.running = false;
                slot.want = Want::Run;
            }
        });
    }
}
