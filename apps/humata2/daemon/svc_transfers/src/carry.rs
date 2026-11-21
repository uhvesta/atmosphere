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

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::pb;
use humata2_sessions::Layout;
use humata2_transfers as tr;
use fusefs_cas::{ChunkHash, FileManifest, hash_bytes};
use serde_json::json;
use tonic::{Code, Status};

use crate::carry_service::SessionCarrySvc;
use crate::ports::{self, Compression, LinkProvider, TrashBinPort, blocking, lock, now_ms};
use crate::remote_end::{RemoteEnd, RouterEnd, virtual_path};

/// A leftover transfer's journal that couldn't be read or marked.
#[derive(Debug, thiserror::Error)]
enum LeftoverError {
    #[error("couldn't read the move's transfer journals in {path}: {source}; check that the folder belongs to you, then try again")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't mark the leftover transfer journal {path} as cancelling: {source}; check the disk has free space, then try again. Nothing was deleted")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Every transfer a last run left running, verifying or committing is marked
/// cancelling in its journal before the engine opens, so the engine takes
/// back what reached the destination through the plan's manifest instead of
/// running the copy again or committing a folder the move never checked
/// (MV-14). The move's own recovery then decides to finish or undo the move.
fn mark_leftovers_cancelling(folder: &std::path::Path) -> Result<(), LeftoverError> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(LeftoverError::Read { path: folder.to_path_buf(), source }),
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        let path = entry.path();
        let Ok(bytes) = std::fs::read(&path) else { continue };
        // A journal the engine can't read is the engine's to set aside.
        let Ok(mut record) = serde_json::from_slice::<serde_json::Value>(&bytes) else { continue };
        if !record.get("upload").is_none_or(serde_json::Value::is_null) {
            continue;
        }
        let live = matches!(
            record.get("phase").and_then(serde_json::Value::as_str),
            Some("Running" | "Verifying" | "Committing")
        );
        if !live {
            continue;
        }
        record["phase"] = json!("Cancelling");
        let version = record.get("version").and_then(serde_json::Value::as_u64).unwrap_or(0);
        record["version"] = json!(version + 1);
        let written = serde_json::to_vec(&record)
            .map_err(std::io::Error::other)
            .and_then(|bytes| humata2_store::write_atomic(&path, &bytes).map_err(std::io::Error::other));
        written.map_err(|source| LeftoverError::Write { path, source })?;
    }
    Ok(())
}

/// Free space the engine leaves on a disk it copies to.
const FREE_FLOOR_BYTES: u64 = 1 << 30;
const SYNC_EVERY_BYTES: u64 = 8 << 20;

/// A channel's folder on one machine.
#[derive(Clone, Debug)]
pub struct Place {
    pub machine: String,
    pub channel: String,
    /// The channel's folder name, which is the same on every machine.
    pub folder: String,
}

struct Inner {
    local: Arc<SessionCarrySvc>,
    links: Arc<dyn LinkProvider>,
    remote: Arc<RemoteEnd>,
    bin: Arc<dyn TrashBinPort>,
    state: PathBuf,
    /// The host's own machine id.
    host: String,
    /// Where the host keeps channels.
    channels: Layout,
    open: Mutex<Option<(Arc<tr::Transfers>, Arc<RouterEnd>)>>,
    /// Who hears of the transfers' progress.
    listeners: Arc<Listeners>,
}

/// What the host's SessionService holds to move sessions. Cheap to clone.
#[derive(Clone)]
pub struct Carry(Arc<Inner>);

