//! The capability check every handler makes (BOUNDARIES.md §3): who the
//! caller is, what it wants and to what, judged by humata2_access.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_access::{
    AccessRequest, Agent, AwayRule, Call, Capability, Context, Effect, MachineId, PeerLink,
    Policy, Principal, Refusal, SessionKind, Source, Verdict,
};
use humata2_grpc::humata::v1 as pb;
use tokio::sync::watch;
use tokio::time::{Instant, timeout, timeout_at};
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::{Code, Status};

use crate::away::HostLink;
use crate::identity::{Caller, UserVia};
use crate::status;

/// The status metadata key that marks a refusal as "needs the Mac, which is
/// away". Only [`Guard::held`] reads it, and strips it before anything
/// leaves the daemon.
const WAITS_FOR_HOST: &str = "humata-waits-for-host";

/// How long a held call of an agent waits for the Mac when it names no
/// timeout of its own: under the 45 seconds the MCP bridge waits.
const AGENT_HOLD: Duration = Duration::from_secs(40);

/// The policy, with the channel rules every channel starts with, and where
/// it is checked. The Mac's reachability comes from the host link: a
/// machine's daemon starts away until the peer link opens PeerUplink (D213).
pub struct Guard {
    policy: Mutex<Policy>,
    here: MachineId,
    /// What a refusal calls machines: the host's own is "This Mac" (D755).
    names: crate::upgrade::items::Names,
    link: Arc<HostLink>,
    /// The revision of what a machine's policy copy holds (AW-8): bumped
    /// when the policy changes, and by [`Guard::policy_changed`] for the
    /// rest the copy carries.
    revision: watch::Sender<u64>,
    /// Whether the host's rules are in force here: always on the host,
    /// and on a machine once a policy copy was kept or restored. Contain
    /// agents is one of the rules, and a machine without them would read
    /// it as off (D266).
    rules: watch::Sender<bool>,
    /// IN-8: `(utc_day, count)` of agent actions allowed today by a rule/grant
    /// without asking.
    allowed_today: Mutex<(u64, u32)>,
}

/// What a check allows.
pub enum Checked {
    Allowed,
    /// The channel's rule asks the user first.
    Ask(AccessRequest),
}

/// What judging a call came to: a result, or a call that needs the Mac
/// while it is away.
enum Judged {
    Done(Checked),
    WaitForHost,
    /// Allowed while the Mac is away only if the call is kept in the
    /// journal for it.
    Journal,
    /// Allowed while the Mac is away only if it is answered from the last
    /// copy of what the Mac said.
    LastCopy,
}

/// What an agent's board call comes to while the Mac is away (AW-7 rows 2
/// and 3, D227): which of them the handler may carry out.
pub enum AwayCall {
    /// A write the rules allow: it may be kept in the journal.
    Journal,
    /// A read the rules allow: it may be answered from the last copy.
    LastCopy,
    /// The rules ask the user first, and the While I'm away rule says to
    /// wait: the request is the machine's to hold (D268).
    Asks(Box<AccessRequest>),
    /// The Mac can be reached after all: the link came back since the
    /// handler looked.
    Reachable,
}

impl Guard {
    /// A machine's daemon starts away: the Mac is reachable only while the
    /// peer link holds PeerUplink open (D213). The host's daemon is always
    /// reachable to itself.
    pub(crate) fn new(machine: &str, role: pb::DaemonRole) -> Self {
        Self {
            policy: Mutex::new(Policy::new()),
            here: MachineId::new(machine),
            names: match role {
                pb::DaemonRole::Host => crate::upgrade::items::Names::of_host(machine),
                _ => crate::upgrade::items::Names::default(),
            },
            link: Arc::new(HostLink::new(!matches!(role, pb::DaemonRole::Machine))),
            revision: watch::Sender::new(next_revision(0)),
            rules: watch::Sender::new(!matches!(role, pb::DaemonRole::Machine)),
            allowed_today: Mutex::new((utc_day_now(), 0)),
        }
    }

