//! TransferService on this machine's own files: copy or move files and
//! folders between this machine's roots, and upload a file from the app,
//! in chunks, resumable and checksummed (FM-2, FM-4, FM-6, FM-7, FM-8,
//! FM-14, LD-19, TR-11, PER-27). The work is done by `humata2_transfers`;
//! this is its gRPC face: it resolves and guards the locations, keeps what
//! the engine doesn't (the locations as the caller named them and the
//! idempotency key), and pushes every change to watchers.
//!
//! The engine is synchronous: it is only called on blocking threads. It is
//! opened the first time it is needed (or by [`TransferSvc::recover`] at
//! start), so a transfer a restart cut short continues or finishes undoing
//! on its own, and a state folder that can't be used fails the calls that
//! need it and never stops the daemon.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use humata2_access::{AccessRequest, Call, Entry};
use humata2_chunk::Decoder;
use humata2_grpc::humata::v1 as pb;
use humata2_transfers as tr;
use humata2_transfers::End as _;
use pb::invalid_field::Problem as Field;
use prost::Message;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status, Streaming};

use crate::ports::{
    Answers, BoardAsk, Caller, Checked, Compression, Deferred, LinkProvider, Placed, Roots, Run,
    HostAskPort, Target, TransferGuardPort, TransferInboxPort, TrashBinPort, actor, answer_key, blocking,
    caller, chain, compression, error, failure, internal, invalid_field, lock, missing, not_found,
    now_ms, timestamp,
};
use crate::remote_end::{self, RemoteEnd, RouterEnd};

/// LD-19: a transfer leaves at least this much free (D107).
const FREE_FLOOR_BYTES: u64 = 5 << 30;

/// Bytes written between syncs and journal writes.
const SYNC_EVERY_BYTES: u64 = 8 << 20;

/// Changes buffered for a slow watcher before it re-reads the transfer.
const WATCH_BUFFER: usize = 256;

/// Transfers per page when the caller names no size.
const PAGE_DEFAULT: usize = 50;

/// The most transfers in one page.
const PAGE_MAX: usize = 500;

pub type WatchTransfersStream = ReceiverStream<Result<pb::Transfer, Status>>;

/// What the engine doesn't keep of a transfer.
#[derive(Clone, PartialEq, Message)]
struct Data {
    #[prost(message, repeated, tag = "1")]
    sources: Vec<pb::FileLocation>,
    #[prost(message, optional, tag = "2")]
    destination: Option<pb::FileLocation>,
    /// The idempotency key with the call and caller it was made by; empty
    /// when the request had none.
    #[prost(string, tag = "3")]
    key: String,
    #[prost(message, optional, tag = "4")]
    by: Option<pb::Actor>,
    /// An upload's name, size and SHA-256 as its header gave them.
    #[prost(string, tag = "5")]
    name: String,
    #[prost(uint64, tag = "6")]
    size: u64,
    #[prost(string, tag = "7")]
    sha256: String,
}

