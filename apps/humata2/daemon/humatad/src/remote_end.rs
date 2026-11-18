//! The host's half of a transfer to or from another machine (FM-5, FM-6,
//! FM-8, FM-9, FM-13; D142, D145). The transfer engine reads and writes only
//! through [`End`] and is synchronous, so a remote end is an `End` that does
//! each step on the machine over its link, and [`RouterEnd`] picks this or
//! the local disk per path. The engine itself doesn't change.
//!
//! A remote entry is named for the engine by a virtual absolute path:
//! `/@humata/<machine>/<hex of the location without its path>/<path in the
//! root>`. [`virtual_path`] makes one and [`parse`] reads it back.
//!
//! Every call goes out as a fresh request on the machine's live link and
//! never waits for one: a link that is down is [`EndError::Offline`], which
//! the engine turns into a transfer waiting for the machine (FM-8, FM-13).
//! [`resume_when_back`] tells the service when a link comes up again.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use humata2_chunk::{CHUNK_MAX, Decoder, Encoder, Link, Sample};
use humata2_grpc::Timestamp;
use humata2_grpc::humata::v1 as pb;
use humata2_transfers::{End, EndError, Entry, Kind, Problem, Sink, Snapshot, State, TrashError};
use prost::Message;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::{Code, Request, Response, Status, Streaming};

use crate::compression::{self, Compression};
use crate::files::Placed;
use crate::host_relay::{LinkState, Links};
use crate::status;

/// Where every virtual path starts.
const PREFIX: &str = "/@humata";

/// Chunks queued between the engine's thread and the link: a couple, so a
/// slow link pushes back instead of buffering the file.
const QUEUE: usize = 2;

/// The link to a machine, as compression sees it: nothing measured yet.
const REMOTE: Link = Link::Remote { bytes_per_second: None };

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| text.get(at..at + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok()))
        .collect()
}

/// The path the engine uses for `location` on `machine`.
pub(crate) fn virtual_path(machine: &str, location: &pb::FileLocation) -> PathBuf {
    let mut root = location.clone();
    root.path.clear();
    // The machine is part of the path already.
    root.machine = None;
    let mut path = PathBuf::from(PREFIX);
    path.push(machine);
    path.push(hex(&root.encode_to_vec()));
    for part in location.path.split('/').filter(|part| !part.is_empty()) {
        path.push(part);
    }
    path
}

/// The machine and location a virtual path names; none for a path that
/// isn't one.
fn parse(path: &Path) -> Option<(String, pb::FileLocation)> {
    let mut parts = path.components();
    if parts.next()? != Component::RootDir || parts.next()? != Component::Normal("@humata".as_ref()) {
        return None;
    }
    let machine = parts.next()?.as_os_str().to_str()?.to_owned();
    let root = unhex(parts.next()?.as_os_str().to_str()?)?;
    let mut location = pb::FileLocation::decode(root.as_slice()).ok()?;
    let mut names = Vec::new();
    for part in parts {
        names.push(part.as_os_str().to_str()?.to_owned());
    }
    location.path = names.join("/");
    location.machine = Some(pb::MachineId { value: machine.clone() });
    Some((machine, location))
}

fn is_virtual(path: &Path) -> bool {
    path.starts_with(PREFIX)
}

fn timestamp_of(unix_ms: i64) -> Timestamp {
    Timestamp {
        seconds: unix_ms.div_euclid(1000),
        nanos: i32::try_from(unix_ms.rem_euclid(1000) * 1_000_000).unwrap_or(0),
    }
}

fn unix_ms_of(stamp: Option<&Timestamp>) -> i64 {
    stamp.map_or(0, |stamp| stamp.seconds.saturating_mul(1000) + i64::from(stamp.nanos / 1_000_000))
}

/// What every call, reader and sink of one [`RemoteEnd`] shares: the links
/// (given once the machines' supervisor exists) and, per machine, the newest
/// link generation a call failed on as offline.
#[derive(Default)]
struct Shared {
    links: OnceLock<Links>,
    failed_on: Mutex<BTreeMap<String, u64>>,
    /// Bytes of chunks written to a machine, as they went on the wire.
    sent: AtomicU64,
    /// Bytes of chunks read from a machine, as they came on the wire.
    received: AtomicU64,
}

impl Shared {
    /// The machine's live channel with the generation of its link. No links
    /// yet, or a link that is down, is offline.
    fn live(&self, machine: &str) -> Result<(Channel, u64), EndError> {
        let offline = || EndError::Offline {
            machine: machine.to_owned(),
        };
        let links = self.links.get().ok_or_else(offline)?;
        // The generation first: a link replaced in between only makes the
        // recorded generation older, which costs one extra resume.
        let generation = match *links.watch(machine).borrow() {
            LinkState::Up { generation } => generation,
            LinkState::Down => return Err(offline()),
        };
        let channel = links.channel(machine).map_err(|_| offline())?;
        Ok((channel, generation))
    }

    fn link_down(&self, machine: &str) -> bool {
        self.links.get().is_none_or(|links| links.channel(machine).is_err())
    }

    /// An offline failure of a call made on link `generation` (0: none).
    fn offline(&self, machine: &str, generation: u64) -> EndError {
        let mut failed = self.failed_on.lock().unwrap_or_else(PoisonError::into_inner);
        let seen = failed.entry(machine.to_owned()).or_insert(0);
        *seen = (*seen).max(generation);
        EndError::Offline {
            machine: machine.to_owned(),
        }
    }

    /// Whether `machine`'s link now is a newer one than any a call failed
    /// on: the transfer that failed offline can go on.
    fn newer_link(&self, machine: &str) -> bool {
        let Some(links) = self.links.get() else {
            return false;
        };
        let LinkState::Up { generation } = *links.watch(machine).borrow() else {
            return false;
        };
        let failed = self.failed_on.lock().unwrap_or_else(PoisonError::into_inner);
        generation > failed.get(machine).copied().unwrap_or(0)
    }
}

