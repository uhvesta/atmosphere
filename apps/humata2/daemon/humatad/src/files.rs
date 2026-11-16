//! FileService on this machine's own files: ReadFile and WriteFile in a
//! session's clones, in chunks of at most 1 MiB, each compressed with the
//! codec the two ends agree on (TR-11, D65). Channel folders, `shared/`,
//! main clones and the other methods aren't built yet.
//!
//! Every file is read, encoded, decoded, hashed and written on a blocking
//! thread, never on the runtime. Each finished transfer counts toward the
//! link's measured speed, which Auto reads to pick a codec.
//!
//! A WriteFile with an idempotency key writes once: a retry with the same
//! key for the same file gets the first upload's result, without the file
//! being written again.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use humata2_access::{Call, Entry, Place, RepoPath, Root};
use humata2_chunk::{CHUNK_MAX, ChunkError, Codec, Decoder, Encoder, Link, Sample};
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use pb::invalid_field::Problem;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status, Streaming};

use crate::compression::{self, Compression};
use crate::grant_target;
use crate::guard::Guard;
use crate::identity;
use crate::status;

pub(crate) type ReadFileStream = Pin<Box<dyn Stream<Item = Result<pb::FileChunk, Status>> + Send>>;

/// Chunks read ahead of a slow reader.
const READ_AHEAD: usize = 2;

/// Finished uploads kept for retries with the same idempotency key.
const REMEMBERED_MAX: usize = 1024;

pub(crate) struct Files {
    store: Arc<Store>,
    guard: Arc<Guard>,
    machine: String,
    compression: Arc<Compression>,
    /// The link the callers are on: the socket's or the peer link's.
    link: Link,
    uploads: Mutex<Uploads>,
}

/// Uploads by idempotency key: running, or finished with this result.
#[derive(Default)]
struct Uploads {
    by_key: HashMap<String, Option<pb::FileEntry>>,
    /// Finished keys, oldest first, to forget beyond `REMEMBERED_MAX`.
    finished: VecDeque<String>,
}

/// A keyed upload under way. Dropped before it finishes, its key is
/// forgotten so a retry writes the file.
struct Running<'a> {
    uploads: &'a Mutex<Uploads>,
    key: Option<String>,
}