fn encode(data: &Data) -> String {
    data.encode_to_vec()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What [`encode`] kept; nothing for data it can't read.
fn decode(text: &str) -> Data {
    let bytes: Option<Vec<u8>> = (text.len() % 2 == 0)
        .then(|| {
            (0..text.len())
                .step_by(2)
                .map(|at| text.get(at..at + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok()))
                .collect()
        })
        .flatten();
    bytes
        .and_then(|bytes| Data::decode(bytes.as_slice()).ok())
        .unwrap_or_default()
}

/// Over for good. A transfer waiting for a machine is Failed in the engine
/// but not finished: it continues when the link is back.
fn finished(snapshot: &tr::Snapshot) -> bool {
    match snapshot.state {
        tr::State::Done | tr::State::Cancelled => true,
        tr::State::Failed => remote_end::waiting_for(snapshot).is_none(),
        _ => false,
    }
}

/// This machine's transfers.
#[derive(Clone)]
pub struct TransferSvc {
    core: Arc<Core>,
    /// Where an agent's copy that the rules send to Ask me waits for the
    /// user (D38.6, IN-10).
    inbox: Arc<dyn TransferInboxPort>,
}

struct Core {
    roots: Arc<Roots>,
    guard: Arc<dyn TransferGuardPort>,
    bin: Arc<dyn TrashBinPort>,
    /// The daemon's state folder; the journals are in `transfers` in it.
    state: PathBuf,
    machine: String,
    /// Reaches files on other machines over their links (FM-9); offline
    /// until [`TransferSvc::use_links`].
    remote: Arc<RemoteEnd>,
    engine: Mutex<Option<Arc<tr::Transfers>>>,
    /// Idempotency key (as kept in `Data::key`) to the transfer it started.
    keys: Mutex<HashMap<String, String>>,
    changes: broadcast::Sender<tr::Snapshot>,
    controlled: Answers<pb::Transfer>,
    /// On a machine: where its agents' copies naming another machine go (D768).
    host: std::sync::OnceLock<Arc<dyn HostAskPort>>,
}

impl TransferSvc {
    /// Never fails and touches no disk.
    pub fn new(
        roots: Arc<Roots>,
        guard: Arc<dyn TransferGuardPort>,
        bin: Arc<dyn TrashBinPort>,
        state: PathBuf,
        machine: String,
        compression: Arc<Compression>,
        inbox: Arc<dyn TransferInboxPort>,
    ) -> Arc<Self> {
        let (changes, _) = broadcast::channel(WATCH_BUFFER);
        // Called from the daemon's async start: the engine's blocking threads
        // reach the runtime through this handle.
        let remote = Arc::new(RemoteEnd::new(tokio::runtime::Handle::current(), compression));
        Arc::new(Self {
            inbox,
            core: Arc::new(Core {
                roots,
                guard,
                bin,
                state,
                machine,
                remote,
                engine: Mutex::new(None),
                keys: Mutex::new(HashMap::new()),
                changes,
                controlled: Answers::new(),
                host: std::sync::OnceLock::new(),
            }),
        })
    }

    /// Opens the engine, so transfers a restart cut short continue (PER-27).
    /// Blocking. A failure is logged and tried again by the next call.
    pub fn recover(self: &Arc<Self>) {
        if let Err(status) = self.core.engine() {
            eprintln!("humatad2: transfers: {}", status.message());
        }
    }

    /// On a machine: its agents' copies that name another machine go to the
    /// host, which holds the links (D768). The first call wins.
    pub fn ask_host_for_agents(self: &Arc<Self>, host: Arc<dyn HostAskPort>) {
        let _ = self.core.host.set(host);
    }

    /// The host's answer to an agent's copy naming another machine, or
    /// `None` when the copy isn't one for the host: not an agent's, all on
    /// this machine, or this daemon is the host. The places on this machine
    /// go up unnamed: the host knows it by its link's id, not this one's.
    async fn ask_host(
        &self,
        who: &Caller,
        request: &pb::StartTransferRequest,
    ) -> Option<Result<Response<pb::Transfer>, Status>> {
        const METHOD: &str = "/humata.v1.TransferService/StartTransfer";
        let host = self.core.host.get()?;
        let Caller::Agent(agent) = who else {
            return None;
        };
        let here = self.core.machine.as_str();
        let elsewhere = |place: &pb::FileLocation| {
            place.machine.as_ref().is_some_and(|machine| !machine.value.is_empty() && machine.value != here)
        };
        if !request.sources.iter().chain(request.destination.as_ref()).any(elsewhere) {
            return None;
        }
        let mut sent = request.clone();
        for place in sent.sources.iter_mut().chain(sent.destination.as_mut()) {
            if !elsewhere(place) {
                place.machine = None;
            }
        }
        let asked = host.ask(agent.session.id.as_str(), METHOD, sent.encode_to_vec()).await;
        Some(asked.and_then(|bytes| {
            pb::Transfer::decode(bytes.as_slice()).map(Response::new).map_err(|source| {
                internal(
                    format!(
                        "the host's answer to {METHOD} couldn't be read ({source}): this machine and the host run different releases of Humata; migrate this machine from the host's Inbox"
                    ),
                    "",
                )
            })
        }))
    }

    /// Gives the transfers the machines' links (the host only; without them
    /// every machine is offline) and resumes the transfers waiting for a
    /// machine whenever its link comes up (FM-8, FM-13). Inside the runtime,
    /// once.
    pub fn use_links(self: &Arc<Self>, links: Arc<dyn LinkProvider>) {
        // The engine's resume writes a journal: never on the runtime.
        let in_background = |work: fn(&Core, &str)| -> Arc<dyn Fn(&str) + Send + Sync> {
            let core = self.core.clone();
            Arc::new(move |what: &str| {
                let (core, what) = (core.clone(), what.to_owned());
                drop(tokio::task::spawn_blocking(move || work(&core, &what)));
            })
        };
        remote_end::follow_links(
            links,
            self.core.remote.clone(),
            self.core.changes.subscribe(),
            in_background(Core::resume_waiting),
            in_background(Core::resume_one),
        );
    }
}

impl Core {
    /// The engine, opened if it wasn't yet. Blocking.
    fn engine(&self) -> Result<Arc<tr::Transfers>, Status> {
        let mut open = lock(&self.engine);
        if let Some(engine) = &*open {
            return Ok(engine.clone());
        }
        let trash = self.bin.get()?;
        let sender = self.changes.clone();
        let router: Arc<dyn tr::End> = Arc::new(RouterEnd::new(
            Arc::new(tr::Disk::new(self.machine.clone(), trash)),
            self.remote.clone(),
        ));
        let config = tr::Config {
            journal: self.state.join("transfers"),
            source: router.clone(),
            destination: router,
            free_floor_bytes: FREE_FLOOR_BYTES,
            sync_every_bytes: SYNC_EVERY_BYTES,
            listener: Arc::new(move |snapshot: &tr::Snapshot| {
                // Nobody watching is fine.
                let _ = sender.send(snapshot.clone());
            }),
        };
        let engine = tr::Transfers::open(config).map_err(|err| {
            internal(
                format!(
                    "the transfer journals in {} can't be used: {}. Check that the folder belongs to you and the disk has free space; the next call tries again",
                    self.state.join("transfers").display(),
                    chain(&err)
                ),
                "",
            )
        })?;
        // Keys of transfers from before a restart still answer retries.
        let mut keys = lock(&self.keys);
        for snapshot in engine.list().iter().rev() {
            let data = decode(&snapshot.caller_data);
            if !data.key.is_empty() {
                keys.insert(data.key, snapshot.id.clone());
            }
        }
        drop(keys);
        *open = Some(engine.clone());
        Ok(engine)
    }

    fn machine_id(&self) -> pb::MachineId {
        pb::MachineId {
            value: self.machine.clone(),
        }
    }

    /// Resumes each transfer that stopped for `machine`'s link. Blocking.
    fn resume_waiting(&self, machine: &str) {
        let Ok(engine) = self.engine() else { return };
        for snapshot in engine.list() {
            if remote_end::waiting_for(&snapshot) == Some(machine) {
                // Already running or taken over by the user: nothing to do.
                let _ = engine.resume(&snapshot.id);
            }
        }
    }

    /// Resumes one transfer that stopped for a machine. Blocking.
    fn resume_one(&self, id: &str) {
        if let Ok(engine) = self.engine() {
            let _ = engine.resume(id);
        }
    }

    /// The transfer a key already started, as it is now.
    fn started_by(&self, engine: &tr::Transfers, key: &str) -> Option<pb::Transfer> {
        let id = lock(&self.keys).get(key).cloned()?;
        match engine.get(&id) {
            Some(snapshot) => Some(self.view(&snapshot)),
            None => {
                lock(&self.keys).remove(key);
                None
            }
        }
    }

    fn view(&self, snapshot: &tr::Snapshot) -> pb::Transfer {
        let data = decode(&snapshot.caller_data);
        transfer(&self.machine, snapshot, data)
    }

    /// An upload's answer, with where the file `name` is on its machine's
    /// disk when the upload went into a session's pastes and is done: what
    /// the app types into the session's terminal (D335). Blocks: asks the
    /// machine when it is another one.
    fn with_pasted_path(&self, mut transfer: pb::Transfer, name: &str) -> Result<pb::Transfer, Status> {
        use pb::file_location::Root as R;
        let Some(folder) = transfer.destination.clone() else {
            return Ok(transfer);
        };
        if transfer.state() != pb::TransferState::Done
            || !matches!(folder.root, Some(R::SessionPastes(_)))
        {
            return Ok(transfer);
        }
        let path = match folder.path.as_str() {
            "" => name.to_owned(),
            path => format!("{path}/{name}"),
        };
        let file = pb::FileLocation { path, ..folder };
        let placed = self.roots.place(Some(file), "header.destination", Entry::File)?;
        transfer.disk_path = match placed.machine() == self.machine {
            true => crate::transfer_end_service::path_text(&placed.into_target().existing()?)?,
            false => self.remote.locate(&placed.location)?,
        };
        Ok(transfer)
    }

    /// A folder to write into, here or on another machine: the caller must
    /// be allowed to write it. Nothing is looked at on any disk.
    fn placed_folder(&self, who: &Caller, location: Option<pb::FileLocation>, field: &str) -> Result<Placed, Status> {
        let placed = self.roots.place(location, field, Entry::Folder)?;
        placed.require_writable(field)?;
        self.guard
            .require(who, &Call::WriteFiles(placed.place.clone()))?;
        Ok(placed)
    }

    /// The engine's path for a destination, which must be an existing folder
    /// (asked of the machine when it is another one).
    fn destination_folder(&self, destination: &Placed) -> Result<PathBuf, Status> {
        match destination.machine() == self.machine {
            true => existing_folder(&destination.clone().into_target(), "destination"),
            false => self.remote.folder(&destination.location, "destination"),
        }
    }

    /// Whether the caller may control or see the transfer: whoever may
    /// write its destination.
    fn authorize(&self, who: &Caller, snapshot: &tr::Snapshot) -> Result<(), Status> {
        let data = decode(&snapshot.caller_data);
        let placed = self.roots.place(data.destination, "destination", Entry::Folder)?;
        self.guard
            .require(who, &Call::WriteFiles(placed.place))
    }

    fn may_see(&self, who: &Caller, snapshot: &tr::Snapshot) -> bool {
        let data = decode(&snapshot.caller_data);
        let Ok(placed) = self.roots.place(data.destination, "destination", Entry::Folder) else {
            // A destination that can't be resolved any more is shown to
            // the user only.
            return matches!(who, Caller::User(_));
        };
        matches!(
            self.guard.check(who, &Call::WriteFiles(placed.place)),
            Ok(Checked::Allowed)
        )
    }

    fn start(&self, who: &Caller, begin: Begin) -> Result<Started, Status> {
        let engine = self.engine()?;
        if let Some(found) = begin.key.as_deref().and_then(|key| self.started_by(&engine, key)) {
            return Ok(Started::Done(found));
        }
        if begin.sources.is_empty() {
            return Err(missing("sources"));
        }
        let name = landing_name(&begin.name, begin.sources.len())?;
        let destination = self.placed_folder(who, begin.destination, "destination")?;
        let mut sources = Vec::new();
        // The first copy the rules send to Ask me (IN-10): the call waits in
        // the Inbox with a preview of what would be copied where.
        let mut asked: Option<AccessRequest> = None;
        for (index, location) in begin.sources.iter().cloned().enumerate() {
            let field = format!("sources[{index}]");
            let source = self.roots.place(Some(location), &field, Entry::File)?;
            if begin.mode == tr::Mode::Move {
                if source.is_root() {
                    return Err(invalid_field(
                        &field,
                        Field::NotAllowedHere,
                        "a root folder can't be moved",
                        "move the things in it, or copy it",
                    ));
                }
                source.require_writable(&field)?;
                self.guard
                    .require(who, &Call::WriteFiles(source.place.clone()))?;
            }
            let copy = Call::CopyFiles {
                from: source.place.clone(),
                into: destination.place.clone(),
            };
            if matches!(who, Caller::Agent(_)) {
                // An approved call goes on without asking again.
                if !begin.approved {
                    if let Checked::Ask(ask) = self.guard.check(who, &copy)? {
                        asked.get_or_insert(ask);
                    }
                }
            } else {
                self.guard.require(who, &copy)?;
            }
            sources.push(source);
        }
        if let Some(asked) = asked {
            let preview = self.copy_preview(&sources, &destination, begin.mode, begin.on_existing, &begin.name)?;
            return Ok(Started::Ask(Box::new(asked), Box::new(preview)));
        }
        let folder = self.destination_folder(&destination)?;
        let mut items = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            let path = remote_end::source_path(source, &self.machine)?;
            // A folder copied into itself would never end.
            if folder.starts_with(&path) {
                return Err(invalid_field(
                    "destination",
                    Field::NotAllowedHere,
                    format!(
                        "the destination is inside {} (sources[{index}]), which is being copied or moved",
                        source.shown
                    ),
                    "pick a destination outside the folders being copied",
                ));
            }
            items.push(tr::Item {
                path,
                shown: source.shown.clone(),
            });
        }
        let data = Data {
            sources: sources.iter().map(|source| source.location.clone()).collect(),
            destination: Some(destination.location.clone()),
            key: begin.key.clone().unwrap_or_default(),
            by: Some(begin.by),
            ..Data::default()
        };
        let request = tr::Request {
            id: format!("tr_{}", uuid::Uuid::new_v4().simple()),
            sources: items,
            destination: tr::Item {
                path: folder,
                shown: destination.shown.clone(),
            },
            mode: begin.mode,
            on_existing: begin.on_existing,
            caller_data: encode(&data),
            now_unix_ms: now_ms(),
        };
        let id = request.id.clone();
        // One start per key, even when two calls race.
        let mut keys = lock(&self.keys);
        if let Some(found) = begin.key.as_deref().and_then(|key| keys.get(key).cloned()) {
            if let Some(snapshot) = engine.get(&found) {
                return Ok(Started::Done(self.view(&snapshot)));
            }
        }
        let snapshot = engine.start_as(request, name).map_err(|err| self.failure(&err))?;
        if let Some(key) = begin.key {
            keys.insert(key, id);
        }
        Ok(Started::Done(self.view(&snapshot)))
    }

    /// What an agent's copy would do, measured at its sources: the machines,
    /// the places, how many files and how many bytes (IN-10, D467). Blocking:
    /// it reads the sources' trees, over a machine's link when they are on
    /// another machine.
    fn copy_preview(
        &self,
        sources: &[Placed],
        destination: &Placed,
        mode: tr::Mode,
        on_existing: tr::OnExisting,
        name: &str,
    ) -> Result<pb::CrossMachineCopyPreview, Status> {
        let router = RouterEnd::new(
            Arc::new(tr::Disk::new(self.machine.clone(), self.bin.get()?)),
            self.remote.clone(),
        );
        let (mut bytes, mut files) = (0u64, 0u32);
        let mut replaced = Vec::new();
        // The destination folder is read only when files there would be
        // replaced; one that can't be read shows none rather than failing
        // the ask.
        let folder = (on_existing == tr::OnExisting::Replace)
            .then(|| self.destination_folder(destination).ok())
            .flatten();
        for source in sources {
            let path = remote_end::source_path(source, &self.machine)?;
            let tree = router.plan(&path).map_err(|err| {
                internal(
                    format!(
                        "{} can't be measured before asking you: {}. Check that its machine is reachable, then repeat the copy",
                        source.shown,
                        chain(&err)
                    ),
                    "",
                )
            })?;
            for entry in tree.iter().filter(|entry| entry.kind == tr::Kind::File) {
                bytes = bytes.saturating_add(entry.size);
                files = files.saturating_add(1);
            }
            if let Some(folder) = &folder {
                let landing = match (name.is_empty(), path.file_name()) {
                    (false, _) => Some(name.to_owned()),
                    (true, Some(file)) => Some(file.to_string_lossy().into_owned()),
                    (true, None) => None,
                };
                if let Some(landing) = landing {
                    replaced_files(&router, &folder.join(&landing), &landing, &tree, &mut replaced);
                }
            }
        }
        let machine = |placed: &Placed| pb::MachineId {
            value: placed.machine().to_owned(),
        };
        Ok(pb::CrossMachineCopyPreview {
            source_machine: sources.first().map(machine),
            source_path: sources
                .iter()
                .map(|source| source.shown.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            destination_machine: Some(machine(destination)),
            destination_path: destination.shown.clone(),
            bytes,
            files,
            destination_place: place_label(&destination.location),
            moves: mode == tr::Mode::Move,
            replaced,
        })
    }

    fn list(&self, who: &Caller, active_only: bool, size: usize, token: &str) -> Result<pb::ListTransfersResponse, Status> {
        let engine = self.engine()?;
        let size = match size {
            0 => PAGE_DEFAULT,
            size => size.min(PAGE_MAX),
        };
        // Newest first; the token is the last one sent.
        let mut rest = engine
            .list()
            .into_iter()
            .filter(|snapshot| !active_only || !finished(snapshot))
            .filter(|snapshot| token.is_empty() || before(token, snapshot))
            .filter(|snapshot| self.may_see(who, snapshot));
        let page: Vec<tr::Snapshot> = rest.by_ref().take(size).collect();
        let next_page_token = match (rest.next(), page.last()) {
            (Some(_), Some(last)) => position(last),
            _ => String::new(),
        };
        Ok(pb::ListTransfersResponse {
            transfers: page.iter().map(|snapshot| self.view(snapshot)).collect(),
            page: Some(pb::PageResponse { next_page_token }),
        })
    }

    /// The transfer as it is, if the caller may see it.
    fn read(&self, who: &Caller, id: &str) -> Result<tr::Snapshot, Status> {
        let engine = self.engine()?;
        let snapshot = engine.get(id).ok_or_else(|| no_transfer(id))?;
        self.authorize(who, &snapshot)?;
        Ok(snapshot)
    }

    fn control(&self, who: &Caller, id: &str, what: Control) -> Result<pb::Transfer, Status> {
        let engine = self.engine()?;
        let snapshot = engine.get(id).ok_or_else(|| no_transfer(id))?;
        self.authorize(who, &snapshot)?;
        let result = match what {
            Control::Pause => engine.pause(id),
            Control::Resume => engine.resume(id),
            Control::Cancel => engine.cancel(id),
        };
        let snapshot = result.map_err(|err| self.failure(&err))?;
        Ok(self.view(&snapshot))
    }

    /// Starts an upload, or continues the one `header.resume` names.
    fn begin_upload(&self, who: &Caller, header: pb::UploadHeader, by: pb::Actor) -> Result<Began, Status> {
        let engine = self.engine()?;
        let on_existing = on_existing(header.on_existing, "header.on_existing")?;
        check_name(&header.name)?;
        let destination = self.placed_folder(who, header.destination, "header.destination")?;
        let key = answer_key("upload", who, header.idempotency_key.as_ref(), "");
        let resume = header
            .resume
            .map(|id| id.value)
            .filter(|id| !id.is_empty());
        let sha256 = header.sha256.to_lowercase();
        if let Some(id) = &resume {
            let snapshot = engine.get(id).ok_or_else(|| no_transfer(id))?;
            let first = decode(&snapshot.caller_data);
            if !snapshot.upload
                || first.destination.as_ref() != Some(&destination.location)
                || first.name != header.name
                || first.size != header.size
                || first.sha256 != sha256
            {
                return Err(invalid_field(
                    "header.resume",
                    Field::NotAllowedHere,
                    format!(
                        "transfer {id} is not an upload of {} ({} bytes) into the same folder, so it can't be continued with this header",
                        header.name, header.size
                    ),
                    "resume with the same destination, name, size and sha256 as the first upload, or start a new upload",
                ));
            }
        } else if let Some(found) = key.as_deref().and_then(|key| self.started_by(&engine, key)) {
            return Ok(Began::Existing(found));
        }
        let folder = self.destination_folder(&destination)?;
        let id = resume
            .clone()
            .unwrap_or_else(|| format!("tr_{}", uuid::Uuid::new_v4().simple()));
        let data = Data {
            destination: Some(destination.location.clone()),
            key: key.clone().unwrap_or_default(),
            by: Some(by),
            name: header.name.clone(),
            size: header.size,
            sha256: sha256.clone(),
            ..Data::default()
        };
        let request = tr::UploadRequest {
            id: id.clone(),
            resume,
            destination: tr::Item {
                path: folder,
                shown: destination.shown.clone(),
            },
            name: header.name,
            size: header.size,
            sha256,
            on_existing,
            caller_data: encode(&data),
            now_unix_ms: now_ms(),
        };
        let upload = engine
            .begin_upload(request)
            .map_err(|err| self.failure(&err))?;
        if let Some(key) = key {
            lock(&self.keys).insert(key, id.clone());
        }
        Ok(Began::Upload(upload, id))
    }

    fn failure(&self, err: &tr::TransferError) -> Status {
        use tr::TransferError as E;
        match err {
            E::Unknown { id } => no_transfer(id),
            E::TooLate { id } => not_cancellable(
                format!("transfer {id} can't be cancelled any more: its files are already being put in place, or it is finished"),
                "let it finish; once it is done, delete what you don't want from the destination",
            ),
            E::IsUpload { id } => precondition(
                format!("transfer {id} is an upload, which only the app's stream can continue"),
                "continue it by starting the upload again with its transfer id as resume, or cancel it",
            ),
            E::NotResumable { id } => precondition(
                format!("transfer {id} isn't paused or failed, so there is nothing to resume"),
                "look at its state; a running transfer continues on its own",
            ),
            E::AlreadyRunning { id } => precondition(
                format!("transfer {id} is already running"),
                "wait for it to finish, or pause it first",
            ),
            E::BadId { .. } | E::BadSource { .. } => invalid_field(
                "sources",
                Field::BadFormat,
                err.to_string(),
                "pick files or folders in a folder, not a root",
            ),
            E::BadName { .. } => invalid_field(
                "header.name",
                Field::BadFormat,
                err.to_string(),
                "give one plain file name without '/'",
            ),
            E::NoSources => missing("sources"),
            E::NameNeedsOneSource { .. } => invalid_field(
                "name",
                Field::NotAllowedHere,
                err.to_string(),
                "copy the one file to its new name, or leave the name out to keep each source's own",
            ),
            E::WrongOffset { id, expected, got } => invalid_field(
                "chunk.offset",
                Field::OutOfRange,
                format!("upload {id} has {expected} bytes; a chunk starts at byte {got}"),
                format!("send the chunks in order, continuing from byte {expected}"),
            ),
            E::Overrun { id, size } => invalid_field(
                "header.size",
                Field::OutOfRange,
                format!("upload {id} was sent more than the {size} bytes its header names"),
                "start a new upload with the file's real size in the header",
            ),
            E::Incomplete { id, offset, size } => invalid_field(
                "chunk.last",
                Field::OutOfRange,
                format!("upload {id} has {offset} of {size} bytes, so it can't finish yet"),
                format!("send the rest, continuing from byte {offset}"),
            ),
            E::Interrupted { id } => failure(
                Code::Aborted,
                error(
                    format!("upload {id} was paused or interrupted"),
                    "resume it with the transfer id as resume in a new upload header; it continues from the bytes it has",
                    pb::error::Kind::InvalidField(pb::InvalidField {
                        field: "chunk.last".to_owned(),
                        problem: Field::Missing as i32,
                    }),
                ),
            ),
            E::Cancelled { id } => failure(
                Code::Aborted,
                error(
                    format!("upload {id} was cancelled, and what had arrived was removed"),
                    "start the upload again if you still want the file",
                    pb::error::Kind::InvalidField(pb::InvalidField {
                        field: "header".to_owned(),
                        problem: Field::NotAllowedHere as i32,
                    }),
                ),
            ),
            E::Refused { id, problem } => {
                let (code, err) = problem_error(&self.machine_id(), id, problem);
                failure(code, err)
            }
            E::Duplicate { .. }
            | E::Journal { .. }
            | E::JournalFormat { .. }
            | E::JournalRead { .. }
            | E::Spawn { .. }
            | E::End { .. } => internal(
                format!("the transfer failed: {}", chain(err)),
                "",
            ),
        }
    }
}

enum Control {
    Pause,
    Resume,
    Cancel,
}

enum Began {
    Upload(tr::Upload, String),
    /// The upload a key already started, as it is now.
    Existing(pb::Transfer),
}

/// How a start call went: it started, or the rules say Ask me first.
enum Started {
    Done(pb::Transfer),
    Ask(Box<AccessRequest>, Box<pb::CrossMachineCopyPreview>),
}

/// The words for where a copy lands, to follow the machine's name: the
/// kind of place, never a path (D467).
fn place_label(location: &pb::FileLocation) -> String {
    use pb::file_location::Root as R;
    match &location.root {
        Some(R::SessionClone(clone)) => clone
            .repo
            .as_ref()
            .map_or_else(|| "clone".to_owned(), |repo| format!("{} clone", repo.name)),
        Some(R::ChannelShared(_)) => "shared".to_owned(),
        Some(R::ChannelFolder(_)) => "channel folder".to_owned(),
        Some(R::SessionFolder(_)) => "session folder".to_owned(),
        _ => String::new(),
    }
}

/// The new name a copy's one source lands under (D769): none when it keeps
/// its own, refused when it isn't one plain path part or more than one
/// source would take it.
/// The files of `tree` (a source's tree) that already exist under `target`,
/// the folder or file they land at, named from `landing` down. Reads each
/// file's size and time from the destination's own tree; what can't be read
/// is left out.
fn replaced_files(
    router: &RouterEnd,
    target: &std::path::Path,
    landing: &str,
    tree: &[tr::Entry],
    replaced: &mut Vec<pb::ReplacedFile>,
) {
    let there = match router.exists(target) {
        Ok(true) => router.plan(target).unwrap_or_default(),
        _ => return,
    };
    for entry in tree.iter().filter(|entry| entry.kind == tr::Kind::File) {
        let Some(old) = there
            .iter()
            .find(|old| old.kind == tr::Kind::File && old.rel == entry.rel)
        else {
            continue;
        };
        let note = if old.size == entry.size && old.modified_unix_ms == entry.modified_unix_ms {
            "same content"
        } else if old.modified_unix_ms > entry.modified_unix_ms {
            "the one there is newer"
        } else {
            ""
        };
        replaced.push(pb::ReplacedFile {
            path: if entry.rel.is_empty() {
                landing.to_owned()
            } else {
                format!("{landing}/{}", entry.rel)
            },
            note: note.to_owned(),
        });
    }
}

fn landing_name(name: &str, sources: usize) -> Result<Option<String>, Status> {
    if name.is_empty() {
        return Ok(None);
    }
    if name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(invalid_field(
            "name",
            Field::BadFormat,
            format!("{name:?} isn't a name a file can land under"),
            "give one plain name without '/', or leave the name out",
        ));
    }
    if sources != 1 {
        return Err(invalid_field(
            "name",
            Field::NotAllowedHere,
            format!("a new name is for one source, and {sources} were given"),
            "copy the one file to its new name, or leave the name out to keep each source's own",
        ));
    }
    Ok(Some(name.to_owned()))
}

