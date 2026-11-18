//! The host stops and starts a box's sessions (D262, D265).
//!
//! A stop is kept on the host first, as a copy that has not reached the box
//! (`host_copy`). It then goes to the box two ways at once: over the peer
//! link, which is quick when the link is healthy, and as a one-off SSH that
//! runs `humatad2 stop` on the box, which is how it gets past a link that
//! is flooded by the box's agents or hangs. The SSH is held back for a
//! moment so a healthy link carries the stop alone. A stop neither way
//! reaches is sent when the box next links, once, and the host keeps it
//! across its own restarts until the box has it.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_machines::{Exit, ManageError, RunError, Runner, SystemRunner};
use tonic::transport::Channel;
use tonic::{Code, Request, Status};

use super::host_copy::{HostCopies, HostCopy, HostCopyError};
use super::service::unix_ms;
use super::{ByKind, Ended, Record, StoppedBy, Via};
use crate::channel_service::failed;
use crate::host_relay::{LinkState, RelayError};
use crate::machine_service::{Work, lock};
use crate::status;

/// How long the box has to answer a stop sent over its link.
const LINK_DEADLINE: Duration = Duration::from_secs(3);
/// How long a healthy link has to carry a stop alone before the one-off SSH
/// starts beside it.
const SSH_AFTER: Duration = Duration::from_secs(1);
/// How long a start waits for a box's link to come back, when a stop has
/// just ended it.
const START_LINK_WAIT: Duration = Duration::from_secs(30);
/// How long a start waits before sending again a call that died with its
/// connection.
const RESEND_AFTER: Duration = Duration::from_secs(1);
/// When a start first has the host dial the box again, and the longest
/// gap between those dials.
const REDIAL_FIRST: Duration = Duration::from_secs(1);
const REDIAL_MOST: Duration = Duration::from_secs(8);

/// The host's copies of its boxes' stops: on disk, and in memory for the
/// reads that list machines and sessions.
#[derive(Clone)]
pub(crate) struct HostStops {
    copies: HostCopies,
    /// The host's own stop record, for This Mac's page.
    own: super::StopFolder,
    kept: Arc<Mutex<BTreeMap<String, HostCopy>>>,
}

impl HostStops {
    /// Opens the copies in the state folder `state`. A copy that can't be
    /// read is logged and left out: the machine shows running until it is
    /// fixed, and the file is not touched.
    pub(crate) fn open(state: &Path) -> Self {
        let copies = HostCopies::new(state);
        let kept = match copies.all() {
            Ok(all) => all.into_iter().collect(),
            Err(error) => {
                crate::log::say!(
                    "humatad2: the stops kept for other machines couldn't be read, so none is shown: {}",
                    status::chain(&error)
                );
                BTreeMap::new()
            }
        };
        Self {
            copies,
            own: super::StopFolder::new(state),
            kept: Arc::new(Mutex::new(kept)),
        }
    }

    /// The host's own stop, while its sessions are stopped (D261). Read
    /// from its record, which the sweep fills in with what ended.
    pub(crate) fn this_machine(&self) -> Option<pb::SessionsStopped> {
        if !super::gate::is_stopped() {
            return None;
        }
        match self.own.read() {
            Ok(super::StopState::Stopped(record)) => Some(super::boot::to_pb(&record)),
            Ok(super::StopState::Running | super::StopState::Starting(_)) => None,
            Err(error) => {
                crate::log::say!(
                    "humatad2: This Mac's stop record couldn't be read, so its page shows no stop: {}",
                    status::chain(&error)
                );
                None
            }
        }
    }

    /// `machine`'s copy, if the host keeps one.
    pub(crate) fn get(&self, machine: &str) -> Option<HostCopy> {
        lock(&self.kept).get(machine).cloned()
    }

    /// Whether `machine`'s stop reached it.
    pub(crate) fn is_delivered(&self, machine: &str) -> bool {
        lock(&self.kept).get(machine).is_some_and(|copy| copy.delivered)
    }