impl Running<'_> {
    fn finish(mut self, entry: &pb::FileEntry) {
        let Some(key) = self.key.take() else { return };
        let mut uploads = lock(self.uploads);
        uploads.by_key.insert(key.clone(), Some(entry.clone()));
        uploads.finished.push_back(key);
        while uploads.finished.len() > REMEMBERED_MAX {
            if let Some(oldest) = uploads.finished.pop_front() {
                uploads.by_key.remove(&oldest);
            }
        }
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            lock(self.uploads).by_key.remove(&key);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Files {
    pub(crate) fn new(
        store: Arc<Store>,
        guard: Arc<Guard>,
        machine: String,
        compression: Arc<Compression>,
        link: Link,
    ) -> Self {
        Self {
            store,
            guard,
            machine,
            compression,
            link,
            uploads: Mutex::new(Uploads::default()),
        }
    }

    /// Claims `key` for an upload to `target`. `Ok(Err(entry))` is the
    /// result of the upload that already used it.
    fn claim(
        &self,
        key: Option<pb::IdempotencyKey>,
        target: &Target,
    ) -> Result<Result<Running<'_>, pb::FileEntry>, Status> {
        let key = key
            .map(|key| key.value)
            .filter(|key| !key.is_empty())
            .map(|key| format!("{}\0{}", target.key(), key));
        let mut uploads = lock(&self.uploads);
        if let Some(key) = &key {
            match uploads.by_key.get(key) {
                Some(Some(entry)) => return Ok(Err(entry.clone())),
                Some(None) => {
                    return Err(status::failure(
                        Code::Aborted,
                        status::error(
                            format!(
                                "an upload of {} with this idempotency key is still running",
                                target.shown
                            ),
                            "wait for it to finish, then retry with the same key to get its result",
                            pb::error::Kind::InvalidField(pb::InvalidField {
                                field: "header.idempotency_key".to_owned(),
                                problem: Problem::NotAllowedHere as i32,
                            }),
                        ),
                    ));
                }
                None => {
                    uploads.by_key.insert(key.clone(), None);
                }
            }
        }
        Ok(Ok(Running {
            uploads: &self.uploads,
            key,
        }))
    }

    /// The file `location` names and the place the policy judges. Only a
    /// session's clone on this machine is served yet.
    fn place(&self, location: Option<pb::FileLocation>, method: &str) -> Result<Target, Status> {
        use pb::file_location::Root as R;

        let location = location.ok_or_else(|| status::missing("location"))?;
        if let Some(machine) = location
            .machine
            .as_ref()
            .filter(|machine| !machine.value.is_empty())
            && machine.value != self.machine
        {
            return Err(status::invalid_field(
                "location.machine",
                Problem::NotAllowedHere,
                format!(
                    "this is machine {}, and files on machine {} aren't relayed through it yet",
                    self.machine, machine.value
                ),
                "ask the daemon on that machine, or leave the machine out for this one",
            ));
        }
        let clone = match location.root {
            Some(R::SessionClone(clone)) => clone,
            Some(R::ChannelShared(_) | R::MainClone(_) | R::ChannelFolder(_)) => {
                return Err(status::not_built(&format!(
                    "{method} outside a session's clone"
                )));
            }
            None => return Err(status::missing("location.root")),
        };
        let id = clone
            .session
            .map(|session| session.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("location.session_clone.session"))?;
        let record = match self.store.session(&id) {
            Ok(Some(record)) => record,
            Ok(None) => {
                return Err(status::not_found(
                    pb::EntityKind::Session,
                    &id,
                    format!("no session {id} on this machine"),
                    "check the session id; sessions on other machines can't be named here yet",
                ));
            }
            Err(error) => return Err(status::store(error, &self.machine, "look up the session")),
        };
        let repo = clone
            .repo
            .filter(|repo| !repo.owner.is_empty() && !repo.name.is_empty())
            .ok_or_else(|| status::missing("location.session_clone.repo"))?;
        if repo.name.contains('/') || repo.name == "." || repo.name == ".." {
            return Err(status::invalid_field(
                "location.session_clone.repo.name",
                Problem::BadFormat,
                format!("{:?} isn't a repository name", repo.name),
                "give the repository's name on GitHub, like api",
            ));
        }
        if location.path.is_empty() {
            return Err(status::missing("location.path"));
        }
        let path = RepoPath::new(location.path.as_str()).map_err(|_| outside(&location.path))?;
        let policy_path = RepoPath::new(format!("{}/{}/{}", repo.owner, repo.name, path.as_str()))
            .map_err(|_| outside(&location.path))?;
        let session = identity::session_ref(&record, &self.machine);
        Ok(Target {
            place: Place {
                machine: session.machine.clone(),
                root: Root::Clone(session.clone()),
                path: policy_path,
                entry: Entry::File,
            },
            clone: record.folder.join(&repo.name),
            path,
            shown: location.path.clone(),
            location: pb::FileLocation {
                machine: Some(pb::MachineId {
                    value: self.machine.clone(),
                }),
                root: Some(R::SessionClone(pb::SessionClone {
                    session: Some(pb::SessionId { value: id }),
                    repo: Some(repo),
                })),
                path: location.path,
            },
        })
    }
}

/// A file in a session's clone.
struct Target {
    place: Place,
    clone: PathBuf,
    path: RepoPath,
    /// The path as the caller gave it, for messages.
    shown: String,
    location: pb::FileLocation,
}

impl Target {
    /// Names the file for idempotency keys.
    fn key(&self) -> String {
        format!("{:?}", self.location)
    }

