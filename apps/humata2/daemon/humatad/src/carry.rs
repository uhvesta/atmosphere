//! The host's side of carrying a session between machines (D195): calls on
//! the machine that has it and on the one that gets it, the copy of its
//! folder and staged files through the transfer engine, and the move's
//! journal. The steps and their order are `session_service/move_session.rs`'
//! own; this is the machinery.
//!
//! The engine here is an instance of its own, kept out of the tray: its
//! journals live in `<state>/carry/transfers`, beside the move journals
//! `<state>/carry/<operation>.json`. The host relays the carried bytes and
//! keeps none of them (SEC-1).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use humata2_grpc::humata::v1 as pb;
use humata2_transfers as tr;
use serde_json::json;
use tonic::{Code, Status};

use crate::compression::Compression;
use crate::host_relay::Links;
use crate::remote_end::{RemoteEnd, RouterEnd, virtual_path};
use crate::carry_service::SessionCarrySvc;
use crate::status;
use crate::trash_service::{Bin, blocking, lock, now_ms};

/// Free space the engine leaves on a disk it copies to.
const FREE_FLOOR_BYTES: u64 = 1 << 30;
const SYNC_EVERY_BYTES: u64 = 8 << 20;

/// A channel's folder on one machine.
#[derive(Clone, Debug)]
pub(crate) struct Place {
    pub(crate) machine: String,
    pub(crate) channel: String,
    /// The channel's folder name, which is the same on every machine.
    pub(crate) folder: String,
}

struct Inner {
    local: Arc<SessionCarrySvc>,
    links: Links,
    remote: Arc<RemoteEnd>,
    bin: Arc<Bin>,
    state: PathBuf,
    /// The host's own machine id.
    host: String,
    /// Where the host keeps channels.
    channels: PathBuf,
    open: Mutex<Option<(Arc<tr::Transfers>, Arc<RouterEnd>)>>,
}

/// What the host's SessionService holds to move sessions. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Carry(Arc<Inner>);

impl Carry {
    /// Inside the runtime: the engine's blocking threads reach it through
    /// the handle taken here. Touches no disk.
    pub(crate) fn new(
        local: Arc<SessionCarrySvc>,
        links: Links,
        bin: Arc<Bin>,
        state: PathBuf,
        host: String,
        channels: PathBuf,
        compression: Arc<Compression>,
    ) -> Self {
        let remote = Arc::new(RemoteEnd::new(tokio::runtime::Handle::current(), compression));
        remote.use_links(links.clone());
        Self(Arc::new(Inner {
            local,
            links,
            remote,
            bin,
            state,
            host,
            channels,
            open: Mutex::new(None),
        }))
    }

    /// The host's links to other machines, for the calls a session's archive and restore relay.
    pub(crate) fn links(&self) -> &Links {
        &self.0.links
    }

    pub(crate) fn is_host(&self, machine: &str) -> bool {
        self.0.is_host(machine)
    }

    fn path(&self, place: &Place, relative: &str) -> PathBuf {
        self.0.path(place, relative)
    }
}

impl Inner {
    fn is_host(&self, machine: &str) -> bool {
        machine.is_empty() || machine == self.host
    }

    /// The engine and the end it reads and writes through, opened on first
    /// use. Blocking.
    fn engine(&self) -> Result<(Arc<tr::Transfers>, Arc<RouterEnd>), Status> {
        let mut open = lock(&self.open);
        if let Some(found) = &*open {
            return Ok(found.clone());
        }
        let trash = self.bin.get()?;
        let router = Arc::new(RouterEnd::new(
            Arc::new(tr::Disk::new(self.host.clone(), trash)),
            self.remote.clone(),
        ));
        let engine = tr::Transfers::open(tr::Config {
            journal: self.state.join("carry").join("transfers"),
            source: router.clone(),
            destination: router.clone(),
            free_floor_bytes: FREE_FLOOR_BYTES,
            sync_every_bytes: SYNC_EVERY_BYTES,
            listener: Arc::new(|_| {}),
        })
        .map_err(|error| {
            status::internal(
                format!(
                    "the move's transfer journals in {} can't be used: {}. Check that the folder belongs to you and the disk has free space; nothing was deleted",
                    self.state.join("carry").join("transfers").display(),
                    status::chain(&error)
                ),
                "",
            )
        })?;
        *open = Some((engine.clone(), router.clone()));
        Ok((engine, router))
    }

