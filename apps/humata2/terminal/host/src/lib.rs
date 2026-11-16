//! Native authoritative terminal host. Viewers never own the PTY or VT state.
#[cfg(test)]
mod adversarial;
mod auth;
mod error;
#[cfg(test)]
mod feature_tests;
mod gateway;
#[cfg(test)]
mod gateway_adversarial;
mod native;
mod provider_host;
mod reconnect;
mod recovery;
#[cfg(test)]
mod roaming_tests;
mod spawn;
pub use error::Error;
pub use gateway::serve;
use ghostty_vt::{Event as VtEvent, Terminal, clipboard_read_reply};
pub use native::{
    ControlPolicy, HandleError, QueuedInjection, QueuedStop, TerminalHandle, WriteUncertainty,
};
pub use provider_host::{ProviderBinding, ProviderObservation, ProviderObserver};
use recovery::{
    Checkpoint, MAX_CHECKPOINT, MAX_ENCODED_CHECKPOINT, Recovery, ReplyCursor, Settings,
};
use serde::Deserialize;
pub use spawn::{CwdSource, SpawnCwd, SpawnError, SpawnFromOptions, SpawnProfile, Spawned};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};
pub use terminal_codec as compression;
pub use terminal_provider as provider;
use terminal_session::{Attached, Endpoint, Record, SessionSpec, SessionStore, Update};
use terminal_wire::{
    ClientFrame, HistoryPolicy, HostInfo, PROTOCOL_VERSION, RecoveryHint, SNAPSHOT_ENGINE,
    ServerFrame, SessionInfo, SessionState,
};
pub use terminal_wire::{
    InjectionMode, InjectionOptions, InputTicket, NameSource, SessionNameOverride,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    task::JoinHandle,
};
const VIEW_QUEUE: usize = 64;
// Output yields to ordered checkpoint/control/receipt traffic before the hard queue cap.
const VIEW_CONTROL_RESERVE: usize = 8;
// Two selected panes can each receive a maximum raw checkpoint, with output/metadata headroom.
const VIEW_BYTES: usize = 2 * terminal_wire::MAX_RAW_CHECKPOINT_BYTES + (4 << 20);
const OUTPUT_WINDOW: u64 = 1 << 20;
const MAX_INPUT: usize = 256 << 10;
const WRITER_QUEUE: usize = 64;
const LIVE_HISTORY_BYTES: usize = 32 << 20;

