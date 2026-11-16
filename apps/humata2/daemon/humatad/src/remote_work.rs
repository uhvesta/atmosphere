//! The host's side of a channel across machines and a session on another
//! machine (CH-5, CH-7, CH-19, SL-1, SL-3, PER-11, HO-5, HO-6, TR-6, TR-7;
//! D139, D144).
//!
//! "A remote session is a local session plus a link": the host writes its
//! own record first, with an id it mints, then makes the SAME client call
//! (`CreateChannel`, `StartSession`, `StopSession`, `ResumeSession`) on the
//! machine's socket daemon through the peer link's relay, carrying the id as
//! `humata-host-assigned-id` ([`assigned`]). The machine runs its ordinary
//! local code and keeps its rows only as a launch cache. What it does
//! afterwards comes back on the Uplink as status and pointer reports
//! ([`PeerReports`]); nothing here polls or sleeps (TR-7).
//!
//! What the host's services hold is a [`RemoteWork`]. Its calls take a
//! [`HostOp`], the host's own operation, and mirror the machine's operation
//! into it step by step. A machine's steps are named for the machine, so
//! progress shows per machine and repository: `clone_main:<machine>:<owner/name>`
//! (D117 gave `clone_main:<owner/name>` for the local machine).
//!
//! Offline machines (CH-4, HO-5):
//! - a session can't be started, stopped or resumed there: the call answers
//!   the machine-offline error at once and nothing is queued (TR-6);
//! - a channel's setup there waits: the machine's steps are shown waiting
//!   and the call is made once when its link returns (CH-7, flow 26.21).
//!
//! After a dropped link the mirror follows the machine again on the new link
//! from its `WatchOperation`, or, when the machine restarted and forgot the
//! operation (operations live in memory), from `GetSession` and the
//! machine's reports. A start never makes a second session: it always names
//! the same id, and the machine answers a known id with the one it has.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{RemoteReportRow, StoreError};
use prost::Message;
use tokio::sync::{broadcast, mpsc};
use tonic::transport::Channel;
use tonic::{Code, Request, Status};

use crate::host_relay::{LinkState, Links, RelayError, assigned};
use crate::machine_service::lock;
use crate::operations::Progress;
use crate::peer_reports::{Freshness, PeerReports, ReportUpdate, SessionReport};
use crate::status;

/// How many times one link tries to bring a lost operation back before it
/// waits for the next link.
const MAX_ATTEMPTS: usize = 3;

/// Why a relayed call or a mirror couldn't be done.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RemoteError {
    /// The machine's link is down (HO-5). The message names Reconnect now.
    #[error(transparent)]
    Relay(#[from] RelayError),
    /// The machine's daemon answered with an error of its own. Its answer
    /// goes to the caller as it is: it already says what to do.
    #[error("{machine} couldn't {doing}: {reason}")]
    Machine {
        machine: String,
        doing: &'static str,
        reason: String,
        #[source]
        source: Status,
    },
    /// The machine made something under another id than the host asked for:
    /// its daemon is older than the host and ignores the host's ids.
    #[error(
        "{machine} made {got:?} where the host asked for {asked:?}, so it runs an older humatad2 that ignores the host's ids; update the Humata tools on {machine} from its page, then try again"
    )]
    IdNotHonoured {
        machine: String,
        asked: String,
        got: String,
    },
}

impl From<RemoteError> for Status {
    fn from(error: RemoteError) -> Self {
        let message = error.to_string();
        match error {
            RemoteError::Relay(relay) => Status::from(relay),
            RemoteError::Machine { source, .. } => source,
            RemoteError::IdNotHonoured { .. } => status::internal(message, ""),
        }
    }
}

/// A call to `machine` failed with `source`. A failure with no `humata.v1.Error`
/// in it is the link breaking under the call, which is offline, not an answer.
fn call_failed(machine: &str, doing: &'static str, source: Status) -> RemoteError {
    if status::error_of(&source).is_none() && matches!(source.code(), Code::Unavailable | Code::Unknown | Code::Cancelled) {
        return RemoteError::Relay(RelayError::Offline {
            machine: machine.to_owned(),
        });
    }
    RemoteError::Machine {
        machine: machine.to_owned(),
        doing,
        reason: source.message().to_owned(),
        source,
    }
}

// ---------------------------------------------------------------------------
// The host's operation, shared between the host's own work and the mirrors.

/// The host's operation, which one or more mirrors write steps into. The
/// caller keeps its own end through [`HostOp::take`] when it ends the
/// operation itself (a channel's setup ends when the local clones and every
/// machine's work have). A session's start or resume is ended here, by its
/// mirror.
pub(crate) struct HostOp {
    progress: Mutex<Option<Progress>>,
}

impl HostOp {
    pub(crate) fn new(progress: Progress) -> Self {
        Self {
            progress: Mutex::new(Some(progress)),
        }
    }

    /// Sets a step, adding it when the operation doesn't list it yet.
    pub(crate) fn set_step(&self, key: &str, state: pb::StepState, output_tail: Option<String>) {
        if let Some(progress) = lock(&self.progress).as_ref() {
            progress.set_step(key, state, output_tail);
        }
    }

    /// The operation as it is now; none once it has ended through this handle.
    pub(crate) fn operation(&self) -> Option<pb::Operation> {
        lock(&self.progress).as_ref().map(Progress::operation)
    }

    /// The caller's own end of the operation, to end it. None when it ended.
    pub(crate) fn take(&self) -> Option<Progress> {
        lock(&self.progress).take()
    }

    fn succeed(&self, result: Option<pb::operation::Result>) {
        if let Some(progress) = self.take() {
            progress.succeed(result);
        }
    }

    fn fail(&self, error: pb::Error) {
        if let Some(progress) = self.take() {
            progress.fail(error);
        }
    }

    fn cancel(&self) {
        if let Some(progress) = self.take() {
            progress.end_cancelled();
        }
    }
}

/// How a machine's step keys are shown in the host's operation.
#[derive(Clone)]
enum Naming {
    /// The host's operation has the machine's own keys (a session's start).
    Same,
    /// Keys carry the machine: `record` becomes `record:<machine>` and
    /// `clone_main:<owner/name>` becomes `clone_main:<machine>:<owner/name>`.
    Machine(String),
}

impl Naming {
    fn name(&self, key: &str) -> String {
        match self {
            Naming::Same => key.to_owned(),
            Naming::Machine(machine) => match key.split_once(':') {
                Some((kind, rest)) => format!("{kind}:{machine}:{rest}"),
                None => format!("{key}:{machine}"),
            },
        }
    }
}

/// Copies the machine's operation into the host's, a step at a time. Only
/// changes are written, so a mirror started again doesn't repeat itself.
struct Mirror {
    host: Arc<HostOp>,
    naming: Naming,
    seen: BTreeMap<String, (i32, String)>,
}

impl Mirror {
    fn new(host: Arc<HostOp>, naming: Naming) -> Self {
        Self {
            host,
            naming,
            seen: BTreeMap::new(),
        }
    }

    fn apply(&mut self, operation: &pb::Operation) {
        for step in &operation.steps {
            let now = (step.state, step.output_tail.clone());
            if self.seen.get(&step.key) == Some(&now) {
                continue;
            }
            let state = pb::StepState::try_from(step.state).unwrap_or(pb::StepState::Waiting);
            let tail = Some(step.output_tail.clone()).filter(|tail| !tail.is_empty());
            self.host.set_step(&self.naming.name(&step.key), state, tail);
            self.seen.insert(step.key.clone(), now);
        }
    }
}

fn finished(operation: &pb::Operation) -> bool {
    operation.state != pb::OperationState::Running as i32
}

/// How following one operation on one link ended.
enum Followed {
    /// The operation ended; every change was mirrored.
    Finished(pb::Operation),
    /// The stream ended with the operation still running: the link broke.
    Dropped,
    /// The machine doesn't know the operation (it restarted).
    Lost,
}

async fn follow(
    operations: &mut pb::operation_service_client::OperationServiceClient<Channel>,
    id: &str,
    mirror: &mut Mirror,
) -> Followed {
    let watching = operations
        .watch_operation(Request::new(pb::WatchOperationRequest {
            id: Some(pb::OperationId { value: id.to_owned() }),
            from_version: 0,
        }))
        .await;
    let mut stream = match watching {
        Ok(stream) => stream.into_inner(),
        Err(status) if status.code() == Code::NotFound => return Followed::Lost,
        Err(_) => return Followed::Dropped,
    };
    loop {
        match stream.message().await {
            Ok(Some(operation)) => {
                mirror.apply(&operation);
                if finished(&operation) {
                    return Followed::Finished(operation);
                }
            }
            Ok(None) => return Followed::Dropped,
            Err(status) if status.code() == Code::NotFound => return Followed::Lost,
            Err(_) => return Followed::Dropped,
        }
    }
}

// ---------------------------------------------------------------------------
// What the host remembers of each remote session.

/// The host's view of one session on another machine: the last thing its
/// machine reported (SL-3), and whether that is live (HO-5).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RemoteView {
    pub(crate) machine: String,
    pub(crate) session: String,
    /// What the machine last reported, as the API names it (D124).
    pub(crate) reported: pb::SessionStatus,
    pub(crate) exit_code: i32,
    pub(crate) problem: Option<pb::Error>,
    pub(crate) pointers: Vec<pb::Pointer>,
    pub(crate) pointers_at: Option<humata2_grpc::Timestamp>,
    pub(crate) reported_at: SystemTime,
    /// The machine's link is up and has reported since it came up.
    pub(crate) live: bool,
    /// When the link was last up, while it isn't (HO-5: "last seen ...").
    pub(crate) last_seen: Option<SystemTime>,
}