    /// Keeps `copy` as `machine`'s. Blocks.
    fn save(&self, machine: &str, copy: &HostCopy) -> Result<(), HostCopyError> {
        self.copies.write(machine, copy)?;
        lock(&self.kept).insert(machine.to_owned(), copy.clone());
        Ok(())
    }

    /// Marks `machine`'s copy delivered when it is stop `stop_id`. Blocks.
    fn mark_delivered(&self, machine: &str, stop_id: &str) -> Result<bool, HostCopyError> {
        let changed = self.copies.mark_delivered(machine, stop_id)?;
        if changed && let Some(copy) = lock(&self.kept).get_mut(machine) {
            copy.delivered = true;
        }
        Ok(changed)
    }

    /// Drops `machine`'s copy. Blocks.
    pub(crate) fn remove(&self, machine: &str) -> Result<(), HostCopyError> {
        self.copies.remove(machine)?;
        lock(&self.kept).remove(machine);
        Ok(())
    }
}

/// What the user asks of a stop.
pub(crate) struct Ask {
    pub(crate) stop_id: String,
    pub(crate) reason: String,
    pub(crate) via: Via,
    /// This host's machine id: who asked.
    pub(crate) by: String,
}

/// What a stop came to.
pub(crate) struct Reach {
    pub(crate) record: Record,
    /// The box has the stop.
    pub(crate) delivered: bool,
}

/// What a start came to.
pub(crate) enum Started {
    /// The box never had the stop, so it was only cancelled.
    Cancelled,
    /// The box was told to start its sessions again.
    Told,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RemoteError {
    #[error(
        "the host isn't keeping stops yet because it is still starting; nothing was stopped. Try again in a moment"
    )]
    NotReady,

    #[error("couldn't keep the stop of {machine} on this Mac")]
    Copy {
        machine: String,
        #[source]
        source: HostCopyError,
    },

    #[error("a background task failed while stopping {machine}")]
    Task {
        machine: String,
        #[source]
        source: tokio::task::JoinError,
    },

    #[error("the sessions on {machine} aren't stopped, so there is nothing to start")]
    NotStopped { machine: String },

    #[error("{machine} can't be reached to start its sessions")]
    Offline {
        machine: String,
        #[source]
        source: RelayError,
    },

    #[error("{machine} refused to start its sessions")]
    Refused {
        machine: String,
        #[source]
        source: Status,
    },
}

impl RemoteError {
    /// The error as the status the caller gets, saying what to do.
    pub(crate) fn status(&self) -> Status {
        match self {
            Self::NotReady => failed(Code::Unavailable, self, "wait a moment and stop again", None),
            Self::Copy { .. } => failed(
                Code::Internal,
                self,
                "check that this Mac's disk has room and Humata's folder belongs to you, then stop again; the machine's sessions were not stopped",
                None,
            ),
            Self::Task { .. } => failed(Code::Internal, self, "stop again", None),
            Self::NotStopped { .. } => failed(
                Code::FailedPrecondition,
                self,
                "nothing to do; the machine's sessions are running",
                None,
            ),
            Self::Offline { machine, .. } => failed(
                Code::Unavailable,
                self,
                &format!(
                    "{machine} stays stopped until it links: use Reconnect now on its page once it is reachable, then start its sessions"
                ),
                None,
            ),
            Self::Refused { source, .. } => failed(
                source.code(),
                self,
                "start again; if it keeps failing, run `humata start` on the machine itself",
                None,
            ),
        }
    }
}