    /// The file's path on disk, checked to stay in the clone once symlinks
    /// are followed. `existing` is false for a file about to be written:
    /// then only its folder must exist.
    fn resolve(&self, existing: bool) -> Result<PathBuf, Status> {
        let clone = fs::canonicalize(&self.clone).map_err(|error| match error.kind() {
            ErrorKind::NotFound => status::not_found(
                pb::EntityKind::File,
                &self.shown,
                format!("the session has no clone at {}", self.clone.display()),
                "check the repository; the session may have been started without it",
            ),
            _ => io_failure(&error, "open the clone", &self.clone),
        })?;
        let full = self.clone.join(self.path.as_str());
        let checked = if existing {
            fs::canonicalize(&full)
        } else {
            let parent = full.parent().unwrap_or(&self.clone);
            if fs::symlink_metadata(&full).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(outside(&self.shown));
            }
            fs::canonicalize(parent).map(|parent| match full.file_name() {
                Some(name) => parent.join(name),
                None => parent,
            })
        };
        let checked = checked.map_err(|error| match error.kind() {
            ErrorKind::NotFound => status::not_found(
                pb::EntityKind::File,
                &self.shown,
                format!("there is no {} in the clone", self.shown),
                "check the path; it is relative to the repository's root",
            ),
            _ => io_failure(&error, "open", &full),
        })?;
        if !checked.starts_with(&clone) || checked == clone {
            return Err(outside(&self.shown));
        }
        Ok(checked)
    }
}

#[tonic::async_trait]
impl pb::file_service_server::FileService for Files {
    async fn list_directory(
        &self,
        _request: Request<pb::ListDirectoryRequest>,
    ) -> Result<Response<pb::ListDirectoryResponse>, Status> {
        Err(status::not_built("humata.v1.FileService.ListDirectory"))
    }

    async fn stat(
        &self,
        _request: Request<pb::StatRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        Err(status::not_built("humata.v1.FileService.Stat"))
    }

    type ReadFileStream = ReadFileStream;