impl RemoteView {
    /// The status the host shows: the reported one, or offline while the
    /// machine can't be reached (the session may well be running there).
    pub(crate) fn shown(&self) -> pb::SessionStatus {
        if self.live {
            self.reported
        } else {
            pb::SessionStatus::Offline
        }
    }

    /// Whether the agent was running when last heard of, so that a
    /// reconnect has something to reattach.
    fn active(&self) -> bool {
        matches!(
            self.reported,
            pb::SessionStatus::Starting
                | pb::SessionStatus::Working
                | pb::SessionStatus::NeedsInput
                | pb::SessionStatus::RunningUnknownTurn
        )
    }
}

/// The last reported status and pointers of every remote session, kept so a
/// host restart still shows them (PER-27). The store implements it (the
/// seam: a table keyed by machine and session). Called from blocking
/// threads, one call at a time per session.
pub(crate) trait Memory: Send + Sync + 'static {
    fn load(&self) -> Result<Vec<RemoteView>, StoreError>;
    fn save(&self, view: &RemoteView) -> Result<(), StoreError>;
    fn forget(&self, machine: &str, session: &str) -> Result<(), StoreError>;
}

/// Where the host tells of a changed remote session: the session service
/// publishes it on the feed the way local changes are (SL-3).
#[tonic::async_trait]
pub(crate) trait Sink: Send + Sync + 'static {
    async fn changed(&self, view: &RemoteView);
    async fn gone(&self, machine: &str, session: &str);
}

/// What a reconnect brought back (HO-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Reattached {
    /// Sessions that were running and have reported again.
    pub(crate) count: u32,
    /// Sessions that were running and haven't reported yet.
    pub(crate) waiting: u32,
}

impl Reattached {
    /// "devbox-2 is back. 3 sessions reattached."
    #[allow(dead_code)]
    pub(crate) fn message(&self, machine: &str) -> String {
        let sessions = if self.count == 1 { "session" } else { "sessions" };
        format!("{machine} is back. {} {sessions} reattached.", self.count)
    }
}

#[derive(Default)]
struct Views {
    views: BTreeMap<(String, String), RemoteView>,
    /// Per machine: running sessions that haven't reported since its link
    /// last went down.
    awaiting: BTreeMap<String, BTreeSet<String>>,
    reattached: BTreeMap<String, u32>,
}

struct Inner {
    links: Links,
    reports: PeerReports,
    memory: Arc<dyn Memory>,
    sink: Arc<dyn Sink>,
    views: Mutex<Views>,
}

/// What the host's ChannelService and SessionService hold to work on other
/// machines. Cheap to clone.
#[derive(Clone)]
pub(crate) struct RemoteWork {
    inner: Arc<Inner>,
}

/// How a machine's part of a channel's setup ended.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MachineOutcome {
    /// The folders and main clones are there.
    Ready,
    /// The machine answered or its operation ended with an error. Only this
    /// machine's steps are failed.
    Failed(pb::Error),
    /// The machine was removed while its setup waited for it.
    Gone,
}

/// The host's steps for a channel's setup on `machine`, as `(key, title)`:
/// the record, then a main clone per repository. The caller adds them to the
/// operation it starts, one set per machine, before calling
/// [`RemoteWork::create_channel_on`].
pub(crate) fn channel_steps(machine: &str, repos: &[pb::RepoRef]) -> Vec<(String, String)> {
    let mut steps = vec![(
        Naming::Machine(machine.to_owned()).name("record"),
        format!("Saving the channel on {machine}"),
    )];
    for repo in repos {
        let shown = format!("{}/{}", repo.owner, repo.name);
        steps.push((
            Naming::Machine(machine.to_owned()).name(&format!("clone_main:{shown}")),
            format!("Cloning {shown} on {machine}"),
        ));
    }
    steps
}

/// Sets `machine`'s steps that hadn't ended to Skipped: it was removed from
/// Humata while its part of the channel's setup waited, so nothing more
/// happens there (D164).
fn skip_machine_steps(op: &HostOp, machine: &str, repos: &[pb::RepoRef]) {
    let ended: Vec<String> = op
        .operation()
        .map(|operation| {
            operation
                .steps
                .into_iter()
                .filter(|step| {
                    matches!(
                        pb::StepState::try_from(step.state),
                        Ok(pb::StepState::Done | pb::StepState::Failed | pb::StepState::Skipped)
                    )
                })
                .map(|step| step.key)
                .collect()
        })
        .unwrap_or_default();
    for (key, _) in channel_steps(machine, repos) {
        if !ended.contains(&key) {
            op.set_step(&key, pb::StepState::Skipped, None);
        }
    }
}

/// A work item for [`RemoteWork::follow_operation`].
#[derive(Clone)]
enum Work {
    Start(pb::StartSessionRequest),
    Resume,
}

impl RemoteWork {
    pub(crate) fn new(links: Links, reports: PeerReports, memory: Arc<dyn Memory>, sink: Arc<dyn Sink>) -> Self {
        Self {
            inner: Arc::new(Inner {
                links,
                reports,
                memory,
                sink,
                views: Mutex::new(Views::default()),
            }),
        }
    }

    // -- a channel on a machine ---------------------------------------------

    /// Makes channel `channel` on `machine` with the id the host minted, and
    /// mirrors the machine's setup into `op` as `record:<machine>` and
    /// `clone_main:<machine>:<owner/name>` (CH-5, CH-7, CH-19). Also adds a
    /// repository or a machine to an existing channel: the machine answers a
    /// known id with its channel and makes the repositories it lacks.
    ///
    /// `request` is what the machine should see; its machine list and
    /// idempotency key are dropped here (the machine is the one it is
    /// running on, and the id is the key). A machine whose link is down has
    /// its steps shown waiting and is called once when its link returns; a
    /// link that drops halfway calls again on the next one, which is
    /// harmless. The returned outcome is this machine's alone. The caller
    /// ends `op` once every machine and the local clones are through.
    pub(crate) async fn create_channel_on(
        &self,
        machine: &str,
        channel: &str,
        request: pb::CreateChannelRequest,
        op: Arc<HostOp>,
    ) -> MachineOutcome {
        let request = pb::CreateChannelRequest {
            idempotency_key: None,
            machines: Vec::new(),
            ..request
        };
        for (key, _) in channel_steps(machine, &request.repos) {
            op.set_step(&key, pb::StepState::Waiting, None);
        }
        let (done, mut outcomes) = mpsc::unbounded_channel();
        let links = &self.inner.links;
        let attempts = links.reopen_on_new_link(machine, |link, _generation| {
            let (this, done, op) = (self.clone(), done.clone(), op.clone());
            let (machine, channel, request) = (machine.to_owned(), channel.to_owned(), request.clone());
            async move {
                if let Some(outcome) = this.channel_attempt(link, &machine, &channel, &request, &op).await {
                    let _ = done.send(outcome);
                }
            }
        });
        tokio::select! {
            () = attempts => {
                skip_machine_steps(&op, machine, &request.repos);
                MachineOutcome::Gone
            }
            Some(outcome) = outcomes.recv() => outcome,
        }
    }

    /// One try on one link. None: the link broke before the machine's
    /// answer, so wait for the next one.
    async fn channel_attempt(
        &self,
        link: Channel,
        machine: &str,
        channel: &str,
        request: &pb::CreateChannelRequest,
        op: &Arc<HostOp>,
    ) -> Option<MachineOutcome> {
        let mut channels = pb::channel_service_client::ChannelServiceClient::new(link.clone());
        let mut operations = pb::operation_service_client::OperationServiceClient::new(link);
        let mut mirror = Mirror::new(op.clone(), Naming::Machine(machine.to_owned()));
        for _ in 0..MAX_ATTEMPTS {
            let call = match assigned(request.clone(), Some(channel)) {
                Ok(call) => call,
                Err(error) => return Some(fail_machine_steps(op, machine, request, plain_error(&error.to_string(), ""))),
            };
            let response = match channels.create_channel(call).await {
                Ok(response) => response.into_inner(),
                Err(status) => {
                    let Some(error) = status::error_of(&status) else {
                        return None;
                    };
                    return Some(fail_machine_steps(op, machine, request, error));
                }
            };
            let got = response.channel.and_then(|channel| channel.id).map(|id| id.value).unwrap_or_default();
            if got != channel {
                let error = RemoteError::IdNotHonoured {
                    machine: machine.to_owned(),
                    asked: channel.to_owned(),
                    got,
                };
                return Some(fail_machine_steps(op, machine, request, plain_error(&error.to_string(), "")));
            }
            let Some(setup) = response.setup else {
                return Some(MachineOutcome::Ready);
            };
            mirror.apply(&setup);
            let ended = if finished(&setup) {
                setup
            } else {
                let id = setup.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
                match follow(&mut operations, &id, &mut mirror).await {
                    Followed::Finished(ended) => ended,
                    Followed::Dropped => return None,
                    // The machine restarted: asking again is what resumes it.
                    Followed::Lost => continue,
                }
            };
            return Some(match pb::OperationState::try_from(ended.state) {
                Ok(pb::OperationState::Succeeded) => MachineOutcome::Ready,
                _ => {
                    let error = ended.error.unwrap_or_else(|| {
                        plain_error(
                            &format!("{machine}'s setup of the channel was cancelled"),
                            "add the machine to the channel again",
                        )
                    });
                    MachineOutcome::Failed(error)
                }
            });
        }
        None
    }

    // -- a session on a machine ---------------------------------------------

    /// Whether `machine`'s link is up now. A start checks this before it
    /// writes any record, so an offline machine leaves nothing behind (CH-4).
    pub(crate) fn require_link(&self, machine: &str) -> Result<(), RemoteError> {
        self.inner.links.relayed(machine)?;
        Ok(())
    }