/// Why one way of reaching a box failed. Said in the log; the user is told
/// only that the stop is kept for the box's next link.
#[derive(Debug, thiserror::Error)]
enum Attempt {
    #[error("{machine}'s link is down")]
    LinkDown {
        machine: String,
        #[source]
        source: RelayError,
    },
    #[error("{machine}'s link didn't answer within {} seconds", LINK_DEADLINE.as_secs())]
    LinkSlow { machine: String },
    #[error("{machine}'s link refused the stop")]
    LinkCall {
        machine: String,
        #[source]
        source: Status,
    },
    #[error("couldn't make the SSH command that stops {machine}")]
    SshCommand {
        machine: String,
        #[source]
        source: ManageError,
    },
    #[error("couldn't run the SSH command that stops {machine}")]
    SshRun {
        machine: String,
        #[source]
        source: RunError,
    },
    #[error("the SSH command that stops {machine} ended with {exit}: {tail}")]
    SshExit { machine: String, exit: Exit, tail: String },
    #[error("the SSH command that stops {machine} was lost")]
    SshTask {
        machine: String,
        #[source]
        source: tokio::task::JoinError,
    },
}

/// Stops `machine`'s sessions as `ask` says. The stop is kept before
/// anything is sent; a stop the machine already holds (delivered) is
/// answered as it is, and one kept but not delivered is sent again.
pub(crate) async fn stop(work: &Arc<Work>, machine: &str, ask: Ask) -> Result<Reach, RemoteError> {
    let stops = work.stops.get().ok_or(RemoteError::NotReady)?.clone();
    let record = match stops.get(machine) {
        Some(copy) if copy.delivered => {
            return Ok(Reach {
                record: copy.record,
                delivered: true,
            });
        }
        Some(copy) => copy.record,
        None => {
            let record = Record {
                stop_id: if ask.stop_id.is_empty() {
                    uuid::Uuid::new_v4().to_string()
                } else {
                    ask.stop_id
                },
                stopped_at_unix_ms: unix_ms(),
                by: StoppedBy {
                    kind: ByKind::User,
                    via: ask.via,
                    machine: ask.by,
                    session: None,
                },
                reason: Some(ask.reason).filter(|reason| !reason.is_empty()),
                ended: None,
            };
            let copy = HostCopy {
                record: record.clone(),
                delivered: false,
            };
            blocking(&stops, machine, move |stops, machine| stops.save(machine, &copy)).await?;
            crate::fault::point("stop.remote.copy_written");
            work.publish_machines();
            record
        }
    };
    let delivered = send(work, machine, &record).await;
    if let Some(told) = delivered {
        let record = settle(work, &stops, machine, record, told).await?;
        return Ok(Reach {
            record,
            delivered: true,
        });
    }
    Ok(Reach {
        record,
        delivered: false,
    })
}

/// Starts `machine`'s sessions again, or cancels a stop it never got.
pub(crate) async fn start(work: &Arc<Work>, machine: &str) -> Result<Started, RemoteError> {
    let stops = work.stops.get().ok_or(RemoteError::NotReady)?.clone();
    let Some(copy) = stops.get(machine) else {
        return Err(RemoteError::NotStopped {
            machine: machine.to_owned(),
        });
    };
    let started = if copy.delivered {
        tell_start(work, machine).await?;
        crate::fault::point("start.remote.sent");
        Started::Told
    } else {
        Started::Cancelled
    };
    blocking(&stops, machine, |stops, machine| stops.remove(machine)).await?;
    crate::fault::point("start.remote.copy_removed");
    work.publish_machines();
    Ok(started)
}