impl Carry {
    /// Inside the runtime: the engine's blocking threads reach it through
    /// the handle taken here. Touches no disk.
    pub fn new(
        local: Arc<SessionCarrySvc>,
        links: Arc<dyn LinkProvider>,
        bin: Arc<dyn TrashBinPort>,
        state: PathBuf,
        host: String,
        channels: impl Into<Layout>,
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
            channels: channels.into(),
            open: Mutex::new(None),
            listeners: Arc::default(),
        }))
    }

    /// The host's links to other machines, for the calls a session's archive and restore relay.
    pub fn links(&self) -> &Arc<dyn LinkProvider> {
        &self.0.links
    }

    pub fn is_host(&self, machine: &str) -> bool {
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
        mark_leftovers_cancelling(&self.state.join("carry").join("transfers")).map_err(|error| {
            ports::internal(
                format!("the move's leftover transfers can't be cancelled: {}", ports::chain(&error)),
                "",
            )
        })?;
        let engine = tr::Transfers::open(tr::Config {
            journal: self.state.join("carry").join("transfers"),
            source: router.clone(),
            destination: router.clone(),
            free_floor_bytes: FREE_FLOOR_BYTES,
            sync_every_bytes: SYNC_EVERY_BYTES,
            listener: {
                let listeners = self.listeners.clone();
                Arc::new(move |snapshot: &tr::Snapshot| listeners.tell(snapshot))
            },
        })
        .map_err(|error| {
            ports::internal(
                format!(
                    "the move's transfer journals in {} can't be used: {}. Check that the folder belongs to you and the disk has free space; nothing was deleted",
                    self.state.join("carry").join("transfers").display(),
                    ports::chain(&error)
                ),
                "",
            )
        })?;
        // A transfer paused or failed before the cut still holds a partial
        // copy; the move's recovery owns the move, so none is resumed here.
        for left in engine.list() {
            if matches!(left.state, tr::State::Paused | tr::State::Failed) {
                engine.cancel(&left.id).map_err(|error| {
                    ports::internal(
                        format!(
                            "the leftover transfer {} couldn't be cancelled: {}. Nothing was deleted; the next start tries again",
                            left.id,
                            ports::chain(&error)
                        ),
                        "",
                    )
                })?;
            }
        }
        *open = Some((engine.clone(), router.clone()));
        Ok((engine, router))
    }

    /// `relative` in `place`'s channel folder, as the engine names it.
    fn path(&self, place: &Place, relative: &str) -> PathBuf {
        if self.is_host(&place.machine) {
            return self.channels.channel(&place.folder).join(relative);
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
    pub async fn exists(&self, place: &Place, relative: &str) -> Result<bool, Status> {
        let path = self.path(place, relative);
        let inner = self.0.clone();
        blocking(move || {
            let (_, router) = inner.engine()?;
            tr::End::exists(&*router, &path).map_err(|error| end_failure(&error))
        })
        .await?
    }

    // -- the calls on a machine ---------------------------------------------

    pub async fn pack(&self, machine: &str, operation: &str, session: &str) -> Result<pb::CarryPackResponse, Status> {
        self.pack_for(machine, operation, session, "").await
    }

    /// As [`Carry::pack`], with the conversation written for `target`, the
    /// folder the session will have on the machine it goes to
    /// ([`Carry::target`]); empty: the conversation is staged as it is.
    pub async fn pack_for(
        &self,
        machine: &str,
        operation: &str,
        session: &str,
        target: &str,
    ) -> Result<pb::CarryPackResponse, Status> {
        if self.is_host(machine) {
            let packed = self.0.local.pack(operation, session, target).await?;
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
                target_folder: target.to_owned(),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// Where the session folder `folder` of `channel` will be on `machine`,
    /// as its CLI will record it (MV-17): what the old machine writes the
    /// conversation for before it packs it.
    pub async fn target(&self, machine: &str, channel: &str, folder: &str) -> Result<String, Status> {
        if self.is_host(machine) {
            return self.0.local.target(channel, folder).await;
        }
        let mut client = self.client(machine)?;
        client
            .target(pb::CarryTargetRequest {
                channel: Some(pb::ChannelId { value: channel.to_owned() }),
                folder: folder.to_owned(),
            })
            .await
            .map(|response| response.into_inner().folder)
    }

    /// Stages a fork of `parent` on `machine` for another machine (FK-8).
    pub async fn pack_fork(
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
                target_folder: String::new(),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// What a fork of `parent` on `machine` would get there (FK-4): the
    /// branch and the bytes, staging nothing. `typed` empty: picked from
    /// `title`.
    pub async fn preview_fork(
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
    pub async fn unpack_new(
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
    pub async fn abort_new(
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

    pub async fn unpack(
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

    pub async fn release(
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

    pub async fn abort(
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

    fn client(&self, machine: &str) -> Result<CarryClient, Status> {
        let channel = self.0.links.channel(machine).map_err(Status::from)?;
        Ok(CarryClient(
            pb::session_carry_service_client::SessionCarryServiceClient::new(channel),
        ))
    }

    // -- the copy -------------------------------------------------------------

    /// Copies `from_relative` of `from`'s channel folder into the folder
    /// `into` of `to`'s (made if it isn't there), checksummed, and returns
    /// once it is all there under its name or has failed. A copy that fails
    /// leaves nothing of itself in `to`.
    pub async fn copy(
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
            let started = engine
                .start(tr::Request {
                    id: id.clone(),
                    sources: vec![tr::Item { path: source, shown: shown_from.clone() }],
                    destination: tr::Item { path: destination, shown: shown_to.clone() },
                    mode: tr::Mode::Copy,
                    on_existing: tr::OnExisting::Fail,
                    caller_data: String::new(),
                    now_unix_ms: now_ms(),
                })
                .map_err(|error| copy_failure(&shown_from, &shown_to, &ports::chain(&error)))?;
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
    pub async fn journal(&self, operation: &str, entry: serde_json::Value) {
        let folder = self.0.state.join("carry");
        let path = folder.join(format!("{operation}.json"));
        let written = blocking(move || {
            make_dirs(&folder)?;
            let bytes = serde_json::to_vec_pretty(&entry).unwrap_or_default();
            humata2_store::write_atomic(&path, &bytes).map_err(std::io::Error::other)
        })
        .await;
        if let Ok(Err(error)) = written {
            humata2_rpc_kit::log::say!("humatad2: couldn't write the journal of move {operation}: {error}");
        }
    }

    /// The move is over; its journal goes.
    pub async fn journal_done(&self, operation: &str) {
        let path = self.0.state.join("carry").join(format!("{operation}.json"));
        let _ = blocking(move || remove_durably(&path)).await;
    }

    /// Writes where a fork onto or from another machine stands, in
    /// `<state>/carry/forks`, so a restart can finish or undo it. Best
    /// effort, as a move's journal is.
    pub async fn journal_fork(&self, entry: &ForkJournal) {
        let folder = self.0.state.join("carry").join("forks");
        let path = folder.join(format!("{}.json", entry.operation));
        let (operation, value) = (entry.operation.clone(), entry.value());
        let written = blocking(move || {
            make_dirs(&folder)?;
            let bytes = serde_json::to_vec_pretty(&value).unwrap_or_default();
            humata2_store::write_atomic(&path, &bytes).map_err(std::io::Error::other)
        })
        .await;
        if let Ok(Err(error)) = written {
            humata2_rpc_kit::log::say!("humatad2: couldn't write the journal of fork {operation}: {error}");
        }
    }

    /// `<state>/carry/remote_archive`, where a remote archive's or restore's
    /// follow-ups wait for a machine's link (D222).
    pub fn remote_archive_folder(&self) -> PathBuf {
        self.0.state.join("carry").join("remote_archive")
    }

    /// The fork is over, made or undone; its journal goes.
    pub async fn fork_done(&self, operation: &str) {
        let path = self.0.state.join("carry").join("forks").join(format!("{operation}.json"));
        let _ = blocking(move || remove_durably(&path)).await;
    }

    /// The forks a run left unfinished. Read once at start; a journal that
    /// can't be read is logged and left in place.
    pub fn fork_journals(&self) -> Vec<ForkJournal> {
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
                None => humata2_rpc_kit::log::say!(
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
    pub fn journals(&self) -> Vec<Journaled> {
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
                        let frozen = value.get("frozen").and_then(serde_json::Value::as_bool).unwrap_or(false);
                        found.push(Journaled { operation, session, from, to, phase, agent_ran, frozen });
                    }
                    _ => humata2_rpc_kit::log::say!(
                        "humatad2: the journal {} of a move names no session or machines, so that move isn't recovered. Nothing was deleted; delete the file if the session is where you want it",
                        path.display()
                    ),
                },
                Err(error) => humata2_rpc_kit::log::say!(
                    "humatad2: couldn't read the journal {} of a move: {error}. Nothing was deleted; check the file, or delete it if the session is where you want it",
                    path.display()
                ),
            }
        }
        found
    }

    /// What a move's journal holds at a step.
    pub fn entry(
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
pub struct ForkJournal {
    pub operation: String,
    pub parent: String,
    /// The fork's id.
    pub session: String,
    pub from: String,
    pub to: String,
    pub channel: String,
    /// `sessions/<name>` once the pack named it; empty before.
    pub folder: String,
    /// The last step reached: started, packed, copying (something of the
    /// fork may be on the target from here), copied, unpacked, or recorded
    /// (the host's record is written and the agent is being started).
    pub phase: String,
    /// Set for a context-only fork on the parent's own machine (D218,
    /// D221): what the host asked that machine, so a restart asks again
    /// under the same key. None for a fork that copies.
    pub context: Option<ContextAsk>,
}

/// What the host asked a machine for a context-only fork of one of its own
/// sessions, as the fork's journal keeps it.
#[derive(Clone, Debug, Default)]
pub struct ContextAsk {
    pub title: String,
    pub before_turn: u32,
    /// The fork's agent as asked; empty: the parent's.
    pub agent: String,
    pub model: String,
    /// The machine's operation once it answered; empty before.
    pub asked: String,
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
pub struct Journaled {
    pub operation: String,
    pub session: String,
    pub from: String,
    pub to: String,
    /// The last step the move reached: started, packed, copied, unpacked,
    /// resumed or switched; aborted when the move was undone but taking
    /// its copy back off a machine is still owed.
    pub phase: String,
    /// The agent ran on the old machine when the move started, so an undo
    /// starts it again there; a session the user had stopped stays stopped.
    /// A journal from before this was kept says true.
    pub agent_ran: bool,
    /// The old agent's program was frozen (SIGSTOP) for the move and may
    /// still be: a restart thaws it, or finishes the move (D801 J). A
    /// journal from before this says false.
    pub frozen: bool,
}

type Ear = Arc<dyn Fn(&tr::Snapshot) + Send + Sync>;

/// Who hears of each transfer's progress: a move listens for its own, whose
/// ids start with its operation's (MV-17).
#[derive(Default)]
struct Listeners {
    heard: Mutex<Vec<(String, Ear)>>,
}

impl Listeners {
    fn tell(&self, snapshot: &tr::Snapshot) {
        let ears: Vec<Ear> = lock(&self.heard)
            .iter()
            .filter(|(prefix, _)| snapshot.id.starts_with(prefix.as_str()))
            .map(|(_, ear)| ear.clone())
            .collect();
        for ear in ears {
            ear(snapshot);
        }
    }
}

/// How far a move's transfers are, summed over them: bytes done of bytes in
/// all, and the chunks of big files it sends or reuses beside them.
#[derive(Default)]
pub struct Tally {
    seen: Mutex<HashMap<String, (u64, u64)>>,
    /// Bytes of chunks moved so far, of bytes of chunks planned.
    chunks: Mutex<(u64, u64)>,
}

/// What a session's copy tells as the chunks of big files go (MV-17).
#[derive(Clone, Copy, Debug)]
pub enum Chunks {
    /// What the chunks the copy sends or reuses come to; known once it is planned.
    Planned(u64),
    /// Bytes of chunks sent or reused since.
    Moved(u64),
}

impl Tally {
    pub fn note(&self, snapshot: &tr::Snapshot) {
        lock(&self.seen).insert(snapshot.id.clone(), (snapshot.done_bytes, snapshot.total_bytes));
    }

    pub fn chunks(&self, event: Chunks) {
        let mut now = lock(&self.chunks);
        match event {
            Chunks::Planned(bytes) => now.1 += bytes,
            Chunks::Moved(bytes) => now.0 += bytes,
        }
    }

    pub fn done(&self) -> u64 {
        let engine: u64 = lock(&self.seen).values().map(|(done, _)| *done).sum();
        engine + lock(&self.chunks).0
    }

    pub fn total(&self) -> u64 {
        let engine: u64 = lock(&self.seen).values().map(|(_, total)| *total).sum();
        engine + lock(&self.chunks).1
    }
}

/// One session's copy: its folder and its staged files (MV-17).
pub struct SessionCopy<'a> {
    /// The copy's name; its transfers are `<id>_folder` and `<id>_stage`.
    pub id: String,
    pub from: &'a Place,
    pub to: &'a Place,
    /// The session's folder under both machines' place: `sessions/<name>`,
    /// `desk/<name>` or `<channel folder>/desk/<name>`.
    pub folder: &'a str,
    /// The staged files under the old machine's place.
    pub stage: &'a str,
    /// The folder the staged files are copied into on the new machine.
    pub into: String,
    /// Told as chunks are planned, reused and sent, for the move's progress.
    pub chunks: Option<Arc<dyn Fn(Chunks) + Send + Sync>>,
}

/// What a session's copy did.
pub struct Copied {
    /// What the new machine took from chunks it already held.
    pub reused_bytes: u64,
    /// The chunks the host sent beside the engine's copies.
    pub chunk_bytes: u64,
}

/// How many chunk names one `HaveChunks` call asks about.
const HAVE_BATCH: usize = 8192;

/// A file the new machine rebuilds from chunks.
struct Chunked {
    /// `folder` (`rel` is under the session's folder) or `stage`.
    at: String,
    rel: String,
    /// Its chunks in order, with their hashes.
    manifest: FileManifest,
}

/// A chunk the new machine lacks, and where the old machine has it.
struct Lacking {
    source: PathBuf,
    offset: u64,
    len: u64,
    hash: ChunkHash,
}

/// What a session's folder copy leaves out because the new machine builds it
/// or holds it already.
#[derive(Default)]
struct Cheap {
    files: Vec<PathBuf>,
    trees: Vec<PathBuf>,
    /// Staged files the stage's copy leaves out.
    stage_files: Vec<PathBuf>,
    chunked: Vec<Chunked>,
    lacking: Vec<Lacking>,
    /// What the new machine already holds of the chunked files.
    reused_bytes: u64,
}

/// Whether `manifest`'s chunks follow each other from the start and end at its size.
fn is_whole(manifest: &FileManifest) -> bool {
    let mut end = 0u64;
    for chunk in &manifest.chunks {
        if chunk.offset != end {
            return false;
        }
        end += u64::from(chunk.len);
    }
    end == manifest.size
}

/// The router leaves `files` and `trees` out of every plan of `source`
/// while this lives.
struct Skipping {
    router: Arc<RouterEnd>,
    source: PathBuf,
}

impl Skipping {
    fn new(router: Arc<RouterEnd>, source: PathBuf, files: Vec<PathBuf>, trees: Vec<PathBuf>) -> Self {
        router.skip(source.clone(), files, trees);
        Self { router, source }
    }
}

impl Drop for Skipping {
    fn drop(&mut self) {
        self.router.unskip(&self.source);
    }
}

impl Inner {
    /// What a session's copy can leave out (MV-17): the clones the new
    /// machine rebuilds, and the chunks of big files it already holds. Reads
    /// the old machine's stage and asks the new one which chunks it has.
    /// Blocking.
    fn cheap_plan(&self, from: &Place, to: &Place, folder: &str, stage: &str) -> Result<Cheap, Status> {
        let (_, router) = self.engine()?;
        let base = self.path(from, folder);
        let mut plan = Cheap::default();
        let mut left_out: HashSet<String> = HashSet::new();
        let wrong = |what: &str, error: &serde_json::Error| copy_failure(what, "the new machine", &error.to_string());
        if let Some(bytes) = self.read_stage(&router, from, &format!("{stage}/{}", humata2_sessions::CLONES_FILE))? {
            let listed: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|error| wrong("the list of clones", &error))?;
            for recipe in listed["clones"].as_array().into_iter().flatten() {
                let Some(name) = recipe["name"].as_str() else { continue };
                plan.trees.push(base.join(name).join(".git"));
                for rel in recipe["clean"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_str) {
                    let rel = format!("{name}/{rel}");
                    plan.files.push(base.join(&rel));
                    left_out.insert(rel);
                }
            }
        }
        if let Some(bytes) = self.read_stage(&router, from, &format!("{stage}/{}", humata2_sessions::CHUNKS_FILE))? {
            let listed: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|error| wrong("the list of chunks", &error))?;
            let staged = self.path(from, stage);
            // `(at, rel, manifest)` of each file worth asking about.
            let mut files = Vec::new();
            for file in listed["files"].as_array().into_iter().flatten() {
                let (Some(at), Some(rel)) = (file["at"].as_str(), file["rel"].as_str()) else {
                    continue;
                };
                let Ok(manifest) = serde_json::from_value::<FileManifest>(file["manifest"].clone()) else {
                    continue;
                };
                if !matches!(at, "folder" | "stage") || !is_whole(&manifest) || (at == "folder" && left_out.contains(rel)) {
                    continue;
                }
                files.push((at, rel, manifest));
            }
            let mut asked: Vec<String> =
                files.iter().flat_map(|file| file.2.chunks.iter().map(|chunk| chunk.hash.to_hex())).collect();
            asked.sort();
            asked.dedup();
            let held = self.held_chunks_on(&router, to, &asked)?;
            let mut queued: HashSet<String> = HashSet::new();
            for (at, rel, manifest) in files {
                if !manifest.chunks.iter().any(|chunk| held.contains(&chunk.hash.to_hex())) {
                    continue;
                }
                let source = if at == "folder" { base.join(rel) } else { staged.join(rel) };
                for chunk in &manifest.chunks {
                    let name = chunk.hash.to_hex();
                    let len = u64::from(chunk.len);
                    if held.contains(&name) {
                        plan.reused_bytes += len;
                    } else if queued.insert(name) {
                        plan.lacking.push(Lacking { source: source.clone(), offset: chunk.offset, len, hash: chunk.hash });
                    }
                }
                if at == "folder" {
                    plan.files.push(source);
                } else {
                    plan.stage_files.push(source);
                }
                plan.chunked.push(Chunked { at: at.to_owned(), rel: rel.to_owned(), manifest });
            }
        }
        Ok(plan)
    }

    /// What the old machine staged in `rel`, or none when it isn't there.
    fn read_stage(&self, router: &Arc<RouterEnd>, from: &Place, rel: &str) -> Result<Option<Vec<u8>>, Status> {
        let path = self.path(from, rel);
        if !tr::End::exists(&**router, &path).map_err(|error| end_failure(&error))? {
            return Ok(None);
        }
        let mut reader = tr::End::open_read(&**router, &path, 0).map_err(|error| end_failure(&error))?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut bytes)
            .map_err(|error| copy_failure(rel, "the host", &error.to_string()))?;
        Ok(Some(bytes))
    }

    /// Which of the named chunks the new machine's chunk store holds. The
    /// host reads its own store; another machine answers `HaveChunks`, asked
    /// through its channel's `.carry` folder, which is made first so there
    /// is something to ask at. Blocking.
    fn held_chunks_on(&self, router: &Arc<RouterEnd>, to: &Place, names: &[String]) -> Result<HashSet<String>, Status> {
        let carry = self.path(to, humata2_sessions::CARRY_DIR);
        if self.is_host(&to.machine) {
            let store = carry.parent().map(humata2_sessions::chunk_store_dir).unwrap_or_default();
            return Ok(humata2_sessions::held_chunks(&store, names).into_iter().collect());
        }
        let failed = |error: tr::EndError| end_failure(&error);
        tr::End::make_folder(&**router, &carry, 0o700).map_err(failed)?;
        let mut held = HashSet::new();
        for batch in names.chunks(HAVE_BATCH) {
            held.extend(tr::End::have_chunks(&**router, &carry, batch).map_err(failed)?);
        }
        Ok(held)
    }
}

impl Carry {
    /// Calls `ear` with each snapshot of a transfer of `operation`'s move
    /// until [`Carry::unhear`].
    pub fn hear(&self, operation: &str, ear: impl Fn(&tr::Snapshot) + Send + Sync + 'static) {
        lock(&self.0.listeners.heard).push((format!("{operation}_"), Arc::new(ear)));
    }

    pub fn unhear(&self, operation: &str) {
        let prefix = format!("{operation}_");
        lock(&self.0.listeners.heard).retain(|(heard, _)| *heard != prefix);
    }

    /// Bytes that crossed to `machine` since the daemon started, as sent:
    /// what the host wrote to a machine, or, for the host itself, what it
    /// read from the machines.
    pub fn crossed(&self, machine: &str) -> u64 {
        if self.is_host(machine) {
            self.0.remote.received_bytes()
        } else {
            self.0.remote.sent_bytes()
        }
    }

    /// Copies a session's folder and staged files to the new machine, leaving
    /// out the clones it rebuilds (their `.git`, and the files a checkout
    /// brings) and the big files it holds chunks of. The chunks it lacks of
    /// those go beside the copy, with `dedupe.json`, which tells it how to
    /// rebuild each file from them and from its own store (MV-17).
    pub async fn copy_session(&self, job: SessionCopy<'_>) -> Result<Copied, Status> {
        let (inner, from, to) = (self.0.clone(), job.from.clone(), job.to.clone());
        let (folder, stage) = (job.folder.to_owned(), job.stage.to_owned());
        let (plan, router) = blocking(move || -> Result<(Cheap, Arc<RouterEnd>), Status> {
            let plan = inner.cheap_plan(&from, &to, &folder, &stage)?;
            let (_, router) = inner.engine()?;
            Ok((plan, router))
        })
        .await??;
        if let Some(tell) = &job.chunks {
            let lacking: u64 = plan.lacking.iter().map(|chunk| chunk.len).sum();
            tell(Chunks::Planned(plan.reused_bytes + lacking));
            if plan.reused_bytes > 0 {
                tell(Chunks::Moved(plan.reused_bytes));
            }
        }
        let parent = job.folder.rsplit_once('/').map_or("", |(parent, _)| parent);
        let leaving = Skipping::new(router.clone(), self.path(job.from, job.folder), plan.files.clone(), plan.trees.clone());
        let folder_copy = self
            .copy(&format!("{}_folder", job.id), (job.from, job.folder), (job.to, parent))
            .await;
        drop(leaving);
        folder_copy?;
        let staying = Skipping::new(router, self.path(job.from, job.stage), plan.stage_files.clone(), Vec::new());
        let stage_copy = self
            .copy(&format!("{}_stage", job.id), (job.from, job.stage), (job.to, &job.into))
            .await;
        drop(staying);
        stage_copy?;
        let mut chunk_bytes = 0;
        if !plan.chunked.is_empty() {
            let name = std::path::Path::new(job.stage)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let beside = format!("{}/{name}", job.into);
            chunk_bytes = self
                .send_chunks(
                    job.to,
                    &format!("{beside}/{}", humata2_sessions::CHUNKS_DIR),
                    plan.lacking,
                    job.chunks.clone(),
                )
                .await?;
            self.write_recipe(job.to, &format!("{beside}/{}", humata2_sessions::REUSE_FILE), &plan.chunked)
                .await?;
        }
        Ok(Copied {
            reused_bytes: plan.reused_bytes,
            chunk_bytes,
        })
    }

    /// Sends each chunk the new machine lacks into `relative`, a file named
    /// by its hash, read from the old machine and checked as it goes. The
    /// bytes sent.
    async fn send_chunks(
        &self,
        to: &Place,
        relative: &str,
        lacking: Vec<Lacking>,
        tell: Option<Arc<dyn Fn(Chunks) + Send + Sync>>,
    ) -> Result<u64, Status> {
        let folder = self.path(to, relative);
        let inner = self.0.clone();
        blocking(move || -> Result<u64, Status> {
            let (_, router) = inner.engine()?;
            let failed = |error: tr::EndError| end_failure(&error);
            tr::End::make_folder(&*router, &folder, 0o700).map_err(failed)?;
            let mut sent = 0u64;
            for chunk in lacking {
                let shown = chunk.source.display().to_string();
                let mut reader = tr::End::open_read(&*router, &chunk.source, chunk.offset).map_err(failed)?;
                let mut bytes = vec![0u8; usize::try_from(chunk.len).unwrap_or(0)];
                std::io::Read::read_exact(&mut reader, &mut bytes)
                    .map_err(|error| copy_failure(&shown, "the host", &error.to_string()))?;
                if hash_bytes(&bytes) != chunk.hash {
                    return Err(copy_failure(
                        &shown,
                        "the new machine",
                        "it changed while it was being copied; stop what is writing to it, then move the session again",
                    ));
                }
                // Each chunk goes compressed as the adaptive planner picks.
                let framed = humata2_sessions::frame_chunk(&bytes)
                    .map_err(|error| copy_failure(&shown, "the new machine", &error.to_string()))?;
                let path = folder.join(chunk.hash.to_hex());
                let mut sink = tr::End::open_write(&*router, &path, 0).map_err(failed)?;
                tr::Sink::write(&mut *sink, &framed).map_err(failed)?;
                tr::Sink::sync(&mut *sink).map_err(failed)?;
                drop(sink);
                tr::End::finish_file(&*router, &path, 0o600, now_ms()).map_err(failed)?;
                sent += chunk.len;
                if let Some(tell) = &tell {
                    tell(Chunks::Moved(chunk.len));
                }
                // A test holds the copy here, once the chunk is sent and told.
                humata2_daemon_runtime::fault::hold_blocking("move.copy.chunk_sent");
            }
            Ok(sent)
        })
        .await?
    }

    /// Writes how the new machine rebuilds each file from chunks.
    async fn write_recipe(&self, to: &Place, relative: &str, chunked: &[Chunked]) -> Result<(), Status> {
        let path = self.path(to, relative);
        let mut listed: Vec<serde_json::Value> = Vec::new();
        for file in chunked {
            let manifest = serde_json::to_value(&file.manifest)
                .map_err(|error| copy_failure(relative, "the new machine", &error.to_string()))?;
            listed.push(json!({ "at": file.at, "rel": file.rel, "manifest": manifest }));
        }
        let body = serde_json::to_vec(&json!({ "files": listed }))
            .map_err(|error| copy_failure(relative, "the new machine", &error.to_string()))?;
        let inner = self.0.clone();
        blocking(move || -> Result<(), Status> {
            let (_, router) = inner.engine()?;
            let failed = |error: tr::EndError| end_failure(&error);
            let mut sink = tr::End::open_write(&*router, &path, 0).map_err(failed)?;
            tr::Sink::write(&mut *sink, &body).map_err(failed)?;
            tr::Sink::sync(&mut *sink).map_err(failed)?;
            drop(sink);
            tr::End::finish_file(&*router, &path, 0o600, now_ms()).map_err(failed)
        })
        .await?
    }
}

/// A client of the machine's carry service over its link.
struct CarryClient(pb::session_carry_service_client::SessionCarryServiceClient<tonic::transport::Channel>);

impl CarryClient {
    async fn pack(&mut self, request: pb::CarryPackRequest) -> Result<tonic::Response<pb::CarryPackResponse>, Status> {
        self.0.pack(tonic::Request::new(request)).await
    }

    async fn unpack(&mut self, request: pb::CarryUnpackRequest) -> Result<tonic::Response<pb::CarryUnpackResponse>, Status> {
        self.0.unpack(tonic::Request::new(request)).await
    }

    async fn release(&mut self, request: pb::CarryReleaseRequest) -> Result<tonic::Response<pb::CarryReleaseResponse>, Status> {
        self.0.release(tonic::Request::new(request)).await
    }

    async fn abort(&mut self, request: pb::CarryAbortRequest) -> Result<tonic::Response<pb::CarryAbortResponse>, Status> {
        self.0.abort(tonic::Request::new(request)).await
    }

    async fn preview_fork(
        &mut self,
        request: pb::CarryPreviewForkRequest,
    ) -> Result<tonic::Response<pb::CarryPreviewForkResponse>, Status> {
        self.0.preview_fork(tonic::Request::new(request)).await
    }

    async fn target(&mut self, request: pb::CarryTargetRequest) -> Result<tonic::Response<pb::CarryTargetResponse>, Status> {
        self.0.target(tonic::Request::new(request)).await
    }
}

fn end_failure(error: &tr::EndError) -> Status {
    ports::failure(
        Code::Unavailable,
        pb::Error {
            message: ports::chain(error),
            fix: "nothing was deleted; check that the machine is reachable and its disk is writable, then move the session again".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

fn copy_failure(from: &str, to: &str, why: &str) -> Status {
    ports::failure(
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

fn sync_dir(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

/// Makes `folder` and its missing parents, and puts each new entry on disk,
/// so a power cut right after can't lose the folder a journal is written in.
fn make_dirs(folder: &std::path::Path) -> std::io::Result<()> {
    let missing: Vec<&std::path::Path> = folder
        .ancestors()
        .take_while(|found| !found.as_os_str().is_empty() && std::fs::symlink_metadata(found).is_err())
        .collect();
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(folder)?;
    for found in missing {
        match found.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => sync_dir(parent)?,
            _ => {}
        }
    }
    Ok(())
}

/// Removes `path` and puts the removal on disk, so a finished move or fork
/// can't have its journal come back after a power cut and be run again.
fn remove_durably(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::remove_file(path)?;
    match path.parent() {
        Some(parent) => sync_dir(parent),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use fusefs_cas::{ChunkRef, hash_bytes};

    use super::*;

    fn chunk(offset: u64, len: u32) -> ChunkRef {
        ChunkRef { offset, len, hash: hash_bytes(&offset.to_le_bytes()) }
    }

    #[test]
    fn a_manifest_whose_chunks_run_from_the_start_to_its_size_is_whole() {
        let manifest = FileManifest { size: 300, generation: 1, chunks: vec![chunk(0, 100), chunk(100, 200)] };
        assert!(is_whole(&manifest));
        assert!(is_whole(&FileManifest { size: 0, generation: 1, chunks: Vec::new() }));
    }

    #[test]
    fn a_manifest_with_a_gap_an_overlap_or_the_wrong_size_is_not_whole() {
        let gap = FileManifest { size: 300, generation: 1, chunks: vec![chunk(0, 100), chunk(150, 150)] };
        let overlap = FileManifest { size: 300, generation: 1, chunks: vec![chunk(0, 200), chunk(100, 200)] };
        let short = FileManifest { size: 400, generation: 1, chunks: vec![chunk(0, 100), chunk(100, 200)] };
        assert!(!is_whole(&gap));
        assert!(!is_whole(&overlap));
        assert!(!is_whole(&short));
    }
}