    /// Chunks from `offset`, each in the first of the reader's codecs this
    /// daemon's setting ranks (none if it lists none); the last carries
    /// the whole file's SHA-256 when the read reaches the end.
    async fn read_file(
        &self,
        request: Request<pb::ReadFileRequest>,
    ) -> Result<Response<ReadFileStream>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let target = self.place(request.location, "humata.v1.FileService.ReadFile")?;
        // The reply is a stream that can't be held and replayed after an approval, so Ask stays refused.
        self.guard
            .require(&caller, &Call::ReadFiles(target.place.clone()))?;
        let codec = self.compression.pick(self.link, &request.accept_codecs);
        let (send, chunks) = mpsc::channel(READ_AHEAD);
        let offset = request.offset;
        let max_bytes = request.max_bytes;
        let compression = self.compression.clone();
        let link = self.link;
        tokio::task::spawn_blocking(
            move || match read(&target, offset, max_bytes, codec, &send) {
                Ok(Some(sample)) => compression.record(link, sample),
                Ok(None) => {}
                Err(status) => {
                    let _ = send.blocking_send(Err(status));
                }
            },
        );
        Ok(Response::new(Box::pin(ReceiverStream::new(chunks))))
    }

    /// The header, then chunks in order from offset 0, each in any codec
    /// this build decodes. The file appears, or is replaced, only once all
    /// of it arrived and its size and SHA-256 match the header's.
    async fn write_file(
        &self,
        request: Request<Streaming<pb::WriteFileMessage>>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        use pb::write_file_message::Message as Part;

        let caller = identity::caller(&request)?;
        let mut messages = request.into_inner();
        let header = match messages.message().await? {
            Some(pb::WriteFileMessage {
                message: Some(Part::Header(header)),
            }) => header,
            _ => return Err(status::missing("header")),
        };
        let target = self.place(header.location, "humata.v1.FileService.WriteFile")?;
        // The upload is a stream that can't be held and replayed after an approval, so Ask stays refused.
        self.guard
            .require(&caller, &Call::WriteFiles(target.place.clone()))?;
        let running = match self.claim(header.idempotency_key.clone(), &target)? {
            Ok(running) => running,
            // Written already: the rest of this upload isn't read.
            Err(entry) => return Ok(Response::new(entry)),
        };
        let replace = header.replace;
        let target = Arc::new(target);
        let mut upload = blocking({
            let target = target.clone();
            move || Upload::start(&target, replace)
        })
        .await??;
        let mut started = None;
        let mut waited = Duration::ZERO;
        let mut moved = 0u64;
        loop {
            let asked = Instant::now();
            let next = messages.message().await?;
            let arrived = Instant::now();
            // The wait for the first chunk is the sender's start-up, not the link.
            match started {
                None => started = Some(arrived),
                Some(_) => waited += arrived - asked,
            }
            let chunk = match next {
                Some(pb::WriteFileMessage {
                    message: Some(Part::Chunk(chunk)),
                }) => chunk,
                Some(_) => {
                    return Err(status::invalid_field(
                        "header",
                        Problem::NotAllowedHere,
                        "the upload sent a second header",
                        "send one header, then the file's chunks",
                    ));
                }
                None => {
                    return Err(status::failure(
                        Code::Aborted,
                        status::error(
                            format!("the upload of {} ended before its last chunk", target.shown),
                            "send the file again; nothing was written",
                            pb::error::Kind::InvalidField(pb::InvalidField {
                                field: "chunk.last".to_owned(),
                                problem: Problem::Missing as i32,
                            }),
                        ),
                    ));
                }
            };
            if chunk.offset != upload.written {
                return Err(status::invalid_field(
                    "chunk.offset",
                    Problem::OutOfRange,
                    format!(
                        "a chunk starts at byte {} of {}, but {} bytes have arrived",
                        chunk.offset, target.shown, upload.written
                    ),
                    "send the chunks in order from the start, with no gaps",
                ));
            }
            let last = chunk.last;
            moved += chunk.data.len() as u64;
            upload = blocking(move || upload.take(chunk).map(|()| upload)).await??;
            if upload.written > header.size {
                return Err(status::invalid_field(
                    "header.size",
                    Problem::OutOfRange,
                    format!(
                        "{} bytes of {} arrived, more than the {} its header names",
                        upload.written, target.shown, header.size
                    ),
                    "send the file again with its size in the header",
                ));
            }
            if last {
                break;
            }
        }
        let size = header.size;
        let sha256 = header.sha256;
        let elapsed = started.map(|started| started.elapsed()).unwrap_or_default();
        let entry = blocking({
            let target = target.clone();
            move || upload.finish(&target, size, &sha256)
        })
        .await??;
        running.finish(&entry);
        self.compression.record(
            self.link,
            Sample {
                bytes: moved,
                elapsed,
                waited,
            },
        );
        Ok(Response::new(entry))
    }

    async fn create_folder(
        &self,
        _request: Request<pb::CreateFolderRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        Err(status::not_built("humata.v1.FileService.CreateFolder"))
    }

    async fn rename(
        &self,
        _request: Request<pb::RenameRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        Err(status::not_built("humata.v1.FileService.Rename"))
    }

    async fn move_to_trash(
        &self,
        _request: Request<pb::MoveToTrashRequest>,
    ) -> Result<Response<pb::MoveToTrashResponse>, Status> {
        Err(status::not_built("humata.v1.FileService.MoveToTrash"))
    }

    async fn find_files(
        &self,
        _request: Request<pb::FindFilesRequest>,
    ) -> Result<Response<pb::FindFilesResponse>, Status> {
        Err(status::not_built("humata.v1.FileService.FindFiles"))
    }
}