/// The end of a transfer that is on another machine.
pub(crate) struct RemoteEnd {
    shared: Arc<Shared>,
    runtime: Handle,
    compression: Arc<Compression>,
}

/// What one call needs: the machine, its live channel and the location.
struct Call {
    machine: String,
    channel: Channel,
    /// The generation of the link `channel` belongs to.
    generation: u64,
    /// The location as the machine knows it: no machine named, so it is
    /// local there.
    location: pb::FileLocation,
}

impl RemoteEnd {
    /// An end with no links yet: every call is offline until [`use_links`].
    ///
    /// [`use_links`]: RemoteEnd::use_links
    pub(crate) fn new(runtime: Handle, compression: Arc<Compression>) -> Self {
        Self {
            shared: Arc::new(Shared::default()),
            runtime,
            compression,
        }
    }

    /// The machines' links, once they exist. The first call wins.
    pub(crate) fn use_links(&self, links: Links) {
        let _ = self.shared.links.set(links);
    }

    /// Bytes of chunks written to machines so far, after compression.
    pub(crate) fn sent_bytes(&self) -> u64 {
        self.shared.sent.load(Ordering::Relaxed)
    }

    /// Bytes of chunks read from machines so far, after compression.
    pub(crate) fn received_bytes(&self) -> u64 {
        self.shared.received.load(Ordering::Relaxed)
    }

    /// Whether `machine`'s link is a newer one than any a call of this end
    /// failed on as offline. A transfer that failed offline while its link
    /// was already back (the failure settled after the link-up was
    /// announced) is resumed on this, and only on this: a link that is up
    /// but keeps failing never counts twice, so there is no loop.
    pub(crate) fn relinked_since_failure(&self, machine: &str) -> bool {
        self.shared.newer_link(machine)
    }

    /// The machine, link and location `path` names. A link that is down is
    /// offline; a path that isn't a virtual one never reaches here.
    fn call(&self, path: &Path) -> Result<Call, EndError> {
        let (machine, mut location) = parse(path).ok_or_else(|| EndError::Io {
            doing: "find",
            path: path.to_owned(),
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                "this is not a path on another machine",
            ),
        })?;
        let (channel, generation) = self.shared.live(&machine)?;
        // On the machine the location is its own.
        location.machine = None;
        Ok(Call {
            machine,
            channel,
            generation,
            location,
        })
    }

    fn files(call: &Call) -> pb::file_service_client::FileServiceClient<Channel> {
        pb::file_service_client::FileServiceClient::new(call.channel.clone())
    }

    fn ends(call: &Call) -> pb::transfer_end_service_client::TransferEndServiceClient<Channel> {
        pb::transfer_end_service_client::TransferEndServiceClient::new(call.channel.clone())
    }

    /// A gRPC failure as the engine understands it.
    fn failed(&self, call: &Call, status: &Status, doing: &'static str, path: &Path) -> EndError {
        failure(&self.shared, call.generation, status, &call.machine, doing, path)
    }

    fn codecs(&self) -> Vec<i32> {
        self.compression.offered(REMOTE)
    }

    /// Whether `from` and `to` are in one root of one machine, so a move
    /// between them is one rename there.
    fn same_root(from: &Path, to: &Path) -> bool {
        match (parse(from), parse(to)) {
            (Some((a, mut x)), Some((b, mut y))) => {
                x.path.clear();
                y.path.clear();
                a == b && x == y
            }
            _ => false,
        }
    }
}

/// Whether the client built `status` itself from a transport error. Tonic
/// gives every status made from an error a source; a status the machine
/// answered arrives over the wire with none. A broken connection reaches the
/// call at the same moment as the link supervisor, on another task, so the
/// links table may not say down yet: the status itself is the evidence.
fn built_locally(status: &Status) -> bool {
    std::error::Error::source(status).is_some()
}

/// `status` as an [`EndError`]: gone and taken as such, a link that is down
/// (or went down in the call) as offline, the rest as what the machine said.
fn failure(
    shared: &Shared,
    generation: u64,
    status: &Status,
    machine: &str,
    doing: &'static str,
    path: &Path,
) -> EndError {
    let message = status::chain(status);
    let link_down = shared.link_down(machine);
    match status.code() {
        Code::NotFound => EndError::NotFound {
            path: path.to_owned(),
            source: io::Error::new(io::ErrorKind::NotFound, message),
        },
        Code::AlreadyExists => EndError::Exists { path: path.to_owned() },
        Code::Unavailable => shared.offline(machine, generation),
        Code::Cancelled | Code::Unknown | Code::Aborted | Code::Internal | Code::DeadlineExceeded
            if link_down || built_locally(status) =>
        {
            shared.offline(machine, generation)
        }
        _ if status::error_of(status)
            .is_some_and(|error| matches!(error.kind, Some(pb::error::Kind::NameTaken(_)))) =>
        {
            EndError::Exists { path: path.to_owned() }
        }
        _ => EndError::Io {
            doing,
            path: path.to_owned(),
            source: io::Error::other(message),
        },
    }
}

impl End for RemoteEnd {
    /// One `RemoteEnd` serves every machine, so there is no single machine
    /// id to show; the machine is in each path.
    fn label(&self) -> &str {
        "remote"
    }

    fn same_disk(&self, _other: &dyn End, from: &Path, to_folder: &Path) -> bool {
        Self::same_root(from, to_folder)
    }

