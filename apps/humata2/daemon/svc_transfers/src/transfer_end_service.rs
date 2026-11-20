//! The machine's half of a transfer the host drives (FM-6, FM-8, FM-9,
//! D142): each step the transfer engine takes on a local disk through its
//! `End`, done here for a host that has the engine on another machine.
//! Every handler does what [`Disk`] does for the same step; this is only
//! the gRPC face: who may call, where the paths may point, and how a
//! failure is told.
//!
//! It also says where a file in a session's pastes is on this disk, so the
//! host can tell the app the path to type (D335); no other path is told.
//!
//! For the user and the host's relayed calls (taken as the user's); an
//! agent, a script or a peer is refused. Every location is still judged by
//! the guard like any `FileService` call, and resolved through [`Roots`] so
//! nothing leaves its root.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use humata2_access::{Call, Entry, Refusal};
use humata2_chunk::{ChunkError, Decoder};
use humata2_grpc::humata::v1 as pb;
use humata2_transfers::{Disk, End, EndError, Entry as Planned, Kind, Sink};
use pb::invalid_field::Problem;
use tokio_stream::Stream;
use tonic::{Code, Request, Response, Status, Streaming};

use crate::ports::{
    Caller, RelayError, Roots, Target, TransferGuardPort, TrashBinPort, blocking, caller, chain,
    compression, error, failure, internal, invalid_field, missing, not_found, refused, timestamp,
};

/// What a transfer names its hidden staging folder (`.humata-transfer-<id>`).
const STAGING_PREFIX: &str = ".humata-transfer-";

pub type PlanTreeStream = Pin<Box<dyn Stream<Item = Result<pb::TreeEntry, Status>> + Send>>;

pub struct TransferEndSvc {
    roots: Arc<Roots>,
    guard: Arc<dyn TransferGuardPort>,
    bin: Arc<dyn TrashBinPort>,
    machine: String,
}

impl TransferEndSvc {
    /// Never fails and touches no disk.
    pub fn new(
        roots: Arc<Roots>,
        guard: Arc<dyn TransferGuardPort>,
        bin: Arc<dyn TrashBinPort>,
        machine: String,
    ) -> Self {
        Self {
            roots,
            guard,
            bin,
            machine,
        }
    }

    /// The user, which includes the host's relayed calls. Everyone else is
    /// refused as for the trash's user-only calls.
    fn user_only(caller: &Caller) -> Result<(), Status> {
        match caller {
            Caller::User(_) => Ok(()),
            Caller::Peer => Err(refused(&Refusal::PeerCannotCall)),
            Caller::Agent(_) | Caller::Script(_) => {
                Err(refused(&Refusal::UserOnly { capability: None }))
            }
        }
    }

    /// The location `field` names, judged for reading (`write` false) or
    /// for changing it (a main clone is refused).
    fn target(
        &self,
        caller: &Caller,
        location: Option<pb::FileLocation>,
        field: &str,
        entry: Entry,
        write: bool,
    ) -> Result<Target, Status> {
        let target = self.roots.target(location, field, entry)?;
        if write {
            self.guard
                .require(caller, &Call::WriteFiles(target.place.clone()))?;
            target.require_writable(field)?;
        } else {
            self.guard
                .require(caller, &Call::ReadFiles(target.place.clone()))?;
        }
        Ok(target)
    }

    /// Runs `work` on a blocking thread with this machine's disk.
    async fn on_disk<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Disk) -> Result<T, Status> + Send + 'static,
    ) -> Result<T, Status> {
        let bin = self.bin.clone();
        let machine = self.machine.clone();
        blocking(move || {
            let disk = Disk::new(machine, bin.get()?);
            work(&disk)
        })
        .await?
    }
}

/// `path` as text to type into a terminal.
pub fn path_text(path: &Path) -> Result<String, Status> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        internal(
            format!("the path {} isn't UTF-8, so it can't be typed", path.display()),
            "",
        )
    })
}

/// The path of an entry that may or may not be there: a link or file that
/// is, as itself; else where it would be made (its folder must exist).
fn entry_path(target: &Target) -> Result<PathBuf, Status> {
    match target.unfollowed() {
        Ok(path) => Ok(path),
        Err(err) if err.code() == Code::NotFound => target.resolve(false),
        Err(err) => Err(err),
    }
}

fn free_near(path: &Path) -> u64 {
    let folder = path.parent().unwrap_or(path);
    rustix::fs::statvfs(folder)
        .map(|stat| stat.f_bavail.saturating_mul(stat.f_frsize))
        .unwrap_or(0)
}