    /// IN-8: how many agent actions followed rules or grants without asking today (UTC day).
    pub fn allowed_without_asking_today(&self) -> u32 {
        let today = utc_day_now();
        let state = lock(&self.allowed_today);
        if state.0 == today { state.1 } else { 0 }
    }

    fn record_allowed_today(&self) {
        let today = utc_day_now();
        let mut state = lock(&self.allowed_today);
        if state.0 == today {
            state.1 = state.1.saturating_add(1);
        } else {
            *state = (today, 1);
        }
    }

    /// The machine this daemon runs on.
    pub fn machine(&self) -> &str {
        self.here.as_str()
    }

    /// The host link this daemon's checks read, for the uplinks to open.
    pub fn host_link(&self) -> Arc<HostLink> {
        Arc::clone(&self.link)
    }

    /// Checks `call` for `caller`. A refusal is the status to return. A call
    /// that needs the Mac while it is away is refused with the mark
    /// [`Guard::held`] waits on; a handler that isn't held returns it as
    /// `host_unreachable`.
    pub fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status> {
        match self.judge(caller, call)? {
            Judged::Done(checked) => Ok(checked),
            Judged::WaitForHost => Err(waits_for_host()),
            // Only the Inbox and the boards journal what they take, and the
            // boards answer from the last copy (D225, D227): see `check_away`.
            Judged::Journal | Judged::LastCopy => Err(status::refused(&Refusal::HostUnreachable)),
        }
    }

    /// Checks an agent's board call made while the Mac is away, saying how
    /// the handler may carry it out. A refusal, and a call that needs the
    /// Mac itself (the Global board), is an error; unlike [`Guard::check`],
    /// a write the rules allow and a read the rules allow are not.
    pub fn check_away(&self, caller: &Caller, call: &Call) -> Result<AwayCall, Status> {
        match self.judge(caller, call)? {
            Judged::Journal => Ok(AwayCall::Journal),
            Judged::LastCopy => Ok(AwayCall::LastCopy),
            Judged::Done(Checked::Allowed) => Ok(AwayCall::Reachable),
            Judged::Done(Checked::Ask(asked)) => Ok(AwayCall::Asks(Box::new(asked))),
            // Not marked for `held`: a board call isn't held.
            Judged::WaitForHost => Err(status::refused(&Refusal::HostUnreachable)),
        }
    }

    /// Checks an agent's ask: a question, an ask-ahead or a permission
    /// card. Like [`Guard::check`], but while the Mac is away the ask is
    /// taken, since it waits in this machine's Inbox and every change to
    /// that Inbox is journaled for the host (AW-3, D225).
    pub(crate) fn check_inbox_ask(&self, caller: &Caller, call: &Call) -> Result<Checked, Status> {
        match self.judge(caller, call)? {
            Judged::Done(checked) => Ok(checked),
            Judged::Journal => Ok(Checked::Allowed),
            Judged::LastCopy => Err(status::refused(&Refusal::HostUnreachable)),
            Judged::WaitForHost => Err(waits_for_host()),
        }
    }

    /// Runs `run`, a handler's body that judges its call first. A call that
    /// needs the Mac while it is away comes back marked; this waits until the
    /// Mac is reachable and runs `run` again, so the call is judged anew
    /// (TP-9). Any other result, a refusal without the mark included, is
    /// returned at once. Only the wait is bounded: `bound` is how long the
    /// Mac may stay away (None waits for ever); a body that is running is
    /// never cut off. Past it the call fails as `host_unreachable`, with
    /// nothing kept, for the caller to repeat with the same key.
    ///
    /// `run` must judge before it changes anything, since it may run again.
    pub(crate) async fn held<T, F, Fut>(&self, bound: Option<Duration>, mut run: F) -> Result<T, Status>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, Status>>,
    {
        let until = bound.and_then(|bound| Instant::now().checked_add(bound));
        loop {
            match run().await {
                Err(failure) if failure.metadata().contains_key(WAITS_FOR_HOST) => {
                    if !self.reachable_by(until).await {
                        return Err(still_away());
                    }
                    // Judged again from a fresh turn, whatever the link did
                    // in between.
                    tokio::task::yield_now().await;
                }
                done => return done,
            }
        }
    }