/// Sends `target` from `offset` as chunks. Stops quietly if the reader
/// went away. A read sent to its end says how the link kept up.
fn read(
    target: &Target,
    offset: u64,
    max_bytes: u64,
    codec: Codec,
    send: &mpsc::Sender<Result<pb::FileChunk, Status>>,
) -> Result<Option<Sample>, Status> {
    let path = target.resolve(true)?;
    let failed = |error: io::Error| io_failure(&error, "read", &path);
    let mut file = File::open(&path).map_err(failed)?;
    let meta = file.metadata().map_err(failed)?;
    if !meta.is_file() {
        return Err(status::invalid_field(
            "location.path",
            Problem::NotAllowedHere,
            format!("{} is a folder, not a file", target.shown),
            "name a file in it",
        ));
    }
    let size = meta.len();
    if offset > size {
        return Err(status::invalid_field(
            "offset",
            Problem::OutOfRange,
            format!(
                "offset {offset} is past the end of {}, which has {size} bytes",
                target.shown
            ),
            "resume from the offset of the last chunk received, or 0",
        ));
    }
    // The last chunk carries the whole file's hash, so a resumed read
    // hashes what it skips.
    let mut hasher = Sha256::new();
    hash_prefix(&mut file, offset, &mut hasher).map_err(failed)?;
    let end = match max_bytes {
        0 => size,
        max => size.min(offset.saturating_add(max)),
    };
    let mut encoder = Encoder::new(codec);
    let mut at = offset;
    let started = Instant::now();
    let mut waited = Duration::ZERO;
    let mut moved = 0u64;
    loop {
        let want = (CHUNK_MAX as u64).min(end - at);
        let mut data = Vec::with_capacity(want as usize);
        (&mut file)
            .take(want)
            .read_to_end(&mut data)
            .map_err(failed)?;
        let read = data.len() as u64;
        hasher.update(&data);
        // A file that shrank while being read ends where it ends now.
        let last = at + read >= end || read < want;
        let whole = last && (at + read >= size || read < want);
        let chunk = encoder
            .encode(data)
            .map_err(|error| chunk_failure(&error, &target.shown))?;
        let message = pb::FileChunk {
            offset: at,
            data: chunk.data,
            sha256: match whole {
                true => hex(&hasher.clone().finalize()),
                false => String::new(),
            },
            last,
            codec: chunk.header.codec.wire(),
            raw_size: chunk.header.raw_len,
            crc32: chunk.header.crc32,
        };
        moved += message.data.len() as u64;
        let handing = Instant::now();
        if send.blocking_send(Ok(message)).is_err() {
            return Ok(None);
        }
        waited += handing.elapsed();
        if last {
            return Ok(Some(Sample {
                bytes: moved,
                elapsed: started.elapsed(),
                waited,
            }));
        }
        at += read;
    }
}

fn hash_prefix(file: &mut File, len: u64, hasher: &mut Sha256) -> io::Result<()> {
    let mut buffer = vec![0; 64 << 10];
    let mut left = len;
    while left > 0 {
        let want = buffer
            .len()
            .min(usize::try_from(left).unwrap_or(usize::MAX));
        let read = file.read(&mut buffer[..want])?;
        if read == 0 {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "the file shrank while being read",
            ));
        }
        hasher.update(&buffer[..read]);
        left -= read as u64;
    }
    Ok(())
}

/// A file being written: a hidden partial file next to it, removed unless
/// it is finished.
struct Upload {
    partial: PathBuf,
    destination: PathBuf,
    file: Option<File>,
    hasher: Sha256,
    decoder: Decoder,
    written: u64,
    replace: bool,
}

