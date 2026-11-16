//! What a check decides.

use crate::call::{Call, GrantTarget};
use crate::capability::Capability;
use crate::ids::{ChannelId, GrantId, MachineId, RepoRef, SessionId};
use crate::principal::PresenceMethod;

/// The outcome of checking one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Do it.
    Allowed { source: Source, effect: Effect },
    /// Put this request in the Inbox (or, while the Mac is away, the
    /// machine's journal) and let the call wait for the answer.
    Ask(AccessRequest),
    /// The call needs the Mac, which is away: it waits until the Mac is
    /// back and is checked again then (TP-9).
    WaitForHost,
    /// Don't do it; tell the caller why and how to fix it.
    Refused(Refusal),
}

/// Why a call was allowed (`DecisionSource`, with the sources it lacks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The caller's own things, which no rule governs (IN-16).
    Owned,
    /// The channel rule is Allow.
    Rule,
    /// A grant covers it.
    Grant(GrantId),
    /// The While I'm away rule lists it as low-risk. The daemon journals it
    /// for the user to review (AW-4).
    AwayRule,
    /// The user, in the app or with presence shown.
    User,
    /// A script token lists it (SEC-14).
    ScriptToken,
    /// The user let this review session push to its PR branch (PR-8).
    ReviewPushSetting,
    /// One daemon calling another over the peer link.
    PeerLink,
}

/// How an allowed call is carried out on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Do it now.
    Apply,
    /// The Mac is away: do it in the journal, to be checked again and
    /// applied when the Mac is back (SL-5, AW-9).
    Journal,
    /// The Mac is away: answer from the last copy, marked with its age.
    LastCopy,
}

/// An Inbox request the policy wants made. The daemon stores it, gives it a
/// `RequestId` and passes both back to [`crate::Policy::approve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessRequest {
    pub capability: Capability,
    /// The channel whose rule asked.
    pub channel: ChannelId,
    pub requester: SessionId,
    pub call: Call,
    /// What "Always allow …" would grant, or `None` when it is not offered.
    pub always_let: Option<GrantTarget>,
    /// True for an ask-ahead (IN-26): approving grants `always_let` and
    /// runs nothing.
    pub ahead: bool,
    pub answer_at: AnswerAt,
}

/// Where a request can be answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerAt {
    /// In the Mac's Inbox.
    Host,
    /// On this machine with `humata inbox` and the approval passphrase,
    /// while the Mac is away; it replays to the Mac later (AW-5, AW-6).
    Machine(MachineId),
}

/// Why a call was refused. The message says what failed and how to fix it;
/// [`Refusal::kind`] gives the proto error and [`Refusal::fix`] the fix.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error(
        "only the user can {}; ask them with inbox_ask, and they can do it themselves in the app",
        capability_or_this(.capability)
    )]
    UserOnly { capability: Option<Capability> },

    #[error(
        "channel {channel}'s rule for \"{capability}\" is Never; nothing was added to the Inbox. Only the user can change the rule, in Agent access"
    )]
    NeverRule {
        channel: ChannelId,
        capability: Capability,
    },

    #[error(
        "session {fork} is a context-only fork and never writes its parent {parent}'s clones (IN-20); write in its own clone, or fork with its own copy"
    )]
    ParentReadOnly { fork: SessionId, parent: SessionId },

    #[error(
        "the main clone of {repo} on {machine} is a read-only reference (FC-2); write in a session clone instead"
    )]
    MainCloneReadOnly { repo: RepoRef, machine: MachineId },

    #[error(
        "the path on {machine} leads outside Humata's folders (by \"..\" or a symlink); use a path inside the session's clone or shared/"
    )]
    OutsideHumataFolders { machine: MachineId },

    #[error(
        "agents write only their own clones and their channel's shared/ folder on their own machine; copy what is needed into the clone, or send the owning session a note"
    )]
    NotItsFolder { machine: MachineId },

    #[error(
        "review session {session} does not push through Humata (PR-7); the user can turn on \"Let the agent push to the PR branch\" in the session's Info panel"
    )]
    ReviewSessionPush { session: SessionId },

    #[error(
        "this reaches into channel {channel}, outside the caller's own channel, and no rule covers that; ask the user with inbox_ask"
    )]
    OtherChannel { channel: ChannelId },

    #[error(
        "this needs the user to show they are there: {}",
        presence_methods(.accepted)
    )]
    PresenceRequired { accepted: Vec<PresenceMethod> },

    #[error(
        "the user is away and channel {channel}'s While I'm away rule is Skip, so the step was skipped; try again when they are back"
    )]
    UserAway { channel: ChannelId },

    #[error(
        "this needs the Mac, which can't be reached; try again when it is back. Under Skip it does not wait"
    )]
    HostUnreachable,

    #[error(
        "the script token does not list \"{capability}\"; make a new token that lists it with `humata token create` (a token can't be widened)"
    )]
    ScriptTokenLacks { capability: Capability },

    #[error(
        "\"{capability}\" needs the user's approval every time (HO-11), and a script can't wait for it; do it from the app or an agent session instead"
    )]
    ScriptTokenCannotAsk { capability: Capability },

    #[error(
        "the script token is not for channel {channel}; make a token for that channel with `humata token create`"
    )]
    ScriptTokenOtherChannel { channel: ChannelId },

    #[error(
        "a script token has no session, so it has no files of its own; do this from an agent session or the app"
    )]
    ScriptTokenHasNoSession,

    #[error("\"{capability}\" is only for one Humata daemon calling another over the peer link")]
    PeerOnly { capability: Capability },

    #[error(
        "the peer link carries only the daemons' own calls; forwarded calls are checked as their original caller"
    )]
    PeerCannotCall,

    #[error(
        "\"{capability}\" has no \"Always allow …\" to ask ahead for; make the call itself and the user is asked then"
    )]
    NothingToAskAhead { capability: Capability },
}