    /// Waits until the Mac is reachable, or until `until`. False if it
    /// still is not.
    async fn reachable_by(&self, until: Option<Instant>) -> bool {
        match until {
            None => {
                self.link.until_reachable().await;
                true
            }
            Some(until) => timeout_at(until, self.link.until_reachable()).await.is_ok(),
        }
    }

    /// Judges `call` against the policy, holding the policy lock only here.
    fn judge(&self, caller: &Caller, call: &Call) -> Result<Judged, Status> {
        // D264: while the machine's sessions are stopped, no agent or
        // script is answered.
        if let Some(machine) = crate::stop::gate::stopped_machine()
            && matches!(caller, Caller::Agent(_) | Caller::Script(_))
        {
            return Err(status::refused(&Refusal::MachineStopped {
                machine: MachineId::new(machine),
            }));
        }
        let principal = principal(caller);
        let context = self.context_for(caller);
        let mut policy = lock(&self.policy);
        // A Desk agent belongs to no channel: the policy answers it with
        // the new-channel rules, and nothing is added for it (D182.3),
        // except the channel its move asks to go into, whose rule decides.
        if let Caller::Agent(agent) = caller
            && let Some(channel) = rules_channel(agent, call)
            && policy.channel_rules(&channel).is_none()
            && let Err(error) = policy.add_channel(channel.clone())
        {
            return Err(status::internal(
                format!(
                    "couldn't load channel {channel}'s access rules: {}",
                    status::chain(&error)
                ),
                "",
            ));
        }
        let verdict = policy
            .check(&principal, call, &context)
            .map_err(|error| {
                status::internal(
                    format!("couldn't check the call: {}", status::chain(&error)),
                    "",
                )
            })?;
        if let Verdict::Allowed { source, .. } = &verdict
            && matches!(caller, Caller::Agent(_))
            && matches!(
                source,
                Source::Rule | Source::Grant(_) | Source::DeskSwitch | Source::ReviewPushSetting
            )
        {
            self.record_allowed_today();
        }
        match verdict {
            Verdict::Allowed {
                effect: Effect::Apply,
                ..
            } => Ok(Judged::Done(Checked::Allowed)),
            Verdict::Allowed {
                effect: Effect::Journal,
                ..
            } => Ok(Judged::Journal),
            // Answered from the last copy of what the Mac said, by a handler
            // that has one (D227); any other refuses it as needing the Mac.
            Verdict::Allowed {
                effect: Effect::LastCopy,
                ..
            } => Ok(Judged::LastCopy),
            Verdict::Ask(request) => Ok(Judged::Done(Checked::Ask(request))),
            Verdict::WaitForHost => Ok(Judged::WaitForHost),
            Verdict::Refused(refusal) => Err(status::refused_named(&refusal, &self.names)),
        }
    }

    /// Where `caller`'s call is judged. A call from the host came through the
    /// peer link itself, so the host is reachable from here whatever the link
    /// state says.
    fn context_for(&self, caller: &Caller) -> Context {
        match caller {
            Caller::User(UserVia::Host) if !self.link.on_host() => {
                Context::machine(self.here.clone(), true)
            }
            _ => self.link.context(&self.here),
        }
    }

    /// Whether the channel's While I'm away rule, as it is now, lists the
    /// kind `asked` is about as low-risk: a journaled write the rules now
    /// ask about is still applied then, since the user chose to let that
    /// kind through while the Mac was away (AW-9, D268).
    pub fn lists_low_risk(&self, asked: &AccessRequest) -> bool {
        if asked.ahead || !asked.capability.may_be_low_risk() {
            return false;
        }
        lock(&self.policy)
            .channel_rules(&asked.channel)
            .is_some_and(|rules| {
                matches!(rules.away(), AwayRule::AllowLowRisk(kinds) if kinds.contains(&asked.capability))
            })
    }