/// What a start call asks for, checked.
#[derive(Clone)]
struct Begin {
    key: Option<String>,
    sources: Vec<pb::FileLocation>,
    destination: Option<pb::FileLocation>,
    mode: tr::Mode,
    on_existing: tr::OnExisting,
    by: pb::Actor,
    /// The user said yes in the Inbox: the copy rule isn't asked again.
    approved: bool,
    /// The one source's new name in the destination (D769); empty keeps
    /// its own.
    name: String,
}

/// Where in the newest-first order a transfer stands, as a page token.
fn position(snapshot: &tr::Snapshot) -> String {
    format!("{}:{}", snapshot.started_unix_ms, snapshot.id)
}

/// Whether `snapshot` comes after the one `token` names, newest first.
fn before(token: &str, snapshot: &tr::Snapshot) -> bool {
    let Some((ms, id)) = token.split_once(':') else {
        return true;
    };
    let Ok(ms) = ms.parse::<i64>() else {
        return true;
    };
    (snapshot.started_unix_ms, snapshot.id.as_str()) < (ms, id)
}

fn mode_of(value: i32) -> Result<tr::Mode, Status> {
    match pb::TransferMode::try_from(value) {
        Ok(pb::TransferMode::Copy) => Ok(tr::Mode::Copy),
        Ok(pb::TransferMode::Move) => Ok(tr::Mode::Move),
        _ => Err(invalid_field(
            "mode",
            Field::Missing,
            "the request doesn't say whether to copy or move",
            "set mode to copy, which leaves the originals, or move, which deletes them once every file is verified",
        )),
    }
}

