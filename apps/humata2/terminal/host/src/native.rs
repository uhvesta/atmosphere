//! Trusted native embedding handles. Viewer authorization is a separate layer.
use super::*;
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use terminal_wire::{
    InjectionMode, InjectionOptions, InputTicket, NameSource, SessionNameOverride,
};

/// Input control is explicit. Taking control revokes the current viewer lease
/// while this handle's queued writes are outstanding; it never resends bytes.
#[derive(Clone, Copy, Default)]
pub enum ControlPolicy {
    #[default]
    RequireUncontrolled,
    TakeControl,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteUncertainty {
    HostClosed,
    ProcessExited,
    TransportLost,
    Refused,
}
#[derive(Debug, thiserror::Error)]
pub enum HandleError {
    #[error(
        "Terminal {uuid} is not available in this host. Reopen its durable UUID on the host that owns it."
    )]
    Missing { uuid: String },
    #[error(
        "The host closed before this terminal operation was queued. Reconnect using its durable UUID before retrying."
    )]
    Closed,
    #[error(
        "The host closed after stop may have been queued for terminal {uuid}. Inspect that terminal on reconnection before requesting another lifecycle action."
    )]
    StopUncertain { uuid: String },
    #[error(
        "Terminal {uuid} is controlled elsewhere. Choose explicit TakeControl or wait for its controller to release it."
    )]
    Controlled { uuid: String },
    #[error(
        "Terminal {uuid} is offline, stopping or hydrating. Wait for its live snapshot or explicitly recover it before sending text."
    )]
    NotRunning { uuid: String },
    #[error(
        "Terminal {uuid} is starting another operation. Wait for launch to finish before changing its metadata."
    )]
    Busy { uuid: String },
    #[error(
        "Encoded input is {actual} bytes, exceeding {maximum}. Split it into smaller explicitly ordered requests."
    )]
    InputLimit { actual: usize, maximum: usize },
    #[error(
        "The terminal input queue is full. Wait for written receipts before queueing more text; this request was not sent."
    )]
    Backpressure,
    #[error("The terminal could not encode this paste. Check the requested terminal input mode.")]
    Encode {
        #[source]
        source: ghostty_vt::Error,
    },
    #[error(
        "Terminal metadata could not be updated. Fix its private workspace metadata before retrying."
    )]
    Metadata {
        #[source]
        source: terminal_session::Error,
    },
    #[error(
        "Input ticket {ticket:?} has uncertain delivery ({reason:?}). Inspect the terminal state; do not automatically resend it or interpret it as agent completion."
    )]
    Uncertain {
        ticket: InputTicket,
        reason: WriteUncertainty,
        #[source]
        source: Option<Arc<terminal_session::Error>>,
    },
    #[error(
        "This host exhausted its input/handle counter. Reconnect to a new host instance after inspecting outstanding writes."
    )]
    CounterExhausted,
    #[error(
        "Terminal metadata field {field} is {actual}, exceeding its limit of {maximum}. Reduce that field before submitting it again."
    )]
    MetadataLimit {
        field: &'static str,
        actual: usize,
        maximum: usize,
    },
}
#[derive(Clone)]
pub struct TerminalHandle {
    host: Host,
    id: u64,
    uuid: String,
    key: u64,
}
impl TerminalHandle {
    pub fn uuid(&self) -> &str {
        &self.uuid
    }
    /// Numeric routes are scoped to this host. Persist uuid(), not this value.
    pub fn route(&self) -> u64 {
        self.id
    }
    /// Request a stop without taking another controller's lease. Success means
    /// queued; it does not mean the process has exited.
    pub async fn stop(&self) -> Result<QueuedStop, HandleError> {
        self.stop_with_control(ControlPolicy::default()).await
    }
    pub async fn stop_with_control(
        &self,
        control: ControlPolicy,
    ) -> Result<QueuedStop, HandleError> {
        let queued = Arc::new(AtomicBool::new(false));
        let (reply, answer) = oneshot::channel();
        self.host
            .tx
            .send(Command::Native(NativeCommand::Stop {
                id: self.id,
                uuid: self.uuid.clone(),
                key: self.key,
                control,
                queued: queued.clone(),
                reply,
            }))
            .await
            .map_err(|_| HandleError::Closed)?;
        answer.await.map_err(|_| {
            if queued.load(Ordering::Acquire) {
                HandleError::StopUncertain {
                    uuid: self.uuid.clone(),
                }
            } else {
                HandleError::Closed
            }
        })?
    }
    /// Launch a fresh sibling from this handle's configured intent and machine.
    pub async fn spawn_from(&self, options: SpawnFromOptions) -> Result<Spawned, SpawnError> {
        self.host.spawn_from(self.id, options).await
    }
    pub async fn info(&self) -> Result<SessionInfo, HandleError> {
        let (reply, answer) = oneshot::channel();
        self.host
            .tx
            .send(Command::Native(NativeCommand::Info {
                id: self.id,
                uuid: self.uuid.clone(),
                reply,
            }))
            .await
            .map_err(|_| HandleError::Closed)?;
        answer.await.map_err(|_| HandleError::Closed)?
    }
    pub async fn set_name(&self, title: String) -> Result<SessionInfo, HandleError> {
        self.set_name_with_source(title, NameSource::Programmatic)
            .await
    }
    pub async fn set_name_with_source(
        &self,
        title: String,
        source: NameSource,
    ) -> Result<SessionInfo, HandleError> {
        self.rename(Some(SessionNameOverride { title, source }))
            .await
    }
    pub async fn clear_name(&self) -> Result<SessionInfo, HandleError> {
        self.rename(None).await
    }
    async fn rename(&self, name: Option<SessionNameOverride>) -> Result<SessionInfo, HandleError> {
        if let Some(name) = &name {
            bound("name bytes", name.title.len(), 512)?;
        }
        let (reply, answer) = oneshot::channel();
        self.host
            .tx
            .send(Command::Native(NativeCommand::Name {
                id: self.id,
                uuid: self.uuid.clone(),
                name,
                reply,
            }))
            .await
            .map_err(|_| HandleError::Closed)?;
        answer.await.map_err(|_| HandleError::Closed)?
    }
    pub async fn set_attrs(
        &self,
        attrs: BTreeMap<String, String>,
    ) -> Result<SessionInfo, HandleError> {
        bound("attribute count", attrs.len(), 32)?;
        let mut total = 0usize;
        for (key, value) in &attrs {
            bound("attribute key bytes", key.len(), 64)?;
            bound("attribute value bytes", value.len(), 4096)?;
            total = total.saturating_add(key.len()).saturating_add(value.len());
        }
        bound("attribute total bytes", total, 32768)?;
        let (reply, answer) = oneshot::channel();
        self.host
            .tx
            .send(Command::Native(NativeCommand::Attrs {
                id: self.id,
                uuid: self.uuid.clone(),
                attrs,
                reply,
            }))
            .await
            .map_err(|_| HandleError::Closed)?;
        answer.await.map_err(|_| HandleError::Closed)?
    }
    /// Queue exactly the supplied text. Raw preserves UTF-8 bytes; Paste uses
    /// the authoritative terminal's paste modes. submit=false adds no Enter.
    /// Newlines already present remain subject to terminal interpretation.
    pub async fn inject_text(
        &self,
        text: String,
        options: InjectionOptions,
    ) -> Result<QueuedInjection, HandleError> {
        self.inject_text_with_control(text, options, ControlPolicy::default())
            .await
    }
    /// Explicitly choose whether this injection may transfer another controller's lease.
    pub async fn inject_text_with_control(
        &self,
        text: String,
        options: InjectionOptions,
        control: ControlPolicy,
    ) -> Result<QueuedInjection, HandleError> {
        if text.len() > MAX_INPUT {
            return Err(HandleError::InputLimit {
                actual: text.len(),
                maximum: MAX_INPUT,
            });
        }
        let queued = Arc::new(QueuedMark::default());
        let (reply, answer) = oneshot::channel();
        self.host
            .tx
            .send(Command::Native(NativeCommand::Inject {
                id: self.id,
                uuid: self.uuid.clone(),
                key: self.key,
                text,
                options,
                control,
                queued: queued.clone(),
                reply,
            }))
            .await
            .map_err(|_| HandleError::Closed)?;
        answer.await.map_err(|_| match queued.ticket(&self.uuid) {
            Some(ticket) => HandleError::Uncertain {
                ticket,
                reason: WriteUncertainty::HostClosed,
                source: None,
            },
            None => HandleError::Closed,
        })?
    }
}
/// Stop was accepted by the ordered writer. Observe info().state to confirm exit.
#[derive(Debug)]
pub struct QueuedStop {
    pub terminal_uuid: String,
}
/// Queued means accepted by the ordered host writer, not written to the PTY.
pub struct QueuedInjection {
    ticket: InputTicket,
    written: oneshot::Receiver<Result<InputTicket, HandleError>>,
}
impl QueuedInjection {
    pub fn ticket(&self) -> &InputTicket {
        &self.ticket
    }
    /// Resolves only from the actual holder ACK. No provider/LLM completion is implied.
    pub async fn written(self) -> Result<InputTicket, HandleError> {
        self.written.await.map_err(|_| HandleError::Uncertain {
            ticket: self.ticket,
            reason: WriteUncertainty::HostClosed,
            source: None,
        })?
    }
}
#[derive(Default)]
pub(super) struct QueuedMark {
    epoch: AtomicU64,
    seq: AtomicU64,
}
impl QueuedMark {
    fn ticket(&self, uuid: &str) -> Option<InputTicket> {
        let seq = self.seq.load(Ordering::Acquire);
        (seq != 0).then(|| InputTicket {
            terminal_uuid: uuid.into(),
            connection_epoch: self.epoch.load(Ordering::Relaxed),
            seq,
        })
    }
}
struct Pending {
    session: u64,
    key: u64,
    ticket: InputTicket,
    written: oneshot::Sender<Result<InputTicket, HandleError>>,
}
pub(super) struct NativeState {
    epoch: u64,
    next_key: u64,
    next_ticket: u64,
    pending: BTreeMap<u64, Pending>,
    closing: bool,
}
impl NativeState {
    pub(super) fn new() -> Result<Self, Error> {
        let path = std::path::Path::new("/dev/urandom");
        let mut random = [0u8; 8];
        std::fs::File::open(path)
            .and_then(|mut file| file.read_exact(&mut random))
            .map_err(|source| crate::error::disk("read native input epoch", path, source))?;
        Ok(Self {
            epoch: u64::from_le_bytes(random),
            next_key: 0,
            next_ticket: 0,
            pending: BTreeMap::new(),
            closing: false,
        })
    }
    pub(super) fn has_pending(&self, id: u64) -> bool {
        self.pending.values().any(|p| p.session == id)
    }
    pub(super) fn closing(&mut self) {
        self.closing = true;
    }
}
pub(super) enum NativeCommand {
    Stop {
        id: u64,
        uuid: String,
        key: u64,
        control: ControlPolicy,
        queued: Arc<AtomicBool>,
        reply: oneshot::Sender<Result<QueuedStop, HandleError>>,
    },
    List {
        reply: oneshot::Sender<Vec<SessionInfo>>,
    },
    #[cfg(test)]
    WriterFixture {
        id: u64,
        reply: oneshot::Sender<Result<(mpsc::Receiver<Write>, mpsc::Sender<Write>), HandleError>>,
    },
    Lookup {
        uuid: String,
        reply: oneshot::Sender<Result<(u64, u64), HandleError>>,
    },
    Info {
        id: u64,
        uuid: String,
        reply: oneshot::Sender<Result<SessionInfo, HandleError>>,
    },
    Name {
        id: u64,
        uuid: String,
        name: Option<SessionNameOverride>,
        reply: oneshot::Sender<Result<SessionInfo, HandleError>>,
    },
    Attrs {
        id: u64,
        uuid: String,
        attrs: BTreeMap<String, String>,
        reply: oneshot::Sender<Result<SessionInfo, HandleError>>,
    },
    Inject {
        id: u64,
        uuid: String,
        key: u64,
        text: String,
        options: InjectionOptions,
        control: ControlPolicy,
        queued: Arc<QueuedMark>,
        reply: oneshot::Sender<Result<QueuedInjection, HandleError>>,
    },
    Sent {
        id: u64,
        ticket: u64,
        internal: u64,
    },
}
fn bound(field: &'static str, actual: usize, maximum: usize) -> Result<(), HandleError> {
    if actual > maximum {
        Err(HandleError::MetadataLimit {
            field,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}
impl Host {
    /// Enumerate sanitized terminal records to discover or reconnect durable UUIDs.
    pub async fn terminals(&self) -> Result<Vec<SessionInfo>, HandleError> {
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Command::Native(NativeCommand::List { reply }))
            .await
            .map_err(|_| HandleError::Closed)?;
        answer.await.map_err(|_| HandleError::Closed)
    }
    /// Reopen a durable terminal UUID. Handle/ticket epochs belong to this Host
    /// instance; reconnect and relookup after restart, never replay pending input.
    pub async fn terminal(&self, uuid: &str) -> Result<TerminalHandle, HandleError> {
        bound("UUID bytes", uuid.len(), 36)?;
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Command::Native(NativeCommand::Lookup {
                uuid: uuid.into(),
                reply,
            }))
            .await
            .map_err(|_| HandleError::Closed)?;
        let (id, key) = answer.await.map_err(|_| HandleError::Closed)??;
        Ok(TerminalHandle {
            host: self.clone(),
            id,
            uuid: uuid.into(),
            key,
        })
    }
}
impl Actor {
    fn check_handle(&self, id: u64, uuid: &str) -> Result<&Process, HandleError> {
        self.processes
            .get(&id)
            .filter(|p| p.info.uuid == uuid)
            .ok_or_else(|| HandleError::Missing { uuid: uuid.into() })
    }
    pub(super) fn native_command(&mut self, command: NativeCommand) {
        match command {
            NativeCommand::Stop {
                id,
                uuid,
                key,
                control,
                queued,
                reply,
            } => {
                let result = self.queue_native_stop(id, &uuid, key, control, &queued);
                let _ = reply.send(result);
            }
            NativeCommand::List { reply } => {
                let _ = reply.send(self.processes.values().map(|p| p.info.clone()).collect());
            }
            #[cfg(test)]
            NativeCommand::WriterFixture { id, reply } => {
                let result = if let Some(p) = self.processes.get_mut(&id) {
                    let (sender, receiver) = mpsc::channel(WRITER_QUEUE);
                    p.writer
                        .replace(sender)
                        .map(|old| (receiver, old))
                        .ok_or_else(|| HandleError::NotRunning {
                            uuid: p.info.uuid.clone(),
                        })
                } else {
                    Err(HandleError::Missing {
                        uuid: String::new(),
                    })
                };
                let _ = reply.send(result);
            }
            NativeCommand::Lookup { uuid, reply } => {
                let result = (|| {
                    let store = self
                        .store
                        .as_ref()
                        .ok_or_else(|| HandleError::Busy { uuid: uuid.clone() })?;
                    let saved = store
                        .session_by_uuid(&uuid)
                        .map_err(|source| HandleError::Metadata { source })?;
                    let id = saved.id;
                    self.check_handle(id, &uuid)?;
                    self.native_state.next_key = self
                        .native_state
                        .next_key
                        .checked_add(1)
                        .ok_or(HandleError::CounterExhausted)?;
                    Ok((id, self.native_state.next_key))
                })();
                let _ = reply.send(result);
            }
            NativeCommand::Info { id, uuid, reply } => {
                let _ = reply.send(self.check_handle(id, &uuid).map(|p| p.info.clone()));
            }
            NativeCommand::Name {
                id,
                uuid,
                name,
                reply,
            } => {
                let result = self.update_native_metadata(id, &uuid, Some(name), None);
                let _ = reply.send(result);
            }
            NativeCommand::Attrs {
                id,
                uuid,
                attrs,
                reply,
            } => {
                let result = self.update_native_metadata(id, &uuid, None, Some(attrs));
                let _ = reply.send(result);
            }
            NativeCommand::Inject {
                id,
                uuid,
                key,
                text,
                options,
                control,
                queued,
                reply,
            } => {
                let result = self.queue_native(id, &uuid, key, text, options, control, &queued);
                let _ = reply.send(result);
            }
            NativeCommand::Sent {
                id,
                ticket,
                internal,
            } => {
                if !self.native_state.pending.contains_key(&ticket) {
                    return;
                }
                let ready = if let Some(p) = self.processes.get_mut(&id) {
                    if p.early_receipts.remove(&internal) {
                        true
                    } else {
                        p.native_receipts.insert(internal, ticket);
                        false
                    }
                } else {
                    false
                };
                if ready {
                    self.native_written(ticket);
                }
            }
        }
    }
    fn queue_native_stop(
        &mut self,
        id: u64,
        uuid: &str,
        key: u64,
        control: ControlPolicy,
        queued: &AtomicBool,
    ) -> Result<QueuedStop, HandleError> {
        if self.native_state.closing {
            return Err(HandleError::Closed);
        }
        let p = self.check_handle(id, uuid)?;
        if !matches!(p.info.state, SessionState::Running) || p.writer.is_none() || p.stopping {
            return Err(HandleError::NotRunning { uuid: uuid.into() });
        }
        if self
            .controllers
            .get(&id)
            .is_some_and(|owner| *owner != Controller::Native(key))
            && !matches!(control, ControlPolicy::TakeControl)
        {
            return Err(HandleError::Controlled { uuid: uuid.into() });
        }
        let permit = p
            .writer
            .as_ref()
            .ok_or_else(|| HandleError::NotRunning { uuid: uuid.into() })?
            .clone()
            .try_reserve_owned()
            .map_err(|_| HandleError::Backpressure)?;
        if let Some(p) = self.processes.get_mut(&id) {
            p.stopping = true;
        }
        if self.controllers.insert(id, Controller::Native(key)) != Some(Controller::Native(key)) {
            self.controls(id);
        }
        queued.store(true, Ordering::Release);
        permit.send(Write::Stop);
        Ok(QueuedStop {
            terminal_uuid: uuid.into(),
        })
    }
    fn update_native_metadata(
        &mut self,
        id: u64,
        uuid: &str,
        name: Option<Option<SessionNameOverride>>,
        attrs: Option<BTreeMap<String, String>>,
    ) -> Result<SessionInfo, HandleError> {
        self.check_handle(id, uuid)?;
        let store = self
            .store
            .as_mut()
            .ok_or_else(|| HandleError::Busy { uuid: uuid.into() })?;
        if let Some(name) = name {
            match name {
                Some(name) => store.set_name_override(id, name.title, name.source),
                None => store.clear_name_override(id),
            }
            .map_err(|source| HandleError::Metadata { source })?;
        }
        if let Some(attrs) = attrs {
            store
                .set_attributes(id, attrs)
                .map_err(|source| HandleError::Metadata { source })?;
        }
        let saved = store
            .session_by_uuid(uuid)
            .map_err(|source| HandleError::Metadata { source })?;
        let p = self
            .processes
            .get_mut(&id)
            .ok_or_else(|| HandleError::Missing { uuid: uuid.into() })?;
        p.info.name_override = saved.name_override.clone();
        p.info.attributes = saved.attributes.clone();
        p.info.title = p
            .info
            .name_override
            .as_ref()
            .map(|n| n.title.clone())
            .unwrap_or_else(|| p.info.automatic_title.clone());
        let info = p.info.clone();
        self.sessions();
        Ok(info)
    }
    fn queue_native(
        &mut self,
        id: u64,
        uuid: &str,
        key: u64,
        text: String,
        options: InjectionOptions,
        control: ControlPolicy,
        queued: &QueuedMark,
    ) -> Result<QueuedInjection, HandleError> {
        if self.native_state.closing {
            return Err(HandleError::Closed);
        }
        let p = self.check_handle(id, uuid)?;
        if !matches!(p.info.state, SessionState::Running)
            || p.writer.is_none()
            || p.stopping
            || p.offset < p.replay_end
        {
            return Err(HandleError::NotRunning { uuid: uuid.into() });
        }
        if let Some(owner) = self.controllers.get(&id) {
            if *owner != Controller::Native(key) && !matches!(control, ControlPolicy::TakeControl) {
                return Err(HandleError::Controlled { uuid: uuid.into() });
            }
        }
        let mut bytes = match options.mode {
            InjectionMode::Raw => text.into_bytes(),
            InjectionMode::Paste => {
                p.vt.paste(&text, true)
                    .map_err(|source| HandleError::Encode { source })?
            }
        };
        if options.submit {
            bytes.push(b'\r');
        }
        if bytes.len() > MAX_INPUT {
            return Err(HandleError::InputLimit {
                actual: bytes.len(),
                maximum: MAX_INPUT,
            });
        }
        if self.native_state.pending.len() >= 128 {
            return Err(HandleError::Backpressure);
        }
        let permit = p
            .writer
            .as_ref()
            .ok_or_else(|| HandleError::NotRunning { uuid: uuid.into() })?
            .clone()
            .try_reserve_owned()
            .map_err(|_| HandleError::Backpressure)?;
        let seq = self
            .native_state
            .next_ticket
            .checked_add(1)
            .ok_or(HandleError::CounterExhausted)?;
        self.native_state.next_ticket = seq;
        let ticket = InputTicket {
            terminal_uuid: uuid.into(),
            connection_epoch: self.native_state.epoch,
            seq,
        };
        let (written, answer) = oneshot::channel();
        self.native_state.pending.insert(
            seq,
            Pending {
                session: id,
                key,
                ticket: ticket.clone(),
                written,
            },
        );
        if self.controllers.insert(id, Controller::Native(key)) != Some(Controller::Native(key)) {
            self.controls(id);
        }
        queued
            .epoch
            .store(ticket.connection_epoch, Ordering::Relaxed);
        queued.seq.store(seq, Ordering::Release);
        permit.send(Write::Native { bytes, ticket: seq });
        Ok(QueuedInjection {
            ticket,
            written: answer,
        })
    }
    pub(super) fn native_written(&mut self, seq: u64) {
        let Some(pending) = self.native_state.pending.remove(&seq) else {
            return;
        };
        let id = pending.session;
        let key = pending.key;
        let _ = pending.written.send(Ok(pending.ticket));
        self.release_native(id, key);
    }
    fn release_native(&mut self, id: u64, key: u64) {
        if self.controllers.get(&id) == Some(&Controller::Native(key))
            && !self
                .native_state
                .pending
                .values()
                .any(|p| p.session == id && p.key == key)
        {
            self.controllers.remove(&id);
            self.controls(id);
        }
    }
    pub(super) fn fail_native(&mut self, id: u64, reason: WriteUncertainty) {
        self.fail_native_with_source(id, reason, None);
    }
    pub(super) fn fail_native_with_source(
        &mut self,
        id: u64,
        reason: WriteUncertainty,
        source: Option<Arc<terminal_session::Error>>,
    ) {
        let seqs: Vec<_> = self
            .native_state
            .pending
            .iter()
            .filter_map(|(seq, p)| (p.session == id).then_some(*seq))
            .collect();
        for seq in seqs {
            if let Some(p) = self.native_state.pending.remove(&seq) {
                let _ = p.written.send(Err(HandleError::Uncertain {
                    ticket: p.ticket,
                    reason,
                    source: source.clone(),
                }));
                self.release_native(id, p.key);
            }
        }
        if let Some(p) = self.processes.get_mut(&id) {
            p.native_receipts.clear();
        }
        if matches!(self.controllers.get(&id), Some(Controller::Native(_))) {
            self.controllers.remove(&id);
            self.controls(id);
        }
    }
}
