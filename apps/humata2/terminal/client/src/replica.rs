//! One embeddable terminal replica. Networking, fonts, themes and app chrome belong to its host.
use ghostty_vt::{Key, KeyEvent, Terminal};
use gpui_kit as gpui;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{
    App, Context, Entity, EventEmitter, Focusable, KeyBinding, Render, ScrollHandle, Subscription,
    Window, div, prelude::*, px,
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    rc::Rc,
    time::Duration,
};
use terminal_pane::{PaneError, PaneEvent, Pty, TerminalPane};
use terminal_provider::ProviderEvent;
use terminal_view::{CellSize, Theme};
pub use terminal_wire::{ClientFrame, InjectionMode, InjectionOptions, InputTicket, ServerFrame};
use terminal_wire::{MAX_INPUT_BYTES, MAX_RAW_CHECKPOINT_BYTES, SNAPSHOT_ENGINE, SessionInfo};

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("The connection is closed. Reconnect before typing; unsent input is not replayed.")]
    Closed,
    #[error(
        "The connection's input queue is full. Wait for it to drain, then retry the unsent input."
    )]
    Full,
}

pub const REPLICA_CHECKPOINT_VERSION: u16 = 1;
gpui_kit::actions!(
    terminal_replica,
    [NewTerminal, CommandPalette, NextTerminal, PreviousTerminal]
);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShortcutPolicy {
    pub new_terminal: bool,
    pub command_palette: bool,
    pub next_terminal: bool,
    pub previous_terminal: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostAction {
    NewTerminal,
    CommandPalette,
    NextTerminal,
    PreviousTerminal,
}
/// Opt-in application bindings. The app handles actions bubbling past a focused
/// component and chooses its own last-focused target and tab order.
/// Browsers can reserve these physical accelerators before DOM delivery.
pub fn register_host_shortcuts(policy: ShortcutPolicy, cx: &mut App) {
    let mac = cx.platform_info().is_mac();
    if policy.new_terminal {
        cx.bind_keys([KeyBinding::new(
            if mac { "cmd-t" } else { "ctrl-t" },
            NewTerminal,
            None,
        )]);
    }
    if policy.command_palette {
        cx.bind_keys([KeyBinding::new(
            if mac { "cmd-k" } else { "ctrl-k" },
            CommandPalette,
            None,
        )]);
    }
    if policy.next_terminal {
        cx.bind_keys([KeyBinding::new("ctrl-tab", NextTerminal, None)]);
    }
    if policy.previous_terminal {
        cx.bind_keys([KeyBinding::new("ctrl-shift-tab", PreviousTerminal, None)]);
    }
}
/// Sending accepts an entire ordered frame, or rejects it without accepting any bytes.
/// The adapter owns framing/authentication and reports flush completion to the entity.
pub trait ReplicaSink {
    fn send(&self, frame: ClientFrame) -> Result<(), SendError>;
}
#[derive(Clone)]
pub struct ReplicaConfig {
    pub session: SessionInfo,
    pub theme: Theme,
    /// None keeps inactive views resident. No process is ever paused.
    pub memory_saver: Option<Duration>,
    pub shortcuts: ShortcutPolicy,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaContinuation {
    pub connection_epoch: u64,
    pub seq: u64,
    pub written_seq: u64,
}
/// Raw opaque bytes; callers choose storage/compression independently.
/// No input payload, draft, clipboard contents or lease authority is exported.
#[derive(Clone, Debug)]
pub struct ReplicaCheckpoint {
    pub version: u16,
    pub terminal_uuid: String,
    pub engine: String,
    pub bytes: Vec<u8>,
    pub offset: u64,
    pub cols: u16,
    pub rows: u16,
    pub continuation: ReplicaContinuation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicaPhase {
    Disconnected,
    Restoring,
    Live,
    ReadOnly,
    Unavailable,
    Detaching,
    Parked,
    Exited,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaState {
    pub terminal_uuid: String,
    pub phase: ReplicaPhase,
    pub visible: bool,
    pub resident: bool,
    pub controller: bool,
    pub live: bool,
    pub pending_snapshot: bool,
    pub offset: u64,
    pub input_seq: u64,
    pub written_seq: u64,
}
#[derive(Debug, thiserror::Error)]
pub enum ReplicaError {
    #[error(
        "Terminal {expected} cannot accept a frame for route {actual}. Route the update to its own terminal."
    )]
    WrongRoute { expected: u64, actual: u64 },
    #[error(
        "This update belongs to connection {actual}, not {expected}. Discard stale connection events."
    )]
    WrongConnection { expected: u64, actual: u64 },
    #[error(
        "This connection has no matching terminal UUID binding. Confirm trusted session metadata before applying terminal updates."
    )]
    UnboundConnection,
    #[error(
        "The checkpoint belongs to a different terminal. Preserve it and restore the matching UUID."
    )]
    WrongIdentity,
    #[error(
        "Checkpoint input counters are inconsistent ({written} written after sequence {seq}). Preserve the checkpoint and restore compatible metadata."
    )]
    ContinuationOrder { seq: u64, written: u64 },
    #[error(
        "Checkpoint metadata version {actual} is unsupported (expected {expected}). Preserve it and use a compatible viewer."
    )]
    Version { actual: u16, expected: u16 },
    #[error(
        "The checkpoint uses another terminal engine. Preserve it and restore it with the matching engine."
    )]
    Engine,
    #[error(
        "The checkpoint dimensions disagree with its metadata. Preserve it and request a fresh host checkpoint."
    )]
    Dimensions,
    #[error(
        "The checkpoint is {actual} bytes, above the {maximum}-byte limit. Reduce retained history and preserve the previous checkpoint."
    )]
    SnapshotTooLarge { actual: usize, maximum: usize },
    #[error(
        "The complete encoded input is {actual} bytes, above the {maximum}-byte limit. Send a smaller input; nothing was queued."
    )]
    InputTooLarge { actual: usize, maximum: usize },
    #[error(
        "Terminal {terminal_uuid} is waiting for a clipboard read or paste confirmation. Finish or cancel that paste before injecting text; nothing was queued."
    )]
    PendingPaste { terminal_uuid: String },
    #[error("The input is empty. Supply text or explicitly request submit; nothing was queued.")]
    EmptyInput,
    #[error(
        "Terminal input is unavailable while {phase:?}. Restore fresh host state and explicitly acquire control before sending input."
    )]
    NotReady { phase: ReplicaPhase },
    #[error("The idle timeout exceeds one day. Choose a shorter timeout or disable memory saver.")]
    IdleTimeout,
    #[error("Could not restore terminal state: {source}")]
    Snapshot {
        #[source]
        source: ghostty_vt::Error,
    },
    #[error("Could not encode terminal input: {source}")]
    Input {
        #[source]
        source: ghostty_vt::Error,
    },
    #[error(
        "The terminal input sequence is exhausted. Reconnect with a new connection epoch; nothing was queued."
    )]
    SequenceExhausted,
    #[error("Could not queue the terminal operation: {source}")]
    Send {
        #[source]
        source: SendError,
    },
    #[error("The terminal operation failed: {source}")]
    Pane {
        #[source]
        source: PaneError,
    },
    #[error(
        "Cached state cannot replace a terminal while an open or detach is in flight. Wait for its ordered host acknowledgement and retry."
    )]
    LifecycleBusy,
    #[error("The host refused a terminal operation ({code}): {message}")]
    Host { code: String, message: String },
    #[error(
        "This frame is not a single-terminal update. Handle authentication and session lists in the application."
    )]
    UnsupportedFrame,
}
pub enum ReplicaEvent {
    StateChanged(ReplicaState),
    Focused {
        terminal_uuid: String,
    },
    ShortcutRequested {
        terminal_uuid: String,
        action: HostAction,
    },
    Pane(PaneEvent),
    Provider(ProviderEvent),
    InputQueued(InputTicket),
    InputWritten(InputTicket),
    /// Accepted input whose written outcome was not confirmed. It is never replayed.
    InputUncertain(InputTicket),
    Failed(ReplicaError),
}
struct InputState {
    route: u64,
    seq: Cell<u64>,
    enabled: Cell<bool>,
    sink: Rc<dyn ReplicaSink>,
    errors: RefCell<Vec<ReplicaError>>,
    pending: RefCell<BTreeSet<u64>>,
}
impl InputState {
    fn write(&self, bytes: Vec<u8>) -> Result<u64, ReplicaError> {
        if bytes.is_empty() {
            return Err(ReplicaError::EmptyInput);
        }
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(ReplicaError::InputTooLarge {
                actual: bytes.len(),
                maximum: MAX_INPUT_BYTES,
            });
        }
        let seq = self
            .seq
            .get()
            .checked_add(1)
            .ok_or(ReplicaError::SequenceExhausted)?;
        self.sink
            .send(ClientFrame::Input {
                id: self.route,
                seq,
                bytes,
            })
            .map_err(|source| ReplicaError::Send { source })?;
        self.seq.set(seq);
        self.pending.borrow_mut().insert(seq);
        Ok(seq)
    }
}
struct ReplicaPty(Rc<InputState>);
impl Pty for ReplicaPty {
    fn write(&mut self, bytes: &[u8]) {
        let _ = self.write_accepted(bytes);
    }
    fn write_accepted(&mut self, bytes: &[u8]) -> bool {
        if !self.0.enabled.get() || bytes.is_empty() {
            return false;
        }
        match self.0.write(bytes.to_vec()) {
            Ok(_) => true,
            Err(error) => {
                self.0.errors.borrow_mut().push(error);
                false
            }
        }
    }
    fn reply(&mut self, _: &[u8]) {}
    fn defer_grid_resize(&self) -> bool {
        true
    }
    fn resize(&mut self, size: ghostty_vt::Size, cell: CellSize) {
        if self.0.enabled.get() {
            if let Err(source) = self.0.sink.send(ClientFrame::Resize {
                id: self.0.route,
                cols: size.cols,
                rows: size.rows,
                cell_width: cell.width.round().clamp(1., u16::MAX as f32) as u16,
                cell_height: cell.height.round().clamp(1., u16::MAX as f32) as u16,
            }) {
                self.0
                    .errors
                    .borrow_mut()
                    .push(ReplicaError::Send { source });
            }
        }
    }
}
/// Single terminal entity. It creates no socket, timer pump, font registration or global theme.
pub struct TerminalReplica {
    config: ReplicaConfig,
    last_state: Option<ReplicaState>,
    input: Rc<InputState>,
    pane: Option<Entity<TerminalPane>>,
    subscriptions: Vec<Subscription>,
    epoch: u64,
    written_seq: u64,
    offset: u64,
    connected: bool,
    bound: bool,
    fresh: bool,
    controller: bool,
    live: bool,
    exited: Option<i32>,
    visible: bool,
    opening: bool,
    needs_snapshot: bool,
    pending_ack: Option<u64>,
    detaching: bool,
    detached: bool,
    pending_focus: bool,
    idle_ready: bool,
    idle_generation: u64,
    idle_timer: Option<gpui_kit::Task<()>>,
    tickets: BTreeSet<u64>,
    scroll: ScrollHandle,
}
impl EventEmitter<ReplicaEvent> for TerminalReplica {}
impl TerminalReplica {
    pub fn new(
        config: ReplicaConfig,
        sink: Rc<dyn ReplicaSink>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Result<Self, ReplicaError> {
        if config
            .memory_saver
            .is_some_and(|duration| duration > Duration::from_secs(86400))
        {
            return Err(ReplicaError::IdleTimeout);
        }
        Ok(Self::build(config, sink))
    }
    fn build(config: ReplicaConfig, sink: Rc<dyn ReplicaSink>) -> Self {
        Self {
            input: Rc::new(InputState {
                route: config.session.id,
                seq: Cell::new(0),
                enabled: Cell::new(false),
                sink,
                errors: RefCell::new(Vec::new()),
                pending: RefCell::new(BTreeSet::new()),
            }),
            config,
            last_state: None,
            pane: None,
            subscriptions: Vec::new(),
            epoch: 0,
            written_seq: 0,
            offset: 0,
            connected: false,
            bound: false,
            fresh: false,
            controller: false,
            live: false,
            exited: None,
            visible: true,
            opening: false,
            needs_snapshot: false,
            pending_ack: None,
            detaching: false,
            detached: false,
            pending_focus: false,
            idle_ready: false,
            idle_generation: 0,
            idle_timer: None,
            tickets: BTreeSet::new(),
            scroll: ScrollHandle::new(),
        }
    }
    /// Fallible entity construction for embedders that cannot use a panicking closure.
    pub fn create(
        config: ReplicaConfig,
        sink: Rc<dyn ReplicaSink>,
        _: &mut Window,
        cx: &mut App,
    ) -> Result<Entity<Self>, ReplicaError> {
        if config
            .memory_saver
            .is_some_and(|duration| duration > Duration::from_secs(86400))
        {
            return Err(ReplicaError::IdleTimeout);
        }
        Ok(cx.new(|_| Self::build(config, sink)))
    }
    pub fn state(&self) -> ReplicaState {
        let phase = if self.detaching {
            ReplicaPhase::Detaching
        } else if self.detached && !self.opening {
            ReplicaPhase::Parked
        } else if !self.connected {
            ReplicaPhase::Disconnected
        } else if self.opening || !self.fresh {
            ReplicaPhase::Restoring
        } else if self.exited.is_some() {
            ReplicaPhase::Exited
        } else if !self.live {
            ReplicaPhase::Unavailable
        } else if self.controller {
            ReplicaPhase::Live
        } else {
            ReplicaPhase::ReadOnly
        };
        ReplicaState {
            terminal_uuid: self.config.session.uuid.clone(),
            phase,
            visible: self.visible,
            resident: self.pane.is_some(),
            controller: self.controller,
            live: self.live,
            pending_snapshot: !self.fresh,
            offset: self.offset,
            input_seq: self.input.seq.get(),
            written_seq: self.written_seq,
        }
    }
    pub fn pane(&self) -> Option<&Entity<TerminalPane>> {
        self.pane.as_ref()
    }
    pub fn session(&self) -> &SessionInfo {
        &self.config.session
    }
    pub fn update_session(
        &mut self,
        session: SessionInfo,
        cx: &mut Context<Self>,
    ) -> Result<(), ReplicaError> {
        if session.id != self.config.session.id {
            return Err(ReplicaError::WrongRoute {
                expected: self.config.session.id,
                actual: session.id,
            });
        }
        if session.uuid != self.config.session.uuid {
            self.bound = false;
            self.fresh = false;
            self.controller = false;
            self.changed(cx);
            return Err(ReplicaError::WrongIdentity);
        }
        self.config.session = session;
        self.bound = self.connected;
        self.changed(cx);
        if self.bound && self.visible && !self.fresh && !self.detaching {
            self.open()?;
        }
        self.changed(cx);
        Ok(())
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        let authorized = self.connected
            && self.bound
            && self.fresh
            && self.controller
            && self.live
            && self.exited.is_none()
            && !self.detaching;
        self.input.enabled.set(self.visible && authorized);
        if let Some(pane) = &self.pane {
            pane.update(cx, |pane, cx| pane.set_connected(authorized, cx));
        }
        let state = self.state();
        let lifecycle_changed = self.last_state.as_ref().is_none_or(|old| {
            old.phase != state.phase
                || old.visible != state.visible
                || old.resident != state.resident
                || old.controller != state.controller
                || old.live != state.live
                || old.pending_snapshot != state.pending_snapshot
                || old.input_seq != state.input_seq
                || old.written_seq != state.written_seq
        });
        self.last_state = Some(state.clone());
        if lifecycle_changed {
            cx.emit(ReplicaEvent::StateChanged(state));
            cx.notify();
        }
    }
    fn send(&self, frame: ClientFrame) -> Result<(), ReplicaError> {
        self.input
            .sink
            .send(frame)
            .map_err(|source| ReplicaError::Send { source })
    }
    fn open(&mut self) -> Result<(), ReplicaError> {
        if !self.opening && !self.detaching && self.connected && self.bound {
            self.send(ClientFrame::Open {
                id: self.config.session.id,
            })?;
            self.opening = true;
            self.fresh = false;
            self.detached = false;
        }
        Ok(())
    }
    fn request_snapshot(&mut self, cx: &mut Context<Self>) -> Result<(), ReplicaError> {
        self.fresh = false;
        self.needs_snapshot = true;
        self.changed(cx);
        if !self.opening {
            self.send(ClientFrame::SnapshotRequest {
                id: self.config.session.id,
            })?;
            self.opening = true;
            self.needs_snapshot = false;
        }
        Ok(())
    }
    fn acknowledge(&mut self, cx: &mut Context<Self>) -> Result<(), ReplicaError> {
        self.pending_ack = Some(self.offset);
        self.changed(cx);
        self.send(ClientFrame::Ack {
            id: self.config.session.id,
            offset: self.offset,
        })?;
        self.pending_ack = None;
        Ok(())
    }
    /// Call only after authentication. Epoch changes discard pending tickets, never replay input.
    pub fn set_connected(
        &mut self,
        connected: bool,
        epoch: u64,
        cx: &mut Context<Self>,
    ) -> Result<(), ReplicaError> {
        if epoch != self.epoch || !connected {
            for seq in std::mem::take(&mut self.tickets) {
                cx.emit(ReplicaEvent::InputUncertain(InputTicket {
                    terminal_uuid: self.config.session.uuid.clone(),
                    connection_epoch: self.epoch,
                    seq,
                }));
            }
        }
        if epoch != self.epoch {
            self.epoch = epoch;
            self.input.seq.set(0);
            self.written_seq = 0;
            self.bound = false;
            self.fresh = false;
            self.controller = false;
            self.opening = false;
            self.needs_snapshot = false;
            self.pending_ack = None;
            self.detaching = false;
            self.detached = false;
            self.input.pending.borrow_mut().clear();
        }
        let newly_connected = connected && !self.connected;
        self.connected = connected;
        if !connected {
            self.bound = false;
            self.controller = false;
            self.fresh = false;
            self.opening = false;
            self.detaching = false;
            self.tickets.clear();
            self.input.pending.borrow_mut().clear();
        }
        self.changed(cx);
        if newly_connected && self.visible && self.bound {
            self.open()?;
        }
        self.changed(cx);
        Ok(())
    }
    /// Bind a connection to metadata authenticated by the application.
    pub fn bind_connection(
        &mut self,
        epoch: u64,
        session: SessionInfo,
        cx: &mut Context<Self>,
    ) -> Result<(), ReplicaError> {
        self.set_connected(true, epoch, cx)?;
        self.update_session(session, cx)
    }
    pub fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fresh && !self.detaching {
            if let Some(pane) = &self.pane {
                pane.update(cx, |pane, cx| pane.focus_input(window, cx));
                self.pending_focus = false;
                return;
            }
        }
        self.pending_focus = true;
        window.blur(cx);
        window.defer(cx, |window, cx| window.draw(cx).clear(cx));
    }
    /// Explicit visibility, independent of keyboard focus.
    pub fn set_visible(
        &mut self,
        visible: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), ReplicaError> {
        if visible == self.visible {
            return Ok(());
        }
        self.visible = visible;
        self.idle_generation = self.idle_generation.wrapping_add(1);
        self.idle_timer.take();
        self.idle_ready = false;
        if visible {
            self.changed(cx);
            if !self.fresh {
                self.open()?;
            }
        } else if let Some(delay) = self.config.memory_saver {
            let generation = self.idle_generation;
            self.idle_timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(delay).await;
                let _ = this.update(cx, |this, cx| {
                    if this.idle_generation == generation && !this.visible {
                        this.idle_ready = true;
                        this.try_park(cx);
                    }
                });
            }));
        }
        self.changed(cx);
        Ok(())
    }
    pub fn set_memory_saver(
        &mut self,
        idle: Option<Duration>,
        cx: &mut Context<Self>,
    ) -> Result<(), ReplicaError> {
        if idle.is_some_and(|duration| duration > Duration::from_secs(86400)) {
            return Err(ReplicaError::IdleTimeout);
        }
        self.config.memory_saver = idle;
        let visible = self.visible;
        self.visible = !visible;
        self.set_visible(visible, cx)
    }
    pub fn outbound_flushed(&mut self, cx: &mut Context<Self>) {
        if self.needs_snapshot {
            if let Err(error) = self.request_snapshot(cx) {
                cx.emit(ReplicaEvent::Failed(error));
            }
        } else if self.pending_ack.is_some() {
            if let Err(error) = self.acknowledge(cx) {
                cx.emit(ReplicaEvent::Failed(error));
            }
        }
        self.try_park(cx);
    }
    fn pinned(&self, cx: &App) -> bool {
        self.pane.as_ref().is_some_and(|pane| {
            let pane = pane.read(cx);
            let view = pane.view().read(cx);
            let grid = view.grid();
            pane.is_find_open()
                || pane.has_pending_paste()
                || view.preedit().is_some()
                || view.has_pending_clipboard()
                || grid.has_selection().unwrap_or(true)
                || grid.scrollbar().is_some_and(|bar| !bar.at_bottom())
                || self.scroll.offset() != gpui_kit::point(px(0.), px(0.))
        })
    }
    fn try_park(&mut self, cx: &mut Context<Self>) {
        if self.visible
            || !self.idle_ready
            || !self.connected
            || !self.fresh
            || self.detaching
            || self.detached
            || !self.input.pending.borrow().is_empty()
            || self.pinned(cx)
        {
            return;
        }
        // ACK then Detach are ordered. Keep the VT until Detached proves both arrived,
        // even if this inactive TUI continues emitting between those operations.
        let result = self
            .send(ClientFrame::Ack {
                id: self.config.session.id,
                offset: self.offset,
            })
            .and_then(|_| {
                self.send(ClientFrame::Detach {
                    id: self.config.session.id,
                })
            });
        match result {
            Ok(()) => {
                self.detaching = true;
                self.changed(cx);
            }
            Err(error) => cx.emit(ReplicaEvent::Failed(error)),
        }
    }
    pub fn checkpoint(
        &self,
        max_bytes: usize,
        cx: &App,
    ) -> Result<ReplicaCheckpoint, ReplicaError> {
        let pane = self.pane.as_ref().ok_or_else(|| ReplicaError::NotReady {
            phase: self.state().phase,
        })?;
        let terminal = pane.read(cx).view().read(cx).grid();
        let size = terminal.size();
        let bytes = terminal
            .checkpoint(max_bytes.min(MAX_RAW_CHECKPOINT_BYTES))
            .map_err(|source| ReplicaError::Snapshot { source })?;
        Ok(ReplicaCheckpoint {
            version: REPLICA_CHECKPOINT_VERSION,
            terminal_uuid: self.config.session.uuid.clone(),
            engine: SNAPSHOT_ENGINE.into(),
            bytes,
            offset: self.offset,
            cols: size.cols,
            rows: size.rows,
            continuation: ReplicaContinuation {
                connection_epoch: self.epoch,
                seq: self.input.seq.get(),
                written_seq: self.written_seq,
            },
        })
    }
    fn decode(&self, checkpoint: &ReplicaCheckpoint) -> Result<Terminal, ReplicaError> {
        if checkpoint.version != REPLICA_CHECKPOINT_VERSION {
            return Err(ReplicaError::Version {
                actual: checkpoint.version,
                expected: REPLICA_CHECKPOINT_VERSION,
            });
        }
        if checkpoint.terminal_uuid != self.config.session.uuid {
            return Err(ReplicaError::WrongIdentity);
        }
        if checkpoint.engine != SNAPSHOT_ENGINE {
            return Err(ReplicaError::Engine);
        }
        if checkpoint.continuation.written_seq > checkpoint.continuation.seq {
            return Err(ReplicaError::ContinuationOrder {
                seq: checkpoint.continuation.seq,
                written: checkpoint.continuation.written_seq,
            });
        }
        if checkpoint.bytes.len() > MAX_RAW_CHECKPOINT_BYTES {
            return Err(ReplicaError::SnapshotTooLarge {
                actual: checkpoint.bytes.len(),
                maximum: MAX_RAW_CHECKPOINT_BYTES,
            });
        }
        let terminal = Terminal::from_checkpoint(&checkpoint.bytes)
            .map_err(|source| ReplicaError::Snapshot { source })?;
        if terminal.size()
            != (ghostty_vt::Size {
                cols: checkpoint.cols,
                rows: checkpoint.rows,
            })
        {
            return Err(ReplicaError::Dimensions);
        }
        Ok(terminal)
    }
    fn install(&mut self, terminal: Terminal, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = &self.pane {
            pane.update(cx, |pane, cx| pane.replace_terminal(terminal, cx));
        } else {
            let input = self.input.clone();
            let theme = self.config.theme.clone();
            let pane =
                cx.new(|cx| TerminalPane::new(terminal, ReplicaPty(input), theme, window, cx));
            pane.update(cx, |pane, cx| {
                pane.set_offline_banner(false, cx);
                pane.use_authoritative_size(cx);
            });
            self.subscriptions
                .push(cx.subscribe(&pane, |_, _, event: &PaneEvent, cx| {
                    cx.emit(ReplicaEvent::Pane(event.clone()))
                }));
            let view = pane.read(cx).view().clone();
            self.subscriptions.push(cx.observe(&view, |this, _, cx| {
                let errors = std::mem::take(&mut *this.input.errors.borrow_mut());
                for error in errors {
                    cx.emit(ReplicaEvent::Failed(error));
                }
                this.try_park(cx);
            }));
            let focus = pane.read(cx).focus_handle(cx);
            self.subscriptions
                .push(cx.on_focus(&focus, window, |this, _, cx| {
                    cx.emit(ReplicaEvent::Focused {
                        terminal_uuid: this.config.session.uuid.clone(),
                    })
                }));
            self.pane = Some(pane);
        }
        cx.notify();
    }
    /// Cached restore is always read-only. A fresh host snapshot and lease are required for input.
    pub fn restore_checkpoint(
        &mut self,
        checkpoint: &ReplicaCheckpoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), ReplicaError> {
        if self.opening || self.detaching {
            return Err(ReplicaError::LifecycleBusy);
        }
        let terminal = self.decode(checkpoint)?;
        for seq in std::mem::take(&mut self.tickets) {
            cx.emit(ReplicaEvent::InputUncertain(InputTicket {
                terminal_uuid: self.config.session.uuid.clone(),
                connection_epoch: self.epoch,
                seq,
            }));
        }
        self.input.pending.borrow_mut().clear();
        self.install(terminal, window, cx);
        self.offset = checkpoint.offset;
        if self.epoch == 0 || self.epoch == checkpoint.continuation.connection_epoch {
            self.epoch = checkpoint.continuation.connection_epoch;
            self.input
                .seq
                .set(self.input.seq.get().max(checkpoint.continuation.seq));
            self.written_seq = self.written_seq.max(
                checkpoint
                    .continuation
                    .written_seq
                    .min(self.input.seq.get()),
            );
        }
        self.bound = false;
        self.fresh = false;
        self.controller = false;
        self.live = false;
        self.detached = false;
        self.needs_snapshot = false;
        self.pending_ack = None;
        self.changed(cx);
        Ok(())
    }
    pub fn inject_text(
        &mut self,
        text: &str,
        options: InjectionOptions,
        cx: &mut Context<Self>,
    ) -> Result<InputTicket, ReplicaError> {
        if self.state().phase != ReplicaPhase::Live || !self.bound {
            return Err(ReplicaError::NotReady {
                phase: self.state().phase,
            });
        }
        if text.len() > MAX_INPUT_BYTES {
            return Err(ReplicaError::InputTooLarge {
                actual: text.len(),
                maximum: MAX_INPUT_BYTES,
            });
        }
        let pane = self.pane.as_ref().ok_or_else(|| ReplicaError::NotReady {
            phase: self.state().phase,
        })?;
        if pane.read(cx).has_pending_paste()
            || pane.read(cx).view().read(cx).has_pending_clipboard()
        {
            return Err(ReplicaError::PendingPaste {
                terminal_uuid: self.config.session.uuid.clone(),
            });
        }
        let mut bytes = match options.mode {
            InjectionMode::Raw => text.as_bytes().to_vec(),
            InjectionMode::Paste => pane
                .read(cx)
                .view()
                .read(cx)
                .grid()
                .paste(text, false)
                .map_err(|source| ReplicaError::Input { source })?,
        };
        if options.submit {
            let enter = pane
                .update(cx, |pane, cx| {
                    pane.view().clone().update(cx, |view, _| {
                        view.grid_mut().encode_key(&KeyEvent::press(Key::Enter))
                    })
                })
                .map_err(|source| ReplicaError::Input { source })?;
            bytes.extend(enter);
        }
        let seq = self.input.write(bytes)?;
        let ticket = InputTicket {
            terminal_uuid: self.config.session.uuid.clone(),
            connection_epoch: self.epoch,
            seq,
        };
        self.tickets.insert(seq);
        cx.emit(ReplicaEvent::InputQueued(ticket.clone()));
        Ok(ticket)
    }
    pub fn provider_event(&mut self, event: ProviderEvent, cx: &mut Context<Self>) {
        cx.emit(ReplicaEvent::Provider(event));
    }
    fn shortcut(&mut self, action: HostAction, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .pane
            .as_ref()
            .is_some_and(|pane| pane.read(cx).focus_handle(cx).is_focused(window))
        {
            cx.emit(ReplicaEvent::ShortcutRequested {
                terminal_uuid: self.config.session.uuid.clone(),
                action,
            });
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }
    pub fn apply(
        &mut self,
        frame: ServerFrame,
        epoch: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), ReplicaError> {
        if epoch != self.epoch {
            return Err(ReplicaError::WrongConnection {
                expected: self.epoch,
                actual: epoch,
            });
        }
        if !self.bound {
            return Err(ReplicaError::UnboundConnection);
        }
        let route = match &frame {
            ServerFrame::Snapshot { id, .. }
            | ServerFrame::Output { id, .. }
            | ServerFrame::Resized { id, .. }
            | ServerFrame::Control { id, .. }
            | ServerFrame::Detached { id }
            | ServerFrame::InputWritten { id, .. }
            | ServerFrame::Exited { id, .. }
            | ServerFrame::ResyncRequired { id, .. } => *id,
            ServerFrame::Error { id: Some(id), .. } => *id,
            _ => return Err(ReplicaError::UnsupportedFrame),
        };
        if route != self.config.session.id {
            return Err(ReplicaError::WrongRoute {
                expected: self.config.session.id,
                actual: route,
            });
        }
        match frame {
            ServerFrame::Snapshot {
                engine,
                bytes,
                offset,
                cols,
                rows,
                live,
                exited,
                ..
            } => {
                if self.detaching || self.detached && !self.opening {
                    return Ok(());
                }
                let checkpoint = ReplicaCheckpoint {
                    version: REPLICA_CHECKPOINT_VERSION,
                    terminal_uuid: self.config.session.uuid.clone(),
                    engine,
                    bytes,
                    offset,
                    cols,
                    rows,
                    continuation: ReplicaContinuation {
                        connection_epoch: self.epoch,
                        seq: self.input.seq.get(),
                        written_seq: self.written_seq,
                    },
                };
                let terminal = self.decode(&checkpoint)?;
                self.install(terminal, window, cx);
                self.offset = offset;
                self.live = live;
                self.exited = exited;
                self.fresh = true;
                self.opening = false;
                self.needs_snapshot = false;
                self.detached = false;
                self.changed(cx);
                if self.pending_focus && self.visible && window.focused(cx).is_none() {
                    self.focus_input(window, cx);
                }
                self.acknowledge(cx)?;
            }
            ServerFrame::Output { offset, bytes, .. } => {
                if self.detaching || !self.fresh || self.detached {
                    return Ok(());
                }
                if offset > self.offset {
                    self.request_snapshot(cx)?;
                } else {
                    let skip = self.offset.saturating_sub(offset).min(bytes.len() as u64) as usize;
                    if skip < bytes.len() {
                        if let Some(pane) = &self.pane {
                            pane.update(cx, |pane, cx| pane.feed(&bytes[skip..], cx));
                        }
                        self.offset = offset.saturating_add(bytes.len() as u64);
                    }
                    self.acknowledge(cx)?;
                }
            }
            ServerFrame::Resized {
                cols,
                rows,
                cell_width,
                cell_height,
                ..
            } => {
                if self.detaching || !self.fresh {
                    return Ok(());
                }
                if let Some(pane) = &self.pane {
                    let result = pane.update(cx, |pane, cx| {
                        pane.view().clone().update(cx, |view, cx| {
                            let result = view.grid_mut().resize_with_cell_size(
                                cols,
                                rows,
                                u32::from(cell_width),
                                u32::from(cell_height),
                            );
                            let _ = view.grid_mut().drain_events();
                            cx.notify();
                            result
                        })
                    });
                    if let Err(source) = result {
                        self.request_snapshot(cx)?;
                        return Err(ReplicaError::Snapshot { source });
                    }
                }
                cx.notify();
            }
            ServerFrame::InputWritten { seq, .. } => {
                self.input.pending.borrow_mut().remove(&seq);
                self.written_seq = self.written_seq.max(seq.min(self.input.seq.get()));
                if self.tickets.remove(&seq) {
                    cx.emit(ReplicaEvent::InputWritten(InputTicket {
                        terminal_uuid: self.config.session.uuid.clone(),
                        connection_epoch: self.epoch,
                        seq,
                    }));
                }
            }
            ServerFrame::Control { controller, .. } => {
                self.controller = controller;
                if controller && self.fresh {
                    if let Some(pane) = &self.pane {
                        pane.update(cx, |pane, cx| {
                            pane.view()
                                .clone()
                                .update(cx, |view, cx| view.request_fit(cx))
                        });
                    }
                }
            }
            ServerFrame::Detached { .. } => {
                if self.detaching {
                    self.detaching = false;
                    self.detached = true;
                    self.controller = false;
                    self.fresh = false;
                    self.subscriptions.clear();
                    self.pane.take();
                    if self.visible {
                        self.open()?;
                    }
                }
            }
            ServerFrame::Exited { code, .. } => {
                self.live = false;
                self.exited = Some(code.unwrap_or(-1));
            }
            ServerFrame::ResyncRequired { .. } => {
                if !self.detaching && !self.detached && !self.opening {
                    self.request_snapshot(cx)?;
                }
            }
            ServerFrame::Error { code, message, .. } => {
                // This protocol error has no input sequence. Pending outcomes are
                // uncertain, never reported written or automatically replayed.
                if code.starts_with("input_") || code == "read_only" {
                    self.input.pending.borrow_mut().clear();
                    for seq in std::mem::take(&mut self.tickets) {
                        cx.emit(ReplicaEvent::InputUncertain(InputTicket {
                            terminal_uuid: self.config.session.uuid.clone(),
                            connection_epoch: self.epoch,
                            seq,
                        }));
                    }
                }
                if code == "read_only" {
                    self.controller = false;
                }
                self.changed(cx);
                cx.emit(ReplicaEvent::Failed(ReplicaError::Host { code, message }));
            }
            _ => {}
        }
        self.changed(cx);
        self.try_park(cx);
        Ok(())
    }
    /// Local read-only viewport pan; it never changes the authoritative grid size.
    pub fn viewport_scroll(&self) -> &ScrollHandle {
        &self.scroll
    }
}
impl Render for TerminalReplica {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.pane.as_ref().filter(|_| self.visible).map(|pane| {
            if self.controller && self.fresh {
                return pane.clone().into_any_element();
            }
            let view = pane.read(cx).view().read(cx);
            let cell = view.cell_size().unwrap_or(CellSize {
                width: 8.,
                height: 19.,
            });
            let width = f32::from(view.grid().size().cols) * cell.width + 1.;
            let height = f32::from(view.grid().size().rows) * cell.height + 1.;
            let wheel = self.scroll.clone();
            div()
                .id("terminal-replica-pan")
                .test_support()
                .size_full()
                .min_w_0()
                .min_h_0()
                .max_w_full()
                .relative()
                .flex()
                .overflow_scroll()
                .track_scroll(&self.scroll)
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(width))
                        .h(px(height))
                        .child(pane.clone()),
                )
                .child(
                    gpui_kit::canvas(
                        |_, _, _| (),
                        move |_, (), window, _| {
                            let visible = wheel.bounds().intersect(&window.content_mask().bounds);
                            let scroll = wheel.clone();
                            window.on_mouse_event(
                                move |event: &gpui_kit::ScrollEvent, phase, window, cx| {
                                    if phase != gpui_kit::DispatchPhase::Capture
                                        || !visible.contains(&event.position)
                                    {
                                        return;
                                    }
                                    let delta = event.delta.pixel_delta(px(cell.height));
                                    let current = scroll.offset();
                                    let maximum = scroll.max_offset();
                                    let next = (f32::from(current.y) + f32::from(delta.y))
                                        .clamp(-f32::from(maximum.y).max(0.), 0.);
                                    if next != f32::from(current.y) {
                                        scroll.set_offset(gpui_kit::point(current.x, px(next)));
                                        window.refresh();
                                        cx.stop_propagation();
                                    }
                                },
                            );
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .horizontal_scrollbar(&self.scroll)
                .vertical_scrollbar(&self.scroll)
                .into_any_element()
        });
        div()
            .id("terminal-replica")
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .when(self.config.shortcuts.new_terminal, |d| {
                d.on_action(cx.listener(|this, _: &NewTerminal, w, cx| {
                    this.shortcut(HostAction::NewTerminal, w, cx)
                }))
            })
            .when(self.config.shortcuts.command_palette, |d| {
                d.on_action(cx.listener(|this, _: &CommandPalette, w, cx| {
                    this.shortcut(HostAction::CommandPalette, w, cx)
                }))
            })
            .when(self.config.shortcuts.next_terminal, |d| {
                d.on_action(cx.listener(|this, _: &NextTerminal, w, cx| {
                    this.shortcut(HostAction::NextTerminal, w, cx)
                }))
            })
            .when(self.config.shortcuts.previous_terminal, |d| {
                d.on_action(cx.listener(|this, _: &PreviousTerminal, w, cx| {
                    this.shortcut(HostAction::PreviousTerminal, w, cx)
                }))
            })
            .children(content)
    }
}