/// Trusted launcher configuration. Never sent to viewers or serialized by the host.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub title: String,
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: PathBuf,
    #[serde(default = "local")]
    pub endpoint: Endpoint,
    #[serde(default)]
    pub recovery: Option<RecoveryHint>,
    #[serde(default)]
    pub history: HistoryPolicy,
}
fn local() -> Endpoint {
    Endpoint::Local
}
impl Profile {
    fn spec(&self) -> SessionSpec {
        SessionSpec {
            title: self.title.clone(),
            program: self.program.clone(),
            args: self.args.clone(),
            cwd: self.cwd.clone(),
            cols: 100,
            rows: 30,
            endpoint: self.endpoint.clone(),
        }
    }
}
/// The host uses its own private state directory, separate from legacy raw journals.
pub struct Config {
    pub state: PathBuf,
    pub label: String,
    pub start_profiles: Vec<String>,
    pub default_history: HistoryPolicy,
}
#[derive(Clone)]
pub struct Host {
    tx: mpsc::Sender<Command>,
}
pub(crate) struct QueuedFrame {
    pub frame: ServerFrame,
    _budget: OwnedSemaphorePermit,
}
pub(crate) struct Subscription {
    pub id: u64,
    pub receiver: mpsc::Receiver<QueuedFrame>,
}
enum Command {
    Reconnected(reconnect::Connected),
    TransportsRetired(u64),
    #[cfg(test)]
    ReconnectBackoffFixture {
        delay: Duration,
        reply: oneshot::Sender<()>,
    },
    Provider(provider_host::ProviderCommand),
    Spawn(spawn::SpawnCommand),
    Native(native::NativeCommand),
    #[cfg(test)]
    PauseFixture {
        ready: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    },
    #[cfg(test)]
    PendingReplyFixture {
        session: u64,
        reply: oneshot::Sender<()>,
    },
    #[cfg(test)]
    ReplayFixture {
        session: u64,
        end: u64,
        reply: oneshot::Sender<()>,
    },
    #[cfg(test)]
    Crash {
        reply: oneshot::Sender<()>,
    },
    ReplySent {
        session: u64,
        batch: u64,
        internal: u64,
    },
    ResizeSent {
        session: u64,
        epoch: u64,
        internal: u64,
    },
    Shutdown {
        reply: oneshot::Sender<Result<(), Error>>,
    },
    Join {
        reply: oneshot::Sender<Result<Subscription, Error>>,
    },
    Leave(u64),
    Frame(u64, ClientFrame),
    Output(u64, Update),
    Disconnected(u64),
    TransportFailed {
        id: u64,
        source: terminal_session::Error,
    },
    Sent {
        session: u64,
        internal: u64,
        user: Option<(u64, u64)>,
    },
    Saved {
        session: u64,
        generation: u64,
        result: Result<(), Error>,
    },
    Removed {
        session: u64,
        result: Result<(), Error>,
    },
    Created {
        store: SessionStore,
        profile: Profile,
        id: Option<u64>,
        attached: Result<Attached, Error>,
    },
    CreateFailed,
}
enum Write {
    Native {
        bytes: Vec<u8>,
        ticket: u64,
    },
    Reply {
        bytes: Vec<u8>,
        batch: u64,
    },
    Input {
        bytes: Vec<u8>,
        user: Option<(u64, u64)>,
    },
    Resize {
        cols: u16,
        rows: u16,
        replies: Vec<Vec<u8>>,
        epoch: u64,
    },
    Stop,
}
enum DiskJob {
    Barrier(oneshot::Sender<()>),
    SaveSync {
        session: u64,
        checkpoint: Checkpoint,
        limit: u64,
        reply: oneshot::Sender<Result<(), Error>>,
    },
    Save {
        session: u64,
        generation: u64,
        checkpoint: Checkpoint,
        limit: u64,
    },
    Remove(u64),
}
struct Process {
    info: SessionInfo,
    fallback_title: String,
    vt: Terminal,
    offset: u64,
    replay_end: u64,
    pending_open: BTreeSet<u64>,
    exited: Option<i32>,
    stopping: bool,
    resize_epoch: u64,
    applied_resize_epoch: u64,
    writer: Option<mpsc::Sender<Write>>,
    _tasks: Vec<JoinHandle<()>>,
    settings: Settings,
    generation: u64,
    saved_generation: u64,
    last_save: Instant,
    saving: bool,
    receipts: BTreeMap<u64, Option<(u64, u64)>>,
    early_receipts: BTreeSet<u64>,
    resize_receipts: BTreeMap<u64, u64>,
    reply_receipts: BTreeMap<u64, u64>,
    reply_cursor: ReplyCursor,
    pending_replies: BTreeMap<u64, u64>,
    next_reply: u64,
    native_receipts: BTreeMap<u64, u64>,
}
impl Drop for Process {
    fn drop(&mut self) {
        for task in &self._tasks {
            task.abort();
        }
    }
}
struct Viewing {
    acked: u64,
    sent: u64,
    resync: bool,
}
struct Viewer {
    tx: mpsc::Sender<QueuedFrame>,
    budget: std::sync::Arc<Semaphore>,
    open: BTreeMap<u64, Viewing>,
    inputs: BTreeMap<u64, u64>,
    settled_inputs: BTreeMap<u64, u64>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Controller {
    Viewer(u64),
    Native(u64),
}
struct Actor {
    store: Option<SessionStore>,
    profiles: BTreeMap<String, Profile>,
    processes: BTreeMap<u64, Process>,
    recovery: Recovery,
    settings: BTreeMap<u64, Settings>,
    host: HostInfo,
    viewers: BTreeMap<u64, Viewer>,
    controllers: BTreeMap<u64, Controller>,
    next_viewer: u64,
    tx: mpsc::Sender<Command>,
    rx: mpsc::Receiver<Command>,
    disk: mpsc::Sender<DiskJob>,
    default_history: HistoryPolicy,
    history_disabled: bool,
    blocked_resize: BTreeSet<u64>,
    blocked_replies: BTreeSet<u64>,
    pending_cwd: BTreeMap<u64, PathBuf>,
    disk_done: mpsc::UnboundedReceiver<Command>,
    provider_state: provider_host::ProviderState,
    native_state: native::NativeState,
    pending_titles: BTreeMap<u64, String>,
    reconnects: reconnect::Reconnects,
}
impl Host {
    pub async fn start(
        config: Config,
        store: SessionStore,
        profiles: Vec<Profile>,
    ) -> Result<Self, Error> {
        Self::start_inner(
            config,
            store,
            profiles,
            std::sync::Arc::new(terminal_codec::Registry::default()),
            vec![terminal_codec::LZ4.into(), terminal_codec::ZLIB.into()],
        )
        .await
    }
    /// Use a trusted codec registry for screen archives. Existing archives keep their
    /// recorded codec; custom codecs must also be registered when reopening them.
    /// Wire compression is independently selected by the transport.
    pub async fn start_with_compression(
        config: Config,
        store: SessionStore,
        profiles: Vec<Profile>,
        codecs: std::sync::Arc<terminal_codec::Registry>,
        codec: String,
    ) -> Result<Self, Error> {
        Self::start_inner(config, store, profiles, codecs, vec![codec]).await
    }
    async fn start_inner(
        config: Config,
        mut store: SessionStore,
        profiles: Vec<Profile>,
        codecs: std::sync::Arc<terminal_codec::Registry>,
        codec: Vec<String>,
    ) -> Result<Self, Error> {
        validate_history(&config.default_history)?;
        let recovery = Recovery::open_with_codecs(config.state.join("recovery"), codecs, codec)?;
        let mut settings = recovery.settings()?;
        let native_state = native::NativeState::new()?;
        let mut configured = BTreeMap::new();
        let history_disabled = matches!(config.default_history, HistoryPolicy::Off);
        if history_disabled {
            for setting in settings.values_mut() {
                setting.history = HistoryPolicy::Off;
            }
            recovery.save_settings(&settings)?;
        }
        for mut profile in profiles {
            if history_disabled {
                profile.history = HistoryPolicy::Off;
            }
            validate_name(&profile.name)?;
            validate_history(&profile.history)?;
            validate_recovery(profile.recovery.as_ref())?;
            if configured.insert(profile.name.clone(), profile).is_some() {
                return Err(Error::Invalid {
                    reason: "launch profile names must be unique",
                });
            }
        }
        for name in &config.start_profiles {
            let profile = configured.get(name).ok_or(Error::Invalid {
                reason: "a startup profile was not configured",
            })?;
            if !store
                .sessions()
                .iter()
                .any(|saved| saved.startup_profile.as_ref() == Some(name))
            {
                let legacy = store
                    .sessions()
                    .iter()
                    .find(|saved| {
                        saved.startup_profile.is_none()
                            && settings
                                .get(&saved.id)
                                .is_some_and(|setting| setting.profile.as_ref() == Some(name))
                    })
                    .map(|saved| saved.id);
                if let Some(id) = legacy {
                    store.bind_startup_profile(id, name)?;
                }
            }
            let previous: BTreeSet<_> = store.sessions().iter().map(|saved| saved.id).collect();
            let launched = store.create_startup(profile.spec(), name);
            let id = launched.as_ref().ok().copied().or_else(|| {
                store
                    .sessions()
                    .iter()
                    .find(|saved| saved.startup_profile.as_ref() == Some(name))
                    .map(|saved| saved.id)
            });
            if let Some(id) = id {
                if !previous.contains(&id) {
                    store.set_recovery(id, profile.recovery.clone())?;
                }
                let saved_hint = store
                    .sessions()
                    .iter()
                    .find(|saved| saved.id == id)
                    .and_then(|saved| saved.recovery.clone());
                let setting = settings.entry(id).or_insert_with(|| Settings {
                    profile: Some(name.clone()),
                    recovery: saved_hint,
                    history: profile.history.clone(),
                });
                setting.profile = Some(name.clone());
                recovery.save_settings(&settings)?;
            }
            let _ = launched?;
        }
        let (tx, rx) = mpsc::channel(256);
        let (disk, disk_jobs) = mpsc::channel(8);
        let (disk_results, disk_done) = mpsc::unbounded_channel();
        disk_worker(recovery.clone(), disk_jobs, disk_results);
        let mut actor = Actor {
            store: Some(store),
            profiles: configured,
            processes: BTreeMap::new(),
            recovery,
            settings,
            host: HostInfo {
                label: clean(&config.label, 80),
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
                protocol_version: PROTOCOL_VERSION,
                engine: SNAPSHOT_ENGINE.into(),
            },
            viewers: BTreeMap::new(),
            controllers: BTreeMap::new(),
            next_viewer: 1,
            tx: tx.clone(),
            rx,
            disk,
            default_history: config.default_history,
            history_disabled,
            blocked_resize: BTreeSet::new(),
            blocked_replies: BTreeSet::new(),
            pending_cwd: BTreeMap::new(),
            disk_done,
            provider_state: provider_host::ProviderState::default(),
            native_state,
            pending_titles: BTreeMap::new(),
            reconnects: reconnect::Reconnects::default(),
        };
        let sessions = actor
            .store
            .as_ref()
            .ok_or(Error::Closed)?
            .sessions()
            .to_vec();
        for saved in sessions {
            let settings = actor
                .settings
                .get(&saved.id)
                .cloned()
                .unwrap_or_else(|| Settings {
                    profile: saved.startup_profile.clone(),
                    history: saved
                        .startup_profile
                        .as_ref()
                        .and_then(|name| actor.profiles.get(name))
                        .map(|profile| profile.history.clone())
                        .unwrap_or_else(|| actor.default_history.clone()),
                    recovery: saved.recovery.clone(),
                });
            let (vt, offset, exit) = actor.restore(saved.id, &saved.spec, &settings)?;
            let cursor = actor.recovery.reply_cursor(saved.id)?;
            if cursor.pending {
                actor.blocked_replies.insert(saved.id);
            }
            let guarded = actor.recovery.resize_epoch(saved.id)?.is_some();
            if guarded {
                actor.blocked_resize.insert(saved.id);
            }
            let mut attached = if guarded || cursor.pending {
                None
            } else {
                actor
                    .store
                    .as_ref()
                    .ok_or(Error::Closed)?
                    .attach_live_from(saved.id, offset)
                    .await
                    .ok()
            };
            if attached
                .as_ref()
                .is_some_and(|a| cursor.answered_through > a.replay_end)
            {
                actor.blocked_replies.insert(saved.id);
                attached = None;
            }
            actor.add(
                saved.id,
                &saved.spec,
                settings,
                vt,
                offset,
                exit,
                attached,
                cursor,
            );
        }
        tokio::spawn(actor.run());
        Ok(Self { tx })
    }
    /// Flush bounded screen recovery, detach viewers/transports, and leave holders running.
    pub async fn shutdown(&self) -> Result<(), Error> {
        let (reply, result) = oneshot::channel();
        self.tx
            .send(Command::Shutdown { reply })
            .await
            .map_err(|_| Error::Closed)?;
        result.await.map_err(|_| Error::Closed)?
    }
    pub(crate) async fn subscribe(&self) -> Result<Subscription, Error> {
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Command::Join { reply })
            .await
            .map_err(|_| Error::Closed)?;
        answer.await.map_err(|_| Error::Closed)?
    }
    pub(crate) async fn frame(&self, viewer: u64, frame: ClientFrame) -> Result<(), Error> {
        self.tx
            .send(Command::Frame(viewer, frame))
            .await
            .map_err(|_| Error::Closed)
    }
    pub(crate) async fn leave(&self, viewer: u64) {
        let _ = self.tx.send(Command::Leave(viewer)).await;
    }
}
impl Actor {
    fn restore(
        &self,
        id: u64,
        spec: &SessionSpec,
        settings: &Settings,
    ) -> Result<(Terminal, u64, Option<i32>), Error> {
        let (lines, _) = history_limits(&settings.history);
        if matches!(settings.history, HistoryPolicy::Off) {
            self.recovery.remove(id)?;
            return Ok((new_terminal(spec, lines)?, 0, None));
        }
        if let Some(cp) = self.recovery.load(id, MAX_ENCODED_CHECKPOINT as u64)? {
            let mut terminal = Terminal::from_checkpoint(&cp.bytes)?;
            terminal.set_scrollback_limits(lines, LIVE_HISTORY_BYTES)?;
            let _ = terminal.drain_events();
            return Ok((terminal, cp.offset, cp.exited));
        }
        Ok((new_terminal(spec, lines)?, 0, None))
    }
    fn add(
        &mut self,
        id: u64,
        spec: &SessionSpec,
        settings: Settings,
        mut vt: Terminal,
        offset: u64,
        exit: Option<i32>,
        attached: Option<Attached>,
        cursor: ReplyCursor,
    ) {
        let Some(saved) = self
            .store
            .as_ref()
            .and_then(|store| store.sessions().iter().find(|s| s.id == id))
            .cloned()
        else {
            self.failure(Some(id),"missing_identity","The terminal's durable identity is missing from workspace metadata. Preserve the workspace and reconnect before launching another process.");
            return;
        };
        let fallback_title = clean(&spec.title, 120);
        let fallback_title = if fallback_title.trim().is_empty() {
            format!("Terminal {id}")
        } else {
            fallback_title
        };
        let automatic_title = vt
            .title()
            .map(|title| display_title(&fallback_title, Some(&title)))
            .unwrap_or_else(|| saved.automatic_title.clone());
        let title = saved
            .name_override
            .as_ref()
            .map(|name| name.title.clone())
            .unwrap_or_else(|| automatic_title.clone());
        let live = attached.is_some();
        if matches!(saved.spec.endpoint, Endpoint::Ssh { .. }) {
            if let Some(target) = self
                .store
                .as_ref()
                .and_then(|store| store.prepare_live_attachment(id).ok())
            {
                self.reconnects.register(
                    id,
                    target,
                    live,
                    self.blocked_resize.contains(&id) || self.blocked_replies.contains(&id),
                );
            }
        }
        let replay_end = attached.as_ref().map(|a| a.replay_end).unwrap_or(offset);
        let exited = attached.as_ref().and_then(|a| a.exited).or(exit);
        let state = if let Some(code) = exited {
            SessionState::Exited { code: Some(code) }
        } else if live {
            SessionState::Running
        } else {
            SessionState::Unavailable
        };
        let mut tasks = Vec::new();
        let writer = attached.map(|a| {
            let (write, requests) = mpsc::channel(WRITER_QUEUE);
            tasks = transport_tasks(id, a, requests, self.tx.clone());
            write
        });
        let _ = vt.drain_events();
        self.settings.insert(id, settings.clone());
        self.processes.insert(
            id,
            Process {
                info: SessionInfo {
                    id,
                    uuid: saved.uuid,
                    automatic_title,
                    name_override: saved.name_override,
                    attributes: saved.attributes,
                    title,
                    cwd: clean(&spec.cwd.to_string_lossy(), 4096),
                    program: clean(
                        &spec
                            .program
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        120,
                    ),
                    state,
                    recovery_available: settings.recovery.is_some(),
                },
                fallback_title,
                vt,
                offset,
                replay_end,
                pending_open: BTreeSet::new(),
                exited,
                stopping: false,
                resize_epoch: 0,
                applied_resize_epoch: 0,
                writer,
                _tasks: tasks,
                settings,
                generation: 0,
                saved_generation: 0,
                last_save: Instant::now(),
                saving: false,
                receipts: BTreeMap::new(),
                early_receipts: BTreeSet::new(),
                resize_receipts: BTreeMap::new(),
                reply_receipts: BTreeMap::new(),
                reply_cursor: cursor,
                pending_replies: BTreeMap::new(),
                next_reply: 1,
                native_receipts: BTreeMap::new(),
            },
        );
    }
    async fn run(mut self) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut disk_open = true;
        loop {
            let command = tokio::select! {
                command=self.rx.recv()=>command,
                command=self.disk_done.recv(),if disk_open=>{if command.is_none(){disk_open=false;self.failure(None,"recovery_worker","The checkpoint worker stopped. Live terminals remain active; restart the host to restore disk recovery.");}command},
                _=tick.tick()=>{self.checkpoints();self.reconnect_tick();continue;},
            };
            let Some(command) = command else {
                if !disk_open {
                    continue;
                }
                break;
            };
            #[cfg(test)]
            if let Command::Crash { reply } = command {
                self.viewers.clear();
                for process in self.processes.values_mut() {
                    process.writer = None;
                    for task in process._tasks.drain(..) {
                        task.abort();
                        let _ = task.await;
                    }
                }
                self.processes.clear();
                self.store = None;
                let _ = reply.send(());
                break;
            }
            if let Command::Shutdown { reply } = command {
                self.native_state.closing();
                self.reconnects.close().await;
                let settled = self.settle_mechanical_writes().await;
                let mut result = self.flush().await;
                if result.is_ok() {
                    result = settled;
                }

                let (barrier, finished) = oneshot::channel();
                if self.disk.send(DiskJob::Barrier(barrier)).await.is_err()
                    || finished.await.is_err()
                {
                    if result.is_ok() {
                        result = Err(Error::Closed);
                    }
                }
                let pending_ids: Vec<_> = self.processes.keys().copied().collect();
                for id in pending_ids {
                    self.fail_native(id, WriteUncertainty::HostClosed);
                }
                self.viewers.clear();
                self.controllers.clear();
                for process in self.processes.values_mut() {
                    process.writer = None;
                    for task in process._tasks.drain(..) {
                        task.abort();
                        let _ = task.await;
                    }
                }
                self.processes.clear();
                self.rx.close();
                while let Ok(command) = self.rx.try_recv() {
                    drop(command);
                }
                self.store = None;
                let _ = reply.send(result);
                break;
            }
            self.command(command).await;
            self.ready_snapshots();
        }
    }
    fn unsettled_mechanical_sessions(&self) -> Vec<u64> {
        self.processes
            .iter()
            .filter_map(|(id, p)| {
                (p.writer.is_some()
                    && (!p.pending_replies.is_empty()
                        || p.resize_epoch > p.applied_resize_epoch
                        || self.native_state.has_pending(*id)))
                .then_some(*id)
            })
            .collect()
    }
    async fn settle_mechanical_writes(&mut self) -> Result<(), Error> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !self.unsettled_mechanical_sessions().is_empty() {
            let command = tokio::time::timeout_at(deadline, self.rx.recv())
                .await
                .map_err(|source| Error::ShutdownPending {
                    sessions: self.unsettled_mechanical_sessions(),
                    source,
                })?
                .ok_or(Error::Closed)?;
            if let Command::Shutdown { reply } = command {
                let _ = reply.send(Err(Error::Closed));
                continue;
            }
            self.command(command).await;
            self.ready_snapshots();
        }
        Ok(())
    }
    async fn flush(&mut self) -> Result<(), Error> {
        let ids: Vec<_> = self.processes.keys().copied().collect();
        for id in ids {
            if self.blocked_replies.contains(&id) {
                continue;
            }
            let pending = {
                let p = &self.processes[&id];
                match p.settings.history {
                    HistoryPolicy::Off => None,
                    HistoryPolicy::Recent { max_bytes, .. } => {
                        let bytes = p.vt.checkpoint(MAX_CHECKPOINT)?;
                        Some((
                            Checkpoint {
                                bytes,
                                offset: p.offset,
                                cols: p.vt.size().cols,
                                rows: p.vt.size().rows,
                                exited: p.exited,
                                resize_epoch: p.applied_resize_epoch,
                            },
                            max_bytes,
                        ))
                    }
                }
            };
            if let Some((checkpoint, limit)) = pending {
                let (reply, done) = oneshot::channel();
                self.disk
                    .send(DiskJob::SaveSync {
                        session: id,
                        checkpoint,
                        limit,
                        reply,
                    })
                    .await
                    .map_err(|_| Error::Closed)?;
                done.await.map_err(|_| Error::Closed)??;
            }
        }
        Ok(())
    }
    fn leave_viewer(&mut self, id: u64) {
        self.viewers.remove(&id);
        for p in self.processes.values_mut() {
            p.pending_open.remove(&id);
        }
        let released: Vec<_> = self
            .controllers
            .iter()
            .filter_map(|(session, owner)| (*owner == Controller::Viewer(id)).then_some(*session))
            .collect();
        for session in released {
            self.controllers.remove(&session);
            self.controls(session);
        }
    }
    fn send_to(&mut self, viewer: u64, frame: ServerFrame) {
        let weight = frame_weight(&frame);
        let failed = if let Some(v) = self.viewers.get(&viewer) {
            match v
                .budget
                .clone()
                .try_acquire_many_owned(weight.min(u32::MAX as usize) as u32)
            {
                Ok(permit) => {
                    let full =
                        v.tx.try_send(QueuedFrame {
                            frame,
                            _budget: permit,
                        })
                        .is_err();
                    if full {
                        eprintln!("terminal viewer disconnected: outbound frame queue exhausted");
                    }
                    full
                }
                Err(_) => {
                    eprintln!("terminal viewer disconnected: outbound byte budget exhausted");
                    true
                }
            }
        } else {
            false
        };
        if failed {
            self.leave_viewer(viewer);
        }
    }
    fn send(&mut self, frame: ServerFrame) {
        let ids: Vec<_> = self.viewers.keys().copied().collect();
        for id in ids {
            self.send_to(id, frame.clone());
        }
    }
    fn controls(&mut self, id: u64) {
        let owner = self.controllers.get(&id).copied();
        let viewers: Vec<_> = self
            .viewers
            .iter()
            .filter_map(|(viewer, v)| v.open.contains_key(&id).then_some(*viewer))
            .collect();
        for viewer in viewers {
            self.send_to(
                viewer,
                ServerFrame::Control {
                    id,
                    controller: owner == Some(Controller::Viewer(viewer)),
                },
            );
        }
    }
    fn require_control(&mut self, viewer: u64, id: u64) -> bool {
        if self
            .processes
            .get(&id)
            .is_some_and(|p| p.writer.is_some() && p.offset < p.replay_end)
        {
            self.failure_to(viewer,Some(id),"not_ready","The host is still reconstructing this terminal's retained state. Wait for its initial snapshot before taking control or sending input.");
            return false;
        }
        if self.controllers.get(&id) == Some(&Controller::Viewer(viewer)) {
            true
        } else {
            self.send_to(viewer,ServerFrame::Error{id:Some(id),code:"read_only".into(),message:"This view is read-only. Choose Take control before sending input, resizing, or changing this terminal.".into()});
            false
        }
    }
    fn failure_to(&mut self, viewer: u64, id: Option<u64>, code: &str, message: &str) {
        self.send_to(
            viewer,
            ServerFrame::Error {
                id,
                code: code.into(),
                message: message.into(),
            },
        );
    }
    fn sessions_to(&mut self, viewer: u64) {
        self.send_to(
            viewer,
            ServerFrame::Sessions {
                host: self.host.clone(),
                sessions: self.processes.values().map(|p| p.info.clone()).collect(),
            },
        );
    }
    fn failure(&mut self, id: Option<u64>, code: &str, message: &str) {
        self.send(ServerFrame::Error {
            id,
            code: code.into(),
            message: message.into(),
        });
    }
    fn sessions(&mut self) {
        self.send(ServerFrame::Sessions {
            host: self.host.clone(),
            sessions: self.processes.values().map(|p| p.info.clone()).collect(),
        });
    }
    fn ready_snapshots(&mut self) {
        self.reconnected_ready();
        let mut ready = Vec::new();
        for (id, p) in &mut self.processes {
            if p.writer.is_none() || p.offset >= p.replay_end {
                for viewer in std::mem::take(&mut p.pending_open) {
                    ready.push((viewer, *id));
                }
            }
        }
        self.persist_cwds();
        if !ready.is_empty() {
            self.sessions();
        }
        for (viewer, id) in ready {
            if self.viewers.contains_key(&viewer) {
                self.snapshot(viewer, id);
            }
        }
    }
    fn snapshot(&mut self, viewer: u64, id: u64) {
        let Some(process) = self.processes.get_mut(&id) else {
            self.failure_to(
                viewer,
                Some(id),
                "missing_session",
                "Select an existing terminal or create one from a configured profile.",
            );
            return;
        };
        if process.writer.is_some() && process.offset < process.replay_end {
            process.pending_open.insert(viewer);
            return;
        }
        match process.vt.checkpoint(MAX_CHECKPOINT){
   Ok(bytes)=>{let frame=ServerFrame::Snapshot{id,engine:SNAPSHOT_ENGINE.into(),bytes,offset:process.offset,cols:process.vt.size().cols,rows:process.vt.size().rows,live:matches!(process.info.state,SessionState::Running)&&!process.stopping,exited:process.exited};let offset=process.offset;if let Some(v)=self.viewers.get_mut(&viewer){v.open.insert(id,Viewing{acked:offset,sent:offset,resync:false});}self.send_to(viewer,frame);if self.blocked_replies.contains(&id){self.send_to(viewer,ServerFrame::Error{id:Some(id),code:"reply_recovery_incomplete".into(),message:"The host stopped while a terminal reply might have been written. The last verified display is preserved and the holder remains alive; live input is disabled to avoid injecting duplicate control bytes. Recover the provider explicitly or start a new session.".into()});}if self.blocked_resize.contains(&id){self.send_to(viewer,ServerFrame::Error{id:Some(id),code:"resize_recovery_incomplete".into(),message:"The host stopped before a resize was covered by a durable checkpoint. This is the last verified display; the original holder remains alive. Use explicit provider recovery or a new session; live input is disabled to avoid reconstructing the wrong terminal state.".into()});}}
   Err(_)=>self.failure_to(viewer,Some(id),"checkpoint_limit","The terminal state could not fit a safe 16 MiB decoded checkpoint. Reduce retained history and request another snapshot; the original process remains running."),
  }
    }
    async fn command(&mut self, command: Command) {
        match command{
  Command::Reconnected(event)=>self.connected(event),
  Command::TransportsRetired(id)=>self.transports_retired(id),
  #[cfg(test)]
  Command::ReconnectBackoffFixture{delay,reply}=>{self.reconnects.test_backoff(delay);let _=reply.send(());},
  Command::Provider(command)=>self.provider_command(command),
  Command::Spawn(command)=>self.spawn_command(command),
  Command::Native(command)=>self.native_command(command),
  #[cfg(test)]
  Command::PauseFixture{ready,release}=>{let _=ready.send(());let _=release.await;},

  #[cfg(test)]
  Command::PendingReplyFixture{session,reply}=>{if let Some(p)=self.processes.get_mut(&session){p.pending_replies.insert(1,p.offset);p.reply_receipts.insert(1,1);p.reply_cursor.pending=true;let _=self.recovery.save_reply_cursor(session,p.reply_cursor);}let _=reply.send(());},

  #[cfg(test)]
  Command::ReplayFixture{session,end,reply}=>{if let Some(p)=self.processes.get_mut(&session){p.replay_end=end;}let _=reply.send(());},

  #[cfg(test)]
  Command::Crash{..}=>{},
  Command::ReplySent{session,batch,internal}=>{let ready=if let Some(p)=self.processes.get_mut(&session){if p.early_receipts.remove(&internal){true}else{p.reply_receipts.insert(internal,batch);false}}else{false};if ready{self.reply_written(session,batch);}},
  Command::ResizeSent{session,epoch,internal}=>{if let Some(p)=self.processes.get_mut(&session){if p.early_receipts.remove(&internal){if !self.blocked_resize.contains(&session){p.applied_resize_epoch=epoch;p.generation=p.generation.wrapping_add(1);}}else{p.resize_receipts.insert(internal,epoch);}}},
  Command::Shutdown{..}=>{},
  Command::Join{reply}=>{
   if self.viewers.len()>=16{let _=reply.send(Err(Error::Invalid{reason:"this host already has 16 viewers; close an unused viewer and retry"}));return;}
   let id=self.next_viewer;self.next_viewer=self.next_viewer.wrapping_add(1);let (tx,receiver)=mpsc::channel(VIEW_QUEUE);self.viewers.insert(id,Viewer{tx,budget:std::sync::Arc::new(Semaphore::new(VIEW_BYTES)),open:BTreeMap::new(),inputs:BTreeMap::new(),settled_inputs:BTreeMap::new()});let _=reply.send(Ok(Subscription{id,receiver}));self.send_to(id,ServerFrame::Sessions{host:self.host.clone(),sessions:self.processes.values().map(|p|p.info.clone()).collect()});
  }
  Command::Leave(id)=>self.leave_viewer(id),
  Command::Frame(viewer,frame)=>{if self.viewers.contains_key(&viewer){self.client(viewer,frame).await;}},
  Command::Output(id,update)=>self.update(id,update),
  Command::Disconnected(id)=>self.transport_disconnected(id,None).await,
  Command::TransportFailed{id,source}=>self.transport_disconnected(id,Some(std::sync::Arc::new(source))).await,
  Command::Sent{session,internal,user}=>{let written=if let Some(p)=self.processes.get_mut(&session){if p.early_receipts.remove(&internal){true}else{p.receipts.insert(internal,user);false}}else{false};if written{self.receipt(session,user);}},
  Command::Saved{session,generation,result}=>{if let Some(p)=self.processes.get_mut(&session){p.saving=false;p.last_save=Instant::now();if result.is_ok(){p.saved_generation=generation;}}if result.is_err(){self.failure(Some(session),"recovery_write","The checkpoint could not be saved. Check private-folder permissions and free disk space; the previous checkpoint was preserved.");}},
  Command::Removed{session,result}=>{if result.is_err(){self.failure(Some(session),"recovery_remove","Screen persistence is disabled, but the old checkpoint could not be removed. Remove it from the private recovery folder after fixing permissions.");}else{self.sessions();}},
  Command::Created{store,profile,id,attached}=>{
   self.store=Some(store);self.persist_cwds();if let Some(id)=id {let spec=profile.spec();let settings=Settings{profile:Some(profile.name),recovery:profile.recovery,history:profile.history};let (lines,_)=history_limits(&settings.history);match new_terminal(&spec,lines){Ok(vt)=>self.add(id,&spec,settings,vt,0,None,attached.ok(),ReplyCursor::default()),Err(_)=>self.failure(Some(id),"terminal_create","The terminal engine could not allocate a new screen. Close unused sessions and retry.")};let _=self.recovery.save_settings(&self.settings);self.sessions();}else{self.failure(None,"create_failed","The configured program could not be started. Check the trusted launch profile and holder installation, then retry.");}
  }
  Command::CreateFailed=>self.failure(None,"create_worker","The launch worker stopped. Existing terminals remain active; restart the host before creating more sessions."),
 }
    }
    async fn transport_disconnected(
        &mut self,
        id: u64,
        source: Option<std::sync::Arc<terminal_session::Error>>,
    ) {
        self.fail_native_with_source(id, WriteUncertainty::TransportLost, source);
        if let Some(p) = self.processes.get_mut(&id) {
            p.writer = None;
            if p.exited.is_none() {
                p.info.state = if p.stopping {
                    SessionState::Exited { code: None }
                } else {
                    SessionState::Unavailable
                };
            }
        }
        let stopped = self
            .processes
            .get(&id)
            .is_some_and(|p| p.stopping || p.exited.is_some());
        if stopped {
            let code = self.processes.get(&id).and_then(|p| p.exited);
            self.send(ServerFrame::Exited { id, code });
        } else {
            self.failure(Some(id),"disconnected","The process transport ended. SSH holders are reattached automatically with bounded backoff; the saved display remains available and uncertain input is not resent. No replacement process was started.");
        }
        self.disconnect_transports(id).await;
        self.sessions();
    }
    fn automatic_replies(&mut self, id: u64, bytes: Vec<u8>, through: u64) {
        if bytes.is_empty() {
            return;
        }
        let Some(p) = self.processes.get_mut(&id) else {
            return;
        };
        if p.exited.is_some() {
            return;
        }
        let queued = if let Some(writer) = p.writer.clone() {
            if let Ok(permit) = writer.try_reserve_owned() {
                let cursor = ReplyCursor {
                    answered_through: p.reply_cursor.answered_through,
                    pending: true,
                };
                if self.recovery.save_reply_cursor(id, cursor).is_ok() {
                    let batch = p.next_reply;
                    p.next_reply = p.next_reply.wrapping_add(1);
                    p.pending_replies.insert(batch, through);
                    p.reply_cursor = cursor;
                    permit.send(Write::Reply { bytes, batch });
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };
        if !queued {
            p.info.state = SessionState::Unavailable;
            p.writer = None;
            self.fail_native(id, WriteUncertainty::TransportLost);
            if self.blocked_replies.insert(id) {
                self.failure(Some(id),"reply_not_queued","A terminal reply could not be safely recorded or queued. The process remains alive and the previous checkpoint is preserved; fix disk/input backpressure and restart the host to reattach.");
                self.sessions();
            }
        }
    }
    fn reply_written(&mut self, id: u64, batch: u64) {
        let cursor = if let Some(p) = self.processes.get_mut(&id) {
            let Some(through) = p.pending_replies.remove(&batch) else {
                return;
            };
            p.reply_cursor.answered_through = p.reply_cursor.answered_through.max(through);
            p.reply_cursor.pending = !p.pending_replies.is_empty();
            Some(p.reply_cursor)
        } else {
            None
        };
        if let Some(cursor) = cursor {
            if self.recovery.save_reply_cursor(id, cursor).is_err() {
                self.failure(Some(id),"reply_recovery_write","A terminal reply reached the process, but its recovery watermark could not be saved. Fix private-folder permissions or disk space; a crash may require explicit recovery rather than replaying uncertain control bytes.");
            }
        }
    }
    fn metadata(&mut self, id: u64, events: &[VtEvent]) {
        let title = events.iter().rev().find_map(|event| {
            if let VtEvent::TitleChanged(title) = event {
                Some(clean(title, 120))
            } else {
                None
            }
        });
        let cwd = events.iter().rev().find_map(|event| {
            if let VtEvent::PwdChanged(cwd) = event {
                reported_cwd(cwd)
            } else {
                None
            }
        });
        let mut changed = false;
        if let Some(p) = self.processes.get_mut(&id) {
            if let Some(title) = title {
                let title = display_title(&p.fallback_title, Some(&title));
                if p.info.automatic_title != title {
                    p.info.automatic_title = title.clone();
                    p.info.title = p
                        .info
                        .name_override
                        .as_ref()
                        .map(|name| name.title.clone())
                        .unwrap_or_else(|| title.clone());
                    self.pending_titles.insert(id, title);
                    changed = true;
                }
            }
            if let Some(cwd) = cwd {
                if p.info.cwd != cwd.to_string_lossy() {
                    p.info.cwd = cwd.to_string_lossy().into_owned();
                    self.pending_cwd.insert(id, cwd);
                    changed = true;
                }
            }
        }
        self.persist_cwds();
        if changed
            && self
                .processes
                .get(&id)
                .is_some_and(|p| p.writer.is_none() || p.offset >= p.replay_end)
        {
            self.sessions();
        }
    }
    fn persist_cwds(&mut self) {
        let Some(store) = &mut self.store else {
            return;
        };
        let pending = std::mem::take(&mut self.pending_cwd);
        let mut failed = false;
        for (id, cwd) in pending {
            if self
                .processes
                .get(&id)
                .is_some_and(|p| p.writer.is_some() && p.offset < p.replay_end)
            {
                self.pending_cwd.insert(id, cwd);
                continue;
            }
            if store.set_cwd(id, cwd.clone()).is_err() {
                self.pending_cwd.insert(id, cwd);
                failed = true;
            }
        }
        let pending = std::mem::take(&mut self.pending_titles);
        for (id, title) in pending {
            if self
                .processes
                .get(&id)
                .is_some_and(|p| p.writer.is_some() && p.offset < p.replay_end)
            {
                self.pending_titles.insert(id, title);
                continue;
            }
            if store.set_automatic_title(id, title.clone()).is_err() {
                self.pending_titles.insert(id, title);
                failed = true;
            }
        }
        if failed {
            self.failure(None,"cwd_metadata","Terminal directory/title metadata could not be recorded. Check the private state folder and free space; the live process remains active.");
        }
    }
    fn receipt(&mut self, id: u64, user: Option<(u64, u64)>) {
        if let Some((viewer, seq)) = user {
            if let Some(v) = self.viewers.get_mut(&viewer) {
                let settled = v.settled_inputs.entry(id).or_default();
                *settled = (*settled).max(seq);
            }
            self.send_to(viewer, ServerFrame::InputWritten { id, seq });
        }
    }
    fn update(&mut self, id: u64, update: Update) {
        match update {
            Update::Record(Record::Output { offset, bytes }) => {
                let Some(p) = self.processes.get_mut(&id) else {
                    return;
                };
                if p.writer.is_none() && matches!(p.info.state, SessionState::Unavailable) {
                    return;
                }
                if offset > p.offset {
                    self.reconnects.halt(id);
                    p.info.state = SessionState::Unavailable;
                    p.writer = None;
                    self.fail_native(id, WriteUncertainty::TransportLost);
                    self.irrecoverable_views(id);
                    self.failure(Some(id),"output_gap","Retained process output has a gap. The last valid display is preserved; start a new session or restore an explicit provider recovery state.");
                    return;
                }
                let skip = (p.offset - offset).min(bytes.len() as u64) as usize;
                if skip == bytes.len() {
                    return;
                }
                let bytes = bytes[skip..].to_vec();
                let offset = p.offset;
                let historical = p
                    .reply_cursor
                    .answered_through
                    .saturating_sub(offset)
                    .min(bytes.len() as u64) as usize;
                let mut metadata = Vec::new();
                if historical > 0 {
                    p.vt.feed(&bytes[..historical]);
                    metadata = p.vt.drain_events();
                    metadata.retain(|event| {
                        matches!(event, VtEvent::TitleChanged(_) | VtEvent::PwdChanged(_))
                    });
                }
                if historical < bytes.len() {
                    p.vt.feed(&bytes[historical..]);
                }
                let events = p.vt.drain_events();
                metadata.extend(
                    events
                        .iter()
                        .filter(|event| {
                            matches!(event, VtEvent::TitleChanged(_) | VtEvent::PwdChanged(_))
                        })
                        .cloned(),
                );
                p.offset += bytes.len() as u64;
                p.generation = p.generation.wrapping_add(1);
                let end = p.offset;
                self.metadata(id, &metadata);
                let replies = engine_replies(events);
                self.automatic_replies(id, replies, end);
                let mut deliveries = Vec::new();
                for (viewer, v) in &mut self.viewers {
                    if let Some(open) = v.open.get_mut(&id) {
                        if !open.resync
                            && end.saturating_sub(open.acked) <= OUTPUT_WINDOW
                            && v.tx.capacity() > VIEW_CONTROL_RESERVE
                        {
                            open.sent = end;
                            deliveries.push((
                                *viewer,
                                ServerFrame::Output {
                                    id,
                                    offset,
                                    bytes: bytes.clone(),
                                },
                            ));
                        } else if !open.resync {
                            open.resync = true;
                            deliveries.push((*viewer,ServerFrame::ResyncRequired{id,reason:"The viewer reached its bounded output window or queue high-water. Request a fresh checkpoint before accepting more output.".into()}));
                        }
                    }
                }
                for (viewer, frame) in deliveries {
                    self.send_to(viewer, frame);
                }
            }
            Update::Record(Record::Gap { .. }) => {
                self.reconnects.halt(id);
                self.fail_native(id, WriteUncertainty::TransportLost);
                if let Some(p) = self.processes.get_mut(&id) {
                    p.info.state = SessionState::Unavailable;
                    p.writer = None;
                }
                self.irrecoverable_views(id);
                self.failure(Some(id),"recovery_gap","The holder no longer retains all bytes after the saved checkpoint. The saved screen is preserved, but live input is disabled; use explicit provider recovery or start a new terminal.");
                self.sessions();
            }
            Update::Record(Record::Resize { .. }) => {}
            Update::Written(internal) => {
                if let Some(ticket) = self
                    .processes
                    .get_mut(&id)
                    .and_then(|p| p.native_receipts.remove(&internal))
                {
                    self.native_written(ticket);
                    return;
                }
                let reply = self
                    .processes
                    .get_mut(&id)
                    .and_then(|p| p.reply_receipts.remove(&internal));
                if let Some(batch) = reply {
                    self.reply_written(id, batch);
                    return;
                }
                let user = if let Some(p) = self.processes.get_mut(&id) {
                    if let Some(epoch) = p.resize_receipts.remove(&internal) {
                        if !self.blocked_resize.contains(&id) {
                            p.applied_resize_epoch = epoch;
                            p.generation = p.generation.wrapping_add(1);
                        }
                        None
                    } else {
                        match p.receipts.remove(&internal) {
                            Some(user) => user,
                            None => {
                                if p.early_receipts.len() < WRITER_QUEUE * 4 {
                                    p.early_receipts.insert(internal);
                                }
                                None
                            }
                        }
                    }
                } else {
                    None
                };
                self.receipt(id, user);
            }
            Update::Exited(code) => {
                self.fail_native(id, WriteUncertainty::ProcessExited);
                if let Some(p) = self.processes.get_mut(&id) {
                    p.exited = Some(code);
                    p.info.state = SessionState::Exited { code: Some(code) };
                    p.generation = p.generation.wrapping_add(1);
                }
                self.send(ServerFrame::Exited {
                    id,
                    code: Some(code),
                });
                self.sessions();
            }
            Update::Refused(_) => {
                self.fail_native(id, WriteUncertainty::Refused);
                self.blocked_resize.insert(id);
                if let Some(p) = self.processes.get_mut(&id) {
                    p.writer = None;
                    p.applied_resize_epoch = 0;
                    p.info.state = SessionState::Unavailable;
                }
                self.failure(Some(id),"input_refused","The holder refused terminal input or resize. The last display is preserved and live input is disabled; inspect the holder before explicitly recovering. Uncertain input was not retried.");
                self.sessions();
            }
        }
    }
    async fn client(&mut self, viewer: u64, frame: ClientFrame) {
        match frame {
            ClientFrame::Authenticate { .. } => self.failure_to(
                viewer,
                None,
                "already_authenticated",
                "This connection is already authenticated.",
            ),
            ClientFrame::List => self.sessions_to(viewer),
            ClientFrame::Open { id } => {
                if !self.processes.contains_key(&id) {
                    self.failure_to(
                        viewer,
                        Some(id),
                        "missing_session",
                        "Select an existing terminal.",
                    );
                    return;
                }
                self.controllers
                    .entry(id)
                    .or_insert(Controller::Viewer(viewer));
                self.send_to(
                    viewer,
                    ServerFrame::Control {
                        id,
                        controller: self.controllers.get(&id) == Some(&Controller::Viewer(viewer)),
                    },
                );
                self.snapshot(viewer, id);
            }
            ClientFrame::Detach { id } => {
                if let Some(view) = self.viewers.get_mut(&viewer) {
                    view.open.remove(&id);
                }
                if let Some(process) = self.processes.get_mut(&id) {
                    process.pending_open.remove(&viewer);
                    if let HistoryPolicy::Recent {
                        interval_seconds, ..
                    } = process.settings.history
                    {
                        if process.generation != process.saved_generation && !process.saving {
                            process.last_save =
                                Instant::now() - Duration::from_secs(interval_seconds);
                        }
                    }
                }
                if self.controllers.get(&id) == Some(&Controller::Viewer(viewer)) {
                    self.controllers.remove(&id);
                    self.controls(id);
                }
                self.send_to(viewer, ServerFrame::Detached { id });
            }
            ClientFrame::TakeControl { id } => {
                if self
                    .viewers
                    .get(&viewer)
                    .is_some_and(|v| v.open.contains_key(&id))
                {
                    self.controllers.insert(id, Controller::Viewer(viewer));
                    self.controls(id);
                } else {
                    self.send_to(
                        viewer,
                        ServerFrame::Error {
                            id: Some(id),
                            code: "not_open".into(),
                            message: "Open this terminal before taking control.".into(),
                        },
                    );
                }
            }
            ClientFrame::SnapshotRequest { id } => self.snapshot(viewer, id),
            ClientFrame::Ack { id, offset } => {
                let mut refresh = false;
                if let Some(v) = self.viewers.get_mut(&viewer) {
                    if let Some(open) = v.open.get_mut(&id) {
                        open.acked = open.acked.max(offset.min(open.sent));
                        refresh = open.resync;
                    }
                }
                if refresh {
                    self.snapshot(viewer, id);
                }
            }
            ClientFrame::Input { id, seq, bytes } => {
                if !self.require_control(viewer, id) {
                    return;
                }
                if bytes.len() > MAX_INPUT {
                    self.failure_to(
                        viewer,
                        Some(id),
                        "input_too_large",
                        "Paste less than 256 KiB at a time; this input was not sent.",
                    );
                    return;
                }
                if self
                    .viewers
                    .get(&viewer)
                    .and_then(|v| v.inputs.get(&id))
                    .is_some_and(|last| seq <= *last)
                {
                    self.failure_to(
                        viewer,
                        Some(id),
                        "input_sequence",
                        "Input sequence numbers must increase; duplicate input was not sent.",
                    );
                    return;
                }
                let Some(p) = self.processes.get(&id) else {
                    self.failure_to(
                        viewer,
                        Some(id),
                        "missing_session",
                        "Select an existing terminal.",
                    );
                    return;
                };
                if !matches!(p.info.state, SessionState::Running) || p.stopping {
                    self.failure_to(viewer,Some(id),"not_running","This terminal is not writable. Inspect its saved state or create a new session from an explicit profile.");
                    return;
                }
                if bytes.is_empty() {
                    if let Some(v) = self.viewers.get_mut(&viewer) {
                        v.inputs.insert(id, seq);
                    }
                    if let Some(v) = self.viewers.get_mut(&viewer) {
                        let settled = v.settled_inputs.entry(id).or_default();
                        *settled = (*settled).max(seq);
                    }
                    self.send_to(viewer, ServerFrame::InputWritten { id, seq });
                    return;
                }
                if p.writer.as_ref().is_none_or(|w| {
                    w.try_send(Write::Input {
                        bytes,
                        user: Some((viewer, seq)),
                    })
                    .is_err()
                }) {
                    self.failure_to(
                        viewer,
                        Some(id),
                        "input_backpressure",
                        "Input was not queued. Wait for pending input to drain, then retry.",
                    );
                    return;
                }
                if let Some(v) = self.viewers.get_mut(&viewer) {
                    v.inputs.insert(id, seq);
                }
            }
            ClientFrame::Resize {
                id,
                cols,
                rows,
                cell_width,
                cell_height,
            } => {
                if !self.require_control(viewer, id) {
                    return;
                }
                if cols == 0
                    || rows == 0
                    || cols > 1000
                    || rows > 1000
                    || cell_width == 0
                    || cell_height == 0
                    || cell_width > 512
                    || cell_height > 512
                {
                    self.failure_to(viewer,Some(id),"invalid_size","Choose terminal dimensions between 1 and 1000 cells with cell dimensions between 1 and 512 pixels.");
                    return;
                }
                let Some(p) = self.processes.get_mut(&id) else {
                    return;
                };
                let Some(writer) = p.writer.clone() else {
                    return;
                };
                let Ok(permit) = writer.try_reserve() else {
                    self.failure_to(
                        viewer,
                        Some(id),
                        "resize_backpressure",
                        "Resize was not queued. Wait for pending input and try again.",
                    );
                    return;
                };
                let epoch = p.resize_epoch.wrapping_add(1).max(1);
                if self.recovery.mark_resize(id, epoch).is_err() {
                    self.failure_to(viewer,Some(id),"resize_recovery_guard","Resize was not applied because its recovery guard could not be saved. Check the private folder and disk space, then retry.");
                    return;
                }
                p.resize_epoch = epoch;
                if p.vt
                    .resize_with_cell_size(
                        cols,
                        rows,
                        u32::from(cell_width),
                        u32::from(cell_height),
                    )
                    .is_err()
                {
                    self.failure_to(viewer,Some(id),"resize_failed","The terminal could not allocate its resized screen. Reduce its dimensions and retry.");
                    return;
                }
                let replies = engine_replies(p.vt.drain_events());
                p.generation = p.generation.wrapping_add(1);
                permit.send(Write::Resize {
                    epoch,
                    cols,
                    rows,
                    replies: (!replies.is_empty())
                        .then_some(replies)
                        .into_iter()
                        .collect(),
                });
                self.send(ServerFrame::Resized {
                    id,
                    cols,
                    rows,
                    cell_width,
                    cell_height,
                });
            }
            ClientFrame::Stop { id } => {
                if !self.require_control(viewer, id) {
                    return;
                }
                let Some(p) = self.processes.get_mut(&id) else {
                    return;
                };
                if p.writer
                    .as_ref()
                    .is_some_and(|w| w.try_send(Write::Stop).is_ok())
                {
                    p.stopping = true;
                } else {
                    self.failure_to(viewer,Some(id),"stop_failed","Stop could not be queued. Reconnect to the existing holder before trying again.");
                }
            }
            ClientFrame::Create { profile } => self.create(viewer, profile),
            ClientFrame::SetRecovery { id, hint } => {
                if !self.require_control(viewer, id) {
                    return;
                }
                if validate_recovery(hint.as_ref()).is_err() {
                    self.failure_to(viewer,Some(id),"invalid_recovery","Recovery identifiers must be explicit, bounded values without control characters.");
                    return;
                }
                let Some(store) = &mut self.store else {
                    self.failure_to(
                        viewer,
                        Some(id),
                        "host_busy",
                        "A terminal is being started. Retry this metadata change shortly.",
                    );
                    return;
                };
                if store.set_recovery(id, hint.clone()).is_err() {
                    self.failure_to(viewer,Some(id),"recovery_metadata","Recovery metadata could not be saved. Check private-folder permissions and free space.");
                    return;
                }
                if let Some(p) = self.processes.get_mut(&id) {
                    p.settings.recovery = hint.clone();
                    p.info.recovery_available = hint.is_some();
                    self.settings.insert(id, p.settings.clone());
                }
                if self.recovery.save_settings(&self.settings).is_err() {
                    self.failure_to(
                        viewer,
                        Some(id),
                        "recovery_metadata",
                        "Recovery settings could not be saved. Check the private recovery folder.",
                    );
                } else {
                    self.sessions();
                }
            }
            ClientFrame::SetHistory { id, policy } => {
                if !self.require_control(viewer, id) {
                    return;
                }
                if self.history_disabled && !matches!(policy, HistoryPolicy::Off) {
                    self.failure_to(viewer,Some(id),"history_disabled","This host was started with disk screen persistence disabled. Restart without --no-history to enable it explicitly.");
                    return;
                }
                if validate_history(&policy).is_err() {
                    self.failure_to(viewer,Some(id),"invalid_history","Choose at most 100,000 history lines, 64 KiB to 4 MiB of checkpoint data, and a 1–3600 second interval.");
                    return;
                }
                let Some(p) = self.processes.get_mut(&id) else {
                    return;
                };
                let (lines, _) = history_limits(&policy);
                if p.vt
                    .set_scrollback_limits(lines, LIVE_HISTORY_BYTES)
                    .is_err()
                {
                    self.failure_to(viewer,Some(id),"history_limit","The terminal could not apply its history limits. Reduce the limits and retry.");
                    return;
                }
                p.settings.history = policy.clone();
                p.generation = p.generation.wrapping_add(1);
                p.last_save = Instant::now() - Duration::from_secs(3601);
                self.settings.insert(id, p.settings.clone());
                if self.recovery.save_settings(&self.settings).is_err() {
                    self.failure_to(
                        viewer,
                        Some(id),
                        "history_metadata",
                        "History settings could not be saved. Check the private recovery folder.",
                    );
                    return;
                }
                if matches!(policy, HistoryPolicy::Off) {
                    if self.disk.send(DiskJob::Remove(id)).await.is_err() {
                        self.failure_to(viewer,Some(id),"recovery_worker","The recovery worker stopped; restart the host and inspect its private recovery folder.");
                    }
                } else {
                    self.sessions();
                }
            }
        }
    }
    fn create(&mut self, viewer: u64, name: String) {
        if self.processes.len() >= 128 {
            self.failure_to(viewer,None,"session_limit","This host retains 128 session records. Use a separate host workspace for additional sessions.");
            return;
        }
        let Some(profile) = self.profiles.get(&name).cloned() else {
            self.failure_to(
                viewer,
                None,
                "unknown_profile",
                "Choose a trusted launch profile configured by the host owner.",
            );
            return;
        };
        let Some(mut store) = self.store.take() else {
            self.failure_to(viewer,None,"host_busy","A program is already starting. Wait for it to finish before creating another terminal.");
            return;
        };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let profile_for_launch = profile.clone();
            let launched = tokio::task::spawn_blocking(move || {
                let before = store.sessions().len();
                let result = store.create(profile_for_launch.spec());
                let id = match &result {
                    Ok(id) => Some(*id),
                    Err(_) => store.sessions().get(before).map(|s| s.id),
                };
                (store, result, id)
            })
            .await;
            match launched {
                Ok((mut store, result, id)) => {
                    let attached = match result {
                        Ok(id) => {
                            let _ = store.set_recovery(id, profile.recovery.clone());
                            store.attach_live_from(id, 0).await.map_err(Error::from)
                        }
                        Err(e) => Err(e.into()),
                    };
                    let _ = tx
                        .send(Command::Created {
                            store,
                            profile,
                            id,
                            attached,
                        })
                        .await;
                }
                Err(_) => {
                    let _ = tx.send(Command::CreateFailed).await;
                }
            }
        });
    }
    fn checkpoints(&mut self) {
        for (id, p) in &mut self.processes {
            if self.blocked_replies.contains(id) {
                continue;
            }
            let HistoryPolicy::Recent {
                max_bytes,
                interval_seconds,
                ..
            } = p.settings.history
            else {
                continue;
            };
            if p.saving
                || p.generation == p.saved_generation
                || p.last_save.elapsed() < Duration::from_secs(interval_seconds)
            {
                continue;
            }
            p.last_save = Instant::now();
            let Ok(bytes) = p.vt.checkpoint(MAX_CHECKPOINT) else {
                continue;
            };
            let cp = Checkpoint {
                bytes,
                offset: p.offset,
                cols: p.vt.size().cols,
                rows: p.vt.size().rows,
                exited: p.exited,
                resize_epoch: p.applied_resize_epoch,
            };
            p.saving = self
                .disk
                .try_send(DiskJob::Save {
                    session: *id,
                    generation: p.generation,
                    checkpoint: cp,
                    limit: max_bytes,
                })
                .is_ok();
        }
    }
}
fn transport_tasks(
    id: u64,
    attached: Attached,
    mut requests: mpsc::Receiver<Write>,
    tx: mpsc::Sender<Command>,
) -> Vec<JoinHandle<()>> {
    let Attached {
        mut reader,
        mut writer,
        ..
    } = attached;
    let output = tx.clone();
    let reader = tokio::spawn(async move {
        loop {
            match reader.next().await {
                Ok(Some(update)) => {
                    if output.send(Command::Output(id, update)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => {
                    let _ = output.send(Command::Disconnected(id)).await;
                    break;
                }
                Err(source) => {
                    let _ = output.send(Command::TransportFailed { id, source }).await;
                    break;
                }
            }
        }
    });
    let writer = tokio::spawn(async move {
        while let Some(request) = requests.recv().await {
            let result = match request {
                Write::Native { bytes, ticket } => match writer.input(&bytes).await {
                    Ok(internal) => {
                        let _ = tx
                            .send(Command::Native(native::NativeCommand::Sent {
                                id,
                                ticket,
                                internal,
                            }))
                            .await;
                        Ok(())
                    }
                    Err(error) => Err(error),
                },
                Write::Reply { bytes, batch } => match writer.input(&bytes).await {
                    Ok(internal) => {
                        let _ = tx
                            .send(Command::ReplySent {
                                session: id,
                                batch,
                                internal,
                            })
                            .await;
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                Write::Input { bytes, user } => match writer.input(&bytes).await {
                    Ok(internal) => {
                        let _ = tx
                            .send(Command::Sent {
                                session: id,
                                internal,
                                user,
                            })
                            .await;
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                Write::Resize {
                    cols,
                    rows,
                    replies,
                    epoch,
                } => {
                    let mut result = writer.resize(cols, rows).await;
                    for bytes in replies {
                        if result.is_ok() {
                            result = match writer.input(&bytes).await {
                                Ok(internal) => {
                                    let _ = tx
                                        .send(Command::Sent {
                                            session: id,
                                            internal,
                                            user: None,
                                        })
                                        .await;
                                    Ok(())
                                }
                                Err(e) => Err(e),
                            };
                        }
                    }
                    if result.is_ok() {
                        result = match writer.input(&[]).await {
                            Ok(internal) => {
                                let _ = tx
                                    .send(Command::ResizeSent {
                                        session: id,
                                        epoch,
                                        internal,
                                    })
                                    .await;
                                Ok(())
                            }
                            Err(e) => Err(e),
                        };
                    }
                    result
                }
                Write::Stop => writer.stop().await,
            };
            if let Err(source) = result {
                let _ = tx.send(Command::TransportFailed { id, source }).await;
                break;
            }
        }
    });
    vec![reader, writer]
}
fn disk_worker(
    recovery: Recovery,
    mut jobs: mpsc::Receiver<DiskJob>,
    tx: mpsc::UnboundedSender<Command>,
) {
    tokio::task::spawn_blocking(move || {
        while let Some(job) = jobs.blocking_recv() {
            let event = match job {
                DiskJob::Barrier(done) => {
                    let _ = done.send(());
                    continue;
                }
                DiskJob::SaveSync {
                    session,
                    checkpoint,
                    limit,
                    reply,
                } => {
                    let _ = reply.send(recovery.save(session, checkpoint, limit));
                    continue;
                }
                DiskJob::Save {
                    session,
                    generation,
                    checkpoint,
                    limit,
                } => Command::Saved {
                    session,
                    generation,
                    result: recovery.save(session, checkpoint, limit),
                },
                DiskJob::Remove(session) => Command::Removed {
                    session,
                    result: recovery.remove(session),
                },
            };
            let _ = tx.send(event);
        }
    });
}
fn display_title(profile: &str, runtime: Option<&str>) -> String {
    let profile = clean(profile, 120);
    let profile = if profile.trim().is_empty() {
        "Terminal".into()
    } else {
        profile
    };
    let runtime = runtime.map(|title| clean(title, 120)).unwrap_or_default();
    if runtime.trim().is_empty() || runtime == profile {
        profile
    } else {
        // Persisted automatic names are bounded in UTF-8 bytes. Preserve the
        // trusted profile prefix while leaving room for a valid runtime suffix.
        let available = 512usize.saturating_sub(profile.len() + " · ".len());
        let mut end = available.min(runtime.len());
        while !runtime.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            profile
        } else {
            format!("{profile} · {}", &runtime[..end])
        }
    }
}
fn reported_cwd(value: &str) -> Option<PathBuf> {
    if value.len() > 16384 || value.chars().any(char::is_control) {
        return None;
    }
    let decoded = if value.starts_with('/') {
        value.to_owned()
    } else {
        let (scheme, rest) = value.split_once(':')?;
        if !scheme.eq_ignore_ascii_case("file") {
            return None;
        }
        let path = if let Some(authority) = rest.strip_prefix("//") {
            let slash = authority.find('/')?;
            let host = &authority[..slash];
            if !host.is_empty()
                && (host.contains('@') || host.parse::<axum::http::uri::Authority>().is_err())
            {
                return None;
            }
            &authority[slash..]
        } else {
            rest
        };
        if path.contains(['?', '#']) {
            return None;
        }
        let raw = path.as_bytes();
        let mut out = Vec::with_capacity(raw.len());
        let mut cursor = 0;
        while cursor < raw.len() {
            if raw[cursor] == b'%' {
                let a = char::from(*raw.get(cursor + 1)?).to_digit(16)?;
                let b = char::from(*raw.get(cursor + 2)?).to_digit(16)?;
                out.push((a * 16 + b) as u8);
                cursor += 3;
            } else {
                out.push(raw[cursor]);
                cursor += 1;
            }
        }
        String::from_utf8(out).ok()?
    };
    if decoded.len() > 4096 || decoded.chars().any(char::is_control) {
        return None;
    }
    let path = PathBuf::from(decoded);
    path.is_absolute().then_some(path)
}
fn frame_weight(frame: &ServerFrame) -> usize {
    256 + match frame {
        ServerFrame::Snapshot { bytes, .. } | ServerFrame::Output { bytes, .. } => bytes.len(),
        ServerFrame::Sessions { host, sessions } => {
            host.label.len()
                + host.os.len()
                + host.arch.len()
                + host.engine.len()
                + sessions
                    .iter()
                    .map(|s| {
                        256 + s.title.len()
                            + s.automatic_title.len()
                            + s.uuid.len()
                            + s.cwd.len()
                            + s.program.len()
                            + s.name_override.as_ref().map_or(0, |n| n.title.len())
                            + s.attributes
                                .iter()
                                .map(|(key, value)| key.len() + value.len())
                                .sum::<usize>()
                    })
                    .sum::<usize>()
        }
        ServerFrame::Error { code, message, .. } => code.len() + message.len(),
        ServerFrame::ResyncRequired { reason, .. } => reason.len(),
        _ => 0,
    }
}
fn new_terminal(spec: &SessionSpec, lines: usize) -> Result<Terminal, Error> {
    let mut vt = Terminal::new(spec.cols, spec.rows, lines)?;
    vt.set_scrollback_limits(lines, LIVE_HISTORY_BYTES)?;
    Ok(vt)
}
fn engine_replies(events: Vec<VtEvent>) -> Vec<u8> {
    let mut replies = Vec::new();
    for event in events {
        match event {
            VtEvent::Reply(bytes) => replies.extend(bytes),
            VtEvent::ClipboardReadRequested { target } => {
                replies.extend(clipboard_read_reply(target, None))
            }
            _ => {}
        }
    }
    replies
}
fn clean(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}
fn validate_name(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(Error::Invalid {
            reason: "profile names must contain 1–64 letters, digits, hyphens or underscores",
        });
    }
    Ok(())
}
fn validate_recovery(hint: Option<&RecoveryHint>) -> Result<(), Error> {
    if let Some(hint) = hint {
        let valid = |value: &str, max: usize| {
            !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
        };
        if !valid(&hint.provider, 64)
            || !valid(&hint.session_id, 512)
            || !valid(&hint.working_directory, 4096)
            || hint.state_paths.len() > 16
            || hint.state_paths.iter().any(|path| !valid(path, 4096))
        {
            return Err(Error::Invalid {
                reason: "recovery metadata needs a provider, session ID and working directory without control characters, plus at most sixteen state paths",
            });
        }
    }
    Ok(())
}
fn validate_history(policy: &HistoryPolicy) -> Result<(), Error> {
    if let HistoryPolicy::Recent {
        max_lines,
        max_bytes,
        interval_seconds,
    } = policy
    {
        if *max_lines == 0
            || *max_lines > 100000
            || *max_bytes < 65536
            || *max_bytes > MAX_ENCODED_CHECKPOINT as u64
            || *interval_seconds == 0
            || *interval_seconds > 3600
        {
            return Err(Error::Invalid {
                reason: "recent history needs 1–100000 lines, 64 KiB–4 MiB, and a 1–3600 second checkpoint interval",
            });
        }
    }
    Ok(())
}
fn history_limits(policy: &HistoryPolicy) -> (usize, usize) {
    match policy {
        HistoryPolicy::Off => (10000, MAX_ENCODED_CHECKPOINT),
        HistoryPolicy::Recent {
            max_lines,
            max_bytes,
            ..
        } => (
            *max_lines as usize,
            (*max_bytes as usize).min(MAX_ENCODED_CHECKPOINT),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn holder() -> PathBuf {
        std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap()
    }
    fn state() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("thost-")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }
    async fn next(view: &mut Subscription) -> ServerFrame {
        tokio::time::timeout(Duration::from_secs(8), view.receiver.recv())
            .await
            .unwrap()
            .unwrap()
            .frame
    }
    #[tokio::test]
    async fn native_host_answers_queries_without_viewers_and_snapshots_the_live_state() {
        let dir = state();
        let marker = dir.path().join("query-ready");
        let profile=Profile{name:"query".into(),title:"Query fixture".into(),program:"/bin/sh".into(),args:vec!["-c".into(),r#"stty -icanon -echo min 1 time 0; printf '\033[6n'; dd bs=1 count=6 2>/dev/null | od -An -tu1; printf 'NO_VIEWER_OK\n'; printf ready > "$1"; stty icanon; exec /bin/sh -i"#.into(),"terminal-test".into(),marker.to_string_lossy().into_owned()],cwd:dir.path().into(),endpoint:Endpoint::Local,recovery:None,history:HistoryPolicy::Recent{max_lines:10000,max_bytes:4<<20,interval_seconds:1}};
        let store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
        let host = Host::start(
            Config {
                state: dir.path().into(),
                label: "test".into(),
                start_profiles: vec!["query".into()],
                default_history: HistoryPolicy::default(),
            },
            store,
            vec![profile],
        )
        .await
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while !marker.exists() {
            assert!(
                Instant::now() < deadline,
                "the host must answer DSR before any viewer connects"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut viewer = host.subscribe().await.unwrap();
        assert!(matches!(
            next(&mut viewer).await,
            ServerFrame::Sessions { .. }
        ));
        host.frame(viewer.id, ClientFrame::Open { id: 1 })
            .await
            .unwrap();
        let snapshot = loop {
            if let ServerFrame::Snapshot { bytes, .. } = next(&mut viewer).await {
                break bytes;
            }
        };
        let mut terminal = Terminal::from_checkpoint(&snapshot).unwrap();
        use ghostty_vt::Grid;
        terminal.sync();
        let text = (0..terminal.size().rows)
            .filter_map(|y| terminal.row(y))
            .map(|r| r.cells.iter().map(|c| c.text.as_str()).collect::<String>())
            .collect::<String>();
        assert!(text.contains("NO_VIEWER_OK"));

        host.frame(
            viewer.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"printf 'LIVE_%s\\n' CONTINUES\r".to_vec(),
            },
        )
        .await
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            assert!(Instant::now() < deadline);
            if matches!(
                next(&mut viewer).await,
                ServerFrame::InputWritten { id: 1, seq: 1 }
            ) {
                break;
            }
        }
        host.leave(viewer.id).await;
        drop(viewer);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut viewer = host.subscribe().await.unwrap();
        let _ = next(&mut viewer).await;
        host.frame(viewer.id, ClientFrame::Open { id: 1 })
            .await
            .unwrap();
        let bytes = loop {
            if let ServerFrame::Snapshot { bytes, .. } = next(&mut viewer).await {
                break bytes;
            }
        };
        let mut terminal = Terminal::from_checkpoint(&bytes).unwrap();
        terminal.sync();
        let text = (0..terminal.size().rows)
            .filter_map(|y| terminal.row(y))
            .map(|r| r.cells.iter().map(|c| c.text.as_str()).collect::<String>())
            .collect::<String>();
        assert!(text.contains("LIVE_CONTINUES"));
        let metadata = std::fs::read_to_string(dir.path().join("workspace.json")).unwrap();
        assert!(!metadata.contains("NO_VIEWER_OK"));
        assert!(!metadata.contains("LIVE_CONTINUES"));
        host.frame(viewer.id, ClientFrame::Stop { id: 1 })
            .await
            .unwrap();
        loop {
            if matches!(next(&mut viewer).await, ServerFrame::Exited { id: 1, .. }) {
                break;
            }
        }
    }

    fn shell_profile(dir: &std::path::Path) -> Profile {
        Profile {
            name: "shell".into(),
            title: "Shell".into(),
            program: "/bin/sh".into(),
            args: vec!["-i".into()],
            cwd: dir.into(),
            endpoint: Endpoint::Local,
            recovery: None,
            history: HistoryPolicy::Recent {
                max_lines: 10000,
                max_bytes: 4 << 20,
                interval_seconds: 1,
            },
        }
    }
    async fn start(dir: &std::path::Path, profiles: Vec<Profile>) -> Host {
        let names = profiles.iter().map(|p| p.name.clone()).collect();
        Host::start(
            Config {
                state: dir.into(),
                label: "test".into(),
                start_profiles: names,
                default_history: HistoryPolicy::default(),
            },
            SessionStore::open_memory_only(dir, holder()).unwrap(),
            profiles,
        )
        .await
        .unwrap()
    }
    async fn open(host: &Host, id: u64) -> Subscription {
        let mut view = host.subscribe().await.unwrap();
        let _ = next(&mut view).await;
        host.frame(view.id, ClientFrame::Open { id }).await.unwrap();
        view
    }
    async fn output_until(view: &mut Subscription, needle: &str) -> String {
        let mut result = String::new();
        loop {
            if let ServerFrame::Output { bytes, .. } = next(view).await {
                result.push_str(&String::from_utf8_lossy(&bytes));
                if result.contains(needle) {
                    return result;
                }
            }
        }
    }
    async fn role(view: &mut Subscription, owner: bool) {
        loop {
            if let ServerFrame::Control { controller, .. } = next(view).await {
                assert_eq!(controller, owner);
                return;
            }
        }
    }
    async fn stop(host: &Host, view: &mut Subscription, id: u64) {
        host.frame(view.id, ClientFrame::Stop { id }).await.unwrap();
        loop {
            if matches!(next(view).await, ServerFrame::Exited { .. }) {
                break;
            }
        }
    }
    #[tokio::test]
    async fn two_viewers_share_output_and_explicitly_transfer_control() {
        let dir = state();
        let host = start(dir.path(), vec![shell_profile(dir.path())]).await;
        let mut first = open(&host, 1).await;
        role(&mut first, true).await;
        let mut mirror = open(&host, 1).await;
        role(&mut mirror, false).await;
        host.frame(
            mirror.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"printf forbidden\\n\r".to_vec(),
            },
        )
        .await
        .unwrap();
        loop {
            if let ServerFrame::Error { code, .. } = next(&mut mirror).await {
                assert_eq!(code, "read_only");
                break;
            }
        }
        host.frame(
            first.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"KEEP=stable; export KEEP; printf 'MIRROR_%s\\n' READY\r".to_vec(),
            },
        )
        .await
        .unwrap();
        let a = output_until(&mut first, "MIRROR_READY").await;
        let b = output_until(&mut mirror, "MIRROR_READY").await;
        assert!(!a.contains("forbidden"));
        assert!(!b.contains("forbidden"));
        host.frame(
            mirror.id,
            ClientFrame::Resize {
                id: 1,
                cols: 44,
                rows: 11,
                cell_width: 9,
                cell_height: 18,
            },
        )
        .await
        .unwrap();
        loop {
            if let ServerFrame::Error { code, .. } = next(&mut mirror).await {
                assert_eq!(code, "read_only");
                break;
            }
        }
        host.frame(mirror.id, ClientFrame::TakeControl { id: 1 })
            .await
            .unwrap();
        role(&mut first, false).await;
        role(&mut mirror, true).await;
        host.frame(
            mirror.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"printf 'CONTROL_%s KEEP=%s\\n' READY $KEEP\r".to_vec(),
            },
        )
        .await
        .unwrap();
        assert!(
            output_until(&mut first, "CONTROL_READY KEEP=stable")
                .await
                .contains("KEEP=stable")
        );
        assert!(
            output_until(&mut mirror, "CONTROL_READY KEEP=stable")
                .await
                .contains("KEEP=stable")
        );
        host.frame(
            mirror.id,
            ClientFrame::Resize {
                id: 1,
                cols: 83,
                rows: 27,
                cell_width: 9,
                cell_height: 18,
            },
        )
        .await
        .unwrap();
        for view in [&mut first, &mut mirror] {
            loop {
                if let ServerFrame::Resized { cols, rows, .. } = next(view).await {
                    assert_eq!((cols, rows), (83, 27));
                    break;
                }
            }
        }
        host.leave(mirror.id).await;
        role(&mut first, false).await;
        host.frame(first.id, ClientFrame::TakeControl { id: 1 })
            .await
            .unwrap();
        role(&mut first, true).await;
        stop(&host, &mut first, 1).await;
        host.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn stalled_viewer_loses_lease_without_interrupting_other_viewers() {
        let dir = state();
        let host = start(dir.path(), vec![shell_profile(dir.path())]).await;
        let mut slow = open(&host, 1).await;
        role(&mut slow, true).await;
        let mut active = open(&host, 1).await;
        role(&mut active, false).await;
        for _ in 0..128 {
            host.frame(slow.id, ClientFrame::SnapshotRequest { id: 1 })
                .await
                .unwrap();
        }
        host.frame(active.id, ClientFrame::TakeControl { id: 1 })
            .await
            .unwrap();
        loop {
            if let ServerFrame::Control {
                controller: true, ..
            } = next(&mut active).await
            {
                break;
            }
        }
        host.frame(
            active.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"printf 'ACTIVE_%s\\n' VIEWER\r".to_vec(),
            },
        )
        .await
        .unwrap();
        assert!(
            output_until(&mut active, "ACTIVE_VIEWER")
                .await
                .contains("ACTIVE_VIEWER")
        );
        stop(&host, &mut active, 1).await;
        host.shutdown().await.unwrap();
    }

    #[test]
    fn composed_unicode_titles_preserve_profile_and_fit_persisted_utf8_limit() {
        for (profile, runtime) in [
            ("🐱".repeat(120), "🐶".repeat(120)),
            ("p".repeat(120), "界".repeat(120)),
            ("p".repeat(119), "🐶".repeat(120)),
        ] {
            let title = display_title(&profile, Some(&runtime));
            assert!(title.starts_with(&profile));
            assert!(title.len() <= 512);
            assert!(title.contains(runtime.chars().next().unwrap()));
            assert!(!title.ends_with(" · "));
        }
    }
    #[test]
    fn cleared_runtime_title_and_blank_legacy_label_keep_a_valid_fallback() {
        for profile in ["", "  ", "\n\0"] {
            for runtime in [None, Some(""), Some("  "), Some("\n\0")] {
                let title = display_title(profile, runtime);
                assert!(!title.trim().is_empty());
                assert!(!title.chars().any(char::is_control));
                assert!(title.len() <= 512);
            }
        }
    }
    #[test]
    fn reported_directories_accept_only_absolute_native_paths_or_valid_file_uris() {
        assert_eq!(
            reported_cwd("file://remote/tmp/project%20space"),
            Some("/tmp/project space".into())
        );
        assert_eq!(
            reported_cwd("/tmp/literal%20"),
            Some("/tmp/literal%20".into())
        );
        for invalid in [
            "relative/path",
            "https://host/tmp",
            "file:///tmp/%zz",
            "file:///tmp/%00",
            "file://user@host/tmp",
            "file:///tmp?query",
            "file:///tmp#fragment",
        ] {
            assert!(reported_cwd(invalid).is_none(), "{invalid}");
        }
    }
    #[tokio::test]
    async fn osc_directory_and_title_survive_backend_reopen_without_changing_recovery_hints() {
        let dir = state();
        let target = dir.path().join("next dir");
        std::fs::create_dir(&target).unwrap();
        let mut profile = shell_profile(dir.path());
        profile.recovery = Some(RecoveryHint {
            provider: "fixture".into(),
            session_id: "explicit-id".into(),
            working_directory: dir.path().to_string_lossy().into_owned(),
            state_paths: vec![],
        });
        let host = start(dir.path(), vec![profile.clone()]).await;
        let mut view = open(&host, 1).await;
        let command = format!(
            "cd '{}'; printf '\\033]2;Updated terminal\\007'; printf '\\033]7;%s\\007' 'file://localhost{}'; printf 'OSC_%s\\n' READY\r",
            target.display(),
            target.to_string_lossy().replace(' ', "%20")
        );
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: command.into_bytes(),
            },
        )
        .await
        .unwrap();
        output_until(&mut view, "OSC_READY").await;
        host.frame(view.id, ClientFrame::List).await.unwrap();
        loop {
            if let ServerFrame::Sessions { sessions, .. } = next(&mut view).await {
                assert_eq!(sessions[0].cwd, target.to_string_lossy());
                assert_eq!(sessions[0].title, "Shell · Updated terminal");
                break;
            }
        }
        host.shutdown().await.unwrap();
        let store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
        assert_eq!(store.sessions()[0].spec.cwd, target);
        assert_eq!(
            store.sessions()[0]
                .recovery
                .as_ref()
                .unwrap()
                .working_directory,
            dir.path().to_string_lossy()
        );
        let host = Host::start(
            Config {
                state: dir.path().into(),
                label: "test".into(),
                start_profiles: vec!["shell".into()],
                default_history: HistoryPolicy::default(),
            },
            store,
            vec![profile],
        )
        .await
        .unwrap();
        let mut view = host.subscribe().await.unwrap();
        if let ServerFrame::Sessions { sessions, .. } = next(&mut view).await {
            assert_eq!(sessions[0].cwd, target.to_string_lossy());
            assert_eq!(sessions[0].title, "Shell · Updated terminal");
        } else {
            panic!("expected updated descriptors");
        }
        host.frame(view.id, ClientFrame::Open { id: 1 })
            .await
            .unwrap();
        stop(&host, &mut view, 1).await;
        host.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn backend_restart_restores_checkpoint_and_reattaches_same_process() {
        let dir = state();
        let profile = shell_profile(dir.path());
        let host = start(dir.path(), vec![profile.clone()]).await;
        let mut view = open(&host, 1).await;
        host.frame(view.id,ClientFrame::Input{id:1,seq:1,bytes:b"stty -echo; KEEP=retained; export KEEP; printf 'BEFORE_%s PID=%s DONE\\n' READY $$\r".to_vec()}).await.unwrap();
        let before = output_until(&mut view, " DONE\r\n").await;
        let pid = before
            .split("BEFORE_READY PID=")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_string();
        host.shutdown().await.unwrap();
        drop(view);
        let host = start(dir.path(), vec![profile]).await;
        let mut view = host.subscribe().await.unwrap();
        if let ServerFrame::Sessions { sessions, .. } = next(&mut view).await {
            assert_eq!(
                sessions.len(),
                1,
                "startup profiles must not duplicate a surviving session"
            );
        } else {
            panic!("expected descriptors");
        }
        host.frame(view.id, ClientFrame::Open { id: 1 })
            .await
            .unwrap();
        let snapshot = loop {
            if let ServerFrame::Snapshot { bytes, .. } = next(&mut view).await {
                break bytes;
            }
        };
        let mut vt = Terminal::from_checkpoint(&snapshot).unwrap();
        use ghostty_vt::Grid;
        vt.sync();
        let screen = (0..vt.size().rows)
            .filter_map(|row| vt.row(row))
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>()
            })
            .collect::<String>();
        assert!(screen.contains("BEFORE_READY"));
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"printf 'AFTER_%s PID=%s KEEP=%s\\n' READY $$ $KEEP\r".to_vec(),
            },
        )
        .await
        .unwrap();
        let after = output_until(&mut view, "KEEP=retained").await;
        assert!(
            after.contains(&format!("AFTER_READY PID={pid} KEEP=retained")),
            "{after}"
        );
        stop(&host, &mut view, 1).await;
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn unfinished_resize_recovery_preserves_verified_display_and_disables_input() {
        let dir = state();
        let mut profile = shell_profile(dir.path());
        profile.history = HistoryPolicy::Recent {
            max_lines: 10000,
            max_bytes: 4 << 20,
            interval_seconds: 3600,
        };
        let host = start(dir.path(), vec![profile.clone()]).await;
        let mut view = open(&host, 1).await;
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"printf 'VERIFIED_%s\\n' SCREEN\r".to_vec(),
            },
        )
        .await
        .unwrap();
        output_until(&mut view, "VERIFIED_SCREEN").await;
        host.shutdown().await.unwrap();
        let host = start(dir.path(), vec![profile.clone()]).await;
        let mut view = open(&host, 1).await;
        host.frame(
            view.id,
            ClientFrame::Resize {
                id: 1,
                cols: 71,
                rows: 19,
                cell_width: 9,
                cell_height: 18,
            },
        )
        .await
        .unwrap();
        loop {
            if matches!(next(&mut view).await, ServerFrame::Resized { .. }) {
                break;
            }
        }
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"printf 'RESIZE_%s\\n' APPLIED\r".to_vec(),
            },
        )
        .await
        .unwrap();
        output_until(&mut view, "RESIZE_APPLIED").await;
        let (reply, done) = oneshot::channel();
        host.tx.send(Command::Crash { reply }).await.unwrap();
        done.await.unwrap();
        assert!(dir.path().join("recovery/s1.resize").exists());
        let host = start(dir.path(), vec![profile]).await;
        let mut view = open(&host, 1).await;
        loop {
            if let ServerFrame::Snapshot {
                live, cols, rows, ..
            } = next(&mut view).await
            {
                assert!(!live);
                assert_eq!((cols, rows), (100, 30));
                break;
            }
        }
        loop {
            if let ServerFrame::Error { code, .. } = next(&mut view).await {
                assert_eq!(code, "resize_recovery_incomplete");
                break;
            }
        }
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"printf unsafe\r".to_vec(),
            },
        )
        .await
        .unwrap();
        loop {
            if let ServerFrame::Error { code, .. } = next(&mut view).await {
                assert_eq!(code, "not_running");
                break;
            }
        }
        host.shutdown().await.unwrap();
        // The safe fallback did not kill or replace the original holder.
        let store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
        let mut attached = store.attach_live_from(1, 0).await.unwrap();
        assert_eq!((attached.cols, attached.rows), (71, 19));
        attached.writer.stop().await.unwrap();
    }
    #[tokio::test]
    #[ignore = "requires an explicitly configured trusted SSH fixture and matching memory-only holder"]
    async fn local_and_ssh_sessions_share_host_and_survive_restart() {
        let dir = state();
        let local = shell_profile(dir.path());
        let mut remote = shell_profile(std::path::Path::new("/tmp"));
        remote.name = "ssh".into();
        remote.title = "SSH fixture".into();
        remote.endpoint = Endpoint::Ssh {
            config: std::env::var_os("TERMINAL_LAB_SSH_CONFIG")
                .expect("set trusted SSH config")
                .into(),
            destination: std::env::var("TERMINAL_LAB_SSH_HOST")
                .unwrap_or("lima-h2-terminal-c1".into()),
            holder: std::env::var_os("TERMINAL_LAB_SSH_HOLDER")
                .map(PathBuf::from)
                .unwrap_or("/tmp/h2-terminal-holder".into()),
            root: format!(
                "/var/tmp/terminal-host-{}-{}",
                std::process::id(),
                dir.path().file_name().unwrap().to_string_lossy()
            ),
        };
        let profiles = vec![local, remote];
        let host = start(dir.path(), profiles.clone()).await;
        let mut controller = host.subscribe().await.unwrap();
        let mut mirror = host.subscribe().await.unwrap();
        let _ = next(&mut controller).await;
        let _ = next(&mut mirror).await;
        let mut pids = Vec::new();
        for id in [1, 2] {
            host.frame(controller.id, ClientFrame::Open { id })
                .await
                .unwrap();
            host.frame(mirror.id, ClientFrame::Open { id })
                .await
                .unwrap();
            role(&mut controller, true).await;
            role(&mut mirror, false).await;
            host.frame(controller.id,ClientFrame::Input{id,seq:1,bytes:b"stty -echo; KEEP=retained; export KEEP; printf 'SSH_HOST_%s PID=%s DONE\\n' READY $$\r".to_vec()}).await.unwrap();
            let a = output_until(&mut controller, " DONE\r\n").await;
            let b = output_until(&mut mirror, " DONE\r\n").await;
            assert!(b.contains("SSH_HOST_READY"));
            pids.push(
                a.split("SSH_HOST_READY PID=")
                    .nth(1)
                    .unwrap()
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .to_string(),
            );
        }
        host.shutdown().await.unwrap();
        let host = start(dir.path(), profiles).await;
        let mut view = host.subscribe().await.unwrap();
        if let ServerFrame::Sessions { sessions, .. } = next(&mut view).await {
            assert_eq!(sessions.len(), 2);
        } else {
            panic!("expected session descriptors");
        }
        for (index, id) in [1, 2].into_iter().enumerate() {
            host.frame(view.id, ClientFrame::Open { id }).await.unwrap();
            host.frame(
                view.id,
                ClientFrame::Input {
                    id,
                    seq: 1,
                    bytes: b"printf 'SSH_RESUME_%s PID=%s KEEP=%s\\n' READY $$ $KEEP\r".to_vec(),
                },
            )
            .await
            .unwrap();
            let result = output_until(&mut view, "KEEP=retained").await;
            assert!(result.contains(&format!(
                "SSH_RESUME_READY PID={} KEEP=retained",
                pids[index]
            )));
            stop(&host, &mut view, id).await;
        }
        host.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn websocket_requires_authentication_and_checks_origin() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{
            connect_async,
            tungstenite::{Message, client::IntoClientRequest},
        };
        let dir = state();
        let assets = dir.path().join("assets");
        std::fs::create_dir(&assets).unwrap();
        std::fs::write(assets.join("index.html"), "terminal fixture").unwrap();
        let token_path = dir.path().join("token");
        let token = auth::Token::load_or_create(&token_path).unwrap();
        let secret = std::fs::read_to_string(token_path).unwrap();
        let store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
        let host = Host::start(
            Config {
                state: dir.path().into(),
                label: "test".into(),
                start_profiles: vec![],
                default_history: HistoryPolicy::Off,
            },
            store,
            vec![],
        )
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = gateway::router(host, assets, token);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let url = format!("ws://{address}/ws");
        let mut wrong_origin = url.clone().into_client_request().unwrap();
        wrong_origin
            .headers_mut()
            .insert("Origin", "https://untrusted.example".parse().unwrap());
        assert!(connect_async(wrong_origin).await.is_err());
        let (mut unauth, _) = connect_async(&url).await.unwrap();
        unauth
            .send(Message::Text(
                serde_json::to_string(&ClientFrame::Open { id: 1 })
                    .unwrap()
                    .into(),
            ))
            .await
            .unwrap();
        let result = unauth.next().await.unwrap().unwrap();
        assert!(
            matches!(serde_json::from_str::<ServerFrame>(result.to_text().unwrap()).unwrap(),ServerFrame::Error{code,..}if code=="authentication")
        );
        let (mut allowed, _) = connect_async(&url).await.unwrap();
        allowed
            .send(Message::Text(
                serde_json::to_string(&ClientFrame::Authenticate {
                    protocol: PROTOCOL_VERSION,
                    token: secret,
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        let result = allowed.next().await.unwrap().unwrap();
        assert!(matches!(
            serde_json::from_str::<ServerFrame>(result.to_text().unwrap()).unwrap(),
            ServerFrame::Sessions { .. }
        ));
        allowed.close(None).await.unwrap();
        server.abort();
    }
    #[tokio::test]
    async fn request_responses_do_not_evict_unrelated_viewers() {
        for request in [ClientFrame::List, ClientFrame::Open { id: 9999 }] {
            let dir = state();
            let host = start(dir.path(), vec![]).await;
            let mut victim = host.subscribe().await.unwrap();
            let _ = next(&mut victim).await;
            let mut abusive = host.subscribe().await.unwrap();
            let _ = next(&mut abusive).await;
            for _ in 0..128 {
                host.frame(abusive.id, request.clone()).await.unwrap();
            }
            let _barrier = host.subscribe().await.unwrap();
            let (mut disconnected, mut unrelated) = (false, 0);
            loop {
                match victim.receiver.try_recv() {
                    Ok(_) => unrelated += 1,
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            host.shutdown().await.unwrap();
            assert!(
                !disconnected,
                "another viewer's requests evicted an idle viewer"
            );
            assert_eq!(
                unrelated, 0,
                "request-only responses must stay with their requester"
            );
        }
    }
    #[tokio::test]
    async fn empty_input_obeys_sequence_and_process_availability() {
        let dir = state();
        let host = start(dir.path(), vec![shell_profile(dir.path())]).await;
        let mut view = open(&host, 1).await;
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 10,
                bytes: vec![],
            },
        )
        .await
        .unwrap();
        loop {
            if matches!(
                next(&mut view).await,
                ServerFrame::InputWritten { seq: 10, .. }
            ) {
                break;
            }
        }
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 9,
                bytes: b"printf 'STALE_%s\\n' INPUT\r".to_vec(),
            },
        )
        .await
        .unwrap();
        let rejected = loop {
            match next(&mut view).await {
                ServerFrame::Error { code, .. } => break code == "input_sequence",
                ServerFrame::InputWritten { seq: 9, .. } => break false,
                _ => {}
            }
        };
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 11,
                bytes: vec![0; MAX_INPUT + 1],
            },
        )
        .await
        .unwrap();
        let oversized_rejected = loop {
            match next(&mut view).await {
                ServerFrame::Error { code, .. } => break code == "input_too_large",
                ServerFrame::InputWritten { seq: 11, .. } => break false,
                _ => {}
            }
        };
        stop(&host, &mut view, 1).await;
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 11,
                bytes: vec![],
            },
        )
        .await
        .unwrap();
        let offline_rejected = loop {
            match next(&mut view).await {
                ServerFrame::Error { code, .. } => break code == "not_running",
                ServerFrame::InputWritten { seq: 12, .. } => break false,
                _ => {}
            }
        };
        host.shutdown().await.unwrap();
        assert!(
            rejected,
            "an acknowledged empty frame must advance the sequence"
        );
        assert!(
            oversized_rejected,
            "oversized terminal input reached the PTY"
        );
        assert!(
            offline_rejected,
            "written acknowledgement must require a running process"
        );
    }

    #[tokio::test]
    async fn global_history_opt_out_removes_old_checkpoint_and_never_persists_new_input() {
        let dir = state();
        let profile = shell_profile(dir.path());
        let host = start(dir.path(), vec![profile.clone()]).await;
        let mut view = open(&host, 1).await;
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"printf 'PRIVATE_%s\\n' BEFORE_OPT_OUT\r".to_vec(),
            },
        )
        .await
        .unwrap();
        output_until(&mut view, "PRIVATE_BEFORE_OPT_OUT").await;
        host.shutdown().await.unwrap();
        let checkpoint = dir.path().join("recovery/s1.checkpoint");
        let had_checkpoint = checkpoint.exists();
        let store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
        let host = Host::start(
            Config {
                state: dir.path().into(),
                label: "private".into(),
                start_profiles: vec!["shell".into()],
                default_history: HistoryPolicy::Off,
            },
            store,
            vec![profile],
        )
        .await
        .unwrap();
        let mut view = open(&host, 1).await;
        let removed = !checkpoint.exists();
        host.frame(
            view.id,
            ClientFrame::SetHistory {
                id: 1,
                policy: HistoryPolicy::default(),
            },
        )
        .await
        .unwrap();
        let forbidden = loop {
            if let ServerFrame::Error { code, .. } = next(&mut view).await {
                break code == "history_disabled";
            }
        };
        host.frame(
            view.id,
            ClientFrame::SetRecovery {
                id: 1,
                hint: Some(RecoveryHint {
                    provider: "fixture".into(),
                    session_id: "explicit".into(),
                    working_directory: dir.path().to_string_lossy().into_owned(),
                    state_paths: vec![],
                }),
            },
        )
        .await
        .unwrap();
        loop {
            if matches!(next(&mut view).await, ServerFrame::Sessions { .. }) {
                break;
            }
        }
        host.frame(
            view.id,
            ClientFrame::SetRecovery {
                id: 1,
                hint: Some(RecoveryHint {
                    provider: "bad\0provider".into(),
                    session_id: "invalid".into(),
                    working_directory: "/tmp".into(),
                    state_paths: vec![],
                }),
            },
        )
        .await
        .unwrap();
        let rejected = loop {
            if let ServerFrame::Error { code, .. } = next(&mut view).await {
                break code == "invalid_recovery";
            }
        };
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"printf 'PRIVATE_%s\\n' AFTER_OPT_OUT\r".to_vec(),
            },
        )
        .await
        .unwrap();
        output_until(&mut view, "PRIVATE_AFTER_OPT_OUT").await;
        stop(&host, &mut view, 1).await;
        host.shutdown().await.unwrap();
        let workspace = std::fs::read_to_string(dir.path().join("workspace.json")).unwrap();
        let settings = std::fs::read_to_string(dir.path().join("recovery/sessions.json")).unwrap();
        assert!(had_checkpoint && removed && !checkpoint.exists());
        assert!(forbidden && rejected);
        for text in [&workspace, &settings] {
            assert!(!text.contains("PRIVATE_"));
            assert!(!text.contains("bad"));
            assert!(text.contains("explicit"));
        }
    }

    #[tokio::test]
    async fn disconnected_viewer_input_and_stale_acks_cannot_affect_the_new_controller() {
        let dir = state();
        let host = start(dir.path(), vec![shell_profile(dir.path())]).await;
        let mut original = open(&host, 1).await;
        let mut mirror = open(&host, 1).await;
        host.frame(
            original.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"KEEP=0; printf 'INITIAL_%s\\n' READY\r".to_vec(),
            },
        )
        .await
        .unwrap();
        output_until(&mut original, "INITIAL_READY").await;
        output_until(&mut mirror, "INITIAL_READY").await;
        host.leave(original.id).await;
        host.frame(
            original.id,
            ClientFrame::Input {
                id: 1,
                seq: 2,
                bytes: b"KEEP=1\r".to_vec(),
            },
        )
        .await
        .unwrap();
        host.frame(original.id, ClientFrame::TakeControl { id: 1 })
            .await
            .unwrap();
        host.frame(mirror.id, ClientFrame::TakeControl { id: 1 })
            .await
            .unwrap();
        loop {
            if matches!(
                next(&mut mirror).await,
                ServerFrame::Control {
                    controller: true,
                    ..
                }
            ) {
                break;
            }
        }
        host.frame(
            mirror.id,
            ClientFrame::Ack {
                id: 1,
                offset: u64::MAX,
            },
        )
        .await
        .unwrap();
        host.frame(mirror.id, ClientFrame::Ack { id: 1, offset: 0 })
            .await
            .unwrap();
        host.frame(
            mirror.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"printf 'CONTROL_CHECK_%s KEEP=%s DONE\\n' READY $KEEP\r".to_vec(),
            },
        )
        .await
        .unwrap();
        let result = output_until(&mut mirror, " DONE\r\n").await;
        stop(&host, &mut mirror, 1).await;
        host.shutdown().await.unwrap();
        assert!(
            result.contains("CONTROL_CHECK_READY KEEP=0"),
            "the disconnected viewer changed the surviving shell: {result}"
        );
    }
    #[tokio::test]
    async fn hydration_sends_history_only_as_snapshot_then_streams_live_bytes_and_settles_disconnect()
     {
        let dir = state();
        let mut profile = shell_profile(dir.path());
        profile.program = "/bin/cat".into();
        profile.args.clear();
        let host = start(dir.path(), vec![profile]).await;
        let history = b"\x1b]52;c;T0xEX0NMSVBCT0FSRA==\x07OLD_SCREEN\r\n";
        let boundary = history.len() as u64;
        let (reply, ready) = oneshot::channel();
        host.tx
            .send(Command::ReplayFixture {
                session: 1,
                end: boundary,
                reply,
            })
            .await
            .unwrap();
        ready.await.unwrap();
        let mut view = open(&host, 1).await;
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"MUST_NOT_REACH_CAT\r".to_vec(),
            },
        )
        .await
        .unwrap();
        let blocked = loop {
            if let ServerFrame::Error { code, .. } = next(&mut view).await {
                break code == "not_ready";
            }
        };
        let cut = history.len() / 2;
        host.tx
            .send(Command::Output(
                1,
                Update::Record(Record::Output {
                    offset: 0,
                    bytes: history[..cut].to_vec(),
                }),
            ))
            .await
            .unwrap();
        let _barrier = host.subscribe().await.unwrap();
        let no_partial = view.receiver.try_recv().is_err();
        host.tx
            .send(Command::Output(
                1,
                Update::Record(Record::Output {
                    offset: cut as u64,
                    bytes: history[cut..].to_vec(),
                }),
            ))
            .await
            .unwrap();
        let (mut history_streamed, mut snapshot) = (false, None);
        while snapshot.is_none() {
            match next(&mut view).await {
                ServerFrame::Output { .. } => history_streamed = true,
                ServerFrame::Snapshot { bytes, .. } => snapshot = Some(bytes),
                _ => {}
            }
        }
        let mut replica = Terminal::from_checkpoint(&snapshot.unwrap()).unwrap();
        let no_historical_effects = replica.drain_events().is_empty();
        let live = b"\x1b]52;c;TElWRV9DTElQQk9BUkQ=\x07LIVE_SCREEN\r\n";
        host.tx
            .send(Command::Output(
                1,
                Update::Record(Record::Output {
                    offset: boundary,
                    bytes: live.to_vec(),
                }),
            ))
            .await
            .unwrap();
        let streamed = loop {
            if let ServerFrame::Output { bytes, .. } = next(&mut view).await {
                break bytes;
            }
        };
        replica.feed(&streamed);
        let live_clipboard = replica
            .drain_events()
            .iter()
            .any(|event| matches!(event,VtEvent::ClipboardWrite{text,..}if text=="LIVE_CLIPBOARD"));
        let (reply, ready) = oneshot::channel();
        host.tx
            .send(Command::ReplayFixture {
                session: 1,
                end: boundary + live.len() as u64 + 100,
                reply,
            })
            .await
            .unwrap();
        ready.await.unwrap();
        host.frame(view.id, ClientFrame::SnapshotRequest { id: 1 })
            .await
            .unwrap();
        host.tx.send(Command::Disconnected(1)).await.unwrap();
        let offline = loop {
            if let ServerFrame::Snapshot { live, .. } = next(&mut view).await {
                break !live;
            }
        };
        host.shutdown().await.unwrap();
        let store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
        let mut attached = store.attach_live_from(1, 0).await.unwrap();
        attached.writer.stop().await.unwrap();
        assert!(
            blocked
                && no_partial
                && !history_streamed
                && no_historical_effects
                && live_clipboard
                && offline
        );
    }
    #[tokio::test]
    async fn clean_shutdown_settles_an_already_queued_reply_acknowledgement_before_detaching() {
        let dir = state();
        let mut profile = shell_profile(dir.path());
        profile.program = "/bin/cat".into();
        profile.args.clear();
        let host = start(dir.path(), vec![profile.clone()]).await;
        let (reply, ready) = oneshot::channel();
        host.tx
            .send(Command::PendingReplyFixture { session: 1, reply })
            .await
            .unwrap();
        ready.await.unwrap();
        let (reply, done) = oneshot::channel();
        host.tx.send(Command::Shutdown { reply }).await.unwrap();
        let ack_queued = host
            .tx
            .send(Command::Output(1, Update::Written(1)))
            .await
            .is_ok();
        let result = done.await.unwrap();
        let cursor = Recovery::open(dir.path().join("recovery"))
            .unwrap()
            .reply_cursor(1)
            .unwrap();
        let host = start(dir.path(), vec![profile]).await;
        let mut view = open(&host, 1).await;
        let writable = loop {
            if let ServerFrame::Snapshot { live, .. } = next(&mut view).await {
                break live;
            }
        };
        host.shutdown().await.unwrap();
        let store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
        let mut attached = store.attach_live_from(1, 0).await.unwrap();
        attached.writer.stop().await.unwrap();
        loop {
            match attached.reader.next().await.unwrap() {
                Some(Update::Exited(_)) | None => break,
                _ => {}
            }
        }
        assert!(
            ack_queued && result.is_ok() && !cursor.pending && writable,
            "clean shutdown discarded a reply receipt that was available to settle"
        );
    }

    #[tokio::test]
    async fn detached_view_reopens_current_state_without_replaying_input_or_stopping_other_viewers()
    {
        let dir = state();
        let host = start(dir.path(), vec![shell_profile(dir.path())]).await;
        let mut first = open(&host, 1).await;
        let mut mirror = open(&host, 1).await;
        host.frame(
            first.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"KEEP=0; printf 'DETACH_%s\\n' READY\r".to_vec(),
            },
        )
        .await
        .unwrap();
        let (mut acknowledged, mut text) = (false, String::new());
        while !acknowledged || !text.contains("DETACH_READY") {
            match next(&mut first).await {
                ServerFrame::InputWritten { seq: 1, .. } => acknowledged = true,
                ServerFrame::Output { bytes, .. } => {
                    text.push_str(&String::from_utf8_lossy(&bytes))
                }
                _ => {}
            }
        }
        output_until(&mut mirror, "DETACH_READY").await;
        host.frame(first.id, ClientFrame::Detach { id: 1 })
            .await
            .unwrap();
        host.frame(
            first.id,
            ClientFrame::Input {
                id: 1,
                seq: 2,
                bytes: b"KEEP=9\r".to_vec(),
            },
        )
        .await
        .unwrap();
        let stale_detached_rejected = loop {
            if let ServerFrame::Error { code, .. } = next(&mut first).await {
                break code == "read_only";
            }
        };
        host.frame(mirror.id, ClientFrame::TakeControl { id: 1 })
            .await
            .unwrap();
        loop {
            if matches!(
                next(&mut mirror).await,
                ServerFrame::Control {
                    controller: true,
                    ..
                }
            ) {
                break;
            }
        }
        host.frame(
            mirror.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"KEEP=$((KEEP+1)); printf 'MISSED_%s KEEP=%s DONE\\n' OUTPUT $KEEP\r"
                    .to_vec(),
            },
        )
        .await
        .unwrap();
        let missed = output_until(&mut mirror, " DONE\r\n").await;
        let _barrier = host.subscribe().await.unwrap();
        let no_detached_output = matches!(
            first.receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        );
        host.frame(first.id, ClientFrame::Open { id: 1 })
            .await
            .unwrap();
        let snapshot = loop {
            if let ServerFrame::Snapshot { bytes, .. } = next(&mut first).await {
                break bytes;
            }
        };
        let mut restored = Terminal::from_checkpoint(&snapshot).unwrap();
        use ghostty_vt::Grid;
        restored.sync();
        let screen = (0..restored.size().rows)
            .filter_map(|row| restored.row(row))
            .flat_map(|row| row.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>();
        host.frame(first.id, ClientFrame::TakeControl { id: 1 })
            .await
            .unwrap();
        loop {
            if matches!(
                next(&mut first).await,
                ServerFrame::Control {
                    controller: true,
                    ..
                }
            ) {
                break;
            }
        }
        host.frame(
            first.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"KEEP=99\r".to_vec(),
            },
        )
        .await
        .unwrap();
        let repeated_rejected = loop {
            if let ServerFrame::Error { code, .. } = next(&mut first).await {
                break code == "input_sequence";
            }
        };
        host.frame(
            first.id,
            ClientFrame::Input {
                id: 1,
                seq: 2,
                bytes: b"printf 'WAKE_%s KEEP=%s DONE\\n' READY $KEEP\r".to_vec(),
            },
        )
        .await
        .unwrap();
        let wake = output_until(&mut first, " DONE\r\n").await;
        stop(&host, &mut first, 1).await;
        host.shutdown().await.unwrap();
        assert!(acknowledged && stale_detached_rejected && no_detached_output && repeated_rejected);
        assert!(missed.contains("MISSED_OUTPUT KEEP=1"));
        assert!(screen.contains("MISSED_OUTPUT KEEP=1"));
        assert!(wake.contains("WAKE_READY KEEP=1"));
    }
}