fn on_existing(value: i32, field: &str) -> Result<tr::OnExisting, Status> {
    match pb::OnExisting::try_from(value) {
        Ok(pb::OnExisting::Fail) => Ok(tr::OnExisting::Fail),
        Ok(pb::OnExisting::Replace) => Ok(tr::OnExisting::Replace),
        Ok(pb::OnExisting::KeepBoth) => Ok(tr::OnExisting::KeepBoth),
        _ => Err(invalid_field(
            field,
            Field::Missing,
            "the request doesn't say what to do with a file that already has the name",
            "set on_existing to fail (stop), replace (the old one goes to the trash) or keep_both (the new one gets a number)",
        )),
    }
}

/// One plain file name, as renaming asks for.
fn check_name(name: &str) -> Result<(), Status> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\0')
        || name.starts_with(".humata-transfer-")
        || (name.starts_with('.') && name.contains(".humata-partial-"));
    if bad {
        return Err(invalid_field(
            "header.name",
            Field::BadFormat,
            format!("{name:?} isn't a usable file name"),
            "give one plain file name: no '/', not empty, not \".\" or \"..\", and not one Humata uses for unfinished files",
        ));
    }
    Ok(())
}

/// The folder's path on disk; it must exist and be a folder.
fn existing_folder(target: &Target, field: &str) -> Result<PathBuf, Status> {
    let path = target.existing()?;
    if !path.is_dir() {
        return Err(invalid_field(
            field,
            Field::NotAllowedHere,
            format!("{} is a file, not a folder", target.shown),
            "name the folder to put the files in",
        ));
    }
    Ok(path)
}