    /// The typed clients on `machine`'s live link, for the host's own calls
    /// to it that no relay of a user's call covers (SL-4's rebuild).
    pub(crate) fn relayed(&self, machine: &str) -> Result<crate::host_relay::Relayed, RemoteError> {
        Ok(self.inner.links.relayed(machine)?)
    }

    /// Starts session `session` on `machine` with the id the host minted. The
    /// caller wrote the host's record first (SL-1, PER-11). Answers as the
    /// local path does once the machine has recorded the session (D121); the
    /// machine's START_SESSION operation is mirrored into `op` as it runs,
    /// whatever happens to the link, and `op` ends here.
    ///
    /// A machine that can't be reached answers the machine-offline error at
    /// once and nothing is queued (CH-4, TR-6).
    pub(crate) async fn start_session_on(
        &self,
        machine: &str,
        session: &str,
        request: pb::StartSessionRequest,
        op: Arc<HostOp>,
    ) -> Result<pb::StartSessionResponse, RemoteError> {
        let relayed = self.inner.links.relayed(machine)?;
        // The machine is the one it is running on.
        let request = pb::StartSessionRequest {
            machine: None,
            ..request
        };
        let response = relayed
            .session()
            .start_session(assigned(request.clone(), Some(session))?)
            .await
            .map_err(|source| call_failed(machine, "start the session", source))?
            .into_inner();
        let got = response.session.map(|id| id.value).unwrap_or_default();
        if got != session {
            return Err(RemoteError::IdNotHonoured {
                machine: machine.to_owned(),
                asked: session.to_owned(),
                got,
            });
        }
        let first = response.operation.unwrap_or_default();
        Mirror::new(op.clone(), Naming::Same).apply(&first);
        let shown = op.operation();
        self.spawn_follow(machine, session, Work::Start(request), first, op);
        Ok(pb::StartSessionResponse {
            session: Some(pb::SessionId { value: session.to_owned() }),
            operation: shown,
        })
    }

    /// Stops session `session` on `machine` (D123) and returns it as the
    /// machine shows it. The caller merges it into the host's record.
    pub(crate) async fn stop_session_on(
        &self,
        machine: &str,
        session: &str,
    ) -> Result<pb::Session, RemoteError> {
        let relayed = self.inner.links.relayed(machine)?;
        let request = pb::StopSessionRequest {
            idempotency_key: None,
            id: Some(pb::SessionId { value: session.to_owned() }),
        };
        let stopped = relayed
            .session()
            .stop_session(assigned(request, None)?)
            .await
            .map_err(|source| call_failed(machine, "stop the session", source))?;
        Ok(stopped.into_inner())
    }

    /// Renames session `session` on `machine`, and its branch there too when
    /// `rename_branch` (FC-17, D196), and returns it as the machine shows it.
    /// The machine refuses a branch that was pushed.
    pub(crate) async fn rename_session_on(
        &self,
        machine: &str,
        session: &str,
        title: &str,
        rename_branch: bool,
    ) -> Result<pb::Session, RemoteError> {
        let relayed = self.inner.links.relayed(machine)?;
        let request = pb::RenameSessionRequest {
            id: Some(pb::SessionId { value: session.to_owned() }),
            title: title.to_owned(),
            rename_branch,
            ..Default::default()
        };
        let renamed = relayed
            .session()
            .rename_session(assigned(request, None)?)
            .await
            .map_err(|source| call_failed(machine, "rename the session's branch", source))?;
        Ok(renamed.into_inner())
    }