/// Tells `machine` to start its sessions, within [`START_LINK_WAIT`]. A
/// stop makes the box's daemon start itself again, and so does the start,
/// so the link the host holds may be one that is closing or gone; a call
/// that dies with its connection is sent again a moment later, over the
/// newest link. Sending it again is safe: a box that already started
/// answers that it isn't stopped.
async fn tell_start(work: &Arc<Work>, machine: &str) -> Result<(), RemoteError> {
    let links = work.links();
    let deadline = tokio::time::Instant::now() + START_LINK_WAIT;
    let mut redial = Redial::new();
    // Why the last call failed, for when time is up.
    let mut lost: Option<Status> = None;
    loop {
        // Watched before the call, so a link that ends during it is seen.
        let mut state = links.watch(machine);
        let channel = match links.channel(machine) {
            Ok(channel) => channel,
            Err(offline) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(match lost {
                        Some(source) => RemoteError::Refused {
                            machine: machine.to_owned(),
                            source,
                        },
                        None => RemoteError::Offline {
                            machine: machine.to_owned(),
                            source: offline,
                        },
                    });
                }
                redial.if_due(work, machine);
                changed_or(&mut state, redial.next.min(deadline)).await;
                continue;
            }
        };
        let told = pb::daemon_service_client::DaemonServiceClient::new(channel)
            .start_sessions(Request::new(pb::StartSessionsRequest {}))
            .await;
        match told {
            Ok(_) => return Ok(()),
            // It isn't stopped any more: started on the box itself, or by
            // a call whose answer was lost with its connection.
            Err(status) if status.code() == Code::FailedPrecondition => return Ok(()),
            Err(status) if connection_lost(&status) && tokio::time::Instant::now() < deadline => {
                // The link may be dead while the host still holds it:
                // dialled again now, not after the link's backoff.
                redial.if_due(work, machine);
                let resend = (tokio::time::Instant::now() + RESEND_AFTER).min(deadline);
                changed_or(&mut state, resend).await;
                lost = Some(status);
            }
            Err(source) => {
                return Err(RemoteError::Refused {
                    machine: machine.to_owned(),
                    source,
                });
            }
        }
    }
}

/// When a start next has the host dial the box again. The gaps grow, so a
/// dial that takes a while is not cut short by the next one each time.
struct Redial {
    next: tokio::time::Instant,
    gap: Duration,
}

impl Redial {
    fn new() -> Self {
        Self {
            next: tokio::time::Instant::now() + REDIAL_FIRST,
            gap: REDIAL_FIRST,
        }
    }

    /// Dials `machine` again now, if it is time to.
    fn if_due(&mut self, work: &Arc<Work>, machine: &str) {
        let now = tokio::time::Instant::now();
        if now < self.next {
            return;
        }
        work.link_up(machine, &tokio::runtime::Handle::current());
        self.gap = (self.gap * 2).min(REDIAL_MOST);
        self.next = now + self.gap;
    }
}

/// Waits for the link's state to change, or until `wake`. A link whose
/// watch has ended waits until `wake` too, so the caller doesn't spin.
async fn changed_or(state: &mut tokio::sync::watch::Receiver<LinkState>, wake: tokio::time::Instant) {
    if let Ok(Err(_)) = tokio::time::timeout_at(wake, state.changed()).await {
        tokio::time::sleep_until(wake).await;
    }
}

/// Whether a call failed because its connection went, not because the box
/// answered no.
fn connection_lost(status: &Status) -> bool {
    matches!(status.code(), Code::Cancelled | Code::Unavailable)
}

/// What a link coming up tells of the box's stop (D262): the host copy is
/// made to agree before the machine shows online, and a stop the box never
/// got is sent now.
pub(crate) fn link_up(work: &Arc<Work>, machine: &str, told: Option<&pb::SessionsStopped>, channel: &Channel) {
    let Some(stops) = work.stops.get() else {
        return;
    };
    let told_record = told.map(record_of);
    let copy = stops.get(machine);
    match reconcile(copy.as_ref(), told_record.as_ref()) {
        Step::Nothing => {}
        Step::Adopt(record) => {
            let copy = HostCopy { record, delivered: true };
            if let Err(error) = stops.save(machine, &copy) {
                crate::log::say!("humatad2: couldn't keep the stop {machine} reported: {}", status::chain(&error));
            }
        }
        Step::Drop => {
            if let Err(error) = stops.remove(machine) {
                crate::log::say!(
                    "humatad2: couldn't drop the stop kept for {machine}, whose sessions are running: {}",
                    status::chain(&error)
                );
            }
        }
        Step::Send(record) => {
            let (work, stops, channel, machine) = (work.clone(), stops.clone(), channel.clone(), machine.to_owned());
            tokio::spawn(async move {
                match over_link(&channel, &machine, &record).await {
                    Ok(told) => {
                        if let Err(error) = settle(&work, &stops, &machine, record, told).await {
                            crate::log::say!(
                                "humatad2: {machine} took the stop kept for it but the host couldn't note that: {}",
                                status::chain(&error)
                            );
                        }
                    }
                    Err(attempt) => crate::log::say!(
                        "humatad2: the stop kept for {machine} wasn't sent on its new link, and is sent again when it links next: {}",
                        status::chain(&attempt)
                    ),
                }
            });
        }
    }
}