    fn plan(&self, source: &Path) -> Result<Vec<Entry>, EndError> {
        let call = self.call(source)?;
        let failed = |status: &Status| self.failed(&call, status, "read the folder", source);
        let tree = pb::PlanTreeRequest {
            source: Some(call.location.clone()),
        };
        let mut client = Self::ends(&call);
        self.runtime.block_on(async {
            let mut stream = client
                .plan_tree(Request::new(tree))
                .await
                .map_err(|status| failed(&status))?
                .into_inner();
            let mut entries = Vec::new();
            while let Some(entry) = stream.message().await.map_err(|status| failed(&status))? {
                let rel_path = if entry.rel.is_empty() {
                    source.to_owned()
                } else {
                    source.join(&entry.rel)
                };
                let kind = match pb::file_entry::Kind::try_from(entry.kind) {
                    Ok(pb::file_entry::Kind::File) => Kind::File,
                    Ok(pb::file_entry::Kind::Folder) => Kind::Folder,
                    Ok(pb::file_entry::Kind::Symlink) => Kind::Link,
                    _ => {
                        return Err(EndError::Unsupported {
                            path: rel_path,
                            what: "thing of a kind the machine didn't name",
                        });
                    }
                };
                entries.push(Entry {
                    rel: entry.rel,
                    kind,
                    size: entry.size,
                    mode: entry.mode,
                    modified_unix_ms: unix_ms_of(entry.modified_at.as_ref()),
                    link_target: (!entry.link_target.is_empty()).then(|| PathBuf::from(entry.link_target)),
                });
            }
            Ok(entries)
        })
    }

    fn open_read(&self, path: &Path, offset: u64) -> Result<Box<dyn Read + Send>, EndError> {
        let call = self.call(path)?;
        let request = pb::ReadFileRequest {
            location: Some(call.location.clone()),
            offset,
            max_bytes: 0,
            reason: String::new(),
            accept_codecs: self.codecs(),
        };
        let mut client = Self::files(&call);
        let stream = self
            .runtime
            .block_on(client.read_file(Request::new(request)))
            .map_err(|status| self.failed(&call, &status, "read", path))?
            .into_inner();
        Ok(Box::new(RemoteRead {
            runtime: self.runtime.clone(),
            shared: self.shared.clone(),
            generation: call.generation,
            machine: call.machine,
            stream,
            decoder: Decoder::new(),
            buffer: Vec::new(),
            at: 0,
            ended: false,
        }))
    }

    fn free_bytes(&self, folder: &Path) -> Result<u64, EndError> {
        let call = self.call(folder)?;
        let request = pb::FreeSpaceRequest {
            folder: Some(call.location.clone()),
        };
        let mut client = Self::ends(&call);
        self.runtime
            .block_on(client.free_space(Request::new(request)))
            .map(|answer| answer.into_inner().free_bytes)
            .map_err(|status| self.failed(&call, &status, "measure", folder))
    }

    fn exists(&self, path: &Path) -> Result<bool, EndError> {
        let call = self.call(path)?;
        let request = pb::EndPathRequest {
            location: Some(call.location.clone()),
        };
        let mut client = Self::ends(&call);
        self.runtime
            .block_on(client.exists(Request::new(request)))
            .map(|answer| answer.into_inner().exists)
            .map_err(|status| self.failed(&call, &status, "look at", path))
    }

    fn have_chunks(&self, folder: &Path, sha256: &[String]) -> Result<Vec<String>, EndError> {
        let call = self.call(folder)?;
        let request = pb::HaveChunksRequest {
            location: Some(call.location.clone()),
            sha256: sha256.to_vec(),
        };
        let mut client = Self::ends(&call);
        self.runtime
            .block_on(client.have_chunks(Request::new(request)))
            .map(|answer| answer.into_inner().sha256)
            .map_err(|status| self.failed(&call, &status, "look at", folder))
    }

    fn make_folder(&self, path: &Path, mode: u32) -> Result<(), EndError> {
        let call = self.call(path)?;
        let request = pb::MakeFolderRequest {
            location: Some(call.location.clone()),
            mode,
        };
        let mut client = Self::ends(&call);
        self.runtime
            .block_on(client.make_folder(Request::new(request)))
            .map(|_| ())
            .map_err(|status| self.failed(&call, &status, "create", path))
    }

    fn make_link(&self, path: &Path, target: &Path) -> Result<(), EndError> {
        let call = self.call(path)?;
        let request = pb::MakeLinkRequest {
            location: Some(call.location.clone()),
            target: target.to_string_lossy().into_owned(),
        };
        let mut client = Self::ends(&call);
        self.runtime
            .block_on(client.make_link(Request::new(request)))
            .map(|_| ())
            .map_err(|status| self.failed(&call, &status, "create the link", path))
    }

    fn open_write(&self, path: &Path, offset: u64) -> Result<Box<dyn Sink>, EndError> {
        let call = self.call(path)?;
        let codec = self.compression.pick(REMOTE, &self.codecs());
        let mut sink = RemoteSink {
            runtime: self.runtime.clone(),
            shared: self.shared.clone(),
            compression: self.compression.clone(),
            path: path.to_owned(),
            call,
            encoder: Encoder::new(codec),
            start: offset,
            sent: 0,
            open: None,
        };
        sink.open_stream()?;
        Ok(Box::new(sink))
    }

