//! Native-only structured provider events, independent of terminal output.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use terminal_provider::{ProviderEvent, ProviderSignal, TrustedContext};

/// A trusted native launcher capability. Rebinding invalidates earlier handles.
/// This handle is never sent to terminal viewers or serialized.
#[derive(Clone)]
pub struct ProviderBinding {
    host: Host,
    terminal: u64,
    key: u64,
    context: TrustedContext,
}
impl ProviderBinding {
    /// Submit an already normalized event, not scraped terminal text.
    pub async fn ingest(&self, event: ProviderEvent) -> Result<(), Error> {
        terminal_provider::validate_event(&self.context, &event)
            .map_err(|source| Error::ProviderEvent { source })?;
        let (reply, answer) = oneshot::channel();
        self.host
            .tx
            .send(Command::Provider(ProviderCommand::Ingest {
                terminal: self.terminal,
                key: self.key,
                event,
                reply,
            }))
            .await
            .map_err(|_| Error::Closed)?;
        answer.await.map_err(|_| Error::Closed)?
    }
}
#[derive(Debug)]
pub enum ProviderObservation {
    Event {
        terminal: u64,
        event: ProviderEvent,
    },
    /// Notifications are ephemeral; this count explicitly reports events missed
    /// by this observer, without replaying or persisting their bodies.
    Lagged {
        dropped: u64,
    },
}
pub struct ProviderObserver {
    receiver: mpsc::Receiver<(u64, ProviderEvent)>,
    dropped: Arc<AtomicU64>,
}
impl ProviderObserver {
    pub async fn recv(&mut self) -> Option<ProviderObservation> {
        let dropped = self.dropped.swap(0, Ordering::AcqRel);
        if dropped > 0 {
            return Some(ProviderObservation::Lagged { dropped });
        }
        self.receiver
            .recv()
            .await
            .map(|(terminal, event)| ProviderObservation::Event { terminal, event })
    }
}
struct Bound {
    key: u64,
    context: TrustedContext,
}
struct Observer {
    sender: mpsc::Sender<(u64, ProviderEvent)>,
    dropped: Arc<AtomicU64>,
}
#[derive(Default)]
pub(super) struct ProviderState {
    bindings: BTreeMap<u64, Bound>,
    observers: Vec<Observer>,
    next: u64,
}
pub(super) enum ProviderCommand {
    Bind {
        terminal: u64,
        context: TrustedContext,
        reply: oneshot::Sender<Result<u64, Error>>,
    },
    Ingest {
        terminal: u64,
        key: u64,
        event: ProviderEvent,
        reply: oneshot::Sender<Result<(), Error>>,
    },
    Observe {
        capacity: usize,
        reply: oneshot::Sender<Result<ProviderObserver, Error>>,
    },
}
impl Host {
    /// Bind a known terminal to launcher-owned provider metadata. No event may be
    /// ingested through the WebSocket viewer protocol.
    pub async fn bind_provider(
        &self,
        terminal: u64,
        context: TrustedContext,
    ) -> Result<ProviderBinding, Error> {
        // Validate trusted fields too; unknown adapter schemas remain extensible.
        let validation = ProviderEvent {
            schema: context.schema.clone(),
            provider: context.provider.clone(),
            session_id: "binding-validation".into(),
            cwd: None,
            signal: ProviderSignal::TurnStarted,
        };
        terminal_provider::validate_event(&context, &validation)
            .map_err(|source| Error::ProviderEvent { source })?;
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Command::Provider(ProviderCommand::Bind {
                terminal,
                context: context.clone(),
                reply,
            }))
            .await
            .map_err(|_| Error::Closed)?;
        let key = answer.await.map_err(|_| Error::Closed)??;
        Ok(ProviderBinding {
            host: self.clone(),
            terminal,
            key,
            context,
        })
    }
    /// Opt in to ephemeral structured notifications. A slow observer loses only
    /// its own events and receives an explicit lag count. No OS notifications.
    pub async fn observe_provider(&self, capacity: usize) -> Result<ProviderObserver, Error> {
        if !(1..=1024).contains(&capacity) {
            return Err(Error::Invalid {
                reason: "provider observer capacity must be 1–1024 events",
            });
        }
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Command::Provider(ProviderCommand::Observe {
                capacity,
                reply,
            }))
            .await
            .map_err(|_| Error::Closed)?;
        answer.await.map_err(|_| Error::Closed)?
    }
}
impl Actor {
    pub(super) fn provider_command(&mut self, command: ProviderCommand) {
        match command {
            ProviderCommand::Bind {
                terminal,
                context,
                reply,
            } => {
                let result = if !self.processes.contains_key(&terminal) {
                    Err(Error::ProviderBinding {
                        terminal,
                        reason: "unknown terminal",
                    })
                } else {
                    self.provider_state.next = self.provider_state.next.wrapping_add(1);
                    let key = self.provider_state.next;
                    self.provider_state
                        .bindings
                        .insert(terminal, Bound { key, context });
                    Ok(key)
                };
                let _ = reply.send(result);
            }
            ProviderCommand::Observe { capacity, reply } => {
                self.provider_state
                    .observers
                    .retain(|o| !o.sender.is_closed());
                let result = if self.provider_state.observers.len() >= 16 {
                    Err(Error::Invalid {
                        reason: "close an unused provider observer before creating another; the limit is sixteen",
                    })
                } else {
                    let (sender, receiver) = mpsc::channel(capacity);
                    let dropped = Arc::new(AtomicU64::new(0));
                    self.provider_state.observers.push(Observer {
                        sender,
                        dropped: dropped.clone(),
                    });
                    Ok(ProviderObserver { receiver, dropped })
                };
                let _ = reply.send(result);
            }
            ProviderCommand::Ingest {
                terminal,
                key,
                event,
                reply,
            } => {
                let result = self.ingest_provider(terminal, key, event);
                let _ = reply.send(result);
            }
        }
    }
    fn ingest_provider(
        &mut self,
        terminal: u64,
        key: u64,
        event: ProviderEvent,
    ) -> Result<(), Error> {
        let bound = self
            .provider_state
            .bindings
            .get(&terminal)
            .filter(|b| b.key == key)
            .ok_or(Error::ProviderBinding {
                terminal,
                reason: "binding was replaced or does not exist",
            })?;
        terminal_provider::validate_event(&bound.context, &event)
            .map_err(|source| Error::ProviderEvent { source })?;
        let process = self
            .processes
            .get(&terminal)
            .ok_or(Error::ProviderBinding {
                terminal,
                reason: "terminal no longer exists",
            })?;
        if matches!(event.signal, ProviderSignal::SessionStarted { .. }) {
            let hint = RecoveryHint {
                provider: bound.context.provider.clone(),
                session_id: event.session_id.clone(),
                working_directory: event
                    .cwd
                    .clone()
                    .unwrap_or_else(|| process.info.cwd.clone()),
                state_paths: bound.context.state_paths.clone(),
            };
            validate_recovery(Some(&hint))?;
            let mut settings = self.settings.clone();
            settings
                .get_mut(&terminal)
                .ok_or(Error::ProviderBinding {
                    terminal,
                    reason: "terminal settings are unavailable",
                })?
                .recovery = Some(hint.clone());
            let store = self.store.as_mut().ok_or(Error::ProviderBinding {
                terminal,
                reason: "host is launching another terminal; retry after launch completes",
            })?;
            let old = process.settings.recovery.clone();
            store.set_recovery(terminal, Some(hint))?;
            if let Err(error) = self.recovery.save_settings(&settings) {
                // No observer event on failure. Preserve native store identity
                // when rollback succeeds; a retry is idempotent if it does not.
                let _ = store.set_recovery(terminal, old);
                return Err(error);
            }
            self.settings = settings;
            if let (Some(p), Some(settings)) = (
                self.processes.get_mut(&terminal),
                self.settings.get(&terminal),
            ) {
                p.settings = settings.clone();
                p.info.recovery_available = true;
            }
            self.sessions();
        } else if !process.settings.recovery.as_ref().is_some_and(|hint| {
            hint.provider == event.provider && hint.session_id == event.session_id
        }) {
            return Err(Error::ProviderBinding {
                terminal,
                reason: "event belongs to a different or not-yet-started provider session",
            });
        }
        self.provider_state
            .observers
            .retain(|o| !o.sender.is_closed());
        for observer in &self.provider_state.observers {
            if let Err(mpsc::error::TrySendError::Full(_)) =
                observer.sender.try_send((terminal, event.clone()))
            {
                observer.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(())
    }
}