/// What a link coming up does about the host's copy.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Nothing,
    /// The box says it is stopped: the host's copy becomes that stop,
    /// delivered.
    Adopt(Record),
    /// The box is running: a copy that had been delivered is stale.
    Drop,
    /// The box is running and never got this stop.
    Send(Record),
}

/// Compares the host's copy with what the box's Hello told.
fn reconcile(copy: Option<&HostCopy>, told: Option<&Record>) -> Step {
    match (copy, told) {
        (Some(copy), Some(told))
            if copy.delivered
                && copy.record.stop_id == told.stop_id
                && (copy.record.ended.is_some() || told.ended.is_none()) =>
        {
            Step::Nothing
        }
        (_, Some(told)) => Step::Adopt(told.clone()),
        (Some(copy), None) if copy.delivered => Step::Drop,
        (Some(copy), None) => Step::Send(copy.record.clone()),
        (None, None) => Step::Nothing,
    }
}

/// Notes that the box has the stop `sent`: `told` is what the box answered,
/// which names a stop of its own when it was stopped already.
async fn settle(
    work: &Arc<Work>,
    stops: &HostStops,
    machine: &str,
    sent: Record,
    told: Option<pb::SessionsStopped>,
) -> Result<Record, RemoteError> {
    crate::fault::point("stop.remote.sent");
    // A box that was stopped already answers with that stop, which stands.
    let theirs = told.as_ref().map(record_of).filter(|theirs| theirs.stop_id != sent.stop_id);
    let record = match theirs {
        Some(theirs) => {
            let copy = HostCopy {
                record: theirs.clone(),
                delivered: true,
            };
            blocking(stops, machine, move |stops, machine| stops.save(machine, &copy)).await?;
            theirs
        }
        None => {
            let stop_id = sent.stop_id.clone();
            blocking(stops, machine, move |stops, machine| stops.mark_delivered(machine, &stop_id)).await?;
            sent
        }
    };
    crate::fault::point("stop.remote.delivered");
    work.publish_machines();
    Ok(record)
}

/// Sends the stop over the link and the one-off SSH; the box's answer when
/// either reached it (`Some(None)` when SSH did, which says nothing of its
/// own), none when neither did.
async fn send(work: &Arc<Work>, machine: &str, record: &Record) -> Option<Option<pb::SessionsStopped>> {
    let links = work.links();
    let link = async {
        match links.channel(machine) {
            Ok(channel) => over_link(&channel, machine, record).await,
            Err(source) => Err(Attempt::LinkDown {
                machine: machine.to_owned(),
                source,
            }),
        }
    };
    tokio::pin!(link);
    let mut failures = Vec::new();
    // A healthy link carries the stop by itself, so the SSH is not started
    // beside it, where a stop it delivers late could meet a daemon that is
    // starting itself again.
    match tokio::time::timeout(SSH_AFTER, &mut link).await {
        Ok(Ok(told)) => return Some(told),
        Ok(Err(attempt)) => {
            failures.push(attempt);
            return match over_ssh(work, machine, record).await {
                Ok(()) => Some(None),
                Err(attempt) => {
                    failures.push(attempt);
                    report(machine, &failures);
                    None
                }
            };
        }
        Err(_) => {}
    }
    let ssh = over_ssh(work, machine, record);
    tokio::pin!(ssh);
    let (mut link_done, mut ssh_done) = (false, false);
    while !(link_done && ssh_done) {
        tokio::select! {
            result = &mut link, if !link_done => {
                link_done = true;
                match result {
                    Ok(told) => return Some(told),
                    Err(attempt) => failures.push(attempt),
                }
            }
            result = &mut ssh, if !ssh_done => {
                ssh_done = true;
                match result {
                    Ok(()) => return Some(None),
                    Err(attempt) => failures.push(attempt),
                }
            }
        }
    }
    report(machine, &failures);
    None
}