    /// Resumes session `session` on `machine` (D123, PER-15). The machine's
    /// operation is mirrored into `op`, which ends here; the answer is `op`
    /// as it is when the machine has accepted the resume.
    pub(crate) async fn resume_session_on(
        &self,
        machine: &str,
        session: &str,
        request: pb::ResumeSessionRequest,
        op: Arc<HostOp>,
    ) -> Result<Option<pb::Operation>, RemoteError> {
        let relayed = self.inner.links.relayed(machine)?;
        let request = pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(pb::SessionId { value: session.to_owned() }),
            ..request
        };
        let first = relayed
            .session()
            .resume_session(assigned(request, None)?)
            .await
            .map_err(|source| call_failed(machine, "resume the session", source))?
            .into_inner();
        Mirror::new(op.clone(), Naming::Same).apply(&first);
        let shown = op.operation();
        self.spawn_follow(machine, session, Work::Resume, first, op);
        Ok(shown)
    }

    fn spawn_follow(&self, machine: &str, session: &str, work: Work, first: pb::Operation, op: Arc<HostOp>) {
        let (this, machine, session) = (self.clone(), machine.to_owned(), session.to_owned());
        tokio::spawn(async move {
            this.follow_operation(machine, session, work, first, op).await;
        });
    }

    /// Follows the machine's operation on every link it has until it ends,
    /// then ends the host's.
    async fn follow_operation(
        &self,
        machine: String,
        session: String,
        work: Work,
        first: pb::Operation,
        op: Arc<HostOp>,
    ) {
        if finished(&first) {
            end(&op, &first, &session);
            return;
        }
        let current = Arc::new(Mutex::new(first.id.as_ref().map(|id| id.value.clone()).unwrap_or_default()));
        let (done, mut ended) = mpsc::unbounded_channel();
        let attempts = self.inner.links.reopen_on_new_link(&machine, |link, _generation| {
            let (this, done, op, current) = (self.clone(), done.clone(), op.clone(), current.clone());
            let (machine, session, work) = (machine.clone(), session.clone(), work.clone());
            async move {
                if let Some(operation) = this.follow_on_link(link, &machine, &session, &work, &current, &op).await {
                    let _ = done.send(operation);
                }
            }
        });
        tokio::select! {
            () = attempts => op.fail(plain_error(
                &format!("{machine} was removed before the work on session {session} finished"),
                "the session's record is kept; archive or delete it from the session's menu",
            )),
            Some(operation) = ended.recv() => end(&op, &operation, &session),
        }
    }

    /// One link's worth of following. None: the link broke first.
    async fn follow_on_link(
        &self,
        link: Channel,
        machine: &str,
        session: &str,
        work: &Work,
        current: &Mutex<String>,
        op: &Arc<HostOp>,
    ) -> Option<pb::Operation> {
        let mut operations = pb::operation_service_client::OperationServiceClient::new(link.clone());
        let mut sessions = pb::session_service_client::SessionServiceClient::new(link);
        let mut mirror = Mirror::new(op.clone(), Naming::Same);
        for _ in 0..MAX_ATTEMPTS {
            let id = lock(current).clone();
            match follow(&mut operations, &id, &mut mirror).await {
                Followed::Finished(ended) => return Some(ended),
                Followed::Dropped => return None,
                Followed::Lost => {}
            }
            // The machine forgot the operation. Its session says how far it got.
            let asked = pb::GetSessionRequest {
                id: Some(pb::SessionId { value: session.to_owned() }),
            };
            match sessions.get_session(Request::new(asked)).await {
                Ok(found) => {
                    let reported = pb::SessionStatus::try_from(found.into_inner().status).unwrap_or(pb::SessionStatus::Starting);
                    return self.settled(machine, session, work, &id, reported).await;
                }
                Err(status) if status.code() == Code::NotFound => match work {
                    // The start never reached the machine's record: the same
                    // id again makes the one session.
                    Work::Start(request) => {
                        let call = assigned(request.clone(), Some(session)).ok()?;
                        let started = sessions.start_session(call).await.ok()?.into_inner();
                        if let Some(operation) = started.operation {
                            *lock(current) = operation.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
                            mirror.apply(&operation);
                        }
                    }
                    Work::Resume => {
                        return Some(synthetic(
                            &id,
                            pb::OperationState::Failed,
                            session,
                            Some(plain_error(
                                &format!("{machine} no longer has session {session}"),
                                "the session may have been removed on the machine; archive it from its menu",
                            )),
                        ));
                    }
                },
                Err(_) => return None,
            }
        }
        None
    }

    /// The machine knows the session but not the operation: wait, if it is
    /// still coming up, for its report that says how it came out.
    async fn settled(
        &self,
        machine: &str,
        session: &str,
        work: &Work,
        operation: &str,
        known: pb::SessionStatus,
    ) -> Option<pb::Operation> {
        let mut updates = self.inner.reports.subscribe();
        let mut status = known;
        loop {
            if let Some((report, _)) = self.inner.reports.session(machine, session)
                && let Some(reported) = report.status
            {
                status = map_status(reported.status);
            }
            if status != pb::SessionStatus::Starting {
                break;
            }
            match updates.recv().await {
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
        let resumed_into_nothing = matches!(work, Work::Resume)
            && matches!(
                status,
                pb::SessionStatus::Idle | pb::SessionStatus::Exited | pb::SessionStatus::ResumeFailed
            );
        Some(if resumed_into_nothing {
            synthetic(
                operation,
                pb::OperationState::Failed,
                session,
                Some(plain_error(
                    &format!("{machine} restarted before the session came back and it is not running"),
                    "resume the session again",
                )),
            )
        } else {
            synthetic(operation, pb::OperationState::Succeeded, session, None)
        })
    }

    // -- what the machines report -------------------------------------------

    /// Turns the machines' reports into the host's view of each remote
    /// session, for the life of the daemon (SL-3, TR-7, HO-5): restore what
    /// the host remembers, then follow every report as it arrives. Run it
    /// in its own task.
    pub(crate) async fn follow_reports(&self) {
        let mut updates = self.inner.reports.subscribe();
        self.restore().await;
        self.resync().await;
        loop {
            match updates.recv().await {
                Ok(update) => self.apply(&update).await,
                Err(broadcast::error::RecvError::Lagged(_)) => self.resync().await,
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    /// The host's view of one remote session, if its machine ever reported it.
    pub(crate) fn view(&self, machine: &str, session: &str) -> Option<RemoteView> {
        lock(&self.inner.views).views.get(&(machine.to_owned(), session.to_owned())).cloned()
    }

    /// Every remote session on `machine` the host has a view of. Not called
    /// yet: the machine's page will show them.
    #[allow(dead_code)]
    pub(crate) fn views_on(&self, machine: &str) -> Vec<RemoteView> {
        lock(&self.inner.views)
            .views
            .values()
            .filter(|view| view.machine == machine)
            .cloned()
            .collect()
    }

    /// Puts the machine's view on `session`, the host's own `Session` for a
    /// session placed on another machine: its status (offline while the
    /// machine is), exit code and pointers (SL-3). Nothing changes when the
    /// machine never reported it.
    pub(crate) fn merge(&self, session: &mut pb::Session) {
        let (Some(id), Some(machine)) = (session.id.as_ref(), session.machine.as_ref()) else {
            return;
        };
        let Some(view) = self.view(&machine.value, &id.value) else {
            // Never reported (an old machine, or before its first report):
            // while the machine's link is down the session can't be shown
            // as anything but offline (HO-5). With the link up the host's
            // own status stands until the machine reports.
            let link_up = self
                .inner
                .links
                .states()
                .into_iter()
                .any(|(name, state)| name == machine.value && matches!(state, LinkState::Up { .. }));
            if !link_up {
                session.status = pb::SessionStatus::Offline as i32;
            }
            return;
        };
        session.status = view.shown() as i32;
        session.exit_code = view.exit_code;
        session.pointers = view.pointers;
    }

    /// What the last reconnect of `machine` brought back (HO-6). Not called
    /// yet: ReconnectMachine's answer will say it.
    #[allow(dead_code)]
    pub(crate) fn reattached(&self, machine: &str) -> Reattached {
        let views = lock(&self.inner.views);
        Reattached {
            count: views.reattached.get(machine).copied().unwrap_or(0),
            waiting: views
                .awaiting
                .get(machine)
                .map_or(0, |sessions| u32::try_from(sessions.len()).unwrap_or(u32::MAX)),
        }
    }

    async fn restore(&self) {
        let memory = self.inner.memory.clone();
        let loaded = tokio::task::spawn_blocking(move || memory.load()).await;
        let views = match loaded {
            Ok(Ok(views)) => views,
            Ok(Err(error)) => {
                crate::log::say!("humatad2: couldn't read the remote sessions' last reports: {}", status::chain(&error));
                return;
            }
            Err(_) => return,
        };
        let mut held = lock(&self.inner.views);
        for mut view in views {
            // Nothing is live until the machine's link reports again.
            view.live = false;
            if view.active() {
                held.awaiting.entry(view.machine.clone()).or_default().insert(view.session.clone());
            }
            held.views.insert((view.machine.clone(), view.session.clone()), view);
        }
    }

    /// Brings the views up to what the reports hold now (at the start, and
    /// after a subscriber fell behind: latest wins, TR-8).
    async fn resync(&self) {
        let mut machines: BTreeSet<String> = self.inner.links.states().into_iter().map(|(name, _)| name).collect();
        machines.extend(lock(&self.inner.views).views.keys().map(|(machine, _)| machine.clone()));
        for machine in machines {
            let Some(held) = self.inner.reports.machine(&machine) else {
                continue;
            };
            for (session, report) in held.sessions {
                self.apply(&ReportUpdate::Session {
                    machine: machine.clone(),
                    session,
                    report,
                })
                .await;
            }
            if let Freshness::Stale { last_up } = held.freshness {
                self.apply(&ReportUpdate::Freshness {
                    machine: machine.clone(),
                    freshness: Freshness::Stale { last_up },
                })
                .await;
            }
        }
    }

    async fn apply(&self, update: &ReportUpdate) {
        match update {
            ReportUpdate::Session { machine, session, report } => {
                let live = matches!(
                    self.inner.reports.session(machine, session).map(|(_, freshness)| freshness),
                    Some(Freshness::Live)
                );
                let view = {
                    let mut held = lock(&self.inner.views);
                    let key = (machine.clone(), session.clone());
                    let view = view_from(machine, session, report, held.views.get(&key), live);
                    if live && held.awaiting.get_mut(machine).is_some_and(|waiting| waiting.remove(session)) {
                        *held.reattached.entry(machine.clone()).or_default() += 1;
                    }
                    held.views.insert(key, view.clone());
                    view
                };
                self.remember(&view).await;
                self.inner.sink.changed(&view).await;
            }
            ReportUpdate::Gone { machine, session } => {
                {
                    let mut held = lock(&self.inner.views);
                    held.views.remove(&(machine.clone(), session.clone()));
                    if let Some(waiting) = held.awaiting.get_mut(machine) {
                        waiting.remove(session);
                    }
                }
                self.forget(machine, session).await;
                self.inner.sink.gone(machine, session).await;
            }
            ReportUpdate::Freshness {
                machine,
                freshness: Freshness::Stale { last_up },
            } => {
                let changed: Vec<RemoteView> = {
                    let mut held = lock(&self.inner.views);
                    let mut changed = Vec::new();
                    let mut running = BTreeSet::new();
                    for view in held.views.values_mut().filter(|view| view.machine == *machine) {
                        view.live = false;
                        view.last_seen = *last_up;
                        if view.active() {
                            running.insert(view.session.clone());
                        }
                        changed.push(view.clone());
                    }
                    // A new count starts with each loss of the link.
                    held.awaiting.insert(machine.clone(), running);
                    held.reattached.insert(machine.clone(), 0);
                    changed
                };
                for view in changed {
                    self.inner.sink.changed(&view).await;
                }
            }
            // The new link's reports follow; each one makes its session live.
            ReportUpdate::Freshness {
                freshness: Freshness::Live, ..
            } => {}
        }
    }

    async fn remember(&self, view: &RemoteView) {
        let (memory, saving) = (self.inner.memory.clone(), view.clone());
        if let Ok(Err(error)) = tokio::task::spawn_blocking(move || memory.save(&saving)).await {
            crate::log::say!(
                "humatad2: couldn't keep session {}'s last report from {}: {}",
                view.session,
                view.machine,
                status::chain(&error)
            );
        }
    }

    /// `session` moved off `machine` (MV-2): what the machine last reported
    /// of it is forgotten, in memory and in the host's state.
    pub(crate) async fn forget_session(&self, machine: &str, session: &str) {
        {
            let mut views = lock(&self.inner.views);
            views.views.remove(&(machine.to_owned(), session.to_owned()));
            if let Some(awaiting) = views.awaiting.get_mut(machine) {
                awaiting.remove(session);
            }
        }
        self.forget(machine, session).await;
    }

    async fn forget(&self, machine: &str, session: &str) {
        let (memory, machine, session) = (self.inner.memory.clone(), machine.to_owned(), session.to_owned());
        if let Ok(Err(error)) = tokio::task::spawn_blocking(move || memory.forget(&machine, &session)).await {
            crate::log::say!("humatad2: couldn't forget a remote session's last report: {}", status::chain(&error));
        }
    }
}

/// [`Memory`] over the store's `remote_session_reports` table (PER-27).
pub(crate) struct StoreMemory(pub(crate) Arc<humata2_store::Store>);

impl Memory for StoreMemory {
    fn load(&self) -> Result<Vec<RemoteView>, StoreError> {
        let mut views = Vec::new();
        for row in self.0.remote_reports()? {
            match view_of(&row) {
                Some(view) => views.push(view),
                None => crate::log::say!(
                    "humatad2: skipped session {}'s last report from {}: it can't be read",
                    row.session,
                    row.machine
                ),
            }
        }
        Ok(views)
    }

    fn save(&self, view: &RemoteView) -> Result<(), StoreError> {
        self.0.put_remote_report(&row_of(view))
    }

    fn forget(&self, machine: &str, session: &str) -> Result<(), StoreError> {
        self.0.delete_remote_report(machine, session)
    }
}

fn unix_ms(time: SystemTime) -> i64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

fn from_unix_ms(ms: i64) -> SystemTime {
    SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(u64::try_from(ms).unwrap_or(0))
}

fn row_of(view: &RemoteView) -> RemoteReportRow {
    let pointers = pb::PointersReport {
        session: Some(pb::SessionId { value: view.session.clone() }),
        pointers: view.pointers.clone(),
        at: None,
    };
    RemoteReportRow {
        machine: view.machine.clone(),
        session: view.session.clone(),
        reported: view.reported as i32,
        exit_code: view.exit_code,
        problem: view.problem.as_ref().map(Message::encode_to_vec),
        pointers: pointers.encode_to_vec(),
        pointers_at_ms: view
            .pointers_at
            .as_ref()
            .map(|at| at.seconds.saturating_mul(1000).saturating_add(i64::from(at.nanos) / 1_000_000)),
        reported_unix_ms: unix_ms(view.reported_at),
        last_seen_unix_ms: view.last_seen.map(unix_ms),
    }
}

/// The view a row holds; none when its stored messages can't be decoded.
fn view_of(row: &RemoteReportRow) -> Option<RemoteView> {
    let problem = match &row.problem {
        Some(bytes) => Some(pb::Error::decode(bytes.as_slice()).ok()?),
        None => None,
    };
    let pointers = pb::PointersReport::decode(row.pointers.as_slice()).ok()?;
    Some(RemoteView {
        machine: row.machine.clone(),
        session: row.session.clone(),
        reported: map_status(row.reported),
        exit_code: row.exit_code,
        problem,
        pointers: pointers.pointers,
        pointers_at: row.pointers_at_ms.map(crate::channel_service::timestamp_ms),
        reported_at: from_unix_ms(row.reported_unix_ms),
        live: false,
        last_seen: row.last_seen_unix_ms.map(from_unix_ms),
    })
}

/// The API's session status for what a machine reported (D124: the same
/// names the local mapping gives). A report that names none is a session
/// still coming up.
fn map_status(reported: i32) -> pb::SessionStatus {
    match pb::SessionStatus::try_from(reported) {
        Ok(pb::SessionStatus::Unspecified) | Err(_) => pb::SessionStatus::Starting,
        Ok(status) => status,
    }
}

fn view_from(machine: &str, session: &str, report: &SessionReport, prior: Option<&RemoteView>, live: bool) -> RemoteView {
    let (reported, exit_code, problem) = match &report.status {
        Some(status) => (map_status(status.status), status.exit_code, status.problem.clone()),
        None => prior.map_or((pb::SessionStatus::Starting, 0, None), |prior| {
            (prior.reported, prior.exit_code, prior.problem.clone())
        }),
    };
    RemoteView {
        machine: machine.to_owned(),
        session: session.to_owned(),
        reported,
        exit_code,
        problem,
        pointers: report.pointers.clone(),
        pointers_at: report.pointers_at.clone(),
        reported_at: SystemTime::now(),
        live,
        last_seen: if live { None } else { prior.and_then(|prior| prior.last_seen) },
    }
}

/// Ends the host's operation as the machine's ended.
fn end(op: &HostOp, ended: &pb::Operation, session: &str) {
    match pb::OperationState::try_from(ended.state) {
        Ok(pb::OperationState::Succeeded) => {
            op.succeed(Some(pb::operation::Result::Session(pb::SessionId { value: session.to_owned() })));
        }
        Ok(pb::OperationState::Cancelled) => op.cancel(),
        _ => op.fail(ended.error.clone().unwrap_or_else(|| {
            plain_error(
                &format!("the machine ended the work on session {session} without saying why"),
                "look at the session's last steps, then try again",
            )
        })),
    }
}

/// An operation standing for one the machine lost when it restarted.
fn synthetic(id: &str, state: pb::OperationState, session: &str, error: Option<pb::Error>) -> pb::Operation {
    pb::Operation {
        id: Some(pb::OperationId { value: id.to_owned() }),
        state: state as i32,
        error,
        result: Some(pb::operation::Result::Session(pb::SessionId { value: session.to_owned() })),
        ..pb::Operation::default()
    }
}

fn plain_error(message: &str, fix: &str) -> pb::Error {
    pb::Error {
        message: message.to_owned(),
        fix: fix.to_owned(),
        retry: None,
        kind: None,
    }
}

/// Marks `machine`'s steps that aren't done as failed, with `error`, and
/// answers the machine's outcome. Other machines' steps are not touched.
fn fail_machine_steps(op: &HostOp, machine: &str, request: &pb::CreateChannelRequest, error: pb::Error) -> MachineOutcome {
    let done = pb::StepState::Done as i32;
    let shown = op.operation();
    for (key, _) in channel_steps(machine, &request.repos) {
        let already = shown
            .as_ref()
            .and_then(|operation| operation.steps.iter().find(|step| step.key == key))
            .is_some_and(|step| step.state == done);
        if !already {
            op.set_step(&key, pb::StepState::Failed, Some(error.message.clone()));
        }
    }
    MachineOutcome::Failed(error)
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use hyper_util::rt::TokioIo;
    use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
    use tokio::sync::watch;
    use tokio_stream::wrappers::ReceiverStream;
    use tonic::transport::server::Connected;
    use tonic::transport::{Endpoint, Server, Uri};
    use tonic::{Response, Status};

    use super::*;
    use crate::events::{Feed, Published};
    use crate::host_relay::HOST_ASSIGNED_ID;
    use crate::identity::{Caller, UserVia};
    use crate::operations::{Operations, Started};

    // -- an in-memory machine -------------------------------------------------

    struct Served(DuplexStream);

    impl Connected for Served {
        type ConnectInfo = ();

        fn connect_info(&self) -> Self::ConnectInfo {}
    }

    impl AsyncRead for Served {
        fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for Served {
        fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.0).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_flush(cx)
        }

        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_shutdown(cx)
        }
    }

    #[derive(Default)]
    struct MachineState {
        channels: BTreeMap<String, Vec<pb::RepoRef>>,
        sessions: BTreeMap<String, pb::SessionStatus>,
        operations: BTreeMap<String, watch::Sender<pb::Operation>>,
        order: Vec<String>,
        calls: Vec<String>,
    }

    /// A machine's daemon: the state outlives its links, so a "restart" is a
    /// new link to the same state with the operations forgotten.
    #[derive(Clone, Default)]
    struct Machine {
        state: Arc<Mutex<MachineState>>,
    }

    fn assigned_id<T>(request: &Request<T>) -> Result<String, Status> {
        request
            .metadata()
            .get(HOST_ASSIGNED_ID)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .ok_or_else(|| Status::invalid_argument("no assigned id"))
    }

    fn step(key: &str, state: pb::StepState) -> pb::OperationStep {
        pb::OperationStep {
            key: key.to_owned(),
            title: key.to_owned(),
            state: state as i32,
            ..pb::OperationStep::default()
        }
    }

    impl Machine {
        fn new_operation(&self, keys: &[String], state: pb::OperationState, step_state: pb::StepState) -> pb::Operation {
            let mut held = lock(&self.state);
            let id = format!("op_{}", held.order.len() + 1);
            let operation = pb::Operation {
                id: Some(pb::OperationId { value: id.clone() }),
                state: state as i32,
                steps: keys.iter().map(|key| step(key, step_state)).collect(),
                version: 1,
                ..pb::Operation::default()
            };
            held.operations.insert(id.clone(), watch::channel(operation.clone()).0);
            held.order.push(id);
            operation
        }

        /// Changes the newest operation, as the machine's work would.
        fn edit(&self, change: impl FnOnce(&mut pb::Operation)) {
            let held = lock(&self.state);
            let id = held.order.last().cloned().unwrap_or_default();
            if let Some(sender) = held.operations.get(&id) {
                sender.send_modify(|operation| {
                    change(operation);
                    operation.version += 1;
                });
            }
        }

        fn set(&self, key: &str, state: pb::StepState) {
            self.edit(|operation| {
                if let Some(found) = operation.steps.iter_mut().find(|step| step.key == key) {
                    found.state = state as i32;
                }
            });
        }

        fn end(&self, state: pb::OperationState) {
            self.edit(|operation| {
                operation.state = state as i32;
                for step in &mut operation.steps {
                    step.state = pb::StepState::Done as i32;
                }
            });
        }

        /// The machine's daemon restarted: its operations are gone.
        fn restart(&self) {
            lock(&self.state).operations.clear();
        }

        fn calls(&self) -> Vec<String> {
            lock(&self.state).calls.clone()
        }

        fn set_session(&self, session: &str, status: pb::SessionStatus) {
            lock(&self.state).sessions.insert(session.to_owned(), status);
        }
    }

    #[tonic::async_trait]
    impl pb::channel_service_server::ChannelService for Machine {
        async fn create_channel(
            &self,
            request: Request<pb::CreateChannelRequest>,
        ) -> Result<Response<pb::CreateChannelResponse>, Status> {
            let id = assigned_id(&request)?;
            let body = request.into_inner();
            let known = {
                let mut held = lock(&self.state);
                held.calls.push(format!("create_channel:{id}"));
                held.channels.insert(id.clone(), body.repos.clone()).is_some()
            };
            let mut keys = vec!["record".to_owned()];
            keys.extend(body.repos.iter().map(|repo| format!("clone_main:{}/{}", repo.owner, repo.name)));
            let setup = if known {
                self.new_operation(&keys, pb::OperationState::Succeeded, pb::StepState::Done)
            } else {
                self.new_operation(&keys, pb::OperationState::Running, pb::StepState::Waiting)
            };
            Ok(Response::new(pb::CreateChannelResponse {
                channel: Some(pb::Channel {
                    id: Some(pb::ChannelId { value: id }),
                    ..pb::Channel::default()
                }),
                setup: Some(setup),
            }))
        }

        async fn get_channel(&self, _: Request<pb::GetChannelRequest>) -> Result<Response<pb::Channel>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn list_channels(&self, _: Request<pb::ListChannelsRequest>) -> Result<Response<pb::ListChannelsResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn rename_channel(&self, _: Request<pb::RenameChannelRequest>) -> Result<Response<pb::Channel>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn search_git_hub_repos(
            &self,
            _: Request<pb::SearchGitHubReposRequest>,
        ) -> Result<Response<pb::SearchGitHubReposResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn add_repo(&self, _: Request<pb::AddRepoRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn plan_remove_repo(&self, _: Request<pb::PlanRemoveRepoRequest>) -> Result<Response<pb::RemoveRepoPlan>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn remove_repo(&self, _: Request<pb::RemoveRepoRequest>) -> Result<Response<pb::Channel>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn add_machine(&self, _: Request<pb::AddChannelMachineRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn plan_remove_machine(
            &self,
            _: Request<pb::PlanRemoveChannelMachineRequest>,
        ) -> Result<Response<pb::RemoveChannelMachinePlan>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn remove_machine(&self, _: Request<pb::RemoveChannelMachineRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn get_grid(&self, _: Request<pb::GetGridRequest>) -> Result<Response<pb::ChannelGrid>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn plan_delete_channel(
            &self,
            _: Request<pb::PlanDeleteChannelRequest>,
        ) -> Result<Response<pb::DeleteChannelPlan>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn delete_channel(&self, _: Request<pb::DeleteChannelRequest>) -> Result<Response<pb::DeleteChannelResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }
    }

    #[tonic::async_trait]
    impl pb::operation_service_server::OperationService for Machine {
        async fn get_operation(&self, request: Request<pb::GetOperationRequest>) -> Result<Response<pb::Operation>, Status> {
            let id = request.into_inner().id.map(|id| id.value).unwrap_or_default();
            lock(&self.state)
                .operations
                .get(&id)
                .map(|sender| Response::new(sender.borrow().clone()))
                .ok_or_else(|| Status::not_found("no such operation"))
        }

        async fn find_operation(&self, _: Request<pb::FindOperationRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn list_operations(&self, _: Request<pb::ListOperationsRequest>) -> Result<Response<pb::ListOperationsResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        type WatchOperationStream = ReceiverStream<Result<pb::Operation, Status>>;

        async fn watch_operation(
            &self,
            request: Request<pb::WatchOperationRequest>,
        ) -> Result<Response<Self::WatchOperationStream>, Status> {
            let id = request.into_inner().id.map(|id| id.value).unwrap_or_default();
            let mut changes = lock(&self.state)
                .operations
                .get(&id)
                .map(watch::Sender::subscribe)
                .ok_or_else(|| Status::not_found("no such operation"))?;
            let (sender, receiver) = mpsc::channel(4);
            tokio::spawn(async move {
                loop {
                    let operation = changes.borrow_and_update().clone();
                    let done = finished(&operation);
                    if sender.send(Ok(operation)).await.is_err() || done {
                        return;
                    }
                    if changes.changed().await.is_err() {
                        return;
                    }
                }
            });
            Ok(Response::new(ReceiverStream::new(receiver)))
        }

        async fn cancel_operation(&self, _: Request<pb::CancelOperationRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn retry_operation(&self, _: Request<pb::RetryOperationRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }
    }

    #[tonic::async_trait]
    impl pb::session_service_server::SessionService for Machine {
        async fn start_session(&self, request: Request<pb::StartSessionRequest>) -> Result<Response<pb::StartSessionResponse>, Status> {
            let id = assigned_id(&request)?;
            let known = {
                let mut held = lock(&self.state);
                held.calls.push(format!("start_session:{id}"));
                held.sessions.insert(id.clone(), pb::SessionStatus::Starting).is_some()
            };
            let keys = ["write_record", "launch", "verify"].map(str::to_owned);
            let operation = if known {
                self.new_operation(&keys, pb::OperationState::Succeeded, pb::StepState::Done)
            } else {
                self.new_operation(&keys, pb::OperationState::Running, pb::StepState::Waiting)
            };
            Ok(Response::new(pb::StartSessionResponse {
                session: Some(pb::SessionId { value: id }),
                operation: Some(operation),
            }))
        }

        async fn get_session(&self, request: Request<pb::GetSessionRequest>) -> Result<Response<pb::Session>, Status> {
            let id = request.into_inner().id.map(|id| id.value).unwrap_or_default();
            let status = lock(&self.state).sessions.get(&id).copied();
            match status {
                Some(status) => Ok(Response::new(pb::Session {
                    id: Some(pb::SessionId { value: id }),
                    status: status as i32,
                    ..pb::Session::default()
                })),
                None => Err(Status::not_found("no such session")),
            }
        }

        async fn stop_session(&self, request: Request<pb::StopSessionRequest>) -> Result<Response<pb::Session>, Status> {
            let id = request.into_inner().id.map(|id| id.value).unwrap_or_default();
            lock(&self.state).calls.push(format!("stop_session:{id}"));
            Ok(Response::new(pb::Session {
                id: Some(pb::SessionId { value: id }),
                status: pb::SessionStatus::Idle as i32,
                ..pb::Session::default()
            }))
        }

        async fn resume_session(&self, request: Request<pb::ResumeSessionRequest>) -> Result<Response<pb::Operation>, Status> {
            let id = request.into_inner().id.map(|id| id.value).unwrap_or_default();
            lock(&self.state).calls.push(format!("resume_session:{id}"));
            let keys = ["launch", "verify"].map(str::to_owned);
            Ok(Response::new(self.new_operation(&keys, pb::OperationState::Running, pb::StepState::Waiting)))
        }

        async fn preview_branch_names(&self, _: Request<pb::PreviewBranchNamesRequest>) -> Result<Response<pb::PreviewBranchNamesResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn list_sessions(&self, _: Request<pb::ListSessionsRequest>) -> Result<Response<pb::ListSessionsResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn rename_session(&self, _: Request<pb::RenameSessionRequest>) -> Result<Response<pb::Session>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn list_turns(&self, _: Request<pb::ListTurnsRequest>) -> Result<Response<pb::ListTurnsResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn preview_fork(&self, _: Request<pb::ForkSessionRequest>) -> Result<Response<pb::ForkPreview>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn fork_session(&self, _: Request<pb::ForkSessionRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn give_own_copy(&self, _: Request<pb::GiveOwnCopyRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn move_session(&self, _: Request<pb::MoveSessionRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn plan_archive(&self, _: Request<pb::PlanArchiveRequest>) -> Result<Response<pb::ArchivePlan>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn archive_session(&self, _: Request<pb::ArchiveSessionRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn restore_session(&self, _: Request<pb::RestoreSessionRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn plan_delete(&self, _: Request<pb::PlanDeleteRequest>) -> Result<Response<pb::DeletePlan>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn delete_session(&self, _: Request<pb::DeleteSessionRequest>) -> Result<Response<pb::DeleteSessionResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn rebuild_session(&self, _: Request<pb::RebuildSessionRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn add_repo_to_session(&self, _: Request<pb::AddRepoToSessionRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn plan_drop_repo(&self, _: Request<pb::PlanDropRepoRequest>) -> Result<Response<pb::DropRepoPlan>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn drop_repo(&self, _: Request<pb::DropRepoRequest>) -> Result<Response<pb::Session>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn plan_push(&self, _: Request<pb::PlanPushRequest>) -> Result<Response<pb::PushPlan>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn push_and_open_pull_requests(&self, _: Request<pb::PushAndOpenPullRequestsRequest>) -> Result<Response<pb::Operation>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn read_conversation(&self, _: Request<pb::ReadConversationRequest>) -> Result<Response<pb::ReadConversationResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn send_note(&self, _: Request<pb::SendNoteRequest>) -> Result<Response<pb::SendNoteResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn ask_session(&self, _: Request<pb::AskSessionRequest>) -> Result<Response<pb::AskSessionResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn answer_question(&self, _: Request<pb::AnswerQuestionRequest>) -> Result<Response<pb::AnswerQuestionResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        async fn report_turn(&self, _: Request<pb::ReportTurnRequest>) -> Result<Response<pb::ReportTurnResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }

        type HookStream = ReceiverStream<Result<pb::HookReply, Status>>;

        async fn hook(&self, _: Request<pb::HookRequest>) -> Result<Response<Self::HookStream>, Status> {
            Err(Status::unimplemented("not used"))
        }
    }

    /// A new link to `machine`: its daemon answering on an in-memory pipe.
    async fn link_to(machine: &Machine) -> Channel {
        let (host, served) = tokio::io::duplex(64 * 1024);
        tokio::spawn(
            Server::builder()
                .add_service(pb::channel_service_server::ChannelServiceServer::new(machine.clone()))
                .add_service(pb::session_service_server::SessionServiceServer::new(machine.clone()))
                .add_service(pb::operation_service_server::OperationServiceServer::new(machine.clone()))
                .serve_with_incoming(tokio_stream::once(Ok::<_, std::io::Error>(Served(served)))),
        );
        let pipe = Arc::new(Mutex::new(Some(host)));
        let connected = Endpoint::from_static("http://relay.test")
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let pipe = lock(&pipe).take();
                async move { pipe.map(TokioIo::new).ok_or_else(|| std::io::Error::other("used")) }
            }))
            .await;
        match connected {
            Ok(channel) => channel,
            Err(error) => panic!("the in-memory link connects: {error}"),
        }
    }

    // -- the host ---------------------------------------------------------------

    #[derive(Default)]
    struct Remembered {
        saved: Mutex<BTreeMap<(String, String), RemoteView>>,
    }

    impl Memory for Remembered {
        fn load(&self) -> Result<Vec<RemoteView>, StoreError> {
            Ok(lock(&self.saved).values().cloned().collect())
        }

        fn save(&self, view: &RemoteView) -> Result<(), StoreError> {
            lock(&self.saved).insert((view.machine.clone(), view.session.clone()), view.clone());
            Ok(())
        }

        fn forget(&self, machine: &str, session: &str) -> Result<(), StoreError> {
            lock(&self.saved).remove(&(machine.to_owned(), session.to_owned()));
            Ok(())
        }
    }

    #[derive(Default)]
    struct Told {
        changes: Mutex<Vec<RemoteView>>,
        gone: Mutex<Vec<(String, String)>>,
    }

    #[tonic::async_trait]
    impl Sink for Told {
        async fn changed(&self, view: &RemoteView) {
            lock(&self.changes).push(view.clone());
        }

        async fn gone(&self, machine: &str, session: &str) {
            lock(&self.gone).push((machine.to_owned(), session.to_owned()));
        }
    }

    struct Rig {
        links: Links,
        reports: PeerReports,
        work: RemoteWork,
        memory: Arc<Remembered>,
        told: Arc<Told>,
        operations: Operations,
        feed: broadcast::Receiver<Arc<Published>>,
    }

    fn rig() -> Rig {
        let (links, reports) = (Links::default(), PeerReports::default());
        let (memory, told) = (Arc::new(Remembered::default()), Arc::new(Told::default()));
        let work = RemoteWork::new(links.clone(), reports.clone(), memory.clone(), told.clone());
        let feed = Arc::new(Feed::default());
        let (_, receiver) = feed.subscribe(None);
        Rig {
            links,
            reports,
            work,
            memory,
            told,
            operations: Operations::new(feed),
            feed: receiver,
        }
    }

    impl Rig {
        /// A host operation of `kind` with `steps`, as the host's services start one.
        fn host_op(&self, kind: pb::OperationKind, steps: &[(String, String)]) -> Arc<HostOp> {
            let target = pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId { value: "ch_1".to_owned() })),
            };
            match self.operations.start_steps(&Caller::User(UserVia::App), kind, target, steps, false, None) {
                Ok(Started::New(progress)) => Arc::new(HostOp::new(progress)),
                _ => panic!("the host's operation starts"),
            }
        }

        /// The newest state of the host's operation, as the feed told it.
        fn told_operation(&mut self) -> Option<pb::Operation> {
            let mut last = None;
            while let Ok(event) = self.feed.try_recv() {
                if let Some(pb::event::Payload::Operation(operation)) = &event.payload {
                    last = Some(operation.clone());
                }
            }
            last
        }
    }

    fn repo(owner: &str, name: &str) -> pb::RepoRef {
        pb::RepoRef {
            owner: owner.to_owned(),
            name: name.to_owned(),
        }
    }

    fn create_request() -> pb::CreateChannelRequest {
        pb::CreateChannelRequest {
            name: "Payments".to_owned(),
            repos: vec![repo("acme", "api")],
            ..pb::CreateChannelRequest::default()
        }
    }

    fn start_request() -> pb::StartSessionRequest {
        pb::StartSessionRequest {
            channel: Some(pb::ChannelId { value: "ch_1".to_owned() }),
            title: "Fix it".to_owned(),
            machine: Some(pb::MachineId { value: "devbox".to_owned() }),
            ..pb::StartSessionRequest::default()
        }
    }

    fn step_state(operation: &pb::Operation, key: &str) -> Option<pb::StepState> {
        operation
            .steps
            .iter()
            .find(|step| step.key == key)
            .and_then(|step| pb::StepState::try_from(step.state).ok())
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    fn status_report(session: &str, status: pb::SessionStatus) -> pb::MachineToHost {
        pb::MachineToHost {
            message: Some(pb::machine_to_host::Message::Status(pb::SessionStatusReport {
                session: Some(pb::SessionId { value: session.to_owned() }),
                status: status as i32,
                ..pb::SessionStatusReport::default()
            })),
        }
    }

    const CLONE: &str = "clone_main:devbox:acme/api";

    // -- channels ------------------------------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn a_channel_made_on_a_machine_shows_its_steps_per_machine_and_repo() {
        let mut rig = rig();
        let machine = Machine::default();
        rig.links.up("devbox", link_to(&machine).await);
        let host = rig.host_op(pb::OperationKind::AddRepo, &channel_steps("devbox", &[repo("acme", "api")]));
        let task = tokio::spawn({
            let (work, host) = (rig.work.clone(), host.clone());
            async move { work.create_channel_on("devbox", "ch_1", create_request(), host).await }
        });
        settle().await;

        machine.set("record", pb::StepState::Done);
        machine.set("clone_main:acme/api", pb::StepState::Running);
        settle().await;
        let shown = rig.told_operation().expect("the host's operation changed");
        assert_eq!(step_state(&shown, CLONE), Some(pb::StepState::Running));
        assert_eq!(step_state(&shown, "record:devbox"), Some(pb::StepState::Done));

        machine.end(pb::OperationState::Succeeded);
        assert_eq!(task.await.unwrap(), MachineOutcome::Ready);
        let shown = rig.told_operation().expect("the host's operation changed");
        assert_eq!(step_state(&shown, CLONE), Some(pb::StepState::Done));
        assert!(shown.steps.iter().all(|step| step.key.contains("devbox")));
    }

    #[tokio::test(start_paused = true)]
    async fn an_offline_machines_steps_wait_and_its_channel_is_made_once_when_the_link_returns() {
        let mut rig = rig();
        let machine = Machine::default();
        let host = rig.host_op(pb::OperationKind::AddRepo, &channel_steps("devbox", &[repo("acme", "api")]));
        let task = tokio::spawn({
            let (work, host) = (rig.work.clone(), host.clone());
            async move { work.create_channel_on("devbox", "ch_1", create_request(), host).await }
        });

        settle().await;
        assert!(machine.calls().is_empty());
        let shown = host.operation().expect("still running");
        assert!(shown.steps.iter().all(|step| step.state == pb::StepState::Waiting as i32));

        rig.links.up("devbox", link_to(&machine).await);
        settle().await;
        machine.end(pb::OperationState::Succeeded);

        assert_eq!(task.await.unwrap(), MachineOutcome::Ready);
        assert_eq!(machine.calls(), vec!["create_channel:ch_1".to_owned()]);
        assert_eq!(step_state(&rig.told_operation().unwrap(), CLONE), Some(pb::StepState::Done));
        // Later links don't call it again.
        rig.links.down("devbox");
        rig.links.up("devbox", link_to(&machine).await);
        settle().await;
        assert_eq!(machine.calls().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_machine_removed_while_its_steps_wait_has_them_skipped() {
        let rig = rig();
        let host = rig.host_op(pb::OperationKind::AddRepo, &channel_steps("devbox", &[repo("acme", "api")]));
        let task = tokio::spawn({
            let (work, host) = (rig.work.clone(), host.clone());
            async move { work.create_channel_on("devbox", "ch_1", create_request(), host).await }
        });
        settle().await;

        rig.links.forget("devbox");

        assert_eq!(task.await.unwrap(), MachineOutcome::Gone);
        let shown = host.operation().expect("the caller ends it");
        assert!(shown.steps.iter().all(|step| step.state == pb::StepState::Skipped as i32));
    }

    #[tokio::test(start_paused = true)]
    async fn the_same_channel_asked_twice_is_one_channel() {
        let rig = rig();
        let machine = Machine::default();
        rig.links.up("devbox", link_to(&machine).await);
        let steps = channel_steps("devbox", &[repo("acme", "api")]);

        for _ in 0..2 {
            let host = rig.host_op(pb::OperationKind::AddRepo, &steps);
            let task = tokio::spawn({
                let work = rig.work.clone();
                async move { work.create_channel_on("devbox", "ch_1", create_request(), host).await }
            });
            settle().await;
            machine.end(pb::OperationState::Succeeded);
            assert_eq!(task.await.unwrap(), MachineOutcome::Ready);
        }

        assert_eq!(lock(&machine.state).channels.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn one_machine_failing_fails_only_its_own_steps() {
        let rig = rig();
        let machine = Machine::default();
        rig.links.up("devbox", link_to(&machine).await);
        let mut steps = channel_steps("devbox", &[repo("acme", "api")]);
        steps.extend(channel_steps("other", &[repo("acme", "api")]));
        let host = rig.host_op(pb::OperationKind::AddRepo, &steps);
        let task = tokio::spawn({
            let (work, host) = (rig.work.clone(), host.clone());
            async move { work.create_channel_on("devbox", "ch_1", create_request(), host).await }
        });
        settle().await;
        machine.edit(|operation| {
            operation.state = pb::OperationState::Failed as i32;
            operation.error = Some(plain_error("the clone failed", "check the repository"));
            for step in &mut operation.steps {
                if step.key.starts_with("clone_main") {
                    step.state = pb::StepState::Failed as i32;
                }
            }
        });

        assert!(matches!(task.await.unwrap(), MachineOutcome::Failed(_)));
        let shown = host.operation().expect("the caller ends it");
        assert_eq!(step_state(&shown, CLONE), Some(pb::StepState::Failed));
        assert_eq!(step_state(&shown, "clone_main:other:acme/api"), Some(pb::StepState::Waiting));
    }

    // -- sessions ---------------------------------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn a_remote_start_answers_early_and_its_steps_are_mirrored_to_the_end() {
        let mut rig = rig();
        let machine = Machine::default();
        rig.links.up("devbox", link_to(&machine).await);
        let host = rig.host_op(
            pb::OperationKind::StartSession,
            &[("write_record", "Saving"), ("launch", "Starting"), ("verify", "Checking")].map(|(key, title)| (key.to_owned(), title.to_owned())),
        );

        let answer = rig.work.start_session_on("devbox", "se_1", start_request(), host.clone()).await.unwrap();

        assert_eq!(answer.session.map(|id| id.value), Some("se_1".to_owned()));
        assert_eq!(answer.operation.map(|operation| operation.state), Some(pb::OperationState::Running as i32));
        assert_eq!(machine.calls(), vec!["start_session:se_1".to_owned()]);
        machine.set("write_record", pb::StepState::Done);
        machine.set("launch", pb::StepState::Running);
        settle().await;
        let shown = rig.told_operation().expect("told");
        assert_eq!(step_state(&shown, "launch"), Some(pb::StepState::Running));
        machine.end(pb::OperationState::Succeeded);
        settle().await;
        let shown = rig.told_operation().expect("told");
        assert_eq!(shown.state, pb::OperationState::Succeeded as i32);
        assert!(matches!(shown.result, Some(pb::operation::Result::Session(id)) if id.value == "se_1"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_that_drops_mid_start_finishes_from_the_machines_report_and_starts_nothing_twice() {
        let mut rig = rig();
        let machine = Machine::default();
        rig.links.up("devbox", link_to(&machine).await);
        let host = rig.host_op(pb::OperationKind::StartSession, &[("launch".to_owned(), "Starting".to_owned())]);
        rig.work.start_session_on("devbox", "se_1", start_request(), host).await.unwrap();
        settle().await;

        rig.links.down("devbox");
        machine.restart();
        rig.links.up("devbox", link_to(&machine).await);
        settle().await;
        // The machine's session is still coming up: nothing ends the start yet.
        assert_eq!(rig.told_operation().map(|operation| operation.state), Some(pb::OperationState::Running as i32));
        rig.reports.link_up("devbox");
        rig.reports.accept("devbox", status_report("se_1", pb::SessionStatus::Working));
        settle().await;

        let shown = rig.told_operation().expect("told");
        assert_eq!(shown.state, pb::OperationState::Succeeded as i32);
        assert_eq!(machine.calls(), vec!["start_session:se_1".to_owned()]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_start_the_machine_lost_before_it_recorded_is_asked_again_under_the_same_id() {
        let mut rig = rig();
        let machine = Machine::default();
        rig.links.up("devbox", link_to(&machine).await);
        let host = rig.host_op(pb::OperationKind::StartSession, &[("launch".to_owned(), "Starting".to_owned())]);
        rig.work.start_session_on("devbox", "se_1", start_request(), host).await.unwrap();
        settle().await;

        rig.links.down("devbox");
        machine.restart();
        lock(&machine.state).sessions.clear();
        rig.links.up("devbox", link_to(&machine).await);
        settle().await;
        machine.end(pb::OperationState::Succeeded);
        settle().await;

        assert_eq!(machine.calls(), vec!["start_session:se_1".to_owned(), "start_session:se_1".to_owned()]);
        assert_eq!(lock(&machine.state).sessions.len(), 1);
        assert_eq!(rig.told_operation().map(|operation| operation.state), Some(pb::OperationState::Succeeded as i32));
    }

    #[tokio::test(start_paused = true)]
    async fn offline_start_stop_and_resume_answer_the_offline_error_at_once_and_queue_nothing() {
        let rig = rig();
        let machine = Machine::default();
        let host = rig.host_op(pb::OperationKind::StartSession, &[]);

        let attempts = async {
            let started = rig.work.start_session_on("devbox", "se_1", start_request(), host.clone()).await.map(|_| ());
            let stopped = rig.work.stop_session_on("devbox", "se_1").await.map(|_| ());
            let resumed = rig.work.resume_session_on("devbox", "se_1", pb::ResumeSessionRequest::default(), host.clone()).await.map(|_| ());
            (started, stopped, resumed)
        };
        let (started, stopped, resumed) = tokio::time::timeout(Duration::from_secs(1), attempts).await.expect("they answer at once");

        for refused in [started, stopped, resumed] {
            let Err(RemoteError::Relay(RelayError::Offline { machine: named })) = refused else {
                panic!("the call is refused as offline");
            };
            assert_eq!(named, "devbox");
        }
        // The machine comes up later: nothing was kept to run.
        rig.links.up("devbox", link_to(&machine).await);
        settle().await;
        assert!(machine.calls().is_empty());
        assert_eq!(Status::from(RemoteError::Relay(RelayError::Offline { machine: "devbox".into() })).code(), Code::Unavailable);
    }

    #[tokio::test(start_paused = true)]
    async fn stop_and_resume_are_relayed_and_resume_is_mirrored() {
        let mut rig = rig();
        let machine = Machine::default();
        rig.links.up("devbox", link_to(&machine).await);
        let host = rig.host_op(pb::OperationKind::StartSession, &[("launch".to_owned(), "Starting".to_owned()), ("verify".to_owned(), "Checking".to_owned())]);

        let stopped = rig.work.stop_session_on("devbox", "se_1").await.unwrap();
        rig.work.resume_session_on("devbox", "se_1", pb::ResumeSessionRequest::default(), host).await.unwrap();
        settle().await;
        machine.end(pb::OperationState::Succeeded);
        settle().await;

        assert_eq!(stopped.status, pb::SessionStatus::Idle as i32);
        assert_eq!(machine.calls(), vec!["stop_session:se_1".to_owned(), "resume_session:se_1".to_owned()]);
        assert_eq!(rig.told_operation().map(|operation| operation.state), Some(pb::OperationState::Succeeded as i32));
    }

    // -- reports ------------------------------------------------------------------------

    /// Waits, on the real clock (the stores are called on blocking threads),
    /// until `ready` holds.
    async fn eventually(mut ready: impl FnMut() -> bool) {
        for _ in 0..500 {
            if ready() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("it never happened");
    }

    fn told_status(told: &Told, session: &str) -> Option<pb::SessionStatus> {
        lock(&told.changes).iter().rev().find(|view| view.session == session).map(RemoteView::shown)
    }

    #[tokio::test]
    async fn reports_become_status_changes_go_offline_with_the_link_and_count_reattached_sessions() {
        let rig = rig();
        let follower = tokio::spawn({
            let work = rig.work.clone();
            async move { work.follow_reports().await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        rig.reports.link_up("devbox");
        rig.reports.accept("devbox", status_report("se_1", pb::SessionStatus::Working));
        rig.reports.accept("devbox", status_report("se_2", pb::SessionStatus::Exited));
        rig.reports.snapshot_complete("devbox");
        eventually(|| told_status(&rig.told, "se_1") == Some(pb::SessionStatus::Working)).await;
        assert!(lock(&rig.memory.saved).contains_key(&("devbox".to_owned(), "se_1".to_owned())));

        let last_up = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        rig.reports.link_down("devbox", last_up);
        eventually(|| told_status(&rig.told, "se_1") == Some(pb::SessionStatus::Offline)).await;
        let offline = rig.work.view("devbox", "se_1").expect("kept");
        assert_eq!(offline.last_seen, Some(last_up));
        assert_eq!(offline.reported, pb::SessionStatus::Working);
        assert_eq!(rig.work.reattached("devbox"), Reattached { count: 0, waiting: 1 });

        rig.reports.link_up("devbox");
        rig.reports.accept("devbox", status_report("se_1", pb::SessionStatus::Working));
        rig.reports.accept("devbox", status_report("se_2", pb::SessionStatus::Exited));
        rig.reports.snapshot_complete("devbox");
        eventually(|| told_status(&rig.told, "se_1") == Some(pb::SessionStatus::Working)).await;
        // The exited session wasn't running, so it isn't "reattached".
        assert_eq!(rig.work.reattached("devbox"), Reattached { count: 1, waiting: 0 });
        follower.abort();
    }

    #[tokio::test]
    async fn a_session_the_machine_no_longer_has_is_told_gone_and_forgotten() {
        let rig = rig();
        let follower = tokio::spawn({
            let work = rig.work.clone();
            async move { work.follow_reports().await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        rig.reports.link_up("devbox");
        rig.reports.accept("devbox", status_report("se_1", pb::SessionStatus::Idle));
        rig.reports.snapshot_complete("devbox");
        eventually(|| rig.work.view("devbox", "se_1").is_some()).await;

        rig.reports.link_down("devbox", SystemTime::UNIX_EPOCH);
        rig.reports.link_up("devbox");
        rig.reports.snapshot_complete("devbox");

        eventually(|| rig.work.view("devbox", "se_1").is_none()).await;
        eventually(|| lock(&rig.memory.saved).is_empty()).await;
        assert_eq!(*lock(&rig.told.gone), vec![("devbox".to_owned(), "se_1".to_owned())]);
        follower.abort();
    }

    #[tokio::test]
    async fn a_hosts_restart_shows_what_it_remembered_as_offline_until_the_machine_reports() {
        let rig = rig();
        let before = RemoteView {
            machine: "devbox".to_owned(),
            session: "se_1".to_owned(),
            reported: pb::SessionStatus::Working,
            exit_code: 0,
            problem: None,
            pointers: Vec::new(),
            pointers_at: None,
            reported_at: SystemTime::UNIX_EPOCH,
            live: true,
            last_seen: None,
        };
        rig.memory.save(&before).unwrap();
        let follower = tokio::spawn({
            let work = rig.work.clone();
            async move { work.follow_reports().await }
        });

        eventually(|| rig.work.view("devbox", "se_1").is_some()).await;

        let mut session = pb::Session {
            id: Some(pb::SessionId { value: "se_1".to_owned() }),
            machine: Some(pb::MachineId { value: "devbox".to_owned() }),
            status: pb::SessionStatus::Starting as i32,
            ..pb::Session::default()
        };
        rig.work.merge(&mut session);
        assert_eq!(session.status, pb::SessionStatus::Offline as i32);
        assert_eq!(rig.work.reattached("devbox").waiting, 1);
        follower.abort();
    }

    fn unreported_session() -> pb::Session {
        pb::Session {
            id: Some(pb::SessionId { value: "se_1".to_owned() }),
            machine: Some(pb::MachineId { value: "devbox".to_owned() }),
            status: pb::SessionStatus::Starting as i32,
            exit_code: 7,
            ..pb::Session::default()
        }
    }

    #[test]
    fn a_session_the_machine_never_reported_shows_offline_while_its_link_is_down() {
        let rig = rig();
        let mut session = unreported_session();
        rig.work.merge(&mut session);
        assert_eq!(session.status, pb::SessionStatus::Offline as i32);
        assert_eq!(session.exit_code, 7);
    }

    #[tokio::test]
    async fn a_session_the_machine_never_reported_keeps_the_hosts_status_while_its_link_is_up() {
        let rig = rig();
        let machine = Machine::default();
        rig.links.up("devbox", link_to(&machine).await);
        let mut session = unreported_session();
        rig.work.merge(&mut session);
        assert_eq!(session.status, pb::SessionStatus::Starting as i32);
    }

    #[test]
    fn a_machines_steps_are_named_for_it_and_its_repo() {
        let steps = channel_steps("devbox", &[repo("acme", "api"), repo("acme", "web")]);
        let keys: Vec<&str> = steps.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, vec!["record:devbox", "clone_main:devbox:acme/api", "clone_main:devbox:acme/web"]);
    }

    #[test]
    fn the_machine_without_the_host_id_is_refused_as_too_old() {
        let error = RemoteError::IdNotHonoured {
            machine: "devbox".to_owned(),
            asked: "se_1".to_owned(),
            got: "se_other".to_owned(),
        };
        assert_eq!(Status::from(error).code(), Code::Internal);
    }
}