fn no_transfer(id: &str) -> Status {
    not_found(
        pb::EntityKind::Transfer,
        id,
        format!("there is no transfer {id}"),
        "it may be one of the oldest, forgotten after 200 later ones; list the transfers to see what is there",
    )
}

/// The transfer is in a state the call doesn't fit. Nothing is wrong with
/// the request itself, so it carries no field problem: a client reads it
/// as a failed precondition, not as a call to correct.
fn precondition(message: String, fix: &str) -> Status {
    failure(
        Code::FailedPrecondition,
        pb::Error {
            message,
            fix: fix.to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// Too late to cancel: the files are being put in place, or it is over.
fn not_cancellable(message: String, fix: &str) -> Status {
    failure(
        Code::FailedPrecondition,
        error(
            message,
            fix,
            pb::error::Kind::NotCancellable(pb::NotCancellable {
                operation: None,
                step: "putting the files in place".to_owned(),
            }),
        ),
    )
}

fn kib(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} bytes"),
        1024..0x10_0000 => format!("{:.1} KiB", bytes as f64 / 1024.0),
        0x10_0000..0x4000_0000 => format!("{:.1} MiB", bytes as f64 / 1_048_576.0),
        _ => format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0),
    }
}

/// Why a transfer stopped, as the status code and error a client branches
/// on.
fn problem_error(machine: &pb::MachineId, id: &str, problem: &tr::Problem) -> (Code, pb::Error) {
    use tr::Problem as P;
    match problem {
        P::DiskFull { folder, needed, free } => (
            Code::ResourceExhausted,
            error(
                format!(
                    "{folder} doesn't have room: {} are needed, including the {} Humata always leaves free, and {} are free",
                    kib(*needed),
                    kib(FREE_FLOOR_BYTES),
                    kib(*free)
                ),
                "free space from this machine's Storage page (empty the trash; deleting clones with unpushed work loses that work), then Resume the transfer. What was copied so far is kept",
                pb::error::Kind::DiskFull(pb::DiskFull {
                    machine: Some(machine.clone()),
                    path: folder.clone(),
                    needed_bytes: *needed,
                    free_bytes: *free,
                }),
            ),
        ),
        P::ChecksumMismatch { path, expected, actual } => (
            Code::DataLoss,
            error(
                format!(
                    "{path} was copied as SHA-256 {actual}, not the {expected} it has at the source; the bad copy was removed and the source is untouched"
                ),
                "Resume the transfer to copy it again. If it fails the same way, the source is changing while it is copied or a disk is failing: stop whatever writes it, or check the disks, before trying again",
                pb::error::Kind::ChecksumMismatch(pb::ChecksumMismatch {
                    transfer: Some(pb::TransferId {
                        value: id.to_owned(),
                    }),
                    path: path.clone(),
                    expected_sha256: expected.clone(),
                    actual_sha256: actual.clone(),
                }),
            ),
        ),
        P::Exists { path } => (
            Code::AlreadyExists,
            error(
                format!("{path} already exists in the destination, so nothing was put there"),
                "start the transfer again choosing replace (the old one goes to the trash) or keep both, or rename what is there",
                pb::error::Kind::NameTaken(pb::NameTaken {
                    kind: pb::EntityKind::File as i32,
                    name: path.clone(),
                    suggestion: String::new(),
                }),
            ),
        ),
        P::SourceGone { path } => (
            Code::NotFound,
            error(
                format!("{path} is gone from the source, so the transfer can't go on"),
                "check that it wasn't moved or deleted; if it was, start a new transfer with what is there now",
                pb::error::Kind::NotFound(pb::NotFound {
                    kind: pb::EntityKind::File as i32,
                    id: path.clone(),
                    in_trash: false,
                }),
            ),
        ),
        P::Unsupported { path } => (
            Code::InvalidArgument,
            error(
                format!("{path} is a kind of file that can't be transferred (a socket, device or similar)"),
                "start the transfer again without it",
                pb::error::Kind::InvalidField(pb::InvalidField {
                    field: "sources".to_owned(),
                    problem: Field::NotAllowedHere as i32,
                }),
            ),
        ),
        P::Offline { machine: name } => (
            Code::Unavailable,
            error(
                format!("{name} can't be reached right now, so the transfer is waiting for it"),
                "nothing to do: the transfer continues by itself when the machine's link is back. Use Reconnect now on the machine's page to hurry it, or cancel the transfer to remove what arrived",
                pb::error::Kind::MachineOffline(pb::MachineOffline {
                    machine: Some(pb::MachineId { value: name.clone() }),
                    machine_name: name.clone(),
                    since: None,
                }),
            ),
        ),
        P::Interrupted => (
            Code::Aborted,
            error(
                "the upload's stream ended before the whole file arrived".to_owned(),
                "resume it with the transfer id as resume in a new upload header; it continues from the bytes it has. Or cancel it to remove them",
                pb::error::Kind::InvalidField(pb::InvalidField {
                    field: "chunk.last".to_owned(),
                    problem: Field::Missing as i32,
                }),
            ),
        ),
        P::Io { doing, path, message } => {
            let incident = format!("in_{}", uuid::Uuid::new_v4().simple());
            eprintln!("humatad2: incident {incident}: transfer {id} couldn't {doing} {path}: {message}");
            (
                Code::Internal,
                error(
                    format!("couldn't {doing} {path}: {message} (incident {incident})"),
                    "check that the folders belong to you and the disk is healthy and not read-only, then Resume the transfer; if it fails again, attach the daemon's log to a report",
                    pb::error::Kind::Internal(pb::Internal {
                        incident_id: incident,
                        log_path: String::new(),
                    }),
                ),
            )
        }
    }
}