fn report(machine: &str, failures: &[Attempt]) {
    for attempt in failures {
        crate::log::say!(
            "humatad2: the stop of {machine} wasn't delivered; it is kept and sent when the machine links: {}",
            status::chain(attempt)
        );
    }
}

/// The stop over `channel`, the box's link: its own answer. A box that
/// refuses it as already applied or already stopped has it.
async fn over_link(channel: &Channel, machine: &str, record: &Record) -> Result<Option<pb::SessionsStopped>, Attempt> {
    let request = pb::StopSessionsRequest {
        stop_id: record.stop_id.clone(),
        reason: record.reason.clone().unwrap_or_default(),
        via: stop_via(&record.by.via) as i32,
        by_machine: record.by.machine.clone(),
    };
    let mut client = pb::daemon_service_client::DaemonServiceClient::new(channel.clone());
    match tokio::time::timeout(LINK_DEADLINE, client.stop_sessions(Request::new(request))).await {
        Err(_) => Err(Attempt::LinkSlow {
            machine: machine.to_owned(),
        }),
        Ok(Ok(reply)) => Ok(reply.into_inner().stopped),
        Ok(Err(status)) if status.code() == Code::FailedPrecondition => Ok(None),
        Ok(Err(source)) => Err(Attempt::LinkCall {
            machine: machine.to_owned(),
            source,
        }),
    }
}

/// The stop as `humatad2 stop` run on the box over a one-off SSH.
async fn over_ssh(work: &Arc<Work>, machine: &str, record: &Record) -> Result<(), Attempt> {
    let invocation = work
        .machines
        .stop_sessions_command(
            machine,
            &record.stop_id,
            &record.by.machine,
            record.reason.as_deref().unwrap_or_default(),
        )
        .map_err(|source| Attempt::SshCommand {
            machine: machine.to_owned(),
            source,
        })?;
    let ran = tokio::task::spawn_blocking(move || SystemRunner::new(&()).run(&invocation))
        .await
        .map_err(|source| Attempt::SshTask {
            machine: machine.to_owned(),
            source,
        })?
        .map_err(|source| Attempt::SshRun {
            machine: machine.to_owned(),
            source,
        })?;
    if ran.exit.success() {
        Ok(())
    } else {
        Err(Attempt::SshExit {
            machine: machine.to_owned(),
            exit: ran.exit,
            tail: ran.tail(5),
        })
    }
}

/// Runs `work` on a blocking thread, as the host's file work is.
async fn blocking<R: Send + 'static>(
    stops: &HostStops,
    machine: &str,
    work: impl FnOnce(&HostStops, &str) -> Result<R, HostCopyError> + Send + 'static,
) -> Result<R, RemoteError> {
    let (stops, name) = (stops.clone(), machine.to_owned());
    let machine = machine.to_owned();
    tokio::task::spawn_blocking(move || work(&stops, &name))
        .await
        .map_err(|source| RemoteError::Task {
            machine: machine.clone(),
            source,
        })?
        .map_err(|source| RemoteError::Copy { machine, source })
}

fn stop_via(via: &Via) -> pb::StopVia {
    match via {
        Via::App => pb::StopVia::App,
        Via::Cli => pb::StopVia::Cli,
        Via::Ssh => pb::StopVia::Ssh,
        Via::Direct => pb::StopVia::Direct,
    }
}