    /// Checks `call`, refusing one that would need the user's approval.
    /// For calls that can't wait in the Inbox; a call that can uses
    /// [`Guard::check`] and, on `Checked::Ask`, `Inbox::defer` (D38.6).
    pub fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status> {
        match self.check(caller, call)? {
            Checked::Allowed => Ok(()),
            Checked::Ask(request) => Err(status::needs_approval(request.capability)),
        }
    }
}

/// The channel whose rules judge an agent's call and must be loaded first:
/// its own channel, or for a Desk agent only the channel its move asks to go
/// into (D182.3, D305).
fn rules_channel(
    agent: &crate::identity::AgentCaller,
    call: &Call,
) -> Option<humata2_access::ChannelId> {
    match (agent.session_kind(), call) {
        (
            SessionKind::Desk(_),
            Call::Use {
                capability: Capability::DeskMoveToChannel,
                target: humata2_access::Target::Channel(channel),
            },
        ) => Some(channel.clone()),
        (SessionKind::Desk(_), _) => None,
        _ => Some(agent.session.channel.clone()),
    }
}

impl Guard {
    /// Runs `with` on the policy, given where this daemon checks it: for
    /// answering requests and changing grants.
    pub(crate) fn with_policy<R>(&self, with: impl FnOnce(&mut Policy, &Context) -> R) -> R {
        let context = self.link.context(&self.here);
        let mut policy = lock(&self.policy);
        let before = policy.clone();
        let result = with(&mut policy, &context);
        if *policy != before {
            self.policy_changed();
        }
        result
    }

    /// The revision a policy copy made now carries.
    pub(crate) fn policy_revision(&self) -> u64 {
        *self.revision.borrow()
    }

    /// Changes each time a policy copy made now would differ.
    pub(crate) fn policy_changes(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    /// Says something else a policy copy carries changed: the approval
    /// passphrase or Contain agents.
    pub fn policy_changed(&self) {
        self.revision.send_modify(|revision| *revision = next_revision(*revision));
    }

    /// Whether the host's rules are in force here.
    pub(crate) fn has_host_rules(&self) -> bool {
        *self.rules.borrow()
    }

    /// Says a policy copy from the host is in force here.
    pub(crate) fn host_rules_arrived(&self) {
        self.rules.send_replace(true);
    }

    /// Waits for the host's rules before an agent is started, so Contain
    /// agents is known: at once on the host or once a copy is in force,
    /// else up to `bound` (the call's own hold bound, or the agents'
    /// 40 seconds). Past it the start is refused, nothing kept.
    pub(crate) async fn host_rules(&self, bound: Option<Duration>) -> Result<(), Status> {
        let mut arrived = self.rules.subscribe();
        let waited = timeout(bound.unwrap_or(AGENT_HOLD), arrived.wait_for(|arrived| *arrived)).await;
        if waited.is_ok_and(|waited| waited.is_ok()) {
            Ok(())
        } else {
            Err(host_rules_missing())
        }
    }
}

/// A revision after `last`: the clock in milliseconds, so one made after a
/// restart is still newer, or one more than `last` if the clock is behind.
fn next_revision(last: u64) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX));
    now.max(last.saturating_add(1))
}

/// Who `caller` is to the policy.
pub(crate) fn principal(caller: &Caller) -> Principal {
    match caller {
        Caller::User(UserVia::App | UserVia::Host) => Principal::App,
        Caller::User(UserVia::CliLogin { method, .. }) => Principal::Cli(*method),
        Caller::Agent(agent) => Principal::Agent(Agent {
            session: agent.session.clone(),
            kind: agent.session_kind(),
        }),
        Caller::Script(script) => Principal::Script(script.token.clone()),
        Caller::Peer => Principal::Peer(PeerLink::HostToDaemon),
    }
}