    /// `relative` in `place`'s channel folder, as the engine names it.
    fn path(&self, place: &Place, relative: &str) -> PathBuf {
        if self.is_host(&place.machine) {
            return self.channels.join(&place.folder).join(relative);
        }
        virtual_path(
            &place.machine,
            &pb::FileLocation {
                machine: None,
                root: Some(pb::file_location::Root::ChannelFolder(pb::ChannelFolder {
                    channel: Some(pb::ChannelId {
                        value: place.channel.clone(),
                    }),
                })),
                path: relative.to_owned(),
            },
        )
    }

}

impl Carry {
    /// Whether `relative` exists in `place`'s channel folder.
    pub(crate) async fn exists(&self, place: &Place, relative: &str) -> Result<bool, Status> {
        let path = self.path(place, relative);
        let inner = self.0.clone();
        blocking(move || {
            let (_, router) = inner.engine()?;
            tr::End::exists(&*router, &path).map_err(|error| end_failure(&error))
        })
        .await?
    }

    // -- the calls on a machine ---------------------------------------------

    pub(crate) async fn pack(&self, machine: &str, operation: &str, session: &str) -> Result<pb::CarryPackResponse, Status> {
        if self.is_host(machine) {
            let packed = self.0.local.pack(operation, session).await?;
            return Ok(pb::CarryPackResponse {
                folder: packed.folder,
                stage: packed.stage,
                digest: packed.digest,
                files: packed.files,
                bytes: packed.bytes,
                agent_running: packed.agent_running,
                ..pb::CarryPackResponse::default()
            });
        }
        let mut client = self.client(machine)?;
        client
            .pack(pb::CarryPackRequest {
                operation: operation.to_owned(),
                session: Some(pb::SessionId { value: session.to_owned() }),
                fork: None,
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// Stages a fork of `parent` on `machine` for another machine (FK-8).
    pub(crate) async fn pack_fork(
        &self,
        machine: &str,
        operation: &str,
        parent: &str,
        fork: pb::CarryFork,
    ) -> Result<pb::CarryPackResponse, Status> {
        if self.is_host(machine) {
            return self.0.local.pack_fork(operation, parent, &fork).await;
        }
        let mut client = self.client(machine)?;
        client
            .pack(pb::CarryPackRequest {
                operation: operation.to_owned(),
                session: Some(pb::SessionId { value: parent.to_owned() }),
                fork: Some(fork),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// What a fork of `parent` on `machine` would get there (FK-4): the
    /// branch and the bytes, staging nothing. `typed` empty: picked from
    /// `title`.
    pub(crate) async fn preview_fork(
        &self,
        machine: &str,
        parent: &str,
        title: &str,
        typed: &str,
    ) -> Result<pb::CarryPreviewForkResponse, Status> {
        if self.is_host(machine) {
            return self.0.local.preview_fork(parent, title, typed).await;
        }
        let mut client = self.client(machine)?;
        client
            .preview_fork(pb::CarryPreviewForkRequest {
                session: Some(pb::SessionId { value: parent.to_owned() }),
                title: title.to_owned(),
                branch: typed.to_owned(),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// Takes a carried new session in whole: a box always does, the host
    /// does here because it has no record of a fork before this.
    pub(crate) async fn unpack_new(
        &self,
        machine: &str,
        operation: &str,
        session: &str,
        channel: &str,
    ) -> Result<(), Status> {
        if self.is_host(machine) {
            return self.0.local.unpack_new(operation, session, channel).await.map(drop);
        }
        self.unpack(machine, operation, session, channel).await.map(drop)
    }

    /// Takes back a carried new session from `machine`, records included.
    pub(crate) async fn abort_new(
        &self,
        machine: &str,
        operation: &str,
        session: &str,
        place: &Place,
        folder: &str,
    ) -> Result<(), Status> {
        if self.is_host(machine) {
            return self.0.local.abort_new(operation, session, &place.channel, folder).await;
        }
        self.abort(machine, operation, session, place, folder, true).await
    }

    pub(crate) async fn unpack(
        &self,
        machine: &str,
        operation: &str,
        session: &str,
        channel: &str,
    ) -> Result<pb::CarryUnpackResponse, Status> {
        if self.is_host(machine) {
            let unpacked = self.0.local.unpack(operation, session, channel).await?;
            return Ok(pb::CarryUnpackResponse {
                files: unpacked.files,
                bytes: unpacked.bytes,
            });
        }
        let mut client = self.client(machine)?;
        client
            .unpack(pb::CarryUnpackRequest {
                operation: operation.to_owned(),
                session: Some(pb::SessionId { value: session.to_owned() }),
                channel: Some(pb::ChannelId { value: channel.to_owned() }),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    pub(crate) async fn release(
        &self,
        machine: &str,
        operation: &str,
        session: &str,
        channel: &str,
    ) -> Result<(), Status> {
        if self.is_host(machine) {
            return self.0.local.release(operation, session, channel).await.map(drop);
        }
        let mut client = self.client(machine)?;
        client
            .release(pb::CarryReleaseRequest {
                operation: operation.to_owned(),
                session: Some(pb::SessionId { value: session.to_owned() }),
            })
            .await
            .map(drop)
    }

    pub(crate) async fn abort(
        &self,
        machine: &str,
        operation: &str,
        session: &str,
        place: &Place,
        folder: &str,
        incoming: bool,
    ) -> Result<(), Status> {
        if self.is_host(machine) {
            return self.0.local.abort(operation, session, &place.channel, folder, incoming).await;
        }
        let mut client = self.client(machine)?;
        client
            .abort(pb::CarryAbortRequest {
                operation: operation.to_owned(),
                session: Some(pb::SessionId { value: session.to_owned() }),
                channel: Some(pb::ChannelId { value: place.channel.clone() }),
                folder: folder.to_owned(),
                incoming,
            })
            .await
            .map(drop)
    }

    fn client(
        &self,
        machine: &str,
    ) -> Result<CarryClient, Status> {
        Ok(CarryClient(self.0.links.relayed(machine).map_err(Status::from)?.carry()))
    }

    // -- the copy -------------------------------------------------------------

    /// Copies `from_relative` of `from`'s channel folder into the folder
    /// `into` of `to`'s (made if it isn't there), checksummed, and returns
    /// once it is all there under its name or has failed. A copy that fails
    /// leaves nothing of itself in `to`.
    pub(crate) async fn copy(
        &self,
        id: &str,
        from: (&Place, &str),
        to: (&Place, &str),
    ) -> Result<(), Status> {
        let (source, destination) = (self.path(from.0, from.1), self.path(to.0, to.1));
        let levels: Vec<PathBuf> = to
            .1
            .match_indices('/')
            .map(|(at, _)| self.path(to.0, &to.1[..at]))
            .chain(std::iter::once(destination.clone()))
            .collect();
        let (shown_from, shown_to) = (
            format!("{} on {}", from.1, from.0.machine),
            format!("{} on {}", to.1, to.0.machine),
        );
        let (this, id) = (self.0.clone(), id.to_owned());
        let outcome = blocking(move || -> Result<(), Status> {
            let (engine, router) = this.engine()?;
            // A machine makes a folder only where its parent exists, so each
            // level is made in turn.
            for level in &levels {
                tr::End::make_folder(&*router, level, 0o700).map_err(|error| end_failure(&error))?;
            }
            let started = engine.start(tr::Request {
                id: id.clone(),
                sources: vec![tr::Item { path: source, shown: shown_from.clone() }],
                destination: tr::Item { path: destination, shown: shown_to.clone() },
                mode: tr::Mode::Copy,
                on_existing: tr::OnExisting::Fail,
                caller_data: String::new(),
                now_unix_ms: now_ms(),
            })
            .map_err(|error| copy_failure(&shown_from, &shown_to, &status::chain(&error)))?;
            let ended = engine.wait(&started.id).unwrap_or(started);
            match (ended.state, ended.problem) {
                (tr::State::Done, _) => Ok(()),
                (_, problem) => {
                    // What reached the destination goes again.
                    let _ = engine.cancel(&id);
                    Err(copy_failure(
                        &shown_from,
                        &shown_to,
                        &problem.map_or_else(|| "the copy stopped".to_owned(), |problem| describe(&problem)),
                    ))
                }
            }
        })
        .await?;
        outcome
    }

    // -- the journal ------------------------------------------------------------

    /// Writes where the move stands, so a restart finds what it was doing
    /// (MV-14, D195). Best effort: a move that can't write its journal still
    /// moves, and says so in the log.
    pub(crate) async fn journal(&self, operation: &str, entry: serde_json::Value) {
        let folder = self.0.state.join("carry");
        let path = folder.join(format!("{operation}.json"));
        let written = blocking(move || {
            std::fs::create_dir_all(&folder)?;
            let bytes = serde_json::to_vec_pretty(&entry).unwrap_or_default();
            humata2_store::write_atomic(&path, &bytes).map_err(std::io::Error::other)
        })
        .await;
        if let Ok(Err(error)) = written {
            crate::log::say!("humatad2: couldn't write the journal of move {operation}: {error}");
        }
    }

    /// The move is over; its journal goes.
    pub(crate) async fn journal_done(&self, operation: &str) {
        let path = self.0.state.join("carry").join(format!("{operation}.json"));
        let _ = blocking(move || std::fs::remove_file(path)).await;
    }

    /// Writes where a fork onto or from another machine stands, in
    /// `<state>/carry/forks`, so a restart can finish or undo it. Best
    /// effort, as a move's journal is.
    pub(crate) async fn journal_fork(&self, entry: &ForkJournal) {
        let folder = self.0.state.join("carry").join("forks");
        let path = folder.join(format!("{}.json", entry.operation));
        let (operation, value) = (entry.operation.clone(), entry.value());
        let written = blocking(move || {
            std::fs::create_dir_all(&folder)?;
            let bytes = serde_json::to_vec_pretty(&value).unwrap_or_default();
            humata2_store::write_atomic(&path, &bytes).map_err(std::io::Error::other)
        })
        .await;
        if let Ok(Err(error)) = written {
            crate::log::say!("humatad2: couldn't write the journal of fork {operation}: {error}");
        }
    }

    /// `<state>/carry/remote_archive`, where a remote archive's or restore's
    /// follow-ups wait for a machine's link (D222).
    pub(crate) fn remote_archive_folder(&self) -> PathBuf {
        self.0.state.join("carry").join("remote_archive")
    }

    /// The fork is over, made or undone; its journal goes.
    pub(crate) async fn fork_done(&self, operation: &str) {
        let path = self.0.state.join("carry").join("forks").join(format!("{operation}.json"));
        let _ = blocking(move || std::fs::remove_file(path)).await;
    }

    /// The forks a run left unfinished. Read once at start; a journal that
    /// can't be read is logged and left in place.
    pub(crate) fn fork_journals(&self) -> Vec<ForkJournal> {
        let folder = self.0.state.join("carry").join("forks");
        let Ok(entries) = std::fs::read_dir(&folder) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") || !path.is_file() {
                continue;
            }
            let read = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|value| ForkJournal::from_value(&value));
            match read {
                Some(journal) => found.push(journal),
                None => crate::log::say!(
                    "humatad2: couldn't read the journal {} of a fork, so that fork isn't finished or undone. Nothing was deleted; look at the fork's machine, then delete the file",
                    path.display()
                ),
            }
        }
        found
    }

    /// The moves a run left unfinished, as their journals say. Read once at
    /// start, before anything is served. A journal that can't be read is
    /// logged and left in place.
    pub(crate) fn journals(&self) -> Vec<Journaled> {
        let folder = self.0.state.join("carry");
        let Ok(entries) = std::fs::read_dir(&folder) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") || !path.is_file() {
                continue;
            }
            let read = std::fs::read(&path).map_err(|error| error.to_string()).and_then(|bytes| {
                serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| error.to_string())
            });
            let text = |value: &serde_json::Value, key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_owned);
            match read {
                Ok(value) => match (
                    text(&value, "operation"),
                    text(&value, "session"),
                    text(&value, "from"),
                    text(&value, "to"),
                    text(&value, "phase"),
                ) {
                    (Some(operation), Some(session), Some(from), Some(to), Some(phase)) => {
                        let agent_ran = value.get("agent_ran").and_then(serde_json::Value::as_bool).unwrap_or(true);
                        found.push(Journaled { operation, session, from, to, phase, agent_ran });
                    }
                    _ => crate::log::say!(
                        "humatad2: the journal {} of a move names no session or machines, so that move isn't recovered. Nothing was deleted; delete the file if the session is where you want it",
                        path.display()
                    ),
                },
                Err(error) => crate::log::say!(
                    "humatad2: couldn't read the journal {} of a move: {error}. Nothing was deleted; check the file, or delete it if the session is where you want it",
                    path.display()
                ),
            }
        }
        found
    }

    /// What a move's journal holds at a step.
    pub(crate) fn entry(
        operation: &str,
        session: &str,
        from: &str,
        to: &str,
        phase: &str,
        agent_ran: bool,
    ) -> serde_json::Value {
        json!({
            "operation": operation,
            "session": session,
            "from": from,
            "to": to,
            "phase": phase,
            "agent_ran": agent_ran,
            "at_unix_ms": now_ms(),
        })
    }
}

/// Where a fork onto or from another machine stands, as its journal keeps
/// it.
#[derive(Clone, Debug)]
pub(crate) struct ForkJournal {
    pub(crate) operation: String,
    pub(crate) parent: String,
    /// The fork's id.
    pub(crate) session: String,
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) channel: String,
    /// `sessions/<name>` once the pack named it; empty before.
    pub(crate) folder: String,
    /// The last step reached: started, packed, copying (something of the
    /// fork may be on the target from here), copied, unpacked, or recorded
    /// (the host's record is written and the agent is being started).
    pub(crate) phase: String,
    /// Set for a context-only fork on the parent's own machine (D218,
    /// D221): what the host asked that machine, so a restart asks again
    /// under the same key. None for a fork that copies.
    pub(crate) context: Option<ContextAsk>,
}

/// What the host asked a machine for a context-only fork of one of its own
/// sessions, as the fork's journal keeps it.
#[derive(Clone, Debug, Default)]
pub(crate) struct ContextAsk {
    pub(crate) title: String,
    pub(crate) before_turn: u32,
    /// The fork's agent as asked; empty: the parent's.
    pub(crate) agent: String,
    pub(crate) model: String,
    /// The machine's operation once it answered; empty before.
    pub(crate) asked: String,
}

impl ContextAsk {
    fn value(&self) -> serde_json::Value {
        json!({
            "title": self.title,
            "before_turn": self.before_turn,
            "agent": self.agent,
            "model": self.model,
            "asked": self.asked,
        })
    }

    fn from_value(value: &serde_json::Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_owned);
        Some(Self {
            title: text("title")?,
            before_turn: u32::try_from(value.get("before_turn")?.as_u64()?).ok()?,
            agent: text("agent")?,
            model: text("model")?,
            asked: text("asked")?,
        })
    }
}

impl ForkJournal {
    fn value(&self) -> serde_json::Value {
        json!({
            "operation": self.operation,
            "parent": self.parent,
            "session": self.session,
            "from": self.from,
            "to": self.to,
            "channel": self.channel,
            "folder": self.folder,
            "phase": self.phase,
            "context": self.context.as_ref().map(ContextAsk::value),
            "at_unix_ms": now_ms(),
        })
    }

    fn from_value(value: &serde_json::Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_owned);
        Some(Self {
            operation: text("operation")?,
            parent: text("parent")?,
            session: text("session")?,
            from: text("from")?,
            to: text("to")?,
            channel: text("channel")?,
            folder: text("folder")?,
            phase: text("phase")?,
            context: match value.get("context") {
                None | Some(serde_json::Value::Null) => None,
                Some(context) => Some(ContextAsk::from_value(context)?),
            },
        })
    }
}

/// A move a run left unfinished, as its journal says (MV-14).
#[derive(Clone, Debug)]
pub(crate) struct Journaled {
    pub(crate) operation: String,
    pub(crate) session: String,
    pub(crate) from: String,
    pub(crate) to: String,
    /// The last step the move reached: started, packed, copied, unpacked,
    /// resumed or switched; aborted when the move was undone but taking
    /// its copy back off a machine is still owed.
    pub(crate) phase: String,
    /// The agent ran on the old machine when the move started, so an undo
    /// starts it again there; a session the user had stopped stays stopped.
    /// A journal from before this was kept says true.
    pub(crate) agent_ran: bool,
}

/// A client of the machine's carry service over its link.
struct CarryClient(pb::session_carry_service_client::SessionCarryServiceClient<tonic::transport::Channel>);

impl CarryClient {
    async fn pack(&mut self, request: pb::CarryPackRequest) -> Result<tonic::Response<pb::CarryPackResponse>, Status> {
        self.0.pack(crate::host_relay::assigned(request, None).map_err(Status::from)?).await
    }

    async fn unpack(&mut self, request: pb::CarryUnpackRequest) -> Result<tonic::Response<pb::CarryUnpackResponse>, Status> {
        self.0.unpack(crate::host_relay::assigned(request, None).map_err(Status::from)?).await
    }

    async fn release(&mut self, request: pb::CarryReleaseRequest) -> Result<tonic::Response<pb::CarryReleaseResponse>, Status> {
        self.0.release(crate::host_relay::assigned(request, None).map_err(Status::from)?).await
    }

    async fn abort(&mut self, request: pb::CarryAbortRequest) -> Result<tonic::Response<pb::CarryAbortResponse>, Status> {
        self.0.abort(crate::host_relay::assigned(request, None).map_err(Status::from)?).await
    }

    async fn preview_fork(
        &mut self,
        request: pb::CarryPreviewForkRequest,
    ) -> Result<tonic::Response<pb::CarryPreviewForkResponse>, Status> {
        self.0.preview_fork(crate::host_relay::assigned(request, None).map_err(Status::from)?).await
    }
}

fn end_failure(error: &tr::EndError) -> Status {
    status::failure(
        Code::Unavailable,
        pb::Error {
            message: status::chain(error),
            fix: "nothing was deleted; check that the machine is reachable and its disk is writable, then move the session again".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

fn copy_failure(from: &str, to: &str, why: &str) -> Status {
    status::failure(
        Code::Unavailable,
        pb::Error {
            message: format!("couldn't copy {from} to {to}: {why}"),
            fix: "nothing was deleted and the session is as it was; fix what the message names, then move it again".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// What stopped a copy, said for the person who fixes it.
fn describe(problem: &tr::Problem) -> String {
    match problem {
        tr::Problem::DiskFull { folder, needed, free } => format!(
            "there is not enough room in {folder}: the copy needs {needed} bytes and {free} are free"
        ),
        tr::Problem::ChecksumMismatch { path, .. } => {
            format!("{path} arrived different from what was sent")
        }
        tr::Problem::Exists { path } => format!("{path} already exists there"),
        tr::Problem::SourceGone { path } => format!("{path} was removed while it was copied"),
        tr::Problem::Unsupported { path } => {
            format!("{path} is a socket, pipe or device, which can't be copied; remove it from the session's folder")
        }
        tr::Problem::Offline { machine } => format!("{machine} went offline"),
        tr::Problem::Interrupted => "the connection ended".to_owned(),
        tr::Problem::Io { doing, path, message } => format!("couldn't {doing} {path}: {message}"),
    }
}
