//! Reconnect existing SSH holders; never spawn a replacement or resend input.
use super::*;
use std::sync::Arc;
use terminal_session::LiveAttacher;
const FIRST_DELAY: Duration = Duration::from_secs(1);
const MAX_DELAY: Duration = Duration::from_secs(30);
const MAX_CONNECTING: usize = 4;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Online,
    Waiting,
    Connecting,
    Hydrating,
    Blocked,
}
struct Entry {
    target: LiveAttacher,
    phase: Phase,
    due: Instant,
    delay: Duration,
    serial: u64,
}
pub(super) struct Reconnects {
    entries: BTreeMap<u64, Entry>,
    tasks: BTreeMap<u64, JoinHandle<()>>,
    slots: Arc<Semaphore>,
    closed: bool,
    max_delay: Duration,
}
impl Default for Reconnects {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            tasks: BTreeMap::new(),
            slots: Arc::new(Semaphore::new(MAX_CONNECTING)),
            closed: false,
            max_delay: MAX_DELAY,
        }
    }
}
impl Drop for Reconnects {
    fn drop(&mut self) {
        for task in self.tasks.values() {
            task.abort();
        }
    }
}
impl Reconnects {
    #[cfg(test)]
    pub(super) fn test_backoff(&mut self, delay: Duration) {
        self.max_delay = delay;
        for entry in self.entries.values_mut() {
            entry.delay = delay;
        }
    }
    pub(super) fn register(&mut self, id: u64, target: LiveAttacher, online: bool, blocked: bool) {
        self.entries.insert(
            id,
            Entry {
                target,
                phase: if blocked {
                    Phase::Blocked
                } else if online {
                    Phase::Online
                } else {
                    Phase::Waiting
                },
                due: Instant::now() + FIRST_DELAY.min(self.max_delay),
                delay: FIRST_DELAY.min(self.max_delay),
                serial: 0,
            },
        );
    }
    pub(super) fn halt(&mut self, id: u64) {
        if let Some(e) = self.entries.get_mut(&id) {
            e.phase = Phase::Blocked;
            e.serial = e.serial.wrapping_add(1);
        }
        if let Some(task) = self.tasks.remove(&id) {
            task.abort();
        }
    }
    fn lost(&mut self, id: u64) {
        if self.closed {
            return;
        }
        if let Some(e) = self.entries.get_mut(&id) {
            if matches!(e.phase, Phase::Blocked | Phase::Waiting | Phase::Connecting) {
                return;
            }
            e.phase = Phase::Waiting;
            e.due = Instant::now() + e.delay;
            e.delay = (e.delay * 2).min(self.max_delay);
        }
    }
    pub(super) async fn close(&mut self) {
        self.closed = true;
        for (_, task) in std::mem::take(&mut self.tasks) {
            task.abort();
            let _ = task.await;
        }
    }
}
pub(super) struct Connected {
    pub id: u64,
    pub serial: u64,
    pub result: Result<Attached, terminal_session::Error>,
}
impl Actor {
    pub(super) fn reconnect_tick(&mut self) {
        if self.reconnects.closed {
            return;
        }
        let mut due: Vec<_> = self
            .reconnects
            .entries
            .iter()
            .filter_map(|(id, e)| {
                (e.phase == Phase::Waiting && e.due <= Instant::now()).then_some((e.due, *id))
            })
            .collect();
        due.sort_unstable();
        for (_, id) in due {
            let Some(p) = self.processes.get(&id) else {
                self.reconnects.halt(id);
                continue;
            };
            if p.exited.is_some()
                || p.stopping
                || self.blocked_resize.contains(&id)
                || self.blocked_replies.contains(&id)
            {
                self.reconnects.halt(id);
                continue;
            }
            let Ok(slot) = self.reconnects.slots.clone().try_acquire_owned() else {
                break;
            };
            let Some(e) = self.reconnects.entries.get_mut(&id) else {
                continue;
            };
            e.phase = Phase::Connecting;
            e.serial = e.serial.wrapping_add(1);
            let serial = e.serial;
            let target = e.target.clone();
            let offset = p.offset;
            let tx = self.tx.clone();
            let task = tokio::spawn(async move {
                let result = target.attach_from(offset).await;
                let _ = tx
                    .send(Command::Reconnected(Connected { id, serial, result }))
                    .await;
                drop(slot);
            });
            self.reconnects.tasks.insert(id, task);
        }
    }
    pub(super) async fn disconnect_transports(&mut self, id: u64) {
        // Await cancellation before any new attachment: all old transport
        // commands are now ahead of the future Connected message in rx.
        let tasks = if let Some(p) = self.processes.get_mut(&id) {
            p.writer = None;
            std::mem::take(&mut p._tasks)
        } else {
            Vec::new()
        };
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(Command::TransportsRetired(id)).await;
        });
        let viewers: Vec<_> = self
            .viewers
            .iter()
            .filter_map(|(viewer, v)| v.open.contains_key(&id).then_some(*viewer))
            .collect();
        // Preserve the display but explicitly disable input while disconnected.
        for viewer in viewers {
            self.snapshot(viewer, id);
        }
    }
    pub(super) fn transports_retired(&mut self, id: u64) {
        // This barrier follows every old task's queued receipt. A late ACK may
        // therefore settle a reply/resize before delivery is called ambiguous.
        if self.processes.get(&id).is_none_or(|p| p.writer.is_some()) {
            return;
        }
        self.settle_uncertain_viewer_inputs(id);
        let unsafe_replies = self
            .processes
            .get(&id)
            .is_some_and(|p| p.reply_cursor.pending || !p.pending_replies.is_empty());
        let unsafe_resize = self
            .processes
            .get(&id)
            .is_some_and(|p| p.resize_epoch > p.applied_resize_epoch);
        if unsafe_replies {
            self.blocked_replies.insert(id);
        }
        if unsafe_resize {
            self.blocked_resize.insert(id);
        }
        if unsafe_replies || unsafe_resize {
            self.reconnects.halt(id);
        } else {
            self.reconnects.lost(id);
        }
        if unsafe_replies || unsafe_resize {
            let viewers: Vec<_> = self
                .viewers
                .iter()
                .filter_map(|(viewer, v)| v.open.contains_key(&id).then_some(*viewer))
                .collect();
            for viewer in viewers {
                self.snapshot(viewer, id);
            }
        }
    }
    pub(super) fn settle_uncertain_viewer_inputs(&mut self, id: u64) {
        let mut pending = Vec::new();
        for (viewer, v) in &mut self.viewers {
            if let Some(last) = v.inputs.get(&id).copied() {
                let settled = v.settled_inputs.entry(id).or_default();
                if last > *settled {
                    *settled = last;
                    pending.push(*viewer);
                }
            }
        }
        for viewer in pending {
            self.failure_to(viewer,Some(id),"input_uncertain","The process connection ended before this viewer received every input acknowledgement. Delivery is uncertain; inspect the resumed terminal before explicitly sending more. Uncertain input was not resent.");
        }
    }
    pub(super) fn irrecoverable_views(&mut self, id: u64) {
        self.settle_uncertain_viewer_inputs(id);
        if self.controllers.remove(&id).is_some() {
            self.controls(id);
        }
        if let Some(p) = self.processes.get_mut(&id) {
            for task in &p._tasks {
                task.abort();
            }
        }
        let viewers: Vec<_> = self
            .viewers
            .iter()
            .filter_map(|(viewer, v)| v.open.contains_key(&id).then_some(*viewer))
            .collect();
        for viewer in viewers {
            self.snapshot(viewer, id);
        }
    }
    pub(super) fn connected(&mut self, event: Connected) {
        let id = event.id;
        let Some(e) = self.reconnects.entries.get_mut(&id) else {
            return;
        };
        if self.reconnects.closed || e.phase != Phase::Connecting || e.serial != event.serial {
            return;
        }
        self.reconnects.tasks.remove(&id);
        let attached = match event.result {
            Ok(attached) => attached,
            Err(_) => {
                e.phase = Phase::Waiting;
                e.due = Instant::now() + e.delay;
                e.delay = (e.delay * 2).min(self.reconnects.max_delay);
                return;
            }
        };
        let Some(p) = self.processes.get_mut(&id) else {
            return;
        };
        if p.stopping || p.exited.is_some() {
            e.phase = Phase::Blocked;
            return;
        }
        if (attached.cols, attached.rows) != (p.vt.size().cols, p.vt.size().rows)
            || p.offset > attached.replay_end
            || p.reply_cursor.answered_through > attached.replay_end
        {
            e.phase = Phase::Blocked;
            self.blocked_resize.insert(id);
            self.failure(Some(id),"reattach_state_changed","The existing holder's grid or retained-output boundary changed while disconnected. The last verified display is preserved; input stays disabled rather than reconstructing uncertain state.");
            return;
        }
        // Never carry old connection sequence ACKs into the new writer epoch.
        p.receipts.clear();
        p.early_receipts.clear();
        p.resize_receipts.clear();
        p.reply_receipts.clear();
        p.native_receipts.clear();
        p.replay_end = attached.replay_end;
        p.exited = attached.exited;
        p.info.state = attached
            .exited
            .map(|code| SessionState::Exited { code: Some(code) })
            .unwrap_or(SessionState::Unavailable);
        let (writer, requests) = mpsc::channel(WRITER_QUEUE);
        p._tasks = transport_tasks(id, attached, requests, self.tx.clone());
        p.writer = Some(writer);
        e.phase = Phase::Hydrating;
        for (viewer, v) in &mut self.viewers {
            if let Some(open) = v.open.get_mut(&id) {
                open.resync = true;
                p.pending_open.insert(*viewer);
            }
        }
        self.sessions();
    }
    pub(super) fn reconnected_ready(&mut self) {
        let ready: Vec<_> = self
            .reconnects
            .entries
            .iter()
            .filter_map(|(id, e)| {
                (e.phase == Phase::Hydrating
                    && self
                        .processes
                        .get(id)
                        .is_some_and(|p| p.offset >= p.replay_end))
                .then_some(*id)
            })
            .collect();
        for id in ready {
            if let Some(e) = self.reconnects.entries.get_mut(&id) {
                e.phase = Phase::Online;
                e.delay = FIRST_DELAY.min(self.reconnects.max_delay);
            }
            if let Some(p) = self.processes.get_mut(&id) {
                p.info.state = p
                    .exited
                    .map(|code| SessionState::Exited { code: Some(code) })
                    .unwrap_or(SessionState::Running);
            }
            self.sessions();
        }
    }
}