/// The proto's name for a capability.
pub(crate) fn capability(capability: Capability) -> pb::Capability {
    use Capability as C;
    use pb::Capability as P;
    match capability {
        C::OwnBoardWrite => P::OwnBoardWrite,
        C::TodoSuggest => P::TodoSuggest,
        C::TodoClose => P::TodoClose,
        C::ChannelRead => P::ChannelRead,
        C::ChannelBoardAdd => P::ChannelBoardAdd,
        C::ChannelBoardEditOwn => P::ChannelBoardEditOwn,
        C::SharedWrite => P::SharedWrite,
        C::FilesFetchLocal => P::FilesFetchLocal,
        C::ViewsPrepare => P::ViewsPrepare,
        C::ReviewDraft => P::ReviewDraft,
        C::InboxAsk => P::InboxAsk,
        C::ReviewFocus => P::ReviewFocus,
        C::ReviewNote => P::ReviewNote,
        C::ReviewOrder => P::ReviewOrder,
        C::CommentReply => P::CommentReply,
        C::CommentResolve => P::CommentResolve,
        C::ChannelBoardEditOthers => P::ChannelBoardEditOthers,
        C::GlobalRead => P::GlobalRead,
        C::GlobalBoardWrite => P::GlobalBoardWrite,
        C::FilesReadOtherSession => P::FilesReadOtherSession,
        C::FilesReadOtherMachine => P::FilesReadOtherMachine,
        C::FilesCopyIn => P::FilesCopyIn,
        C::SessionNote => P::SessionNote,
        C::SessionType => P::SessionType,
        C::SessionStartSameMachine => P::SessionStartSameMachine,
        C::SessionStartOtherMachine => P::SessionStartOtherMachine,
        C::PrReviewStart => P::PrReviewStart,
        C::OwnSessionManage => P::OwnSessionManage,
        C::OwnSessionEnd => P::OwnSessionEnd,
        C::SessionMove => P::SessionMove,
        C::GitPush => P::GitPush,
        C::MachineProvision => P::MachineProvision,
        C::TrashRestore => P::TrashRestore,
        C::ChannelCreate => P::ChannelCreate,
        C::ChannelRepoAdd => P::ChannelRepoAdd,
        C::NetworkReach => P::NetworkReach,
        C::DeskMoveToChannel => P::DeskMoveToChannel,
        C::SessionStopOther => P::SessionStopOther,
        C::SessionDeleteOther => P::SessionDeleteOther,
        C::SessionsDeleteMany => P::SessionsDeleteMany,
        C::MachineSessionsStop => P::MachineSessionsStop,
        C::ChannelChange => P::ChannelChange,
        C::MachineDestroy => P::MachineDestroy,
        C::MachineScriptRun => P::MachineScriptRun,
        C::ProviderImport => P::ProviderImport,
        C::InboxAnswer => P::InboxAnswer,
        C::AccessChange => P::AccessChange,
        C::SettingsChange => P::SettingsChange,
        C::MarksChange => P::MarksChange,
        C::PrSubmit => P::PrSubmit,
        C::UserReviewState => P::UserReviewState,
        C::TrashEmpty => P::TrashEmpty,
        C::HostSecrets => P::HostSecrets,
        C::ChannelDelete => P::ChannelDelete,
        C::MachineSettings => P::MachineSettings,
        C::TerminalInput => P::TerminalInput,
        C::PeerReport => P::PeerReport,
        C::HostControl => P::HostControl,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn utc_day_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400)
}

/// The refusal for a call that needs the Mac while it is away, marked for
/// [`Guard::held`].
pub(crate) fn waits_for_host() -> Status {
    let mut failure = status::refused(&Refusal::HostUnreachable);
    failure
        .metadata_mut()
        .insert(WAITS_FOR_HOST, MetadataValue::from_static("1"));
    failure
}

/// What a held call gets when the Mac stayed away past its wait.
fn still_away() -> Status {
    status::failure(
        Code::Unavailable,
        pb::Error {
            message: "the Mac is still away, so this call could not go on; nothing was kept"
                .to_owned(),
            fix: "repeat the call with the same idempotency key once the Mac can be reached"
                .to_owned(),
            retry: Some(pb::RetryAdvice {
                after: None,
                same_key: true,
            }),
            kind: Some(pb::error::Kind::HostUnreachable(pb::HostUnreachable {
                since: None,
            })),
        },
    )
}