/// A transfer as clients see it.
fn transfer(machine: &str, snapshot: &tr::Snapshot, data: Data) -> pb::Transfer {
    let machine = pb::MachineId {
        value: machine.to_owned(),
    };
    pb::Transfer {
        id: Some(pb::TransferId {
            value: snapshot.id.clone(),
        }),
        sources: data.sources,
        destination: data.destination,
        mode: match snapshot.mode {
            tr::Mode::Copy => pb::TransferMode::Copy,
            tr::Mode::Move => pb::TransferMode::Move,
        } as i32,
        state: match snapshot.state {
            tr::State::Running => pb::TransferState::Running,
            tr::State::Paused => pb::TransferState::Paused,
            tr::State::Verifying => pb::TransferState::Verifying,
            tr::State::Done => pb::TransferState::Done,
            tr::State::Failed if remote_end::waiting_for(snapshot).is_some() => {
                pb::TransferState::WaitingForMachine
            }
            tr::State::Failed => pb::TransferState::Failed,
            tr::State::Cancelled => pb::TransferState::Cancelled,
        } as i32,
        done_bytes: snapshot.done_bytes,
        total_bytes: snapshot.total_bytes,
        done_files: snapshot.done_files,
        total_files: snapshot.total_files,
        problem: snapshot
            .problem
            .as_ref()
            .map(|problem| problem_error(&machine, &snapshot.id, problem).1),
        started_at: Some(timestamp(
            UNIX_EPOCH + Duration::from_millis(u64::try_from(snapshot.started_unix_ms).unwrap_or(0)),
        )),
        version: snapshot.version,
        disk_path: String::new(),
        outcomes: outcomes(snapshot),
    }
}

/// What the tray says about each source of a finished transfer (PL-13).
fn outcomes(snapshot: &tr::Snapshot) -> Vec<pb::TransferOutcome> {
    if snapshot.state != tr::State::Done {
        return Vec::new();
    }
    snapshot
        .landings
        .iter()
        .map(|landing| match landing {
            tr::Landing::AsAsked => pb::TransferOutcome {
                landing: pb::TransferLanding::Unspecified as i32,
                saved_as: String::new(),
            },
            tr::Landing::SavedAs(name) => pb::TransferOutcome {
                landing: pb::TransferLanding::SavedAs as i32,
                saved_as: name.clone(),
            },
            tr::Landing::AlreadyThere => pb::TransferOutcome {
                landing: pb::TransferLanding::AlreadyThere as i32,
                saved_as: String::new(),
            },
        })
        .collect()
}

fn id_of(id: Option<pb::TransferId>) -> Result<String, Status> {
    id.map(|id| id.value)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| missing("id"))
}

impl TransferSvc {
    async fn controlled(
        &self,
        who: Caller,
        request: pb::TransferControlRequest,
        name: &'static str,
        what: Control,
    ) -> Result<Response<pb::Transfer>, Status> {
        let id = id_of(request.id)?;
        let key = answer_key(name, &who, request.idempotency_key.as_ref(), &id);
        if let Some(answer) = key.as_deref().and_then(|key| self.core.controlled.get(key)) {
            return Ok(Response::new(answer));
        }
        let core = self.core.clone();
        let answer = blocking(move || core.control(&who, &id, what)).await??;
        if let Some(key) = key {
            self.core.controlled.put(key, answer.clone());
        }
        Ok(Response::new(answer))
    }
}