fn capability_or_this(capability: &Option<Capability>) -> &'static str {
    match capability {
        Some(capability) => capability.label(),
        None => "do this",
    }
}

fn presence_methods(accepted: &[PresenceMethod]) -> String {
    if accepted.is_empty() {
        return "no way works from a terminal Humata holds; run it from the app or a terminal outside Humata".to_owned();
    }
    let ways: Vec<&str> = accepted
        .iter()
        .map(|method| match method {
            PresenceMethod::ConfirmInApp => "confirm in the app",
            PresenceMethod::CliLogin => "run `humata login` in this terminal first",
            PresenceMethod::ApprovalPassphrase => "give the approval passphrase",
        })
        .collect();
    ways.join(", or ")
}

/// The proto error a refusal maps to (`humata.v1.Error`'s kinds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Denied(DeniedSource),
    UserOnly,
    PresenceRequired,
    UserAway,
    HostUnreachable,
    ParentReadOnly,
    MainCloneReadOnly,
    OutsideHumataFolders,
    InvalidField,
}

/// `Denied.Source`, plus the sources it lacks (see the decision log).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeniedSource {
    NeverRule,
    UserDenied,
    HardLimit,
    AwaySkip,
    /// Not in `errors.proto`: the script token does not allow it.
    ScriptToken,
    /// Not in `errors.proto`: the target is in another channel.
    OtherChannel,
    /// Not in `errors.proto`: peer-link calls and peer-only capabilities.
    PeerLink,
}

/// How to fix a refusal, for clients that offer the fix as an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// Ask the user with `inbox_ask`.
    AskTheUser,
    /// The user changes the channel's rule in Agent access.
    UserChangesRule {
        channel: ChannelId,
        capability: Capability,
    },
    /// Work in the caller's own clone or `shared/`.
    UseOwnFolders,
    /// The user lets the review session push (PR-8).
    UserAllowsReviewPush { session: SessionId },
    /// Show presence in one of these ways.
    ShowPresence { accepted: Vec<PresenceMethod> },
    /// Try again when the user is back.
    WaitForUser,
    /// Try again when the Mac is back.
    WaitForHost,
    /// Make a token that lists the capability.
    TokenWithCapability { capability: Capability },
    /// Make a token for the channel.
    TokenForChannel { channel: ChannelId },
    /// Do it from the app or an agent session.
    NotFromAScript,
    /// Make the call itself.
    MakeTheCall,
    /// Nothing the caller can do.
    Nothing,
}

impl Refusal {
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::UserOnly { .. } => ErrorKind::UserOnly,
            Self::NeverRule { .. } => ErrorKind::Denied(DeniedSource::NeverRule),
            Self::ParentReadOnly { .. } => ErrorKind::ParentReadOnly,
            Self::MainCloneReadOnly { .. } => ErrorKind::MainCloneReadOnly,
            Self::OutsideHumataFolders { .. } => ErrorKind::OutsideHumataFolders,
            Self::NotItsFolder { .. } | Self::ReviewSessionPush { .. } => {
                ErrorKind::Denied(DeniedSource::HardLimit)
            }
            Self::OtherChannel { .. } => ErrorKind::Denied(DeniedSource::OtherChannel),
            Self::PresenceRequired { .. } => ErrorKind::PresenceRequired,
            Self::UserAway { .. } => ErrorKind::UserAway,
            Self::HostUnreachable => ErrorKind::HostUnreachable,
            Self::ScriptTokenLacks { .. }
            | Self::ScriptTokenCannotAsk { .. }
            | Self::ScriptTokenOtherChannel { .. }
            | Self::ScriptTokenHasNoSession => ErrorKind::Denied(DeniedSource::ScriptToken),
            Self::PeerOnly { .. } | Self::PeerCannotCall => {
                ErrorKind::Denied(DeniedSource::PeerLink)
            }
            Self::NothingToAskAhead { .. } => ErrorKind::InvalidField,
        }
    }

    pub fn fix(&self) -> Fix {
        match self {
            Self::UserOnly { .. } | Self::OtherChannel { .. } => Fix::AskTheUser,
            Self::NeverRule {
                channel,
                capability,
            } => Fix::UserChangesRule {
                channel: channel.clone(),
                capability: *capability,
            },
            Self::ParentReadOnly { .. }
            | Self::MainCloneReadOnly { .. }
            | Self::OutsideHumataFolders { .. }
            | Self::NotItsFolder { .. } => Fix::UseOwnFolders,
            Self::ReviewSessionPush { session } => Fix::UserAllowsReviewPush {
                session: session.clone(),
            },
            Self::PresenceRequired { accepted } => Fix::ShowPresence {
                accepted: accepted.clone(),
            },
            Self::UserAway { .. } => Fix::WaitForUser,
            Self::HostUnreachable => Fix::WaitForHost,
            Self::ScriptTokenLacks { capability } => Fix::TokenWithCapability {
                capability: *capability,
            },
            Self::ScriptTokenOtherChannel { channel } => Fix::TokenForChannel {
                channel: channel.clone(),
            },
            Self::ScriptTokenCannotAsk { .. } | Self::ScriptTokenHasNoSession => {
                Fix::NotFromAScript
            }
            Self::NothingToAskAhead { .. } => Fix::MakeTheCall,
            Self::PeerOnly { .. } | Self::PeerCannotCall => Fix::Nothing,
        }
    }
}
