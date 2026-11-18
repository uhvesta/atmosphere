//! FileService on this machine's own files, in any root a location can
//! name (`roots`): ReadFile and WriteFile in chunks of at most 1 MiB, each
//! compressed with the codec the two ends agree on (TR-11, D65), and the
//! file manager's operations (`ops`): list, stat, new folder, rename and
//! move to the trash (FM-2, FM-3), and Go to file (`find`, RV-11).
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
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use humata2_access::{Call, Entry, Place};
use humata2_chunk::{CHUNK_MAX, ChunkError, Codec, Decoder, Encoder, Link, Sample};
use humata2_grpc::humata::v1 as pb;
use pb::invalid_field::Problem;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status, Streaming};

use crate::compression::{self, Compression};
use crate::grant_target;
use crate::guard::{Checked, Guard};
use crate::host_relay::Links;
use crate::identity;
use crate::inbox::{Deferred, Inbox, Run};
use crate::status;
use crate::trash_service::Bin;

mod changes;
mod find;
mod forward;
mod ops;
mod roots;

pub(crate) use changes::FileChanges;
use changes::Spot;
use find::Find;
use forward::Forward;
pub(crate) use forward::{TrashForward, qualified};
use ops::Ops;
pub(crate) use roots::{Placed, Roots, Target};
use roots::io_failure;

pub(crate) type ReadFileStream = Pin<Box<dyn Stream<Item = Result<pb::FileChunk, Status>> + Send>>;

/// Judges an agent's file read. One the channel's rule asks about raises an
/// Inbox card and answers `approval_pending` at once (D768).
#[derive(Clone)]
pub(in crate::files) struct Reads {
    guard: Arc<Guard>,
    inbox: Arc<Inbox>,
}

impl Reads {
    /// `run`, made only when the rule asks, is the read itself: approving
    /// the card runs it, and waiting on the card returns its result. With
    /// none, the agent repeats the read once it is approved.
    pub(in crate::files) async fn require(
        &self,
        caller: &identity::Caller,
        place: Place,
        operation: &str,
        run: impl FnOnce() -> Option<Run>,
    ) -> Result<(), Status> {
        match self.guard.check(caller, &Call::ReadFiles(place))? {
            Checked::Allowed => Ok(()),
            Checked::Ask(asked) => self.inbox.ask_read(caller, asked, operation, run()).await,
        }
    }
}