#[tonic::async_trait]
impl pb::transfer_service_server::TransferService for TransferSvc {
    /// Copies or moves files and folders between this machine's roots.
    /// The transfer runs on after the call returns; watch it.
    async fn start_transfer(
        &self,
        request: Request<pb::StartTransferRequest>,
    ) -> Result<Response<pb::Transfer>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        if let Some(answer) = self.ask_host(&who, &request).await {
            return answer;
        }
        let mode = mode_of(request.mode)?;
        let on_existing = on_existing(request.on_existing, "on_existing")?;
        let begin = Begin {
            key: answer_key("start", &who, request.idempotency_key.as_ref(), ""),
            sources: request.sources,
            destination: request.destination,
            mode,
            on_existing,
            by: actor(&who),
            approved: false,
            name: request.name,
        };
        let key = request.idempotency_key.as_ref().map(|key| key.value.clone());
        let again = begin.clone();
        let core = self.core.clone();
        let first = {
            let who = who.clone();
            blocking(move || core.start(&who, begin)).await??
        };
        match first {
            Started::Done(started) => Ok(Response::new(started)),
            Started::Ask(asked, preview) => {
                let core = self.core.clone();
                let by = who.clone();
                let run: Run = Box::new(move |_| {
                    Box::pin(async move {
                        let begin = Begin { approved: true, ..again };
                        match blocking(move || core.start(&by, begin)).await?? {
                            Started::Done(done) => Ok(Deferred {
                                result: done.encode_to_vec(),
                                result_type: "humata.v1.Transfer".to_owned(),
                            }),
                            Started::Ask(..) => Err(internal(
                                "an approved copy was sent to Ask me a second time",
                                "",
                            )),
                        }
                    })
                });
                let card = BoardAsk {
                    preview: pb::request_preview::Preview::CrossMachineCopy(*preview),
                    reason: request.reason.clone(),
                };
                match self
                    .inbox
                    .ask(&who, *asked, key.as_deref().unwrap_or(""), Some(run), Some(card))
                    .await
                {
                    Ok(()) => Err(internal("copy approval did not return a deferred status", "")),
                    Err(status) => Err(status),
                }
            }
        }
    }

    /// The header, then chunks in order from the upload's offset, each in
    /// any codec this build decodes. The file appears only once all of it
    /// arrived and its size and SHA-256 match the header's. A stream that
    /// ends early leaves a paused transfer to resume or cancel.
    async fn upload(
        &self,
        request: Request<Streaming<pb::UploadMessage>>,
    ) -> Result<Response<pb::Transfer>, Status> {
        use pb::upload_message::Message as Part;

        let who = caller(&request)?;
        let mut messages = request.into_inner();
        let header = match messages.message().await? {
            Some(pb::UploadMessage {
                message: Some(Part::Header(header)),
            }) => header,
            _ => return Err(missing("header")),
        };
        let by = actor(&who);
        let name = header.name.clone();
        let core = self.core.clone();
        let began = blocking(move || core.begin_upload(&who, header, by)).await??;
        let (mut upload, id) = match began {
            Began::Existing(found) => {
                let core = self.core.clone();
                let found = blocking(move || core.with_pasted_path(found, &name)).await??;
                return Ok(Response::new(found));
            }
            Began::Upload(upload, id) => (Some(upload), id),
        };
        let core = self.core.clone();
        let mut decoder = Some(Decoder::new());
        let mut offset = upload.as_ref().map(tr::Upload::offset).unwrap_or(0);
        loop {
            let chunk = match messages.message().await {
                Ok(Some(pb::UploadMessage {
                    message: Some(Part::Chunk(chunk)),
                })) => chunk,
                Ok(Some(_)) => {
                    release(upload).await;
                    return Err(invalid_field(
                        "header",
                        Field::NotAllowedHere,
                        "the upload sent a second header",
                        "send one header, then the file's chunks",
                    ));
                }
                // The stream ended or broke before the last chunk: what
                // arrived is kept, paused, to resume (PER-27, FM-4).
                Ok(None) | Err(_) => {
                    release(upload).await;
                    return Err(interrupted(&id, offset));
                }
            };
            if chunk.offset != offset {
                release(upload).await;
                return Err(invalid_field(
                    "chunk.offset",
                    Field::OutOfRange,
                    format!(
                        "a chunk starts at byte {}, but upload {id} has {offset} bytes",
                        chunk.offset
                    ),
                    format!("send the chunks in order, continuing from byte {offset}"),
                ));
            }
            let last = chunk.last;
            let (Some(mut up), Some(mut unpack)) = (upload.take(), decoder.take()) else {
                return Err(internal("an upload lost its state", ""));
            };
            let core = core.clone();
            // A write that fails drops the upload here, which pauses it.
            let (up, unpack, wrote) = blocking(move || {
                let header = compression::chunk_header(&chunk).ok_or_else(|| {
                    chunk_failure(&format!("chunk codec {} isn't one this build knows", chunk.codec))
                })?;
                let raw = unpack
                    .decode(header, chunk.data)
                    .map_err(|err| chunk_failure(&chain(&err)))?;
                let wrote = raw.len() as u64;
                up.write(chunk.offset, &raw).map_err(|err| core.failure(&err))?;
                Ok::<_, Status>((up, unpack, wrote))
            })
            .await??;
            offset += wrote;
            if !last {
                upload = Some(up);
                decoder = Some(unpack);
                continue;
            }
            let core = self.core.clone();
            let finished = blocking(move || {
                up.finish()
                    .map(|snapshot| core.view(&snapshot))
                    .map_err(|err| core.failure(&err))
                    .and_then(|finished| core.with_pasted_path(finished, &name))
            })
            .await??;
            return Ok(Response::new(finished));
        }
    }

    /// Newest first.
    async fn list_transfers(
        &self,
        request: Request<pb::ListTransfersRequest>,
    ) -> Result<Response<pb::ListTransfersResponse>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        let (size, token) = match request.page {
            Some(page) => (page.page_size as usize, page.page_token),
            None => (0, String::new()),
        };
        let core = self.core.clone();
        let listed = blocking(move || core.list(&who, request.active_only, size, &token)).await??;
        Ok(Response::new(listed))
    }

    type WatchTransferStream = WatchTransfersStream;

    /// The transfer as it is (when newer than `from_version`), then each
    /// change as it is made, then the stream ends with its final state. A
    /// watcher that falls behind gets the latest state, not every step
    /// between.
    async fn watch_transfer(
        &self,
        request: Request<pb::WatchTransferRequest>,
    ) -> Result<Response<Self::WatchTransferStream>, Status> {
        let who = caller(&request)?;
        let request = request.into_inner();
        let id = id_of(request.id)?;
        // Subscribed before the read, so no change falls between them.
        let mut changes = self.core.changes.subscribe();
        let core = self.core.clone();
        let wanted = id.clone();
        let current = blocking(move || core.read(&who, &wanted)).await??;
        let (sender, receiver) = mpsc::channel(4);
        let core = self.core.clone();
        let mut from = request.from_version;
        tokio::spawn(async move {
            let mut next = Some(current);
            loop {
                if let Some(snapshot) = next.take() {
                    if snapshot.version > from {
                        from = snapshot.version;
                        let done = finished(&snapshot);
                        if sender.send(Ok(core.view(&snapshot))).await.is_err() || done {
                            return;
                        }
                    } else if finished(&snapshot) {
                        return;
                    }
                }
                tokio::select! {
                    got = changes.recv() => match got {
                        Ok(snapshot) if snapshot.id == id => next = Some(snapshot),
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            // Read where it stands now.
                            let engine = core.clone();
                            let wanted = id.clone();
                            match blocking(move || engine.engine().map(|engine| engine.get(&wanted))).await {
                                Ok(Ok(Some(snapshot))) => next = Some(snapshot),
                                _ => return,
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => return,
                    },
                    () = sender.closed() => return,
                }
            }
        });
        Ok(Response::new(ReceiverStream::new(receiver)))
    }

    /// Stops a running transfer where it is; what arrived is kept. An
    /// upload keeps its place to continue from.
    async fn pause_transfer(
        &self,
        request: Request<pb::TransferControlRequest>,
    ) -> Result<Response<pb::Transfer>, Status> {
        let who = caller(&request)?;
        self.controlled(who, request.into_inner(), "pause", Control::Pause)
            .await
    }

    /// Continues a paused or failed copy or move from where it stopped.
    async fn resume_transfer(
        &self,
        request: Request<pb::TransferControlRequest>,
    ) -> Result<Response<pb::Transfer>, Status> {
        let who = caller(&request)?;
        self.controlled(who, request.into_inner(), "resume", Control::Resume)
            .await
    }

    /// Removes what reached the destination; the source is untouched.
    async fn cancel_transfer(
        &self,
        request: Request<pb::TransferControlRequest>,
    ) -> Result<Response<pb::Transfer>, Status> {
        let who = caller(&request)?;
        self.controlled(who, request.into_inner(), "cancel", Control::Cancel)
            .await
    }
}

