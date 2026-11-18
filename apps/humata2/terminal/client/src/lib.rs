//! The same GPUI terminal viewer for native and browser transports.
//! No PTY, process launcher, Unix socket, SSH or filesystem dependency.
use ghostty_vt::Grid;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AssetSource, Bounds, ClipboardItem, Context, Entity, Focusable, Pixels,
    Render, SharedString, Subscription, Window, div, prelude::*, px, rgb,
};
use replica::{
    FitPolicy, ReplicaConfig, ReplicaEvent, ReplicaPhase, ReplicaSink, ShortcutPolicy,
    TerminalReplica,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    rc::Rc,
    time::Duration,
};
use terminal_pane::{PaneEvent, TerminalPane};
pub use terminal_replica as replica;
pub use terminal_replica::SendError;
use terminal_view::Theme;
use terminal_wire::{
    ClientFrame, HostInfo, PROTOCOL_VERSION, SNAPSHOT_ENGINE, ServerFrame, SessionInfo,
};

/// Wire events carry text unchanged; JS never parses offsets or snapshot bytes.
pub enum TransportEvent {
    Opened,
    Text(String),
    Closed {
        reason: String,
    },
    Failed {
        operation: &'static str,
        reason: String,
    },
}
/// The platform adapter owns only networking, never terminal rendering or state.
pub trait Transport {
    fn drain(&mut self) -> Vec<TransportEvent>;
    fn send(&mut self, text: String) -> Result<(), SendError>;
    fn reconnect(&mut self);
    /// True only after all accepted outbound frames have reached the socket.
    /// Unknown adapters conservatively keep replicas resident.
    fn is_idle(&self) -> bool {
        false
    }
}
#[derive(Debug, thiserror::Error)]
enum ViewerError {
    #[error(
        "The terminal host sent an invalid message. Reconnect; if this repeats, update the viewer and host together."
    )]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("Could not encode terminal input. Reconnect and report this error if it repeats.")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
#[derive(Default)]
struct PendingFrames {
    frames: Vec<ClientFrame>,
    overflow: bool,
}
impl PendingFrames {
    fn push(&mut self, frame: ClientFrame) -> bool {
        if let ClientFrame::Resize { id, .. } = &frame {
            for previous in self.frames.iter_mut().rev() {
                let other = match previous {
                    ClientFrame::Ack { .. } => continue,
                    ClientFrame::Resize { id: other, .. } if *other == *id => {
                        *previous = frame;
                        return true;
                    }
                    ClientFrame::Resize { id, .. }
                    | ClientFrame::Input { id, .. }
                    | ClientFrame::Open { id }
                    | ClientFrame::Detach { id }
                    | ClientFrame::TakeControl { id }
                    | ClientFrame::Stop { id }
                    | ClientFrame::SnapshotRequest { id }
                    | ClientFrame::SetRecovery { id, .. }
                    | ClientFrame::SetHistory { id, .. } => *id,
                    _ => break,
                };
                if other == *id {
                    break;
                }
            }
        }
        if let ClientFrame::Ack { id, offset } = &frame {
            if let Some(ClientFrame::Ack {
                offset: previous, ..
            }) = self
                .frames
                .iter_mut()
                .rev()
                .find(|f| matches!(f,ClientFrame::Ack{id:other,..} if other==id))
            {
                *previous = (*previous).max(*offset);
                return true;
            }
        }
        if self.frames.len() >= 256 {
            self.overflow = true;
            return false;
        }
        self.frames.push(frame);
        true
    }
}
type Outbox = Rc<RefCell<PendingFrames>>;
struct WorkspaceSink(Outbox);
impl ReplicaSink for WorkspaceSink {
    fn send(&self, frame: ClientFrame) -> Result<(), SendError> {
        if self.0.borrow_mut().push(frame) {
            Ok(())
        } else {
            Err(SendError::Full)
        }
    }
}