impl Upload {
    fn start(target: &Target, replace: bool) -> Result<Self, Status> {
        let destination = target.resolve(false)?;
        if !replace && fs::symlink_metadata(&destination).is_ok() {
            return Err(exists(&target.shown));
        }
        let name = destination
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let partial = destination.with_file_name(format!(
            ".{name}.humata-partial-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|error| io_failure(&error, "write", &partial))?;
        Ok(Self {
            partial,
            destination,
            file: Some(file),
            hasher: Sha256::new(),
            decoder: Decoder::new(),
            written: 0,
            replace,
        })
    }

    fn take(&mut self, chunk: pb::FileChunk) -> Result<(), Status> {
        let header = compression::chunk_header(&chunk).ok_or_else(|| {
            chunk_failure(
                &ChunkError::UnknownCodec { wire: chunk.codec },
                &self.shown(),
            )
        })?;
        let raw = self
            .decoder
            .decode(header, chunk.data)
            .map_err(|error| chunk_failure(&error, &self.shown()))?;
        self.hasher.update(&raw);
        if let Some(file) = self.file.as_mut() {
            file.write_all(&raw)
                .map_err(|error| io_failure(&error, "write", &self.partial))?;
        }
        self.written += raw.len() as u64;
        Ok(())
    }

    fn shown(&self) -> String {
        self.destination.display().to_string()
    }

    /// Moves the file into place if it is what the header said.
    fn finish(mut self, target: &Target, size: u64, sha256: &str) -> Result<pb::FileEntry, Status> {
        let actual = hex(&self.hasher.clone().finalize());
        if self.written != size || !actual.eq_ignore_ascii_case(sha256) {
            return Err(status::failure(
                Code::DataLoss,
                status::error(
                    format!(
                        "{} arrived as {} bytes with SHA-256 {actual}, not the {size} bytes with SHA-256 {sha256} its header names; nothing was written",
                        target.shown, self.written
                    ),
                    "send the file again. If it fails the same way, the file changed while it was being sent: wait until it stops changing",
                    pb::error::Kind::ChecksumMismatch(pb::ChecksumMismatch {
                        transfer: None,
                        path: target.shown.clone(),
                        expected_sha256: sha256.to_owned(),
                        actual_sha256: actual,
                    }),
                ),
            ));
        }
        let file = self.file.take();
        if let Some(file) = &file {
            file.sync_all()
                .map_err(|error| io_failure(&error, "write", &self.partial))?;
        }
        drop(file);
        if !self.replace && fs::symlink_metadata(&self.destination).is_ok() {
            return Err(exists(&target.shown));
        }
        fs::rename(&self.partial, &self.destination)
            .map_err(|error| io_failure(&error, "write", &self.destination))?;
        let modified = fs::metadata(&self.destination)
            .and_then(|meta| meta.modified())
            .ok()
            .map(grant_target::timestamp);
        Ok(pb::FileEntry {
            location: Some(target.location.clone()),
            kind: pb::file_entry::Kind::File as i32,
            size,
            modified_at: modified,
            changed: false,
            writable: true,
        })
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        // Gone already once the upload is moved into place.
        let _ = fs::remove_file(&self.partial);
    }
}

/// Runs `work` on a blocking thread.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Status> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| status::internal(format!("a file worker stopped: {error}"), ""))
}

fn outside(path: &str) -> Status {
    status::failure(
        Code::PermissionDenied,
        status::error(
            format!("{path:?} leaves the clone, by \"..\", an absolute path or a symlink"),
            "name a file inside the clone, relative to the repository's root",
            pb::error::Kind::OutsideHumataFolders(pb::OutsideHumataFolders {
                path: path.to_owned(),
            }),
        ),
    )
}

fn exists(path: &str) -> Status {
    status::failure(
        Code::AlreadyExists,
        status::error(
            format!("{path} already exists"),
            "send it with replace set to overwrite it, or pick another name",
            pb::error::Kind::NameTaken(pb::NameTaken::default()),
        ),
    )
}

fn chunk_failure(error: &ChunkError, path: &str) -> Status {
    status::failure(
        Code::DataLoss,
        status::error(
            format!("a chunk of {path}: {}", status::chain(error)),
            "send the file again from the start",
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "chunk.data".to_owned(),
                problem: Problem::BadFormat as i32,
            }),
        ),
    )
}

fn io_failure(error: &io::Error, doing: &str, path: &Path) -> Status {
    status::internal(format!("couldn't {doing} {}: {error}", path.display()), "")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
