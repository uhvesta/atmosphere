//! The host's mirror of sessions that run on other machines (LD-8, TR-4,
//! D189): when a machine's link comes up or the machine reports on a
//! session, the host asks that machine for the next part of the session's
//! conversation from the offset its mirror saved, and adds the answer.
//!
//! Nothing polls (TR-7). A read that fails is logged and waits for the next
//! link-up or report. Each machine has a task of its own that reads its
//! sessions one at a time, so a slow or dead machine never holds up another
//! machine or the follower of this host's own sessions (TR-6).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use humata2_chunk::{ChunkError, Decoder};
use humata2_grpc::humata::v1 as pb;
use humata2_mirror::{Chunk, MirrorError};
use humata2_store::StoreError;
use tokio::runtime::Handle;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::mpsc;
use tonic::Status;

use super::source::REMOTE;
use super::{Shared, SyncError};
use crate::channel_service::blocking;
use crate::compression;
use crate::host_relay::{Follow, LinkState, Links};
use crate::peer_reports::ReportUpdate;
use crate::status;

/// Why a session on another machine couldn't be caught up. Logged, never
/// shown to a caller: the next link-up or report from the machine tries again.
#[derive(Debug, thiserror::Error)]
pub(super) enum RemoteError {
    #[error("couldn't read session {session}'s record from the store; check that the state folder is readable and has free space")]
    Store {
        session: String,
        #[source]
        source: StoreError,
    },

    #[error("couldn't name session {session} to its mirror")]
    Describe {
        session: String,
        #[source]
        source: SyncError,
    },

    #[error("couldn't read or update the mirror of session {session} on {machine}; check that the state folder is readable and has free space. What was mirrored is kept")]
    Mirror {
        session: String,
        machine: String,
        #[source]
        source: MirrorError,
    },

    #[error("machine {machine} didn't give session {session}'s conversation; the next time its link comes up or it reports on the session the host asks again")]
    Call {
        machine: String,
        session: String,
        #[source]
        source: Status,
    },

    #[error("machine {machine}'s part of session {session}'s conversation couldn't be decoded; it was not mirrored and the next read asks for it again. If this repeats, update the daemon on {machine}")]
    Decode {
        machine: String,
        session: String,
        #[source]
        source: ChunkError,
    },

    #[error("machine {machine} sent session {session}'s conversation in codec {codec}, which this host doesn't know; update the host or the daemon on {machine}")]
    UnknownCodec {
        machine: String,
        session: String,
        codec: i32,
    },

    #[error("machine {machine} said session {session} has more after byte {from} but its next offset is {next}, which is no further; the host stopped asking. Report this as a bug in the daemon on {machine}")]
    Stalled {
        machine: String,
        session: String,
        from: u64,
        next: u64,
    },

    #[error("a background task failed while mirroring session {session}")]
    Task {
        session: String,
        #[source]
        source: Status,
    },
}

/// The per-machine tasks that read other machines' conversations.
pub(super) struct Remote {
    handle: Handle,
    shared: Weak<Shared>,
    links: Links,
    /// A machine's task, fed the sessions to read.
    workers: Mutex<BTreeMap<String, mpsc::UnboundedSender<String>>>,
}

impl Remote {
    pub(super) fn new(handle: Handle, shared: Weak<Shared>, links: Links) -> Self {
        Self {
            handle,
            shared,
            links,
            workers: Mutex::new(BTreeMap::new()),
        }
    }

    /// Asks `machine`'s task to catch `session` up. Never waits.
    pub(super) fn trigger(&self, machine: &str, session: &str) {
        let mut workers = self.workers.lock().unwrap_or_else(PoisonError::into_inner);
        let sender = workers.entry(machine.to_owned()).or_insert_with(|| {
            let (send, inbox) = mpsc::unbounded_channel();
            self.handle.spawn(work(
                self.shared.clone(),
                self.links.clone(),
                machine.to_owned(),
                inbox,
            ));
            send
        });
        let _ = sender.send(session.to_owned());
    }

    /// Triggers every session of the host's store that runs on `machine`.
    async fn trigger_all(&self, machine: &str) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        let listed = blocking(move || shared.store.sessions()).await;
        match listed {
            Ok(Ok(records)) => {
                for record in records.into_iter().filter(|record| record.machine == machine) {
                    self.trigger(machine, &record.id);
                }
            }
            Ok(Err(error)) => crate::log::say!(
                "humatad2: couldn't list sessions to mirror {machine}'s history: {}",
                status::chain(&error)
            ),
            Err(error) => crate::log::say!("humatad2: couldn't mirror {machine}'s history: {error}"),
        }
    }

    /// Every machine whose link is up now.
    async fn trigger_all_up(&self) {
        for (machine, state) in self.links.states() {
            if matches!(state, LinkState::Up { .. }) {
                self.trigger_all(&machine).await;
            }
        }
    }
}

