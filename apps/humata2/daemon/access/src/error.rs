//! Errors in using the policy, as opposed to refusals of a call.

use crate::capability::{Capability, Decision};
use crate::ids::{ChannelId, GrantId};

/// A call could not be checked: the daemon passed something the policy
/// can't judge. These are bugs or stale state in the daemon, not refusals.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckError {
    #[error(
        "channel {channel} has no access rules loaded; add the channel to the policy (or refresh the policy snapshot) before checking calls in it"
    )]
    UnknownChannel { channel: ChannelId },

    #[error(
        "the call's target is not one \"{capability}\" takes; build the target that capability's call names, and use the files or board call for files and boards"
    )]
    TargetDoesNotFit { capability: Capability },
}

/// Changing rules or grants failed. Nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error(
        "access rules, grants and the While I'm away rule change only on the Mac (AW-8); make the change in the app when the Mac is reachable"
    )]
    NeedsHost,

    #[error("\"{capability}\" has no channel rule; only the user does it, and no setting changes that")]
    NotARule { capability: Capability },

    #[error(
        "the rule for \"{capability}\" can't be set to {decision} (D20 offers only Ask me and Never for it); pick one of the settings it offers"
    )]
    SettingNotOffered {
        capability: Capability,
        decision: Decision,
    },

    #[error(
        "\"{capability}\" can't count as low-risk (AW-11): it may cross machines, reach the network, type into a session or is Never by default; remove it from the low-risk kinds"
    )]
    NotLowRisk { capability: Capability },

    #[error("channel {channel} has no access rules; add the channel first")]
    UnknownChannel { channel: ChannelId },

    #[error("channel {channel} already has access rules; change its rules instead of adding it again")]
    ChannelExists { channel: ChannelId },

    #[error("{grant} does not exist; it may have been revoked or ended with its session already")]
    UnknownGrant { grant: GrantId },

    #[error(
        "{grant} for \"{capability}\" has a target of the wrong kind for it (IN-22); the stored policy is damaged, so drop that grant and let the user grant it again"
    )]
    GrantDoesNotFit {
        grant: GrantId,
        capability: Capability,
    },

    #[error(
        "{grant} was not made on a machine with the approval passphrase while the Mac was away, so it is not replayed; only the user grants on the Mac, by answering a request"
    )]
    NotAMachineGrant { grant: GrantId },

    #[error(
        "{grant} appears twice in the stored policy; it is damaged, so drop the copies and let the user grant it again"
    )]
    DuplicateGrant { grant: GrantId },
}

/// Approving a request failed. Nothing ran and no grant was made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnswerError {
    #[error(
        "channel {channel} has no access rules loaded; refresh the policy before answering requests in it"
    )]
    UnknownChannel { channel: ChannelId },

    #[error(
        "channel {channel}'s rule for \"{capability}\" changed to Never after this was asked (IN-21), so it can't be approved; change the rule back in Agent access first if it should run"
    )]
    RuleIsNever {
        channel: ChannelId,
        capability: Capability,
    },

    #[error("\"{capability}\" has no channel rule, so no request for it can be approved")]
    NotARule { capability: Capability },

    #[error(
        "answering on a machine while the Mac is away needs the approval passphrase (AW-5); run `humata inbox` with it, or answer in the app when the Mac is back"
    )]
    PassphraseNeeded,

    #[error(
        "this request can only be answered on the Mac (a network connection or something on another machine); answer it in the app when the Mac is back"
    )]
    NeedsHost,

    #[error(
        "\"Always let …\" is not offered for this request; approve it once instead"
    )]
    NoStandingApproval,

    #[error(
        "the \"Always let …\" target is not the one the request offered (IN-14: it can't be widened or changed); approve with the offered target, or approve once"
    )]
    TargetNotOffered,
}