/// The record a Hello's stop tells. The count of processes left is kept as
/// that many unnamed entries, which is all the host shows of it.
fn record_of(told: &pb::SessionsStopped) -> Record {
    let by = told.by.clone().unwrap_or_default();
    Record {
        stop_id: told.stop_id.clone(),
        stopped_at_unix_ms: told.stopped_at_unix_ms,
        by: StoppedBy {
            kind: match by.kind() {
                pb::StoppedByKind::Agent => ByKind::Agent,
                pb::StoppedByKind::Terminal => ByKind::Terminal,
                pb::StoppedByKind::Script => ByKind::Script,
                pb::StoppedByKind::User | pb::StoppedByKind::Unspecified => ByKind::User,
            },
            via: match by.via() {
                pb::StopVia::App => Via::App,
                pb::StopVia::Cli => Via::Cli,
                pb::StopVia::Ssh => Via::Ssh,
                pb::StopVia::Direct | pb::StopVia::Unspecified => Via::Direct,
            },
            machine: by.machine,
            session: Some(by.session).filter(|session| !session.is_empty()),
        },
        reason: Some(told.reason.clone()).filter(|reason| !reason.is_empty()),
        ended: told.ended.as_ref().map(|ended| Ended {
            agents: ended.agents,
            shells: ended.shells,
            killed: ended.killed,
            left: vec![0; usize::try_from(ended.left).unwrap_or(0)],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stop::boot::to_pb;

    fn record(stop_id: &str, ended: bool) -> Record {
        Record {
            stop_id: stop_id.to_owned(),
            stopped_at_unix_ms: 1_763_300_000_000,
            by: StoppedBy {
                kind: ByKind::User,
                via: Via::App,
                machine: "host-mac".to_owned(),
                session: None,
            },
            reason: Some("agents running wild".to_owned()),
            ended: ended.then(|| Ended {
                agents: 2,
                shells: 1,
                killed: 0,
                left: vec![],
            }),
        }
    }

    fn copy(stop_id: &str, delivered: bool) -> HostCopy {
        HostCopy {
            record: record(stop_id, false),
            delivered,
        }
    }

    #[test]
    fn a_stop_the_box_never_got_is_sent_when_it_links_running() {
        assert_eq!(reconcile(Some(&copy("s1", false)), None), Step::Send(record("s1", false)));
    }

    #[test]
    fn a_stop_the_box_took_is_noted_when_its_hello_names_it() {
        let told = record("s1", true);
        assert_eq!(reconcile(Some(&copy("s1", false)), Some(&told)), Step::Adopt(told));
    }

    #[test]
    fn a_stop_made_on_the_box_is_kept_by_the_host_when_it_links() {
        let told = record("s9", false);
        assert_eq!(reconcile(None, Some(&told)), Step::Adopt(told.clone()));
        // The box's own stop wins over another the host had kept.
        assert_eq!(reconcile(Some(&copy("s1", true)), Some(&told)), Step::Adopt(told));
    }

    #[test]
    fn a_delivered_stop_the_box_no_longer_has_is_dropped() {
        assert_eq!(reconcile(Some(&copy("s1", true)), None), Step::Drop);
    }

    #[test]
    fn a_box_and_host_that_agree_change_nothing() {
        assert_eq!(reconcile(None, None), Step::Nothing);
        assert_eq!(reconcile(Some(&copy("s1", true)), Some(&record("s1", false))), Step::Nothing);
    }

    #[test]
    fn what_the_box_finished_ending_replaces_the_copy_made_before_it_did() {
        let told = record("s1", true);
        assert_eq!(reconcile(Some(&copy("s1", true)), Some(&told)), Step::Adopt(told));
    }

    #[test]
    fn a_stop_the_box_tells_reads_back_as_the_record_it_came_from() {
        let mut original = record("s1", true);
        original.by.kind = ByKind::Terminal;
        original.by.via = Via::Ssh;
        original.by.session = Some("se_1".to_owned());
        assert_eq!(record_of(&to_pb(&original)), original);
    }
}