/// Follows the machines' links and reports for the service's life: a link
/// coming up reads that machine's sessions (the catch-up after a reconnect or
/// a host restart), and a machine's report on a session reads that session.
/// Both subscriptions were made before this started, so nothing falls between.
pub(super) async fn dispatch(
    remote: Arc<Remote>,
    mut follow: Follow,
    mut updates: broadcast::Receiver<Arc<ReportUpdate>>,
) {
    remote.trigger_all_up().await;
    loop {
        tokio::select! {
            change = follow.next() => match change {
                None => return,
                Some((machine, LinkState::Up { .. })) => remote.trigger_all(&machine).await,
                Some((_, LinkState::Down)) => {}
            },
            update = updates.recv() => match update {
                Ok(update) => {
                    if let ReportUpdate::Session { machine, session, .. } = &*update {
                        remote.trigger(machine, session);
                    }
                }
                // Reports missed while behind: ask for everything that is up.
                Err(RecvError::Lagged(_)) => remote.trigger_all_up().await,
                Err(RecvError::Closed) => return,
            },
        }
        if remote.shared.strong_count() == 0 {
            return;
        }
    }
}

/// One machine's task: its sessions one at a time, each at most once per
/// round however many times it was triggered.
async fn work(
    shared: Weak<Shared>,
    links: Links,
    machine: String,
    mut inbox: mpsc::UnboundedReceiver<String>,
) {
    while let Some(first) = inbox.recv().await {
        let mut sessions = BTreeSet::from([first]);
        while let Ok(next) = inbox.try_recv() {
            sessions.insert(next);
        }
        for session in sessions {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            if let Err(error) = sync(&shared, &links, &machine, &session).await {
                crate::log::say!(
                    "humatad2: couldn't keep session {session}'s history from {machine}: {}",
                    status::chain(&error)
                );
            }
        }
    }
}

/// Reads session `session`'s conversation from `machine` until nothing more
/// is waiting there. A machine that is offline is not an error: its link
/// coming up is the trigger.
async fn sync(
    shared: &Arc<Shared>,
    links: &Links,
    machine: &str,
    session: &str,
) -> Result<(), RemoteError> {
    let named = {
        let (shared, id, machine) = (shared.clone(), session.to_owned(), machine.to_owned());
        blocking(move || -> Result<_, RemoteError> {
            let Some(record) = shared.store.session(&id).map_err(|source| RemoteError::Store {
                session: id.clone(),
                source,
            })?
            else {
                // The follower of this host's sessions forgets it (LD-22).
                return Ok(None);
            };
            if record.machine != machine {
                return Ok(None);
            }
            shared
                .describe(&record)
                .map(Some)
                .map_err(|source| RemoteError::Describe { session: id, source })
        })
        .await
        .map_err(|source| RemoteError::Task {
            session: session.to_owned(),
            source,
        })??
    };
    let Some(mirror) = named else {
        return Ok(());
    };
    let mirror_failed = |source| RemoteError::Mirror {
        session: session.to_owned(),
        machine: machine.to_owned(),
        source,
    };
    let task_failed = |source| RemoteError::Task {
        session: session.to_owned(),
        source,
    };
    let mut decoder = Decoder::new();
    loop {
        // A session mirrored before the host kept raw lines (D195) reads its
        // machine's file again for them; `stop` ends each such read where the
        // entries already are.
        let (from, source, stop) = {
            let mirror = mirror.clone();
            let id = session.to_owned();
            blocking(move || -> Result<_, MirrorError> {
                if !mirror.keeps_raw_lines(&id)? {
                    mirror.begin_raw_backfill(&id)?;
                }
                let (from, source) = mirror.source_offset(&id)?;
                Ok((from, source, mirror.raw_backfill_stop(&id)?))
            })
            .await
            .map_err(task_failed)?
            .map_err(mirror_failed)?
        };
        let max_bytes = stop.map_or(0, |stop| {
            u32::try_from(stop.saturating_sub(from)).unwrap_or(u32::MAX)
        });
        let Ok(channel) = links.channel(machine) else {
            return Ok(());
        };
        let request = pb::ReadMirrorRequest {
            session: Some(pb::SessionId {
                value: session.to_owned(),
            }),
            offset: from,
            max_bytes,
            source,
            accept_codecs: shared.compression.offered(REMOTE),
        };
        let answer = pb::history_service_client::HistoryServiceClient::new(channel)
            .read_mirror(request)
            .await
            .map_err(|source| RemoteError::Call {
                machine: machine.to_owned(),
                session: session.to_owned(),
                source,
            })?
            .into_inner();

        let header = compression::header(answer.codec, answer.raw_size, answer.crc32, &answer.data)
            .ok_or_else(|| RemoteError::UnknownCodec {
                machine: machine.to_owned(),
                session: session.to_owned(),
                codec: answer.codec,
            })?;
        let data = decoder
            .decode(header, answer.data)
            .map_err(|source| RemoteError::Decode {
                machine: machine.to_owned(),
                session: session.to_owned(),
                source,
            })?;
        let (more, rewritten, next_offset) = (answer.more, answer.rewritten, answer.next_offset);
        let start = if rewritten { 0 } else { from };
        let taken = {
            let mirror = mirror.clone();
            let id = session.to_owned();
            let new_source = answer.source;
            blocking(move || {
                mirror.take(
                    &id,
                    &Chunk {
                        from: start,
                        data: &data,
                        next_offset,
                        source: &new_source,
                        rewritten,
                    },
                )
            })
            .await
            .map_err(task_failed)?
            .map_err(mirror_failed)?
        };
        // None: another read got there first and the mirror is already past this part.
        if taken.is_none() {
            return Ok(());
        }
        if !more {
            return Ok(());
        }
        if next_offset <= start {
            return Err(RemoteError::Stalled {
                machine: machine.to_owned(),
                session: session.to_owned(),
                from: start,
                next: next_offset,
            });
        }
    }
}