pub struct Options {
    pub token: String,
    pub title: String,
    pub diagnostics: Option<Rc<RefCell<String>>>,
}
/// Shared viewer chrome and actual `TerminalPane` entities on both platforms.
pub struct RemoteWorkspace {
    transport: Box<dyn Transport>,
    token: String,
    title: String,
    host: Option<HostInfo>,
    sessions: Vec<SessionInfo>,
    replicas: BTreeMap<u64, Entity<TerminalReplica>>,
    replica_events: BTreeMap<u64, Subscription>,
    panes: BTreeMap<u64, gpui_kit::WeakEntity<TerminalPane>>,
    epoch: u64,
    memory_saver: bool,
    memory_idle: Duration,

    control_bounds: Rc<RefCell<BTreeMap<u64, Bounds<Pixels>>>>,
    tab_bounds: Rc<RefCell<BTreeMap<(usize, u64), Bounds<Pixels>>>>,
    selected: [Option<u64>; 2],
    pending_focus: Option<u64>,
    focused: usize,
    outbox: Outbox,
    connected: bool,
    notice: Option<String>,
    first_failure: Option<String>,
    window: AnyWindowHandle,
    diagnostics: Option<Rc<RefCell<String>>>,
    diagnostics_tick: u8,
    focus_snapshot: serde_json::Value,
    focus_trace: VecDeque<serde_json::Value>,
    focus_sequence: u64,
}
gpui_kit::assets::icon_assets!(Icons, [WifiOff, Terminal]);
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        Icons.load(path)
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        Icons.list(path)
    }
}
pub fn resources() -> humata2_engine::Resources {
    humata2_engine::Resources::new(Assets)
        .font(include_bytes!("../assets/Inter-Regular.ttf"))
        .font(include_bytes!("../assets/JetBrainsMono-Regular.ttf"))
}
fn replica_clip(replica: &TerminalReplica) -> Option<serde_json::Value> {
    if replica.state().controller {
        return None;
    }
    let scroll = replica.viewport_scroll();
    let bounds = scroll.bounds();
    Some(
        serde_json::json!({"x":f32::from(bounds.origin.x),"y":f32::from(bounds.origin.y),"width":f32::from(bounds.size.width),"height":f32::from(bounds.size.height),"offset_x":f32::from(scroll.offset().x),"max_offset_x":f32::from(scroll.max_offset().x),"offset_y":f32::from(scroll.offset().y),"max_offset_y":f32::from(scroll.max_offset().y)}),
    )
}
impl RemoteWorkspace {
    pub fn new(
        transport: impl Transport + 'static,
        options: Options,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        gpui_kit::component::theme::Theme::change(
            gpui_kit::component::theme::ThemeMode::Dark,
            None,
            cx,
        );
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if weak.update(cx, |this, cx| this.poll(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            transport: Box::new(transport),
            token: options.token,
            title: options.title,
            host: None,
            sessions: Vec::new(),
            replicas: BTreeMap::new(),
            replica_events: BTreeMap::new(),
            panes: BTreeMap::new(),
            epoch: 0,
            memory_saver: false,
            memory_idle: Duration::from_secs(30),

            control_bounds: Rc::new(RefCell::new(BTreeMap::new())),
            tab_bounds: Rc::new(RefCell::new(BTreeMap::new())),
            selected: [None, None],
            pending_focus: None,
            focused: 0,
            outbox: Rc::new(RefCell::new(PendingFrames::default())),
            connected: false,
            notice: Some("Connecting to terminal host…".into()),
            first_failure: None,
            window: window.window_handle(),
            diagnostics: options.diagnostics,
            diagnostics_tick: 0,
            focus_snapshot: serde_json::Value::Null,
            focus_trace: VecDeque::new(),
            focus_sequence: 0,
        }
    }
    fn trace_focus(&mut self, id: u64, event: &'static str) {
        self.focus_sequence = self.focus_sequence.saturating_add(1);
        if self.focus_trace.len() == 64 {
            self.focus_trace.pop_front();
        }
        self.focus_trace
            .push_back(serde_json::json!({"sequence":self.focus_sequence,"id":id,"event":event}));
    }
    fn queue(&self, frame: ClientFrame) {
        self.outbox.borrow_mut().push(frame);
    }
    /// The demo owns tab policy; each reusable entity owns its idle lifecycle.
    pub fn set_memory_saver(&mut self, enabled: bool, idle: Duration, cx: &mut Context<Self>) {
        self.memory_saver = enabled;
        self.memory_idle = idle;
        for replica in self.replicas.values() {
            let result =
                replica.update(cx, |r, cx| r.set_memory_saver(enabled.then_some(idle), cx));
            if let Err(error) = result {
                self.notice = Some(error.to_string());
            }
        }
        cx.notify();
    }
    fn synchronize_visibility(&mut self, cx: &mut Context<Self>) {
        for (id, replica) in &self.replicas {
            let result = replica.update(cx, |r, cx| {
                r.set_visible(self.selected.contains(&Some(*id)), cx)
            });
            if let Err(error) = result {
                self.notice = Some(error.to_string());
            }
        }
    }
    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        self.disable(cx);
        self.transport.reconnect();
        self.notice = Some("Reconnecting to the existing terminal host…".into());
        cx.notify();
    }
    fn disable(&mut self, cx: &mut Context<Self>) {
        self.connected = false;
        self.outbox.borrow_mut().frames.clear();
        for replica in self.replicas.values() {
            let _ = replica.update(cx, |r, cx| r.set_connected(false, self.epoch, cx));
        }
    }
    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        if self.first_failure.is_none() {
            self.first_failure = Some(message.clone());
        }
        self.notice = Some(message);
        cx.notify();
    }
    pub fn poll(&mut self, cx: &mut Context<Self>) {
        for event in self.transport.drain() {
            match event {
                TransportEvent::Opened => {
                    self.connected = true;
                    self.first_failure = None;
                    self.epoch = self.epoch.wrapping_add(1).max(1);
                    for replica in self.replicas.values() {
                        let _ = replica.update(cx, |r, cx| r.set_connected(false, self.epoch, cx));
                    }
                    self.notice = Some("Authenticating…".into());
                    self.queue(ClientFrame::Authenticate {
                        protocol: PROTOCOL_VERSION,
                        token: self.token.clone(),
                    });
                }
                TransportEvent::Text(text) => match serde_json::from_str::<ServerFrame>(&text) {
                    Ok(frame) => self.receive(frame, cx),
                    Err(source) => {
                        self.disable(cx);
                        self.fail(ViewerError::Decode { source }.to_string(), cx);
                    }
                },
                TransportEvent::Closed { reason } => {
                    self.disable(cx);
                    self.fail(format!("Disconnected: {reason}. Reconnect to resume existing processes. Unsent input is not replayed."),cx);
                }
                TransportEvent::Failed { operation, reason } => {
                    self.disable(cx);
                    self.fail(format!("Could not {operation}: {reason}. Check the host address and connection, then reconnect."),cx);
                }
            }
        }
        for replica in self.replicas.values() {
            replica.update(cx, |r, cx| r.outbound_flushed(cx));
        }
        let pending = std::mem::take(&mut *self.outbox.borrow_mut());
        if pending.overflow {
            self.fail("Some input was not sent because the viewer queue was full. Wait for it to drain, then retry the unsent input.".into(),cx);
        }
        let frames = pending.frames;
        for frame in frames {
            if !self.connected {
                break;
            }
            match serde_json::to_string(&frame).map_err(|source| ViewerError::Encode { source }) {
                Ok(text) => {
                    if let Err(error) = self.transport.send(text) {
                        self.disable(cx);
                        self.fail(error.to_string(), cx);
                        break;
                    }
                }
                Err(error) => {
                    self.fail(error.to_string(), cx);
                    break;
                }
            }
        }
        let replicas: Vec<_> = self
            .replicas
            .iter()
            .map(|(id, r)| (*id, r.clone()))
            .collect();
        if let Ok(snapshot)=cx.update_window(self.window,|_,window,cx|{
            let terminals:Vec<_>=replicas.iter().map(|(id,r)|{
                let pane=r.read(cx).pane();
                let focused=pane.is_some_and(|p|p.read(cx).focus_handle(cx).is_focused(window));
                serde_json::json!({"id":id,"focused":focused})
            }).collect();
            serde_json::json!({"window_active":window.is_window_active(),"has_focused_element":window.focused(cx).is_some(),"terminals":terminals})
        }) {self.focus_snapshot=snapshot;}
        self.diagnostics_tick = self.diagnostics_tick.wrapping_add(1);
        if self.diagnostics_tick % 6 == 0 {
            if let Some(diagnostics) = &self.diagnostics {
                *diagnostics.borrow_mut() = self.inspect(cx);
            }
        }
    }
    pub fn receive(&mut self, frame: ServerFrame, cx: &mut Context<Self>) {
        if let ServerFrame::Sessions { host, sessions } = frame {
            if host.protocol_version != PROTOCOL_VERSION || host.engine != SNAPSHOT_ENGINE {
                self.disable(cx);
                self.fail("The host and viewer use different protocols/engines. Update them together, then reconnect.".into(),cx);
                return;
            }
            self.host = Some(host);
            self.sessions = sessions;
            self.notice = None;
            let initial = self.selected[0].is_none();
            for group in 0..2 {
                if self.selected[group].is_some_and(|id| !self.sessions.iter().any(|s| s.id == id))
                {
                    self.selected[group] = None;
                }
                if self.selected[group].is_none() {
                    self.selected[group] = self
                        .sessions
                        .iter()
                        .find(|s| !self.selected.contains(&Some(s.id)))
                        .map(|s| s.id);
                }
            }
            if initial {
                self.pending_focus = self.selected[0];
            }
            self.replicas
                .retain(|id, _| self.sessions.iter().any(|s| s.id == *id));
            self.replica_events
                .retain(|id, _| self.replicas.contains_key(id));
            self.panes.retain(|id, _| self.replicas.contains_key(id));
            for session in self.sessions.clone() {
                let id = session.id;
                let existing = self
                    .replicas
                    .get(&id)
                    .filter(|r| r.read(cx).session().uuid == session.uuid)
                    .cloned();
                let replica = if let Some(replica) = existing {
                    replica
                } else {
                    self.replica_events.remove(&id);
                    self.replicas.remove(&id);
                    self.panes.remove(&id);
                    let config = ReplicaConfig {
                        session: session.clone(),
                        theme: Theme {
                            font_family: "JetBrains Mono".into(),
                            font_size: 13.,
                            line_height: 19.,
                            ..Theme::default()
                        },
                        memory_saver: self.memory_saver.then_some(self.memory_idle),
                        shortcuts: ShortcutPolicy::default(),
                        fit: FitPolicy::default(),
                        default_colors: None,
                    };
                    let sink = Rc::new(WorkspaceSink(self.outbox.clone()));
                    let result = cx.update_window(self.window, |_, window, cx| {
                        TerminalReplica::create(config, sink, window, cx)
                    });
                    let replica = match result {
                        Ok(Ok(replica)) => replica,
                        Ok(Err(error)) => {
                            self.fail(error.to_string(), cx);
                            continue;
                        }
                        Err(error) => {
                            self.fail(format!("Could not create terminal view: {error}"), cx);
                            continue;
                        }
                    };
                    let events=cx.subscribe(&replica,move|this,entity,event:&ReplicaEvent,cx|match event{
                        ReplicaEvent::Focused{..}=>this.trace_focus(id,"terminal-focused"),
                        ReplicaEvent::StateChanged(_)=>{if let Some(pane)=entity.read(cx).pane(){this.panes.insert(id,pane.downgrade());}else{this.panes.remove(&id);}cx.notify();},
                        ReplicaEvent::Pane(PaneEvent::ClipboardWrite{text,..})=>cx.write_to_clipboard(ClipboardItem::new_string(text.clone())),
                        ReplicaEvent::Pane(PaneEvent::OpenLink(uri))=>{let scheme=uri.split(':').next().unwrap_or_default();if ["http","https","mailto"].iter().any(|s|scheme.eq_ignore_ascii_case(s))&&!uri.chars().any(char::is_control){cx.open_url(uri);}},
                        ReplicaEvent::Pane(PaneEvent::Failed(error))=>this.fail(error.to_string(),cx),
                        ReplicaEvent::Pane(PaneEvent::PasteFiles(_)|PaneEvent::PasteImage{..})=>this.fail("This viewer pastes text. File/image transfer requires a host upload integration.".into(),cx),
                        ReplicaEvent::Failed(error)=>this.fail(error.to_string(),cx),
                        _=>{}
                    });
                    self.replica_events.insert(id, events);
                    self.replicas.insert(id, replica.clone());
                    replica
                };
                let visible = self.selected.contains(&Some(id));
                let result = replica.update(cx, |r, cx| {
                    r.set_visible(visible, cx)?;
                    r.bind_connection(self.epoch, session, cx)
                });
                if let Err(error) = result {
                    self.fail(error.to_string(), cx);
                }
            }
            cx.notify();
            return;
        }
        let id = match &frame {
            ServerFrame::Snapshot { id, .. }
            | ServerFrame::Output { id, .. }
            | ServerFrame::Resized { id, .. }
            | ServerFrame::Control { id, .. }
            | ServerFrame::Detached { id }
            | ServerFrame::InputWritten { id, .. }
            | ServerFrame::Exited { id, .. }
            | ServerFrame::ResyncRequired { id, .. } => *id,
            ServerFrame::Error { id: Some(id), .. } => *id,
            ServerFrame::Error {
                id: None,
                code,
                message,
            } => {
                self.fail(format!("Terminal host: {message} ({code})"), cx);
                return;
            }
            _ => return,
        };
        let Some(replica) = self.replicas.get(&id).cloned() else {
            return;
        };
        let snapshot = matches!(&frame, ServerFrame::Snapshot { .. });
        if snapshot {
            self.trace_focus(id, "snapshot-before");
        }
        let explicit = self.pending_focus == Some(id) && self.selected.contains(&Some(id));
        let result = cx.update_window(self.window, |_, window, cx| {
            replica.update(cx, |r, cx| {
                r.apply(frame, self.epoch, window, cx)?;
                if snapshot && explicit && window.focused(cx).is_none() {
                    r.focus_input(window, cx);
                }
                Ok::<(), replica::ReplicaError>(())
            })
        });
        match result {
            Ok(Ok(())) => {
                if snapshot && explicit {
                    self.pending_focus = None;
                }
            }
            Ok(Err(error)) => self.fail(error.to_string(), cx),
            Err(error) => self.fail(format!("Could not update terminal view: {error}"), cx),
        }
        if snapshot {
            self.trace_focus(id, "snapshot-after");
        }
        cx.notify();
    }
    pub fn pane(&self, id: u64) -> Option<Entity<TerminalPane>> {
        self.panes.get(&id).and_then(|pane| pane.upgrade())
    }
    pub fn inspect(&self, cx: &App) -> String {
        let terminals:Vec<_>=self.replicas.iter().filter_map(|(id,entity)|{
            let r=entity.read(cx);let state=r.state();let pane=r.pane()?;
            let view=pane.read(cx).view().read(cx);let g=view.grid();let text=(0..g.size().rows).filter_map(|y|g.row(y)).map(|row|row.cells.iter().map(|c|c.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n");
            let bounds=view.viewport_bounds();let cell=view.cell_size().unwrap_or_default();
            let input=view.input_stats();let (shortcut_events,find_shortcuts)=pane.read(cx).shortcut_stats();
            let input_stats=serde_json::json!({"focus_gains":input.focus_gains,"focus_losses":input.focus_losses,"mouse_downs":input.mouse_downs,"key_downs":input.key_downs,"unfocused_key_downs":input.unfocused_key_downs,"key_ups":input.key_ups,"unfocused_key_ups":input.unfocused_key_ups,"text_commits":input.text_commits,"shortcut_events":shortcut_events,"find_shortcuts":find_shortcuts});
            let colors:std::collections::BTreeSet<_>=(0..g.size().rows).filter_map(|y|g.row(y)).flat_map(|row|row.cells.iter()).filter(|cell|!cell.text.trim().is_empty()).map(|cell|{let color=g.colors().resolve(cell.fg).unwrap_or(g.colors().foreground);format!("#{:02x}{:02x}{:02x}",color.r,color.g,color.b)}).collect();
            let clip=replica_clip(r);
            let control=self.control_bounds.borrow().get(id).copied().map(|b|serde_json::json!({"x":f32::from(b.origin.x),"y":f32::from(b.origin.y),"width":f32::from(b.size.width),"height":f32::from(b.size.height)}));
            Some(serde_json::json!({"id":id,"visible":self.selected.contains(&Some(*id)),"history_rows":g.scrollback_rows(),"offset":state.offset,"cols":g.size().cols,"rows":g.size().rows,"enabled":state.phase==ReplicaPhase::Live,"controller":state.controller,"live":state.live,"find_open":pane.read(cx).find_status().is_some(),"input_stats":input_stats,"take_control":control,"mirror_clip":clip,"pending_snapshot":state.pending_snapshot,"text":text,"cursor":{"x":g.cursor().x,"y":g.cursor().y},"viewport":{"x":f32::from(bounds.origin.x),"y":f32::from(bounds.origin.y),"width":f32::from(bounds.size.width),"height":f32::from(bounds.size.height)},"cell":{"width":cell.width,"height":cell.height},"foreground_colors":colors,"alternate_screen":g.is_alternate_screen(),"paint_failures":view.stats().paint_failures,"rows_rebuilt":view.stats().rows_rebuilt}))
        }).collect();
        let tabs:Vec<_>=self.tab_bounds.borrow().iter().map(|((group,id),b)|serde_json::json!({"group":group,"id":id,"x":f32::from(b.origin.x),"y":f32::from(b.origin.y),"width":f32::from(b.size.width),"height":f32::from(b.size.height)})).collect();
        let parked:Vec<_>=self.replicas.iter().filter_map(|(id,entity)|{let state=entity.read(cx).state();matches!(state.phase,ReplicaPhase::Parked|ReplicaPhase::Detaching).then(||serde_json::json!({"id":id,"offset":state.offset,"seq":state.input_seq,"written_seq":state.written_seq,"waking":state.visible,"detached":state.phase==ReplicaPhase::Parked}))}).collect();
        let resident = self
            .replicas
            .values()
            .filter(|r| r.read(cx).state().resident)
            .count();
        serde_json::json!({"connected":self.connected,"host":self.host,"sessions":self.sessions,"selected":self.selected,"tabs":tabs,"terminals":terminals,"memory_saver":{"enabled":self.memory_saver,"idle_ms":self.memory_idle.as_millis().min(u128::from(u64::MAX)) as u64,"resident_count":resident,"parked_count":parked.len(),"parked":parked},"notice":self.notice,"first_failure":self.first_failure,"client_os":format!("{:?}",cx.platform_info().os),"input_focus":self.focus_snapshot,"focus_trace":self.focus_trace,"renderer":"gpui-terminal-pane-ghostty"}).to_string()
    }
    fn select(&mut self, id: u64, group: usize, window: &mut Window, cx: &mut Context<Self>) {
        let other = 1 - group;
        if self.selected[other] == Some(id) {
            self.selected[other] = self.selected[group];
        }
        self.selected[group] = Some(id);
        self.focused = group;
        self.pending_focus = None;
        self.synchronize_visibility(cx);
        if let Some(replica) = self.replicas.get(&id) {
            let has_pane = replica.read(cx).pane().is_some();
            replica.update(cx, |r, cx| r.focus_input(window, cx));
            if !has_pane {
                self.pending_focus = Some(id);
                window.blur(cx);
                window.defer(cx, |window, cx| window.draw(cx).clear(cx));
            }
        }
        cx.notify();
    }
}
impl Render for RemoteWorkspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let heading = self
            .host
            .as_ref()
            .map(|h| format!("{} · {} / {}", h.label, h.os, h.arch))
            .unwrap_or_else(|| self.title.clone());
        let mut columns = div()
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_px()
            .bg(rgb(0x3a3a3d));
        for group in 0..2 {
            if group == 1 && self.selected[1].is_none() {
                continue;
            }
            let mut tabs = div()
                .flex()
                .flex_wrap()
                .flex_shrink_0()
                .min_h(px(36.))
                .bg(rgb(0x252527))
                .border_b_1()
                .border_color(rgb(0x3a3a3d));
            for session in &self.sessions {
                let id = session.id;
                let tab_bounds = self.tab_bounds.clone();
                tabs = tabs.child(
                    div()
                        .on_children_prepainted(move |children, _, _| {
                            if let Some(bounds) = children.first() {
                                tab_bounds.borrow_mut().insert((group, id), *bounds);
                            }
                        })
                        .id(SharedString::from(format!("remote-tab-{group}-{id}")))
                        .test_support()
                        .max_w(px(240.))
                        .min_w_0()
                        .px(px(12.))
                        .py(px(8.))
                        .text_size(px(12.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .cursor_pointer()
                        .when(self.selected[group] == Some(id), |d| d.bg(rgb(0x17181b)))
                        .child(
                            if self
                                .replicas
                                .get(&id)
                                .is_some_and(|r| r.read(cx).state().phase == ReplicaPhase::Parked)
                            {
                                format!("{} · parked", session.title)
                            } else {
                                session.title.clone()
                            },
                        )
                        .on_click(cx.listener(move |this, _, w, cx| this.select(id, group, w, cx))),
                );
            }
            if let Some(id) = self.selected[group] {
                tabs = tabs.child(div().flex_1());
                if let Some(replica) = self.replicas.get(&id) {
                    if self.connected
                        && replica.read(cx).state().live
                        && !replica.read(cx).state().controller
                    {
                        let shared_size = replica
                            .read(cx)
                            .pane()
                            .map(|pane| pane.read(cx).view().read(cx).grid().size())
                            .unwrap_or(ghostty_vt::Size { cols: 0, rows: 0 });
                        tabs = tabs.child(
                            div()
                                .px(px(6.))
                                .py(px(8.))
                                .text_size(px(10.))
                                .text_color(rgb(0x98989e))
                                .child(format!(
                                    "Viewing {}×{}",
                                    shared_size.cols, shared_size.rows
                                )),
                        );
                        let control_bounds = self.control_bounds.clone();
                        tabs = tabs.child(
                            div()
                                .on_children_prepainted(move |children, _, _| {
                                    if let Some(bounds) = children.first() {
                                        control_bounds.borrow_mut().insert(id, *bounds);
                                    }
                                })
                                .id(SharedString::from(format!("remote-take-control-{id}")))
                                .test_support()
                                .mx(px(6.))
                                .my(px(4.))
                                .px(px(9.))
                                .py(px(3.))
                                .rounded(px(5.))
                                .bg(rgb(0x323235))
                                .text_size(px(11.))
                                .cursor_pointer()
                                .child("Take control")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.queue(ClientFrame::TakeControl { id });
                                    this.notice = Some("Requesting terminal control…".into());
                                    cx.notify();
                                })),
                        );
                    } else {
                        tabs = tabs.child(
                            div()
                                .px(px(10.))
                                .py(px(8.))
                                .text_size(px(10.))
                                .text_color(rgb(0x98989e))
                                .child(if replica.read(cx).state().controller && self.connected {
                                    "Controlling"
                                } else {
                                    "Viewing"
                                }),
                        );
                    }
                }
            }
            let mut column = div()
                .id(SharedString::from(format!("remote-group-{group}")))
                .test_support()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .bg(rgb(0x17181b))
                .child(tabs);
            if let Some(id) = self.selected[group] {
                if let Some(replica) = self.replicas.get(&id) {
                    let content = replica.clone();
                    column = column.child(
                        div()
                            .id(SharedString::from(format!("remote-terminal-{id}")))
                            .test_support()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .p(px(10.))
                            .child(content),
                    );
                } else {
                    column = column.child(
                        div()
                            .p_4()
                            .text_size(px(13.))
                            .child("Restoring terminal snapshot…"),
                    );
                }
            }
            columns = columns.child(column);
        }
        div().size_full().flex().flex_col().font_family("Inter").bg(rgb(0x252527)).text_color(rgb(0xe8e8ea))
            .child(div().h(px(52.)).flex_shrink_0().flex().items_center().gap(px(12.)).px(px(18.)).border_b_1().border_color(rgb(0x3a3a3d)).child(div().flex_1().text_size(px(13.)).child(heading)).child(div().id("remote-memory-saver").test_support().px_3().py_1().rounded(px(5.)).bg(rgb(0x323235)).cursor_pointer().text_size(px(11.)).child(if self.memory_saver {"Memory saver: on"} else {"Memory saver: off"}).on_click(cx.listener(|this,_,_,cx|this.set_memory_saver(!this.memory_saver,this.memory_idle,cx)))).child(div().text_size(px(11.)).text_color(rgb(if self.connected{0x55cc88}else{0xc79f5f})).child(if self.connected{"Connected"}else{"Disconnected"})).child(div().id("remote-reconnect").test_support().px_3().py_1().rounded(px(5.)).bg(rgb(0x323235)).cursor_pointer().text_size(px(12.)).child("Reconnect").on_click(cx.listener(|this,_,_,cx|this.reconnect(cx)))))
            .when_some(self.notice.clone(),|d,n|d.child(div().px_3().py_2().text_size(px(12.)).bg(rgb(0x332b1c)).child(n)))
            .child(columns)
            .child(div().flex_shrink_0().px_3().py_2().text_size(px(11.)).text_color(rgb(0x98989e)).child("GPUI terminal viewer · process and files remain on the host · keyboard follows terminal focus"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resize(id: u64, cols: u16) -> ClientFrame {
        ClientFrame::Resize {
            id,
            cols,
            rows: 24,
            cell_width: 8,
            cell_height: 19,
        }
    }

    #[test]
    fn rapid_resizes_keep_the_latest_geometry_on_each_side_of_input() {
        let mut pending = PendingFrames::default();
        pending.push(resize(1, 80));
        pending.push(resize(2, 60));
        pending.push(ClientFrame::Ack { id: 1, offset: 100 });
        pending.push(resize(1, 90));
        pending.push(ClientFrame::Input {
            id: 1,
            seq: 1,
            bytes: b"x".to_vec(),
        });
        pending.push(resize(1, 100));
        pending.push(resize(1, 110));
        assert!(matches!(pending.frames.as_slice(),[
            ClientFrame::Resize{id:1,cols:90,..},
            ClientFrame::Resize{id:2,cols:60,..},
            ClientFrame::Ack{id:1,offset:100},
            ClientFrame::Input{id:1,bytes,..},
            ClientFrame::Resize{id:1,cols:110,..},
        ] if bytes==b"x"));
    }
}