    fn hash(&self, path: &Path, len: u64) -> Result<[u8; 32], EndError> {
        let call = self.call(path)?;
        let request = pb::HashPrefixRequest {
            location: Some(call.location.clone()),
            bytes: len,
        };
        let mut client = Self::ends(&call);
        let answer = self
            .runtime
            .block_on(client.hash_prefix(Request::new(request)))
            .map_err(|status| self.failed(&call, &status, "check", path))?
            .into_inner();
        unhex(&answer.sha256)
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .ok_or_else(|| EndError::Io {
                doing: "check",
                path: path.to_owned(),
                source: io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} answered a hash that isn't 32 bytes of hex", call.machine),
                ),
            })
    }

    fn finish_file(&self, path: &Path, mode: u32, modified_unix_ms: i64) -> Result<(), EndError> {
        let call = self.call(path)?;
        let request = pb::FinishFileRequest {
            location: Some(call.location.clone()),
            mode,
            modified_at: Some(timestamp_of(modified_unix_ms)),
        };
        let mut client = Self::ends(&call);
        self.runtime
            .block_on(client.finish_file(Request::new(request)))
            .map(|_| ())
            .map_err(|status| self.failed(&call, &status, "finish", path))
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), EndError> {
        let call = self.call(from)?;
        if !Self::same_root(from, to) {
            return Err(EndError::Io {
                doing: "move",
                path: from.to_owned(),
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a rename stays in one root of one machine",
                ),
            });
        }
        // On the machine the location is its own, as in `call`.
        let target = parse(to).map(|(_, mut location)| {
            location.machine = None;
            location
        });
        let request = pb::MoveEntryRequest {
            from: Some(call.location.clone()),
            to: target,
        };
        let mut client = Self::ends(&call);
        self.runtime
            .block_on(client.move_entry(Request::new(request)))
            .map(|_| ())
            .map_err(|status| match self.failed(&call, &status, "move", from) {
                // The name taken is the destination's.
                EndError::Exists { .. } => EndError::Exists { path: to.to_owned() },
                other => other,
            })
    }

    fn sync_folder(&self, folder: &Path) -> Result<(), EndError> {
        let call = self.call(folder)?;
        let request = pb::EndPathRequest {
            location: Some(call.location.clone()),
        };
        let mut client = Self::ends(&call);
        self.runtime
            .block_on(client.sync_folder(Request::new(request)))
            .map(|_| ())
            .map_err(|status| self.failed(&call, &status, "save", folder))
    }

    fn remove_tree(&self, path: &Path) -> Result<(), EndError> {
        let call = self.call(path)?;
        let request = pb::EndPathRequest {
            location: Some(call.location.clone()),
        };
        let mut client = Self::ends(&call);
        self.runtime
            .block_on(client.remove_staged(Request::new(request)))
            .map(|_| ())
            .map_err(|status| self.failed(&call, &status, "remove", path))
    }

    fn trash(&self, path: &Path, description: &str, now_unix_ms: i64) -> Result<(), EndError> {
        let call = self.call(path)?;
        // The same move asked again (a retry after a cut link) is one move.
        let key = format!("transfer-trash:{}:{now_unix_ms}:{description}", path.display());
        let request = pb::MoveToTrashRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: key }),
            locations: vec![call.location.clone()],
        };
        let mut client = Self::files(&call);
        self.runtime
            .block_on(client.move_to_trash(Request::new(request)))
            .map(|_| ())
            .map_err(|status| match self.failed(&call, &status, "move to the trash", path) {
                EndError::NotFound { .. } => EndError::Trash {
                    path: path.to_owned(),
                    source: TrashError::Missing { path: path.to_owned() },
                },
                EndError::Io { path, source, .. } => EndError::Trash {
                    path: path.clone(),
                    source: TrashError::Io {
                        doing: "move to the trash",
                        path,
                        source,
                    },
                },
                other => other,
            })
    }
}

/// A file being read from the machine, one chunk in memory at a time.
struct RemoteRead {
    runtime: Handle,
    shared: Arc<Shared>,
    generation: u64,
    machine: String,
    stream: Streaming<pb::FileChunk>,
    decoder: Decoder,
    buffer: Vec<u8>,
    at: usize,
    ended: bool,
}

impl RemoteRead {
    /// The error a read reports: a cut link carries [`EndError::Offline`]
    /// inside so the engine can tell it from a bad read.
    fn broken(&self, status: &Status) -> io::Error {
        let down = status.code() == Code::Unavailable || self.shared.link_down(&self.machine);
        match down {
            true => io::Error::other(self.shared.offline(&self.machine, self.generation)),
            false => io::Error::other(status::chain(status)),
        }
    }

    /// Fills the buffer with the next chunk; false at the end.
    fn refill(&mut self) -> io::Result<bool> {
        loop {
            let next = self.runtime.block_on(self.stream.message());
            let chunk = match next {
                Ok(Some(chunk)) => chunk,
                Ok(None) => return Ok(false),
                Err(status) => return Err(self.broken(&status)),
            };
            let header = compression::chunk_header(&chunk).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} sent a chunk in a codec this build doesn't know", self.machine),
                )
            })?;
            let last = chunk.last;
            self.shared.received.fetch_add(chunk.data.len() as u64, Ordering::Relaxed);
            self.buffer = self.decoder.decode(header, chunk.data).map_err(io::Error::other)?;
            self.at = 0;
            if !self.buffer.is_empty() {
                return Ok(true);
            }
            if last {
                return Ok(false);
            }
        }
    }
}