/// Drops an upload on a blocking thread: that syncs what arrived and
/// pauses the transfer.
async fn release(upload: Option<tr::Upload>) {
    if let Some(upload) = upload {
        let _ = blocking(move || drop(upload)).await;
    }
}

fn interrupted(id: &str, offset: u64) -> Status {
    failure(
        Code::Aborted,
        error(
            format!(
                "the upload ended before its last chunk; transfer {id} keeps the {offset} bytes that arrived"
            ),
            format!(
                "continue it with an upload header whose resume is {id} (same destination, name, size and sha256), sending from byte {offset}, or cancel the transfer to remove what arrived"
            ),
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "chunk.last".to_owned(),
                problem: Field::Missing as i32,
            }),
        ),
    )
}

fn chunk_failure(what: &str) -> Status {
    failure(
        Code::DataLoss,
        error(
            format!("a chunk couldn't be read: {what}"),
            "send the chunk again; if it fails the same way, resume the upload from the transfer's done bytes",
            pb::error::Kind::InvalidField(pb::InvalidField {
                field: "chunk.data".to_owned(),
                problem: Field::BadFormat as i32,
            }),
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use humata2_daemon_ports::UserVia;
    use humata2_grpc::humata::v1::transfer_service_server::TransferService;
    use humata2_store::Store;

    use super::*;

    struct AllowAllGuard;

    impl TransferGuardPort for AllowAllGuard {
        fn check(&self, _caller: &Caller, _call: &Call) -> Result<Checked, Status> {
            Ok(Checked::Allowed)
        }
        fn require(&self, _caller: &Caller, _call: &Call) -> Result<(), Status> {
            Ok(())
        }
    }

    struct NoopInbox;

    #[tonic::async_trait]
    impl TransferInboxPort for NoopInbox {
        async fn ask(
            &self,
            _caller: &Caller,
            _asked: AccessRequest,
            _operation: &str,
            _run: Option<Run>,
            _board: Option<BoardAsk>,
        ) -> Result<(), Status> {
            Ok(())
        }
    }

    struct TempTrash(PathBuf);

    impl TrashBinPort for TempTrash {
        fn get(&self) -> Result<Arc<tr::Trash>, Status> {
            tr::Trash::open(self.0.clone())
                .map(Arc::new)
                .map_err(|err| internal(err.to_string(), ""))
        }
    }

    fn user_req<T>(msg: T) -> Request<T> {
        let mut req = Request::new(msg);
        req.extensions_mut().insert(Caller::User(UserVia::App));
        req
    }

    fn test_svc(dir: &tempfile::TempDir) -> Arc<TransferSvc> {
        let state = dir.path().join("state");
        let channels = dir.path().join("channels");
        let home = dir.path().join("home");
        let trash = dir.path().join("trash");
        fs::create_dir_all(&channels).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&trash).unwrap();
        let store = Arc::new(Store::open(&state).expect("open store"));
        let roots = Arc::new(Roots::new(store, "local".to_owned(), channels, home));
        TransferSvc::new(
            roots,
            Arc::new(AllowAllGuard),
            Arc::new(TempTrash(trash)),
            state.clone(),
            "local".to_owned(),
            Arc::new(Compression::new(&state)),
            Arc::new(NoopInbox),
        )
    }

    #[test]
    fn encode_and_decode_transfer_data_round_trips_and_tolerates_corrupt_hex() {
        let original = Data {
            sources: vec![pb::FileLocation {
                machine: Some(pb::MachineId {
                    value: "local".to_owned(),
                }),
                root: None,
                path: "notes.txt".to_owned(),
            }],
            destination: Some(pb::FileLocation {
                machine: Some(pb::MachineId {
                    value: "local".to_owned(),
                }),
                root: None,
                path: "dest".to_owned(),
            }),
            key: "idem-1".to_owned(),
            by: None,
            name: "notes.txt".to_owned(),
            size: 42,
            sha256: "abc123".to_owned(),
        };
        let encoded = encode(&original);
        assert_eq!(decode(&encoded), original);
        assert_eq!(decode("not-valid-hex!"), Data::default());
        assert_eq!(decode("abc"), Data::default());
    }

    #[tokio::test]
    async fn start_transfer_rejects_empty_sources_and_missing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);

        let no_sources = svc
            .start_transfer(user_req(pb::StartTransferRequest {
                sources: Vec::new(),
                destination: Some(pb::FileLocation {
                    machine: Some(pb::MachineId {
                        value: "local".to_owned(),
                    }),
                    root: None,
                    path: "dest".to_owned(),
                }),
                ..Default::default()
            }))
            .await
            .expect_err("empty sources must be rejected");
        assert_eq!(no_sources.code(), Code::InvalidArgument);

        let no_dest = svc
            .start_transfer(user_req(pb::StartTransferRequest {
                sources: vec![pb::FileLocation {
                    machine: Some(pb::MachineId {
                        value: "local".to_owned(),
                    }),
                    root: Some(pb::file_location::Root::Home(Default::default())),
                    path: "a.txt".to_owned(),
                }],
                destination: None,
                ..Default::default()
            }))
            .await
            .expect_err("missing destination must be rejected");
        assert_eq!(no_dest.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn list_and_cancel_unknown_transfer_over_temp_state_dir() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);

        let listed = svc
            .list_transfers(user_req(pb::ListTransfersRequest::default()))
            .await
            .unwrap()
            .into_inner();
        assert!(listed.transfers.is_empty());

        let missing_err = svc
            .cancel_transfer(user_req(pb::TransferControlRequest {
                id: Some(pb::TransferId {
                    value: "tr_does_not_exist".to_owned(),
                }),
                idempotency_key: None,
            }))
            .await
            .expect_err("unknown transfer id must return NotFound");
        assert_eq!(missing_err.code(), Code::NotFound);
    }
}