/// A step's failure, said in the API's terms.
fn end_failure(err: EndError, machine: &str, field: &str, shown: &str) -> Status {
    match &err {
        EndError::NotFound { .. } => not_found(
            pb::EntityKind::File,
            shown,
            format!("there is no {shown} there"),
            "check the path; if a transfer was cancelled meanwhile, start it again",
        ),
        EndError::Exists { .. } => failure(
            Code::AlreadyExists,
            error(
                format!("{shown} already exists"),
                "pick another name, or choose to replace what is there",
                pb::error::Kind::NameTaken(pb::NameTaken::default()),
            ),
        ),
        EndError::Unsupported { path, what } => invalid_field(
            field,
            Problem::NotAllowedHere,
            format!("{} is a {what}, which can't be transferred", path.display()),
            "leave it out and transfer the rest",
        ),
        EndError::Io { path, source, .. } if source.kind() == std::io::ErrorKind::StorageFull => {
            failure(
                Code::ResourceExhausted,
                error(
                    format!("the disk is full: {}", chain(&err)),
                    "free space on this machine (empty the trash; deleting clones with unpushed work loses that work), then resume the transfer. What was copied so far is kept",
                    pb::error::Kind::DiskFull(pb::DiskFull {
                        machine: Some(pb::MachineId {
                            value: machine.to_owned(),
                        }),
                        path: path.display().to_string(),
                        needed_bytes: 0,
                        free_bytes: free_near(path),
                    }),
                ),
            )
        }
        // A machine further on (another machine's end, reached from here)
        // is down: said as that machine being offline, so the caller waits.
        EndError::Offline { machine } => RelayError::Offline {
            machine: machine.clone(),
        }
        .into(),
        EndError::Io { .. } | EndError::Trash { .. } => {
            internal(format!("couldn't do that to {shown}: {}", chain(&err)), "")
        }
    }
}

fn kind_of(kind: Kind) -> pb::file_entry::Kind {
    match kind {
        Kind::File => pb::file_entry::Kind::File,
        Kind::Folder => pb::file_entry::Kind::Folder,
        Kind::Link => pb::file_entry::Kind::Symlink,
    }
}