impl Read for RemoteRead {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.ended {
            return Ok(0);
        }
        if self.at >= self.buffer.len() && !self.refill()? {
            self.ended = true;
            return Ok(0);
        }
        let n = out.len().min(self.buffer.len() - self.at);
        out[..n].copy_from_slice(&self.buffer[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// One open `WriteAt` stream.
struct OpenStream {
    chunks: mpsc::Sender<pb::WriteAtMessage>,
    reply: JoinHandle<Result<Response<pb::WriteAtResponse>, Status>>,
    started: Instant,
    waited: Duration,
    wire_bytes: u64,
}

/// A file being written on the machine: chunks go into one `WriteAt` stream
/// through a small queue, and `sync` closes the stream, which the machine
/// answers with how far it has the file on disk.
struct RemoteSink {
    runtime: Handle,
    shared: Arc<Shared>,
    compression: Arc<Compression>,
    call: Call,
    path: PathBuf,
    encoder: Encoder,
    /// Where the current (or next) stream starts in the file.
    start: u64,
    /// Bytes sent in the current stream.
    sent: u64,
    open: Option<OpenStream>,
}

impl RemoteSink {
    fn error(&self, status: &Status) -> EndError {
        failure(&self.shared, self.call.generation, status, &self.call.machine, "write", &self.path)
    }

    fn io(&self, message: String) -> EndError {
        EndError::Io {
            doing: "write",
            path: self.path.clone(),
            source: io::Error::other(message),
        }
    }

    /// Opens a stream at `start` with its header sent.
    fn open_stream(&mut self) -> Result<(), EndError> {
        let (chunks, queue) = mpsc::channel(QUEUE);
        let mut client = pb::transfer_end_service_client::TransferEndServiceClient::new(self.call.channel.clone());
        let reply = self
            .runtime
            .spawn(async move { client.write_at(Request::new(ReceiverStream::new(queue))).await });
        let header = pb::WriteAtMessage {
            message: Some(pb::write_at_message::Message::Header(pb::WriteAtHeader {
                location: Some(self.call.location.clone()),
                offset: self.start,
            })),
        };
        self.sent = 0;
        let opened = OpenStream {
            chunks,
            reply,
            started: Instant::now(),
            waited: Duration::ZERO,
            wire_bytes: 0,
        };
        let sent = opened.chunks.blocking_send(header).is_ok();
        self.open = Some(opened);
        if !sent {
            return Err(self.ended_early());
        }
        Ok(())
    }

    /// The stream is gone before the engine closed it: what the machine
    /// answered says why.
    fn ended_early(&mut self) -> EndError {
        let Some(open) = self.open.take() else {
            return self.io("the stream to the machine was closed".to_owned());
        };
        drop(open.chunks);
        match self.runtime.block_on(open.reply) {
            Ok(Err(status)) => self.error(&status),
            Ok(Ok(_)) => self.io("the machine closed the stream before the file was sent".to_owned()),
            Err(error) => self.io(format!("the write to the machine stopped: {error}")),
        }
    }
}

impl Sink for RemoteSink {
    fn write(&mut self, bytes: &[u8]) -> Result<(), EndError> {
        for piece in bytes.chunks(CHUNK_MAX) {
            if self.open.is_none() {
                self.open_stream()?;
            }
            let chunk = self
                .encoder
                .encode(piece.to_vec())
                .map_err(|error| self.io(status::chain(&error)))?;
            self.shared.sent.fetch_add(chunk.data.len() as u64, Ordering::Relaxed);
            let message = pb::WriteAtMessage {
                message: Some(pb::write_at_message::Message::Chunk(pb::FileChunk {
                    offset: self.start + self.sent,
                    sha256: String::new(),
                    last: false,
                    codec: chunk.header.codec.wire(),
                    raw_size: chunk.header.raw_len,
                    crc32: chunk.header.crc32,
                    data: chunk.data,
                })),
            };
            let size = piece.len() as u64;
            let Some(open) = self.open.as_mut() else {
                return Err(self.io("the stream to the machine was closed".to_owned()));
            };
            open.wire_bytes += message_len(&message);
            let handing = Instant::now();
            let sent = open.chunks.blocking_send(message).is_ok();
            open.waited += handing.elapsed();
            if !sent {
                return Err(self.ended_early());
            }
            self.sent += size;
        }
        Ok(())
    }

    fn sync(&mut self) -> Result<(), EndError> {
        let Some(open) = self.open.take() else {
            return Ok(());
        };
        let OpenStream {
            chunks,
            reply,
            started,
            waited,
            wire_bytes,
        } = open;
        drop(chunks);
        let answer = match self.runtime.block_on(reply) {
            Ok(Ok(answer)) => answer.into_inner(),
            Ok(Err(status)) => return Err(self.error(&status)),
            Err(error) => return Err(self.io(format!("the write to the machine stopped: {error}"))),
        };
        let expected = self.start + self.sent;
        if answer.confirmed_offset != expected {
            return Err(self.io(format!(
                "{} confirmed {} bytes on disk, but {} were sent (from byte {}); resume the transfer to check what is there",
                self.call.machine, answer.confirmed_offset, expected, self.start
            )));
        }
        self.compression.record(
            REMOTE,
            Sample {
                bytes: wire_bytes,
                elapsed: started.elapsed(),
                waited,
            },
        );
        self.start = expected;
        self.sent = 0;
        Ok(())
    }
}

fn message_len(message: &pb::WriteAtMessage) -> u64 {
    message.encoded_len() as u64
}

/// Sends each call to the other machine when the path is a virtual one and to
/// this machine's disk otherwise.
pub(crate) struct RouterEnd {
    local: Arc<dyn End>,
    remote: Arc<RemoteEnd>,
    /// What a plan leaves out while a move copies a session (MV-17).
    skipped: Mutex<HashMap<PathBuf, Skipped>>,
}

/// Files, and whole folders, a plan leaves out.
#[derive(Default)]
struct Skipped {
    files: HashSet<PathBuf>,
    trees: Vec<PathBuf>,
}

impl RouterEnd {
    pub(crate) fn new(local: Arc<dyn End>, remote: Arc<RemoteEnd>) -> Self {
        Self {
            local,
            remote,
            skipped: Mutex::default(),
        }
    }

    /// From now until [`RouterEnd::unskip`], a plan of `source` leaves out
    /// `files` and everything under `trees`.
    pub(crate) fn skip(&self, source: PathBuf, files: Vec<PathBuf>, trees: Vec<PathBuf>) {
        let skipped = Skipped {
            files: files.into_iter().collect(),
            trees,
        };
        self.skipped.lock().unwrap_or_else(PoisonError::into_inner).insert(source, skipped);
    }

    pub(crate) fn unskip(&self, source: &Path) {
        self.skipped.lock().unwrap_or_else(PoisonError::into_inner).remove(source);
    }

    fn pick(&self, path: &Path) -> &dyn End {
        match is_virtual(path) {
            true => &*self.remote,
            false => &*self.local,
        }
    }
}

impl End for RouterEnd {
    fn label(&self) -> &str {
        self.local.label()
    }

    fn same_disk(&self, other: &dyn End, from: &Path, to_folder: &Path) -> bool {
        match (is_virtual(from), is_virtual(to_folder)) {
            (false, false) => self.local.same_disk(&*self.local, from, to_folder),
            (true, true) => self.remote.same_disk(other, from, to_folder),
            _ => false,
        }
    }

    fn plan(&self, source: &Path) -> Result<Vec<Entry>, EndError> {
        let entries = self.pick(source).plan(source)?;
        let held = self.skipped.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(skipped) = held.get(source) else {
            return Ok(entries);
        };
        Ok(entries
            .into_iter()
            .filter(|entry| {
                if entry.rel.is_empty() {
                    return true;
                }
                let path = source.join(&entry.rel);
                !skipped.files.contains(&path) && !skipped.trees.iter().any(|tree| path.starts_with(tree))
            })
            .collect())
    }

    fn open_read(&self, path: &Path, offset: u64) -> Result<Box<dyn Read + Send>, EndError> {
        self.pick(path).open_read(path, offset)
    }

    fn free_bytes(&self, folder: &Path) -> Result<u64, EndError> {
        self.pick(folder).free_bytes(folder)
    }

    fn exists(&self, path: &Path) -> Result<bool, EndError> {
        self.pick(path).exists(path)
    }

    fn have_chunks(&self, folder: &Path, sha256: &[String]) -> Result<Vec<String>, EndError> {
        self.pick(folder).have_chunks(folder, sha256)
    }

    fn make_folder(&self, path: &Path, mode: u32) -> Result<(), EndError> {
        self.pick(path).make_folder(path, mode)
    }

    fn make_link(&self, path: &Path, target: &Path) -> Result<(), EndError> {
        self.pick(path).make_link(path, target)
    }

    fn open_write(&self, path: &Path, offset: u64) -> Result<Box<dyn Sink>, EndError> {
        self.pick(path).open_write(path, offset)
    }

    fn hash(&self, path: &Path, len: u64) -> Result<[u8; 32], EndError> {
        self.pick(path).hash(path, len)
    }

    fn finish_file(&self, path: &Path, mode: u32, modified_unix_ms: i64) -> Result<(), EndError> {
        self.pick(path).finish_file(path, mode, modified_unix_ms)
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), EndError> {
        self.pick(from).rename(from, to)
    }

    fn sync_folder(&self, folder: &Path) -> Result<(), EndError> {
        self.pick(folder).sync_folder(folder)
    }

    fn remove_tree(&self, path: &Path) -> Result<(), EndError> {
        self.pick(path).remove_tree(path)
    }

    fn trash(&self, path: &Path, description: &str, now_unix_ms: i64) -> Result<(), EndError> {
        self.pick(path).trash(path, description, now_unix_ms)
    }
}

/// Calls `on_up(machine)` each time a machine's link comes up, including
/// links already up when it starts, until the registry is gone. Nothing
/// polls: it follows the links' pushed changes (TR-7). `on_up` runs on the
/// runtime, so it must not block; hand blocking work (the engine's resume
/// writes a journal) to `spawn_blocking`.
pub(crate) async fn resume_when_back(links: Links, on_up: Arc<dyn Fn(&str) + Send + Sync>) {
    // Follow first, so a link that comes up while the states are read isn't
    // missed; a machine told of twice is harmless.
    let mut follow = links.watch_all();
    for (machine, state) in links.states() {
        if matches!(state, LinkState::Up { .. }) {
            on_up(&machine);
        }
    }
    while let Some((machine, state)) = follow.next().await {
        if matches!(state, LinkState::Up { .. }) {
            on_up(&machine);
        }
    }
}

/// The engine's path for a source: the disk path on this machine, the
/// virtual path on another.
pub(crate) fn source_path(placed: &Placed, here: &str) -> Result<PathBuf, Status> {
    match placed.machine() == here {
        true => placed.clone().into_target().unfollowed(),
        false => Ok(virtual_path(placed.machine(), &placed.location)),
    }
}

impl RemoteEnd {
    /// Where `location`, a file in a session's pastes on another machine, is
    /// on that machine's disk (D335). Blocks: call it off the runtime.
    pub(crate) fn locate(&self, location: &pb::FileLocation) -> Result<String, Status> {
        let machine = location.machine.as_ref().map_or("", |machine| machine.value.as_str());
        let call = match self.call(&virtual_path(machine, location)) {
            Ok(call) => call,
            Err(EndError::Offline { machine: name }) => {
                return Err(status::failure(
                    Code::Unavailable,
                    status::error(
                        format!("{name} went out of reach before it said where the pasted file is"),
                        "the file is in the session's pastes there; paste it again once the machine is back",
                        pb::error::Kind::MachineOffline(pb::MachineOffline {
                            machine: Some(pb::MachineId { value: name.clone() }),
                            machine_name: name,
                            since: None,
                        }),
                    ),
                ));
            }
            Err(error) => return Err(status::internal(status::chain(&error), "")),
        };
        let request = pb::EndPathRequest {
            location: Some(call.location.clone()),
        };
        let mut client = Self::ends(&call);
        self.runtime
            .block_on(client.locate(Request::new(request)))
            .map(|answer| answer.into_inner().path)
    }

    /// The engine's path for a destination folder on another machine,
    /// which must exist there. A machine that can't be asked (its link is
    /// down) is trusted: the transfer is accepted and waits for it.
    pub(crate) fn folder(&self, location: &pb::FileLocation, field: &str) -> Result<PathBuf, Status> {
        let machine = location.machine.as_ref().map_or("", |machine| machine.value.as_str());
        let path = virtual_path(machine, location);
        match self.exists(&path) {
            Ok(true) | Err(EndError::Offline { .. }) => Ok(path),
            Ok(false) => Err(status::invalid_field(
                field,
                pb::invalid_field::Problem::NotAllowedHere,
                format!("there is no folder {:?} on {machine}", location.path),
                "name a folder that exists on that machine, or make it there first",
            )),
            Err(error) => Err(status::internal(
                format!("couldn't look for {:?} on {machine}: {}", location.path, status::chain(&error)),
                "",
            )),
        }
    }
}

/// Starts what resumes transfers that wait for a machine: on each link-up
/// (`resume_machine(machine)`), and for a failure that settles after its
/// link is already back (`resume_one(id)`). Inside the runtime.
pub(crate) fn follow_links(
    links: Links,
    remote: Arc<RemoteEnd>,
    changes: broadcast::Receiver<Snapshot>,
    resume_machine: Arc<dyn Fn(&str) + Send + Sync>,
    resume_one: Arc<dyn Fn(&str) + Send + Sync>,
) {
    remote.use_links(links.clone());
    drop(tokio::spawn(resume_when_back(links, resume_machine)));
    drop(tokio::spawn(resume_if_already_back(remote, changes, resume_one)));
}

/// The machine a transfer stopped for: Failed in the engine with an offline
/// problem. Shown as waiting for the machine.
pub(crate) fn waiting_for(snapshot: &Snapshot) -> Option<&str> {
    match (&snapshot.state, &snapshot.problem) {
        (State::Failed, Some(Problem::Offline { machine })) => Some(machine),
        _ => None,
    }
}

/// Calls `resume(id)` for a transfer that just failed offline when its
/// machine's link is already a newer one than the link the failure was on:
/// the link-up was announced before the failure settled, so
/// [`resume_when_back`] had nothing to resume then. Driven by the engine's
/// own change feed; no timer, and no loop, because a link that keeps failing
/// is never newer than its own failure ([`RemoteEnd::relinked_since_failure`]).
pub(crate) async fn resume_if_already_back(
    remote: Arc<RemoteEnd>,
    mut changes: broadcast::Receiver<Snapshot>,
    resume: Arc<dyn Fn(&str) + Send + Sync>,
) {
    loop {
        match changes.recv().await {
            Ok(snapshot) => {
                if waiting_for(&snapshot).is_some_and(|machine| remote.relinked_since_failure(machine)) {
                    resume(&snapshot.id);
                }
            }
            // Missed some: the next change or link-up catches up.
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn clone_of(session: &str) -> pb::FileLocation {
        pb::FileLocation {
            machine: None,
            root: Some(pb::file_location::Root::SessionClone(pb::SessionClone {
                session: Some(pb::SessionId { value: session.to_owned() }),
                repo: Some(pb::RepoRef {
                    owner: "acme".to_owned(),
                    name: "api".to_owned(),
                }),
            })),
            path: String::new(),
        }
    }

    fn at(location: pb::FileLocation, path: &str) -> pb::FileLocation {
        pb::FileLocation {
            path: path.to_owned(),
            ..location
        }
    }

    fn named(machine: &str, location: pb::FileLocation) -> pb::FileLocation {
        pb::FileLocation {
            machine: Some(pb::MachineId { value: machine.to_owned() }),
            ..location
        }
    }

    #[test]
    fn a_location_comes_back_from_its_virtual_path() {
        for path in ["", "docs", "docs/a b/naïve ファイル.txt"] {
            let location = at(clone_of("se_1"), path);
            let virtual_path = virtual_path("box-1", &location);
            assert!(is_virtual(&virtual_path));
            let (machine, back) = parse(&virtual_path).expect("a virtual path parses");
            assert_eq!(machine, "box-1");
            assert_eq!(back, named("box-1", location));
        }
    }

    #[test]
    fn the_root_is_the_path_with_no_tail_and_children_extend_it() {
        let root = virtual_path("box-1", &clone_of("se_1"));
        let child = virtual_path("box-1", &at(clone_of("se_1"), "a/b"));
        assert_eq!(child, root.join("a").join("b"));
        assert_eq!(parse(&root).map(|(_, location)| location.path), Some(String::new()));
    }

    #[test]
    fn roots_and_machines_keep_paths_apart() {
        let one = virtual_path("box-1", &at(clone_of("se_1"), "a"));
        assert_ne!(one, virtual_path("box-2", &at(clone_of("se_1"), "a")));
        assert_ne!(one, virtual_path("box-1", &at(clone_of("se_2"), "a")));
    }

    #[test]
    fn what_is_not_a_virtual_path_does_not_parse() {
        for path in ["/home/me/file", "relative/@humata/x", "/@humata", "/@humata/box-1", "/@humata/box-1/zz"] {
            assert!(parse(Path::new(path)).is_none(), "{path}");
        }
        assert!(!is_virtual(Path::new("/home/me/@humata")));
    }

    fn end(links: &Links) -> (tokio::runtime::Runtime, RemoteEnd) {
        let runtime = tokio::runtime::Runtime::new().expect("a runtime starts");
        let dir = tempfile::tempdir().expect("a folder");
        let end = RemoteEnd::new(runtime.handle().clone(), Arc::new(Compression::new(dir.path())));
        end.use_links(links.clone());
        (runtime, end)
    }

    #[test]
    fn a_machine_with_no_link_is_offline_for_every_step() {
        let (_runtime, end) = end(&Links::default());
        let path = virtual_path("box-1", &at(clone_of("se_1"), "a"));
        let offline = |result: Result<(), EndError>| {
            matches!(result, Err(EndError::Offline { machine }) if machine == "box-1")
        };
        assert!(matches!(end.plan(&path), Err(EndError::Offline { .. })));
        assert!(matches!(end.exists(&path), Err(EndError::Offline { .. })));
        assert!(matches!(end.free_bytes(&path), Err(EndError::Offline { .. })));
        assert!(matches!(end.hash(&path, 3), Err(EndError::Offline { .. })));
        assert!(matches!(end.open_write(&path, 0), Err(EndError::Offline { .. })));
        assert!(matches!(end.open_read(&path, 0), Err(EndError::Offline { .. })));
        assert!(offline(end.make_folder(&path, 0o755)));
        assert!(offline(end.remove_tree(&path)));
        assert!(offline(end.trash(&path, "moved", 1)));
    }

    #[test]
    fn a_path_that_is_not_virtual_is_refused_not_sent_anywhere() {
        let (_runtime, end) = end(&Links::default());
        assert!(matches!(end.exists(Path::new("/tmp/x")), Err(EndError::Io { .. })));
    }

    #[test]
    fn statuses_map_back_to_the_engines_errors() {
        let shared = Shared::default();
        let path = Path::new("/@humata/box-1/aa/x");
        let make = |code| failure(&shared, 0, &Status::new(code, "said so"), "box-1", "write", path);
        assert!(matches!(make(Code::NotFound), EndError::NotFound { .. }));
        assert!(matches!(make(Code::AlreadyExists), EndError::Exists { .. }));
        assert!(matches!(make(Code::Unavailable), EndError::Offline { .. }));
        // A cut link shows as an unknown error while no link is up.
        assert!(matches!(make(Code::Unknown), EndError::Offline { .. }));
        assert!(matches!(make(Code::PermissionDenied), EndError::Io { .. }));
        assert!(matches!(make(Code::InvalidArgument), EndError::Io { .. }));
    }

    #[tokio::test]
    async fn a_broken_connection_is_offline_even_before_the_link_is_marked_down() {
        // The call hears the broken connection before the supervisor marks
        // the link down: the links still say Up.
        let links = Links::default();
        let shared = Shared::default();
        let _ = shared.links.set(links.clone());
        let channel = tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        let generation = links.up("box-1", channel);
        let path = Path::new("/@humata/box-1/aa/x");
        let broken = Status::from_error(Box::new(io::Error::new(io::ErrorKind::BrokenPipe, "connection reset")));
        assert!(
            matches!(failure(&shared, generation, &broken, "box-1", "write", path), EndError::Offline { .. }),
            "a transport failure is the link, so the transfer waits for the machine"
        );
        // What the machine itself answered stays its answer.
        let said = Status::new(Code::Internal, "the disk said no");
        assert!(matches!(failure(&shared, generation, &said, "box-1", "write", path), EndError::Io { .. }));
    }

    #[test]
    fn a_move_stays_one_rename_only_inside_one_root_of_one_machine() {
        let a = virtual_path("box-1", &at(clone_of("se_1"), "a/f"));
        let folder = virtual_path("box-1", &at(clone_of("se_1"), "b"));
        let other_root = virtual_path("box-1", &at(clone_of("se_2"), "b"));
        let other_machine = virtual_path("box-2", &at(clone_of("se_1"), "b"));
        assert!(RemoteEnd::same_root(&a, &folder));
        assert!(!RemoteEnd::same_root(&a, &other_root));
        assert!(!RemoteEnd::same_root(&a, &other_machine));
        assert!(!RemoteEnd::same_root(&a, Path::new("/tmp")));
    }

    #[tokio::test]
    async fn a_failure_is_resumed_only_by_a_link_newer_than_the_one_it_failed_on() {
        let links = Links::default();
        let shared = Shared::default();
        let _ = shared.links.set(links.clone());
        let channel = tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        // Down: nothing to resume on yet.
        assert!(!shared.newer_link("box-1"));
        // The machine's first link comes up and a call on it fails offline.
        let first = links.up("box-1", channel.clone());
        let _ = shared.offline("box-1", first);
        assert!(!shared.newer_link("box-1"), "the same link, failing again, must not loop");
        links.down("box-1");
        assert!(!shared.newer_link("box-1"));
        links.up("box-1", channel);
        assert!(shared.newer_link("box-1"), "a newer link may go on");
        // A failure with no link at all (generation 0) is resumed by any link.
        assert!(shared.newer_link("box-1"));
        assert!(!shared.newer_link("box-2"));
    }

    #[tokio::test]
    async fn every_link_that_comes_up_is_reported_once_and_downs_are_not() {
        let links = Links::default();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        let task = tokio::spawn(resume_when_back(
            links.clone(),
            Arc::new(move |machine| sink.lock().expect("the list").push(machine.to_owned())),
        ));
        // On this one-thread runtime the task runs only when the test
        // yields: let it subscribe before the links change, so every change
        // is one it follows rather than a state it reads at the start.
        tokio::task::yield_now().await;
        let channel = tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        links.up("box-1", channel.clone());
        links.down("box-1");
        links.up("box-1", channel.clone());
        links.up("box-2", channel);
        for _ in 0..200 {
            if heard.lock().expect("the list").len() >= 3 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(*heard.lock().expect("the list"), ["box-1", "box-1", "box-2"]);
        task.abort();
    }
}