/// What a start gets on a machine the host's rules haven't reached.
fn host_rules_missing() -> Status {
    status::failure(
        Code::FailedPrecondition,
        pb::Error {
            message: "the Mac's rules haven't reached this machine yet, so the agent was not started: without them it can't tell whether to run it contained".to_owned(),
            fix: "wait until this machine shows linked to the Mac in Humata, then start or resume the session again".to_owned(),
            retry: Some(pb::RetryAdvice {
                after: None,
                same_key: true,
            }),
            kind: None,
        },
    )
}

/// How long a held call of `caller` waits for the Mac: three quarters of the
/// call's own `grpc-timeout`, so this ends the wait before the transport
/// cuts the call off; else 40 seconds for an agent and no limit for a user
/// at a terminal.
pub(crate) fn hold_bound(caller: &Caller, metadata: &MetadataMap) -> Option<Duration> {
    let asked = metadata
        .get("grpc-timeout")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_timeout);
    match asked {
        Some(asked) => Some(asked - asked / 4),
        None => matches!(caller, Caller::Agent(_)).then_some(AGENT_HOLD),
    }
}

/// A `grpc-timeout` value: up to eight digits and a unit (H, M, S, m, u, n).
fn parse_timeout(text: &str) -> Option<Duration> {
    let (digits, unit) = text.split_at_checked(text.len().checked_sub(1)?)?;
    let amount: u64 = digits.parse().ok()?;
    Some(match unit {
        "H" => Duration::from_secs(amount.checked_mul(3600)?),
        "M" => Duration::from_secs(amount.checked_mul(60)?),
        "S" => Duration::from_secs(amount),
        "m" => Duration::from_millis(amount),
        "u" => Duration::from_micros(amount),
        "n" => Duration::from_nanos(amount),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use humata2_access::{
        BoardAct, BoardRef, ChannelId, Entry, Place, RepoPath, Root, SessionId, SessionRef,
    };
    use humata2_store::SessionRecord;

    use super::*;

    /// The machine this daemon is on, where the agent's session runs.
    const MACHINE: &str = "m1";
    /// Another machine, where the other session runs.
    const OTHER: &str = "m2";

    fn record() -> SessionRecord {
        SessionRecord {
            id: "se_a".to_owned(),
            channel: "ch_a".to_owned(),
            machine: MACHINE.to_owned(),
            title: "asker".to_owned(),
            agent: "claude".to_owned(),
            folder: std::path::PathBuf::new(),
            created_unix_ms: 0,
        }
    }

    /// The agent of a session on this machine, in channel `ch_a`.
    fn agent() -> Caller {
        Caller::Agent(crate::identity::agent_caller_of(&record(), MACHINE))
    }

    /// A session of the same channel on the other machine.
    fn other_session() -> SessionRef {
        SessionRef {
            id: SessionId::new("se_b"),
            channel: ChannelId::new("ch_a"),
            machine: MachineId::new(OTHER),
        }
    }

    /// A file in the other session's clone: needs the Mac when it is away.
    fn read_other_machine_file() -> Call {
        Call::ReadFiles(Place {
            machine: MachineId::new(OTHER),
            root: Root::Clone(other_session()),
            path: RepoPath::new("src/lib.rs").expect("a clean path"),
            entry: Entry::File,
        })
    }

    /// Whether `result` is the refusal a call gets while the Mac is away.
    fn refused_host_unreachable(result: &Result<Checked, Status>) -> bool {
        match result {
            Err(failure) => matches!(
                status::error_of(failure).and_then(|error| error.kind),
                Some(pb::error::Kind::HostUnreachable(_))
            ),
            Ok(_) => false,
        }
    }

    #[tokio::test]
    async fn a_host_has_its_rules_at_once() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Host);
        assert!(guard.host_rules(Some(Duration::from_millis(1))).await.is_ok());
    }

    #[tokio::test]
    async fn a_machine_with_no_copy_refuses_a_start_at_its_bound_and_says_to_retry() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Machine);
        let refused = guard
            .host_rules(Some(Duration::from_millis(50)))
            .await
            .expect_err("a machine with no copy can't tell whether to contain");
        assert_eq!(refused.code(), Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn a_start_waiting_for_the_copy_goes_on_when_it_arrives() {
        let guard = Arc::new(Guard::new(MACHINE, pb::DaemonRole::Machine));
        let waiting = tokio::spawn({
            let guard = Arc::clone(&guard);
            async move { guard.host_rules(Some(Duration::from_secs(30))).await }
        });
        guard.host_rules_arrived();
        let result = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the start should go on once the copy is in force")
            .expect("the wait should not panic");
        assert!(result.is_ok());
        assert!(guard.has_host_rules());
    }

    #[test]
    fn a_machine_refuses_a_read_of_another_machines_files_while_away() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Machine);
        let result = guard.check(&agent(), &read_other_machine_file());
        assert!(refused_host_unreachable(&result));
    }

    #[test]
    fn the_same_read_is_judged_once_the_link_is_open() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Machine);
        let _opened = guard.host_link().open();
        let result = guard.check(&agent(), &read_other_machine_file());
        assert!(matches!(result, Ok(Checked::Ask(_)) | Ok(Checked::Allowed)));
    }

    #[tokio::test]
    async fn a_held_check_runs_again_once_the_host_is_reachable() {
        let guard = Arc::new(Guard::new(MACHINE, pb::DaemonRole::Machine));
        let runs = Arc::new(AtomicUsize::new(0));
        let (caller, call) = (agent(), read_other_machine_file());
        let held = tokio::spawn({
            let (guard, runs) = (Arc::clone(&guard), Arc::clone(&runs));
            async move {
                guard
                    .held(None, || {
                        runs.fetch_add(1, Ordering::SeqCst);
                        async { guard.check(&caller, &call) }
                    })
                    .await
            }
        });
        // The first run was judged away and is now waiting for the host.
        while runs.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        assert!(!held.is_finished(), "the call should wait for the Mac");

        let _opened = guard.host_link().open();
        let result = tokio::time::timeout(Duration::from_secs(5), held)
            .await
            .expect("the check should finish once the Mac is reachable")
            .expect("the check task should not panic");
        assert!(matches!(result, Ok(Checked::Ask(_)) | Ok(Checked::Allowed)));
        assert_eq!(runs.load(Ordering::SeqCst), 2, "it was judged again, once");
    }

    #[tokio::test]
    async fn a_held_check_gives_up_at_its_bound_as_a_retryable_host_unreachable() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Machine);
        let (caller, call) = (agent(), read_other_machine_file());
        let result = guard
            .held(Some(Duration::from_millis(50)), || async {
                guard.check(&caller, &call)
            })
            .await;
        let Err(failure) = result else {
            panic!("a call held past its bound was let through");
        };
        let error = status::error_of(&failure).expect("the failure is typed");
        assert!(matches!(error.kind, Some(pb::error::Kind::HostUnreachable(_))));
        assert!(error.retry.is_some_and(|retry| retry.same_key));
        assert!(
            !failure.metadata().contains_key(WAITS_FOR_HOST),
            "the mark never leaves the guard"
        );
    }

    #[tokio::test]
    async fn a_held_run_that_fails_without_the_mark_fails_at_once() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Machine);
        let runs = AtomicUsize::new(0);
        let result: Result<(), Status> = guard
            .held(None, || {
                runs.fetch_add(1, Ordering::SeqCst);
                async { Err(status::missing("question")) }
            })
            .await;
        assert_eq!(
            result.expect_err("the failure is passed on").code(),
            Code::InvalidArgument
        );
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_hold_is_bounded_by_the_calls_own_timeout_else_by_who_calls() {
        let mut metadata = MetadataMap::new();
        assert_eq!(hold_bound(&agent(), &metadata), Some(AGENT_HOLD));
        assert_eq!(hold_bound(&Caller::User(UserVia::App), &metadata), None);

        metadata.insert("grpc-timeout", MetadataValue::from_static("8S"));
        let bound = hold_bound(&agent(), &metadata).expect("a timeout bounds the hold");
        assert!(!bound.is_zero() && bound < Duration::from_secs(8));
        assert_eq!(hold_bound(&Caller::User(UserVia::App), &metadata), Some(bound));

        metadata.insert("grpc-timeout", MetadataValue::from_static("8000m"));
        assert_eq!(hold_bound(&agent(), &metadata), Some(bound));
    }

    #[test]
    fn the_host_user_is_judged_with_the_host_reachable() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Machine);
        let result = guard.check(&Caller::User(UserVia::Host), &read_other_machine_file());
        assert!(matches!(result, Ok(Checked::Allowed)));
    }

    #[test]
    fn reading_a_session_on_another_machine_while_away_is_host_unreachable() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Machine);
        let read = Call::ReadSession(other_session());
        assert!(refused_host_unreachable(&guard.check(&agent(), &read)));

        let _opened = guard.host_link().open();
        assert!(matches!(
            guard.check(&agent(), &read),
            Ok(Checked::Allowed)
        ));
    }

    #[test]
    fn a_host_never_refuses_as_host_unreachable() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Host);
        let result = guard.check(&agent(), &read_other_machine_file());
        assert!(!refused_host_unreachable(&result));
        assert!(matches!(result, Ok(Checked::Ask(_))));
    }

    fn on_channel_board(act: BoardAct) -> Call {
        Call::Board {
            board: BoardRef::Channel(ChannelId::new("ch_a")),
            act,
        }
    }

    #[test]
    fn while_away_a_board_read_is_answered_from_the_copy_and_an_add_is_journaled() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Machine);
        assert!(matches!(
            guard.check_away(&agent(), &on_channel_board(BoardAct::Read)),
            Ok(AwayCall::LastCopy)
        ));
        assert!(matches!(
            guard.check_away(&agent(), &on_channel_board(BoardAct::Add)),
            Ok(AwayCall::Journal)
        ));
        // Neither is carried out by the ordinary check, which has no copy to use.
        let read = guard.check(&agent(), &on_channel_board(BoardAct::Read));
        assert!(refused_host_unreachable(&read));
        let add = guard.check(&agent(), &on_channel_board(BoardAct::Add));
        assert!(refused_host_unreachable(&add));
    }

    #[test]
    fn while_away_the_global_board_is_host_unreachable_unmarked() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Machine);
        let call = Call::Board {
            board: BoardRef::Global,
            act: BoardAct::Read,
        };
        let Err(failure) = guard.check_away(&agent(), &call) else {
            panic!("the Global board was served while the Mac is away");
        };
        assert!(matches!(
            status::error_of(&failure).and_then(|error| error.kind),
            Some(pb::error::Kind::HostUnreachable(_))
        ));
        assert!(!failure.metadata().contains_key(WAITS_FOR_HOST));
    }

    #[test]
    fn a_board_call_judged_once_the_link_is_open_is_not_an_away_call() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Machine);
        let _opened = guard.host_link().open();
        assert!(matches!(
            guard.check_away(&agent(), &on_channel_board(BoardAct::Read)),
            Ok(AwayCall::Reachable)
        ));
    }

    #[test]
    fn a_policy_change_moves_the_copy_revision_and_a_read_does_not() {
        let guard = Guard::new(MACHINE, pb::DaemonRole::Host);
        let first = guard.policy_revision();
        guard.with_policy(|policy, _| policy.channel_rules(&ChannelId::new("ch_a")).is_some());
        assert_eq!(guard.policy_revision(), first);

        guard.with_policy(|policy, _| policy.add_channel(ChannelId::new("ch_a")).unwrap());
        let added = guard.policy_revision();
        assert!(added > first);

        guard.policy_changed();
        assert!(guard.policy_revision() > added);
    }
}
