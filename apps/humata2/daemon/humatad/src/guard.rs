//! The capability check every handler makes (BOUNDARIES.md §3): who the
//! caller is, what it wants and to what, judged by humata2_access.

use std::sync::{Mutex, MutexGuard, PoisonError};

use humata2_access::{
    AccessRequest, Agent, Call, Capability, Context, Effect, MachineId, Policy, Principal,
    SessionKind, Verdict,
};
use humata2_grpc::humata::v1 as pb;
use tonic::Status;

use crate::identity::Caller;
use crate::status;

/// The policy, with the channel rules every channel starts with, and where
/// it is checked.
pub(crate) struct Guard {
    policy: Mutex<Policy>,
    context: Context,
}

/// What a check allows.
pub(crate) enum Checked {
    Allowed,
    /// The channel's rule asks the user first.
    Ask(AccessRequest),
}

impl Guard {
    /// A machine's daemon takes the Mac as reachable: the link to it isn't
    /// built yet.
    pub(crate) fn new(machine: &str, role: pb::DaemonRole) -> Self {
        let here = MachineId::new(machine);
        let context = match role {
            pb::DaemonRole::Machine => Context::machine(here, true),
            _ => Context::on_host(here),
        };
        Self {
            policy: Mutex::new(Policy::new()),
            context,
        }
    }

    /// Checks `call` for `caller`. A refusal is the status to return.
    pub(crate) fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status> {
        let principal = match caller {
            Caller::User => Principal::App,
            Caller::Agent(agent) => Principal::Agent(Agent {
                session: agent.session.clone(),
                kind: SessionKind::Ordinary,
            }),
        };
        let mut policy = lock(&self.policy);
        if let Caller::Agent(agent) = caller
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
        let verdict = policy.check(&principal, call, &self.context).map_err(|error| {
            status::internal(
                format!("couldn't check the call: {}", status::chain(&error)),
                "",
            )
        })?;
        match verdict {
            Verdict::Allowed {
                effect: Effect::Apply,
                ..
            } => Ok(Checked::Allowed),
            Verdict::Allowed { effect, .. } => Err(status::internal(
                format!("the policy said to carry the call out as {effect:?}, which this daemon doesn't do while the Mac is reachable"),
                "",
            )),
            Verdict::Ask(request) => Ok(Checked::Ask(request)),
            Verdict::WaitForHost => Err(status::refused(&humata2_access::Refusal::HostUnreachable)),
            Verdict::Refused(refusal) => Err(status::refused(&refusal)),
        }
    }

    /// Checks `call`, refusing one that would need the user's approval:
    /// only questions go to the Inbox in this build.
    pub(crate) fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status> {
        match self.check(caller, call)? {
            Checked::Allowed => Ok(()),
            Checked::Ask(request) => Err(status::needs_approval(request.capability)),
        }
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
        C::ChannelBoardEditOthers => P::ChannelBoardEditOthers,
        C::GlobalRead => P::GlobalRead,
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