/// An approved read's run: `read` on its own, its reply kept as `result_type`.
pub(in crate::files) fn held_read<R, F>(result_type: &'static str, read: F) -> Option<Run>
where
    R: prost::Message + Send + 'static,
    F: std::future::Future<Output = Result<R, Status>> + Send + 'static,
{
    Some(Box::new(move |_| {
        Box::pin(async move {
            let reply = read.await?;
            Ok(Deferred {
                result: reply.encode_to_vec(),
                result_type: result_type.to_owned(),
            })
        })
    }))
}

/// The most of a file an approved read keeps for its waiter (D768). A
/// longer file's held chunk isn't `last`, and the rest is read as usual.
const HELD_MAX: u64 = 8 * 1024 * 1024;

/// The byte count an approved read of `max_bytes` (0 for all) keeps.
fn held_max(max_bytes: u64) -> u64 {
    match max_bytes {
        0 => HELD_MAX,
        max => max.min(HELD_MAX),
    }
}

/// `chunks` of one read, uncompressed, as the one chunk an approved read
/// keeps: `last` only when the read reached where the caller asked it to
/// end, not where [`HELD_MAX`] cut it.
fn held_chunk(offset: u64, max_bytes: u64, chunks: Vec<pb::FileChunk>) -> pb::FileChunk {
    let mut data = Vec::new();
    let mut sha256 = String::new();
    for chunk in chunks {
        data.extend_from_slice(&chunk.data);
        if !chunk.sha256.is_empty() {
            sha256 = chunk.sha256;
        }
    }
    let cut = (max_bytes == 0 || max_bytes > HELD_MAX) && data.len() as u64 >= HELD_MAX && sha256.is_empty();
    pb::FileChunk {
        offset,
        raw_size: u32::try_from(data.len()).unwrap_or(u32::MAX),
        data,
        sha256,
        last: !cut,
        codec: Codec::None.wire(),
        crc32: None,
    }
}

/// Chunks read ahead of a slow reader.
const READ_AHEAD: usize = 2;

/// Finished uploads kept for retries with the same idempotency key.
const REMEMBERED_MAX: usize = 1024;

pub(crate) struct Files {
    roots: Arc<Roots>,
    guard: Arc<Guard>,
    ops: Ops,
    find: Find,
    compression: Arc<Compression>,
    /// The link the callers are on: the socket's or the peer link's.
    link: Link,
    uploads: Mutex<Uploads>,
    /// Told of the user's writes to a session's clone.
    user_writes: std::sync::OnceLock<Arc<dyn UserWrites>>,
    /// Told when a call adds, removes or renames files in a session's clone.
    changes: std::sync::OnceLock<Arc<FileChanges>>,
    /// Calls that name another machine (FM-9, FM-12).
    forward: Forward,
    reads: Reads,
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

/// Who is told when the user's WriteFile puts a file into a session's clone,
/// so the review service can tell the user's edit from the agent's.
pub(crate) trait UserWrites: Send + Sync {
    /// A user write in `session` is about to move its file into place.
    /// `None` when nobody is told of that session. Held until the write is
    /// over; while held, the review does not look at the session's comments.
    fn writing(&self, session: &str) -> Option<Box<dyn UserWrite>>;
}

/// One user write under way, from [`UserWrites::writing`].
#[tonic::async_trait]
pub(crate) trait UserWrite: Send {
    /// The file is in place: `path` of the session's clone of `repo`.
    /// Returns once the review has taken the new text as the baseline.
    async fn wrote(self: Box<Self>, repo: &pb::RepoRef, path: &str);
}

impl Files {
    pub(crate) fn new(
        roots: Arc<Roots>,
        guard: Arc<Guard>,
        bin: Arc<Bin>,
        compression: Arc<Compression>,
        link: Link,
        inbox: Arc<Inbox>,
    ) -> Self {
        let reads = Reads {
            guard: guard.clone(),
            inbox,
        };
        Self {
            ops: Ops::new(roots.clone(), guard.clone(), bin, reads.clone()),
            forward: Forward::new(roots.clone(), guard.clone(), reads.clone()),
            find: Find::new(roots.clone(), reads.clone(), roots.store().clone()),
            reads,
            roots,
            guard,
            compression,
            link,
            uploads: Mutex::new(Uploads::default()),
            user_writes: std::sync::OnceLock::new(),
            changes: std::sync::OnceLock::new(),
        }
    }

    /// The list of session clones' files, told on the feed as these calls
    /// change it (bug 23).
    pub(crate) fn use_changes(&self, changes: Arc<FileChanges>) {
        let _ = self.changes.set(changes);
    }

    /// The session clones `locations` are in, each once; none unless the
    /// changes are told.
    fn spots<'a>(&self, locations: impl IntoIterator<Item = Option<&'a pb::FileLocation>>) -> Vec<Spot> {
        let mut spots: Vec<Spot> = Vec::new();
        if self.changes.get().is_none() {
            return spots;
        }
        for location in locations {
            if let Some(spot) = Spot::of(&self.roots, location) {
                match spots.iter_mut().find(|known| known.same_clone(&spot)) {
                    Some(known) => known.also(spot),
                    None => spots.push(spot),
                }
            }
        }
        spots
    }

    /// Remembers the files of `spots` before a call changes them, so the
    /// change is a difference; says nothing.
    async fn before(&self, spots: &[Spot]) {
        let Some(changes) = self.changes.get().filter(|_| !spots.is_empty()).cloned() else {
            return;
        };
        let spots = spots.to_vec();
        // Best effort: a worker that failed leaves the list to the next look.
        let _ = blocking(move || spots.iter().for_each(|spot| spot.remember(&changes))).await;
    }

    /// Tells what a finished call changed in the files of `spots`.
    async fn after(&self, spots: &[Spot]) {
        let Some(changes) = self.changes.get().filter(|_| !spots.is_empty()).cloned() else {
            return;
        };
        let spots = spots.to_vec();
        let _ = blocking(move || spots.iter().for_each(|spot| spot.look(&changes))).await;
    }

    /// The machines' links (the host only; without them every other
    /// machine is offline), for calls that name one.
    pub(crate) fn use_links(&self, links: Links) {
        self.forward.use_links(links);
    }

    /// On a machine: its agents' reads of another machine's files go up
    /// `host`, the Uplink (D768).
    pub(crate) fn ask_host_for_agents(&self, host: Arc<crate::away::Forwarder>) {
        self.forward.ask_host_for_agents(host);
    }

    /// The review service, told when the user writes a file in a session's
    /// clone, so the comments on it answer only to the agent's changes.
    pub(crate) fn use_user_writes(&self, writes: Arc<dyn UserWrites>) {
        let _ = self.user_writes.set(writes);
    }

    /// The write to tell of, if the user is writing a file in a session's clone.
    fn user_write(&self, caller: &identity::Caller, location: &pb::FileLocation) -> Option<(Box<dyn UserWrite>, pb::RepoRef)> {
        if !matches!(caller, identity::Caller::User(_)) {
            return None;
        }
        let Some(pb::file_location::Root::SessionClone(clone)) = &location.root else {
            return None;
        };
        let session = clone.session.as_ref().filter(|session| !session.value.is_empty())?;
        let held = self.user_writes.get()?.writing(&session.value)?;
        Some((held, clone.repo.clone()?))
    }

    /// The host's Migrate items, so a change to a machine at another
    /// release is refused (UP-1).
    pub(crate) fn use_migrations(&self, migrations: crate::upgrade::items::Migrations) {
        self.forward.use_migrations(migrations);
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
}

#[tonic::async_trait]
impl pb::file_service_server::FileService for Files {
    async fn list_directory(
        &self,
        request: Request<pb::ListDirectoryRequest>,
    ) -> Result<Response<pb::ListDirectoryResponse>, Status> {
        if self.forward.elsewhere(request.get_ref().location.as_ref()).is_some() {
            return self.forward.list_directory(request).await;
        }
        self.ops.list_directory(request).await
    }

    async fn stat(
        &self,
        request: Request<pb::StatRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        if self.forward.elsewhere(request.get_ref().location.as_ref()).is_some() {
            return self.forward.stat(request).await;
        }
        self.ops.stat(request).await
    }

    type ReadFileStream = ReadFileStream;

    /// Chunks from `offset`, each in the first of the reader's codecs this
    /// daemon's setting ranks (none if it lists none); the last carries
    /// the whole file's SHA-256 when the read reaches the end.
    async fn read_file(
        &self,
        request: Request<pb::ReadFileRequest>,
    ) -> Result<Response<ReadFileStream>, Status> {
        if self.forward.elsewhere(request.get_ref().location.as_ref()).is_some() {
            return self.forward.read_file(request).await;
        }
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let location = request.location.clone();
        let target = self.roots.target(request.location, "location", Entry::File)?;
        // An asked read answers approval_pending at once; approving it reads
        // the file (up to HELD_MAX, uncompressed) for the waiter (D768).
        let (roots, offset, max_bytes) = (self.roots.clone(), request.offset, request.max_bytes);
        self.reads
            .require(&caller, target.place.clone(), "read", move || {
                held_read("humata.v1.FileChunk", async move {
                    let target = roots.target(location, "location", Entry::File)?;
                    let kept = held_max(max_bytes);
                    blocking_read(move || {
                        let (send, mut chunks) = mpsc::channel((kept / CHUNK_MAX as u64 + 2) as usize);
                        read(&target, offset, kept, Codec::None, &send)?;
                        drop(send);
                        let mut read = Vec::new();
                        while let Ok(chunk) = chunks.try_recv() {
                            read.push(chunk?);
                        }
                        Ok(held_chunk(offset, max_bytes, read))
                    })
                    .await
                })
            })
            .await?;
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
        if self.forward.elsewhere(header.location.as_ref()).is_some() {
            return self.forward.write_file(&caller, header, messages).await;
        }
        let target = self.roots.target(header.location, "location", Entry::File)?;
        // The upload is a stream that can't be held and replayed after an approval, so Ask stays refused.
        self.guard
            .require(&caller, &Call::WriteFiles(target.place.clone()))?;
        target.require_writable("location")?;
        let running = match self.claim(header.idempotency_key.clone(), &target)? {
            Ok(running) => running,
            // Written already: the rest of this upload isn't read.
            Err(entry) => return Ok(Response::new(entry)),
        };
        let replace = header.replace;
        let expected = header.expected_sha256.clone().filter(|hash| !hash.is_empty());
        let target = Arc::new(target);
        let mut upload = blocking({
            let target = target.clone();
            move || Upload::start(&target, replace, expected)
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
        // Taken before the file is moved into place, so the review never sees the new text unannounced.
        let held = self.user_write(&caller, &target.location);
        let spots = self.spots([Some(&target.location)]);
        self.before(&spots).await;
        let entry = blocking({
            let target = target.clone();
            move || upload.finish(&target, size, &sha256)
        })
        .await??;
        self.after(&spots).await;
        if let Some((held, repo)) = held {
            held.wrote(&repo, &target.location.path).await;
        }
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
        request: Request<pb::CreateFolderRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        if self.forward.elsewhere(request.get_ref().location.as_ref()).is_some() {
            return self.forward.create_folder(request).await;
        }
        self.ops.create_folder(request).await
    }

    async fn rename(
        &self,
        request: Request<pb::RenameRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        if self.forward.elsewhere(request.get_ref().location.as_ref()).is_some() {
            return self.forward.rename(request).await;
        }
        let mut spots = self.spots([request.get_ref().location.as_ref()]);
        if let Some(location) = request.get_ref().location.as_ref() {
            // The new name is beside the old one: both paths are told, nothing is listed.
            for spot in &mut spots {
                spot.renamed(&location.path, &request.get_ref().new_name);
            }
        }
        self.before(&spots).await;
        let renamed = self.ops.rename(request).await;
        if renamed.is_ok() {
            self.after(&spots).await;
        }
        renamed
    }

    async fn move_to_trash(
        &self,
        request: Request<pb::MoveToTrashRequest>,
    ) -> Result<Response<pb::MoveToTrashResponse>, Status> {
        if self.forward.elsewhere_any(&request.get_ref().locations).is_some() {
            return self.forward.move_to_trash(request).await;
        }
        let spots = self.spots(request.get_ref().locations.iter().map(Some));
        self.before(&spots).await;
        let trashed = self.ops.move_to_trash(request).await;
        if trashed.is_ok() {
            self.after(&spots).await;
        }
        trashed
    }

    async fn find_files(
        &self,
        request: Request<pb::FindFilesRequest>,
    ) -> Result<Response<pb::FindFilesResponse>, Status> {
        if self.forward.elsewhere_session(request.get_ref().session.as_ref()).is_some() {
            return self.forward.find_files(request).await;
        }
        self.find.find_files(request).await
    }
}

/// Sends `target` from `offset` as chunks. Stops quietly if the reader
/// went away. A read sent to its end says how the link kept up.
/// Runs `work` on a blocking thread.
async fn blocking_read<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Status> + Send + 'static,
) -> Result<T, Status> {
    crate::trash_service::blocking(work).await?
}

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
    /// With `replace`: the SHA-256 the file must still have at the moment it
    /// is replaced.
    expected: Option<String>,
}

impl Upload {
    fn start(target: &Target, replace: bool, expected: Option<String>) -> Result<Self, Status> {
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
            expected,
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
        if self.replace
            && let Some(expected) = &self.expected
        {
            let now = current_sha256(&self.destination)
                .map_err(|error| io_failure(&error, "read", &self.destination))?;
            if !now.as_deref().is_some_and(|now| now.eq_ignore_ascii_case(expected)) {
                return Err(changed_underneath(&target.shown, now.is_some()));
            }
        }
        fs::rename(&self.partial, &self.destination)
            .map_err(|error| io_failure(&error, "write", &self.destination))?;
        if let Some(parent) = self.destination.parent() {
            // Best effort past this point: the file is in place and complete.
            let _ = fs::File::open(parent).and_then(|folder| folder.sync_all());
        }
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

/// The SHA-256 of the file at `path`, or `None` when there is no file.
fn current_sha256(path: &std::path::Path) -> io::Result<Option<String>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(Some(hex(&hasher.finalize())))
}

/// A write that named the version it was made from, refused because the
/// file is no longer that version.
fn changed_underneath(path: &str, still_there: bool) -> Status {
    let what = if still_there { "was changed by someone else" } else { "was removed" };
    status::failure(
        Code::Aborted,
        status::error(
            format!("{path} {what} since it was read, so this write was refused and nothing was written"),
            "reload the file, apply your change to what is there now, and save again. Saving over it without reloading would lose the other change",
            pb::error::Kind::PlanStale(pb::PlanStale {
                changes: vec![format!("{path} {what}")],
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