fn tree_entry(entry: Planned) -> pb::TreeEntry {
    let modified = UNIX_EPOCH + Duration::from_millis(u64::try_from(entry.modified_unix_ms).unwrap_or(0));
    pb::TreeEntry {
        rel: entry.rel,
        kind: kind_of(entry.kind) as i32,
        size: entry.size,
        mode: entry.mode,
        modified_at: Some(timestamp(modified)),
        link_target: entry
            .link_target
            .map(|target| target.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn chunk_failure(err: &ChunkError, path: &str) -> Status {
    failure(
        Code::DataLoss,
        error(
            format!("a chunk of {path}: {}", chain(err)),
            "send the file again from the confirmed offset",
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "chunk.data".to_owned(),
                problem: Problem::BadFormat as i32,
            }),
        ),
    )
}

/// An open file taking chunks in order.
struct Writer {
    sink: Box<dyn Sink>,
    decoder: Decoder,
    /// Where the next chunk must start: what is on disk, synced or not.
    offset: u64,
    machine: String,
    shown: String,
}

impl Writer {
    fn take(&mut self, chunk: pb::FileChunk) -> Result<(), Status> {
        let header = compression::chunk_header(&chunk).ok_or_else(|| {
            chunk_failure(&ChunkError::UnknownCodec { wire: chunk.codec }, &self.shown)
        })?;
        let raw = self
            .decoder
            .decode(header, chunk.data)
            .map_err(|err| chunk_failure(&err, &self.shown))?;
        self.sink
            .write(&raw)
            .map_err(|err| end_failure(err, &self.machine, "chunk", &self.shown))?;
        self.offset += raw.len() as u64;
        Ok(())
    }
}

/// Milliseconds since the epoch of `at`.
fn millis(at: &humata2_grpc::Timestamp) -> i64 {
    at.seconds
        .saturating_mul(1000)
        .saturating_add(i64::from(at.nanos) / 1_000_000)
}

#[tonic::async_trait]
impl pb::transfer_end_service_server::TransferEndService for TransferEndSvc {
    type PlanTreeStream = PlanTreeStream;

    /// The tree under a source, folders before what they hold. A link is
    /// reported, never followed.
    async fn plan_tree(
        &self,
        request: Request<pb::PlanTreeRequest>,
    ) -> Result<Response<Self::PlanTreeStream>, Status> {
        let who = caller(&request)?;
        Self::user_only(&who)?;
        let request = request.into_inner();
        let target = self.target(&who, request.source, "source", Entry::File, false)?;
        let machine = self.machine.clone();
        let entries = self
            .on_disk(move |disk| {
                let path = target.unfollowed()?;
                disk.plan(&path)
                    .map_err(|err| end_failure(err, &machine, "source", &target.shown))
            })
            .await?;
        let entries: Vec<Result<pb::TreeEntry, Status>> =
            entries.into_iter().map(|entry| Ok(tree_entry(entry))).collect();
        Ok(Response::new(Box::pin(tokio_stream::iter(entries))))
    }

    async fn free_space(
        &self,
        request: Request<pb::FreeSpaceRequest>,
    ) -> Result<Response<pb::FreeSpaceResponse>, Status> {
        let who = caller(&request)?;
        Self::user_only(&who)?;
        let request = request.into_inner();
        let target = self.target(&who, request.folder, "folder", Entry::Folder, true)?;
        let machine = self.machine.clone();
        let free_bytes = self
            .on_disk(move |disk| {
                let path = target.existing()?;
                disk.free_bytes(&path)
                    .map_err(|err| end_failure(err, &machine, "folder", &target.shown))
            })
            .await?;
        Ok(Response::new(pb::FreeSpaceResponse { free_bytes }))
    }

    /// True for anything at the path, a dangling link included.
    async fn exists(
        &self,
        request: Request<pb::EndPathRequest>,
    ) -> Result<Response<pb::ExistsResponse>, Status> {
        let who = caller(&request)?;
        Self::user_only(&who)?;
        let request = request.into_inner();
        let target = self.target(&who, request.location, "location", Entry::Folder, true)?;
        let machine = self.machine.clone();
        let exists = self
            .on_disk(move |disk| {
                if target.is_root() {
                    // The root is there when it can be opened.
                    return match target.existing() {
                        Ok(_) => Ok(true),
                        Err(err) if err.code() == Code::NotFound => Ok(false),
                        Err(err) => Err(err),
                    };
                }
                match target.unfollowed() {
                    Ok(path) => disk
                        .exists(&path)
                        .map_err(|err| end_failure(err, &machine, "location", &target.shown)),
                    Err(err) if err.code() == Code::NotFound => Ok(false),
                    Err(err) => Err(err),
                }
            })
            .await?;
        Ok(Response::new(pb::ExistsResponse { exists }))
    }

    /// Which of the named chunks (BLAKE3 hashes in hex) this machine's chunk
    /// store holds (MV-17). The location is a folder in the channel whose
    /// store is asked: the store is beside the channel folders, so one that
    /// isn't there, like a location that isn't, holds none.
    async fn have_chunks(
        &self,
        request: Request<pb::HaveChunksRequest>,
    ) -> Result<Response<pb::HaveChunksResponse>, Status> {
        let who = caller(&request)?;
        Self::user_only(&who)?;
        let request = request.into_inner();
        let target = self.target(&who, request.location, "location", Entry::Folder, true)?;
        let names = request.sha256;
        let sha256 = blocking(move || -> Result<Vec<String>, Status> {
            match target.unfollowed() {
                Ok(path) => Ok(path
                    .parent()
                    .map(|channel| humata2_sessions::held_chunks(&humata2_sessions::chunk_store_dir(channel), &names))
                    .unwrap_or_default()),
                Err(err) if err.code() == Code::NotFound => Ok(Vec::new()),
                Err(err) => Err(err),
            }
        })
        .await??;
        Ok(Response::new(pb::HaveChunksResponse { sha256 }))
    }

    async fn make_folder(
        &self,
        request: Request<pb::MakeFolderRequest>,
    ) -> Result<Response<pb::EndAck>, Status> {
        let who = caller(&request)?;
        Self::user_only(&who)?;
        let request = request.into_inner();
        let target = self.target(&who, request.location, "location", Entry::File, true)?;
        let mode = request.mode;
        let machine = self.machine.clone();
        self.on_disk(move |disk| {
            let path = target.resolve(false)?;
            disk.make_folder(&path, mode)
                .map_err(|err| end_failure(err, &machine, "location", &target.shown))
        })
        .await?;
        Ok(Response::new(pb::EndAck {}))
    }

    async fn make_link(
        &self,
        request: Request<pb::MakeLinkRequest>,
    ) -> Result<Response<pb::EndAck>, Status> {
        let who = caller(&request)?;
        Self::user_only(&who)?;
        let request = request.into_inner();
        let target = self.target(&who, request.location, "location", Entry::File, true)?;
        let link_to = PathBuf::from(request.target);
        let machine = self.machine.clone();
        self.on_disk(move |disk| {
            // A link made before a drop is left as it is, like the disk does.
            let path = entry_path(&target)?;
            disk.make_link(&path, &link_to)
                .map_err(|err| end_failure(err, &machine, "location", &target.shown))
        })
        .await?;
        Ok(Response::new(pb::EndAck {}))
    }

    /// The header, then chunks from its offset: the file is made if
    /// missing, cut to the offset, then appended to, and synced before the
    /// answer, which is the confirmed offset. A broken stream syncs what
    /// arrived before it fails, so nothing unsynced is ever counted.
    async fn write_at(
        &self,
        request: Request<Streaming<pb::WriteAtMessage>>,
    ) -> Result<Response<pb::WriteAtResponse>, Status> {
        use pb::write_at_message::Message as Part;

        let who = caller(&request)?;
        Self::user_only(&who)?;
        let mut messages = request.into_inner();
        let header = match messages.message().await? {
            Some(pb::WriteAtMessage {
                message: Some(Part::Header(header)),
            }) => header,
            _ => return Err(missing("header")),
        };
        let target = self.target(&who, header.location, "header.location", Entry::File, true)?;
        let shown = target.shown.clone();
        let offset = header.offset;
        let machine = self.machine.clone();
        let mut writer = self
            .on_disk({
                let shown = shown.clone();
                move |disk| {
                    let path = target.resolve(false)?;
                    let sink = disk.open_write(&path, offset).map_err(|err| match &err {
                        EndError::Io { source, .. }
                            if source.kind() == std::io::ErrorKind::UnexpectedEof =>
                        {
                            invalid_field(
                                "header.offset",
                                Problem::OutOfRange,
                                format!("{shown} is shorter than byte {offset}"),
                                "resume from the confirmed offset, which HashPrefix and the last answer give",
                            )
                        }
                        _ => end_failure(err, &machine, "header.location", &shown),
                    })?;
                    Ok(Writer {
                        sink,
                        decoder: Decoder::new(),
                        offset,
                        machine,
                        shown,
                    })
                }
            })
            .await?;

        let outcome: Result<(), Status> = loop {
            let chunk = match messages.message().await {
                Err(err) => break Err(err),
                Ok(None) => break Ok(()),
                Ok(Some(pb::WriteAtMessage {
                    message: Some(Part::Chunk(chunk)),
                })) => chunk,
                Ok(Some(_)) => {
                    break Err(invalid_field(
                        "header",
                        Problem::NotAllowedHere,
                        "the stream sent a second header",
                        "send one header, then the file's chunks",
                    ));
                }
            };
            if chunk.offset != writer.offset {
                break Err(invalid_field(
                    "chunk.offset",
                    Problem::OutOfRange,
                    format!(
                        "a chunk starts at byte {} of {shown}, but the next byte expected is {}",
                        chunk.offset, writer.offset
                    ),
                    format!("send the chunks in order, with no gaps, from byte {}", writer.offset),
                ));
            }
            let last = chunk.last;
            let (back, taken) = match blocking(move || {
                let taken = writer.take(chunk);
                (writer, taken)
            })
            .await
            {
                Ok(done) => done,
                Err(err) => return Err(err),
            };
            writer = back;
            if let Err(err) = taken {
                break Err(err);
            }
            if last {
                break Ok(());
            }
        };

        let (confirmed, synced) = blocking(move || {
            let synced = writer
                .sink
                .sync()
                .map_err(|err| end_failure(err, &writer.machine, "header.location", &writer.shown));
            (writer.offset, synced)
        })
        .await?;
        outcome?;
        synced?;
        Ok(Response::new(pb::WriteAtResponse {
            confirmed_offset: confirmed,
        }))
    }

    async fn hash_prefix(
        &self,
        request: Request<pb::HashPrefixRequest>,
    ) -> Result<Response<pb::HashPrefixResponse>, Status> {
        let who = caller(&request)?;
        Self::user_only(&who)?;
        let request = request.into_inner();
        let target = self.target(&who, request.location, "location", Entry::File, true)?;
        let bytes = request.bytes;
        let machine = self.machine.clone();
        let digest = self
            .on_disk(move |disk| {
                let path = target.existing()?;
                disk.hash(&path, bytes)
                    .map_err(|err| end_failure(err, &machine, "location", &target.shown))
            })
            .await?;
        Ok(Response::new(pb::HashPrefixResponse {
            sha256: hex(&digest),
        }))
    }

    async fn finish_file(
        &self,
        request: Request<pb::FinishFileRequest>,
    ) -> Result<Response<pb::EndAck>, Status> {
        let who = caller(&request)?;
        Self::user_only(&who)?;
        let request = request.into_inner();
        let target = self.target(&who, request.location, "location", Entry::File, true)?;
        let mode = request.mode;
        let modified = request
            .modified_at
            .as_ref()
            .map(millis)
            .ok_or_else(|| missing("modified_at"))?;
        let machine = self.machine.clone();
        self.on_disk(move |disk| {
            let path = target.existing()?;
            disk.finish_file(&path, mode, modified)
                .map_err(|err| end_failure(err, &machine, "location", &target.shown))
        })
        .await?;
        Ok(Response::new(pb::EndAck {}))
    }

    /// Renames within one root; fails with `name_taken` if `to` exists.
    async fn move_entry(
        &self,
        request: Request<pb::MoveEntryRequest>,
    ) -> Result<Response<pb::EndAck>, Status> {
        let who = caller(&request)?;
        Self::user_only(&who)?;
        let request = request.into_inner();
        let from = self.target(&who, request.from, "from", Entry::File, true)?;
        let to = self.target(&who, request.to, "to", Entry::File, true)?;
        if from.location.root != to.location.root {
            return Err(invalid_field(
                "to",
                Problem::NotAllowedHere,
                format!(
                    "{} and {} are in different folders, and a rename stays inside one",
                    from.shown, to.shown
                ),
                "name a place in the same root as `from`; copy between roots instead",
            ));
        }
        let machine = self.machine.clone();
        self.on_disk(move |disk| {
            let source = from.unfollowed()?;
            let place = entry_path(&to)?;
            disk.rename(&source, &place)
                .map_err(|err| end_failure(err, &machine, "to", &to.shown))
        })
        .await?;
        Ok(Response::new(pb::EndAck {}))
    }

    async fn sync_folder(
        &self,
        request: Request<pb::EndPathRequest>,
    ) -> Result<Response<pb::EndAck>, Status> {
        let who = caller(&request)?;
        Self::user_only(&who)?;
        let request = request.into_inner();
        let target = self.target(&who, request.location, "location", Entry::Folder, true)?;
        let machine = self.machine.clone();
        self.on_disk(move |disk| {
            let path = target.existing()?;
            disk.sync_folder(&path)
                .map_err(|err| end_failure(err, &machine, "location", &target.shown))
        })
        .await?;
        Ok(Response::new(pb::EndAck {}))
    }

    /// Removes a transfer's staging folder, or something in one. A missing
    /// path is fine. Anything not inside a staging folder is refused: this
    /// call only removes what a transfer staged.
    async fn remove_staged(
        &self,
        request: Request<pb::EndPathRequest>,
    ) -> Result<Response<pb::EndAck>, Status> {
        let who = caller(&request)?;
        Self::user_only(&who)?;
        let request = request.into_inner();
        let target = self.target(&who, request.location, "location", Entry::File, true)?;
        if !target
            .location
            .path
            .split('/')
            .any(|part| part.starts_with(STAGING_PREFIX))
        {
            return Err(invalid_field(
                "location.path",
                Problem::NotAllowedHere,
                format!("{} isn't inside a transfer's staging folder", target.shown),
                format!("this call only removes `{STAGING_PREFIX}<id>` folders and what is in them; delete other things with MoveToTrash"),
            ));
        }
        let machine = self.machine.clone();
        self.on_disk(move |disk| {
            let path = match target.unfollowed() {
                Ok(path) => path,
                Err(err) if err.code() == Code::NotFound => return Ok(()),
                Err(err) => return Err(err),
            };
            disk.remove_tree(&path)
                .map_err(|err| end_failure(err, &machine, "location", &target.shown))
        })
        .await?;
        Ok(Response::new(pb::EndAck {}))
    }

    async fn locate(
        &self,
        request: Request<pb::EndPathRequest>,
    ) -> Result<Response<pb::LocateResponse>, Status> {
        let who = caller(&request)?;
        Self::user_only(&who)?;
        let request = request.into_inner();
        let target = self.target(&who, request.location, "location", Entry::File, false)?;
        if !matches!(target.place.root, humata2_access::Root::Pastes(_)) {
            return Err(invalid_field(
                "location",
                Problem::NotAllowedHere,
                format!("{} isn't in a session's pastes", target.shown),
                "this call only says where pasted files are; name a file in a session's pastes",
            ));
        }
        let path = blocking(move || target.existing().and_then(|path| path_text(&path))).await??;
        Ok(Response::new(pb::LocateResponse { path }))
    }
}
