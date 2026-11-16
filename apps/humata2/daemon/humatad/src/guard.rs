//! The capability check every handler makes (BOUNDARIES.md §3): who the
//! caller is, what it wants and to what, judged by humata2_access.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use humata2_access::{
    AccessRequest, Agent, Call, Capability, Context, Effect, MachineId, PeerLink, Policy,
    PresenceMethod, Principal, Refusal, SessionKind, Verdict,
};
use humata2_grpc::humata::v1 as pb;
use tonic::Status;

use crate::away::HostLink;
use crate::identity::{Caller, UserVia};
use crate::status;

/// The policy, with the channel rules every channel starts with, and where
/// it is checked. The Mac's reachability comes from the host link: a
/// machine's daemon starts away until the peer link opens PeerUplink (D213).
pub(crate) struct Guard {
    policy: Mutex<Policy>,
    here: MachineId,
    link: Arc<HostLink>,
}

/// What a check allows.
pub(crate) enum Checked {
    Allowed,
    /// The channel's rule asks the user first.
    Ask(AccessRequest),
}

/// What judging a call came to: a result, or a call that needs the Mac
/// while it is away.
enum Judged {
    Done(Checked),
    WaitForHost,
}

impl Guard {
    /// A machine's daemon starts away: the Mac is reachable only while the
    /// peer link holds PeerUplink open (D213). The host's daemon is always
    /// reachable to itself.
    pub(crate) fn new(machine: &str, role: pb::DaemonRole) -> Self {
        Self {
            policy: Mutex::new(Policy::new()),
            here: MachineId::new(machine),
            link: Arc::new(HostLink::new(!matches!(role, pb::DaemonRole::Machine))),
        }
    }

    /// The machine this daemon runs on.
    pub(crate) fn machine(&self) -> &str {
        self.here.as_str()
    }

    /// The host link this daemon's checks read, for the uplinks to open.
    pub(crate) fn host_link(&self) -> Arc<HostLink> {
        Arc::clone(&self.link)
    }

    /// Checks `call` for `caller`. A refusal is the status to return. A call
    /// that needs the Mac while it is away is refused, as it is by
    /// [`Guard::check_held`] once that waits for the Mac.
    pub(crate) fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status> {
        match self.judge(caller, call)? {
            Judged::Done(checked) => Ok(checked),
            Judged::WaitForHost => Err(status::refused(&Refusal::HostUnreachable)),
        }
    }

    /// Checks `call` like [`Guard::check`], but a call that needs the Mac
    /// while it is away waits until the Mac is reachable, then is judged
    /// again (TP-9).
    // Unused until AWAY T5 (the calls that wait for the Mac).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn check_held(
        &self,
        caller: &Caller,
        call: &Call,
    ) -> Result<Checked, Status> {
        loop {
            match self.judge(caller, call)? {
                Judged::Done(checked) => return Ok(checked),
                Judged::WaitForHost => self.link.until_reachable().await,
            }
        }
    }

    /// Judges `call` against the policy, holding the policy lock only here.
    fn judge(&self, caller: &Caller, call: &Call) -> Result<Judged, Status> {
        let principal = principal(caller);
        let context = self.context_for(caller);
        let mut policy = lock(&self.policy);
        // A Desk agent belongs to no channel: the policy answers it with
        // the new-channel rules, and nothing is added for it (D182.3).
        if let Caller::Agent(agent) = caller
            && !matches!(agent.session_kind(), SessionKind::Desk(_))
            && policy.channel_rules(&agent.session.channel).is_none()
            && let Err(error) = policy.add_channel(agent.session.channel.clone())
        {
            return Err(status::internal(
                format!(
                    "couldn't load channel {}'s access rules: {}",
                    agent.session.channel,
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
        match verdict {
            Verdict::Allowed {
                effect: Effect::Apply,
                ..
            } => Ok(Judged::Done(Checked::Allowed)),
            // The away journal and last copy are AWAY T5. Until they exist, a
            // call that would need them is refused as one that needs the Mac.
            Verdict::Allowed {
                effect: Effect::Journal | Effect::LastCopy,
                ..
            } => Err(status::refused(&Refusal::HostUnreachable)),
            Verdict::Ask(request) => Ok(Judged::Done(Checked::Ask(request))),
            Verdict::WaitForHost => Ok(Judged::WaitForHost),
            Verdict::Refused(refusal) => Err(status::refused(&refusal)),
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

    /// Checks `call`, refusing one that would need the user's approval.
    /// For calls that can't wait in the Inbox; a call that can uses
    /// [`Guard::check`] and, on `Checked::Ask`, `Inbox::defer` (D38.6).
    pub(crate) fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status> {
        match self.check(caller, call)? {
            Checked::Allowed => Ok(()),
            Checked::Ask(request) => Err(status::needs_approval(request.capability)),
        }
    }
}

impl Guard {
    /// Runs `with` on the policy, given where this daemon checks it: for
    /// answering requests and changing grants.
    pub(crate) fn with_policy<R>(&self, with: impl FnOnce(&mut Policy, &Context) -> R) -> R {
        let context = self.link.context(&self.here);
        with(&mut lock(&self.policy), &context)
    }
}

/// Who `caller` is to the policy.
pub(crate) fn principal(caller: &Caller) -> Principal {
    match caller {
        Caller::User(UserVia::App | UserVia::Host) => Principal::App,
        Caller::User(UserVia::CliLogin { .. }) => Principal::Cli(PresenceMethod::CliLogin),
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
        C::NetworkReach => P::NetworkReach,
        C::SessionStopOther => P::SessionStopOther,
        C::ChannelChange => P::ChannelChange,
        C::MachineDestroy => P::MachineDestroy,
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use humata2_access::{ChannelId, Entry, Place, RepoPath, Root, SessionId, SessionRef};
    use humata2_store::SessionRecord;

    use super::*;
    use crate::identity::AgentCaller;

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
        Caller::Agent(AgentCaller::of(&record(), MACHINE))
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
    async fn a_held_check_waits_while_away_and_runs_once_the_link_opens() {
        let guard = Arc::new(Guard::new(MACHINE, pb::DaemonRole::Machine));
        let caller = agent();
        let call = read_other_machine_file();
        let mut held = tokio::spawn({
            let guard = Arc::clone(&guard);
            async move { guard.check_held(&caller, &call).await }
        });
        let still_waiting = tokio::time::timeout(Duration::from_millis(100), &mut held).await;
        assert!(still_waiting.is_err(), "the check should wait for the Mac");

        let _opened = guard.host_link().open();
        let result = tokio::time::timeout(Duration::from_secs(5), held)
            .await
            .expect("the check should finish once the Mac is reachable")
            .expect("the check task should not panic");
        assert!(matches!(result, Ok(Checked::Ask(_)) | Ok(Checked::Allowed)));
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
}
