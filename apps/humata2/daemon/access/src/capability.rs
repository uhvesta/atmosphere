//! Capabilities, their tiers, and the settings a channel rule can take.
//!
//! These mirror `humata.v1.Capability`, `Decision` and `AwayMode`; the
//! daemon converts. The deprecated proto value `OWN_COMMENTS_READ` (D21)
//! is not here: the daemon reads it as a read of the caller's own session.
//!
//! Reads of the Global board and other channels are `GlobalRead`, Default
//! Allow (D127.4); writes there are `GlobalBoardWrite`, Ask me (D155).

use std::collections::BTreeSet;
use std::fmt;

/// One kind of action the access rules know about (BOUNDARIES.md §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    // Default Allow.
    OwnBoardWrite,
    TodoSuggest,
    ChannelRead,
    ChannelBoardAdd,
    ChannelBoardEditOwn,
    SharedWrite,
    FilesFetchLocal,
    ViewsPrepare,
    ReviewDraft,
    InboxAsk,
    // The review chat's three switches (PR-25): Default Allow in a PR
    // review session, and the user turns each off in the review itself.
    ReviewFocus,
    ReviewNote,
    ReviewOrder,
    // The agent's two tools on comments typed into its own session
    // (DK-9, SM-19): reply in the thread, resolve with a note.
    CommentReply,
    CommentResolve,
    // Reads of other channels and the global board (D127.4).
    GlobalRead,
    // Ask me.
    // An agent closing a to-do item.
    TodoClose,
    ChannelBoardEditOthers,
    // Add, edit, move or delete on the Global board or another channel's
    // boards, whoever made the item (BD-9, D155).
    GlobalBoardWrite,
    FilesReadOtherSession,
    FilesReadOtherMachine,
    FilesCopyIn,
    SessionNote,
    SessionType,
    SessionStartSameMachine,
    SessionStartOtherMachine,
    PrReviewStart,
    OwnSessionManage,
    OwnSessionEnd,
    SessionMove,
    GitPush,
    MachineProvision,
    TrashRestore,
    ChannelCreate,
    ChannelRepoAdd,
    // Ask me, no Allow (D20).
    NetworkReach,
    DeskMoveToChannel,
    // An agent asks to import a machine provider (MP-23, D792).
    ProviderImport,
    // Ask me by default (D306).
    SessionStopOther,
    // Never by default.
    SessionDeleteOther,
    // Delete many sessions at once (MASSDELETE); never by default, and
    // never the agent's own session.
    SessionsDeleteMany,
    MachineSessionsStop,
    ChannelChange,
    MachineDestroy,
    // SEAM (MACHINESCRIPTS, D688): run a machine script now.
    MachineScriptRun,
    // User-only (SEC-7).
    InboxAnswer,
    AccessChange,
    SettingsChange,
    MarksChange,
    PrSubmit,
    UserReviewState,
    TrashEmpty,
    HostSecrets,
    ChannelDelete,
    MachineSettings,
    TerminalInput,
    // Daemon to daemon.
    PeerReport,
    HostControl,
}

/// How a capability is governed (BOUNDARIES.md §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// A channel rule that starts at Allow.
    DefaultAllow,
    /// A channel rule that starts at Ask me.
    AskMe,
    /// A channel rule that starts at Ask me and offers only Ask me and
    /// Never (D20).
    AskNoAllow,
    /// A channel rule that starts at Never.
    NeverByDefault,
    /// No rule: only the user, present, may do it (SEC-7, IN-23).
    UserOnly,
    /// No rule: only one daemon calling another over the peer link.
    Peer,
}

/// A channel rule's setting (`humata.v1.Decision`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Decision {
    Allow,
    Ask,
    Never,
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Allow => "Allow",
            Self::Ask => "Ask me",
            Self::Never => "Never",
        })
    }
}

/// A channel's While I'm away rule (AW-4), for calls that would ask while
/// the Mac is away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AwayRule {
    /// The request is kept on the machine and the call waits.
    Wait,
    /// The call is refused, saying the user is away.
    Skip,
    /// The listed kinds run (and are journaled for the record); anything
    /// else waits. Only kinds with [`Capability::may_be_low_risk`].
    AllowLowRisk(BTreeSet<Capability>),
}

const SETTINGS_ALL: &[Decision] = &[Decision::Allow, Decision::Ask, Decision::Never];
const SETTINGS_NO_ALLOW: &[Decision] = &[Decision::Ask, Decision::Never];
const SETTINGS_NONE: &[Decision] = &[];

impl Capability {
    /// Every capability, once.
    pub const ALL: [Capability; 58] = [
        Self::OwnBoardWrite,
        Self::TodoSuggest,
        Self::TodoClose,
        Self::ChannelRead,
        Self::ChannelBoardAdd,
        Self::ChannelBoardEditOwn,
        Self::SharedWrite,
        Self::FilesFetchLocal,
        Self::ViewsPrepare,
        Self::ReviewDraft,
        Self::InboxAsk,
        Self::ReviewFocus,
        Self::ReviewNote,
        Self::ReviewOrder,
        Self::CommentReply,
        Self::CommentResolve,
        Self::ChannelBoardEditOthers,
        Self::GlobalRead,
        Self::GlobalBoardWrite,
        Self::FilesReadOtherSession,
        Self::FilesReadOtherMachine,
        Self::FilesCopyIn,
        Self::SessionNote,
        Self::SessionType,
        Self::SessionStartSameMachine,
        Self::SessionStartOtherMachine,
        Self::PrReviewStart,
        Self::OwnSessionManage,
        Self::OwnSessionEnd,
        Self::SessionMove,
        Self::GitPush,
        Self::MachineProvision,
        Self::TrashRestore,
        Self::ChannelCreate,
        Self::ChannelRepoAdd,
        Self::NetworkReach,
        Self::DeskMoveToChannel,
        Self::ProviderImport,
        Self::SessionStopOther,
        Self::SessionDeleteOther,
        Self::SessionsDeleteMany,
        Self::MachineSessionsStop,
        Self::ChannelChange,
        Self::MachineDestroy,
        Self::MachineScriptRun,
        Self::InboxAnswer,
        Self::AccessChange,
        Self::SettingsChange,
        Self::MarksChange,
        Self::PrSubmit,
        Self::UserReviewState,
        Self::TrashEmpty,
        Self::HostSecrets,
        Self::ChannelDelete,
        Self::MachineSettings,
        Self::TerminalInput,
        Self::PeerReport,
        Self::HostControl,
    ];

    /// The 10 safe local same-channel capabilities promoted from `AskMe` to
    /// `DefaultAllow` by the PERMAUDIT policy audit (ATM-30).
    pub fn permaudit_promoted() -> &'static [Capability] {
        &[
            Self::TodoClose,
            Self::ChannelBoardEditOthers,
            Self::FilesReadOtherSession,
            Self::FilesCopyIn,
            Self::SessionNote,
            Self::SessionStartSameMachine,
            Self::PrReviewStart,
            Self::OwnSessionManage,
            Self::OwnSessionEnd,
            Self::TrashRestore,
        ]
    }

    pub fn tier(self) -> Tier {
        match self {
            Self::OwnBoardWrite
            | Self::TodoSuggest
            | Self::TodoClose
            | Self::ChannelRead
            | Self::ChannelBoardAdd
            | Self::ChannelBoardEditOwn
            | Self::ChannelBoardEditOthers
            | Self::SharedWrite
            | Self::FilesFetchLocal
            | Self::FilesReadOtherSession
            | Self::FilesCopyIn
            | Self::ViewsPrepare
            | Self::ReviewDraft
            | Self::InboxAsk
            | Self::ReviewFocus
            | Self::ReviewNote
            | Self::ReviewOrder
            | Self::CommentReply
            | Self::CommentResolve
            | Self::GlobalRead
            | Self::SessionNote
            | Self::SessionStartSameMachine
            | Self::PrReviewStart
            | Self::OwnSessionManage
            | Self::OwnSessionEnd
            | Self::TrashRestore => Tier::DefaultAllow,
            Self::GlobalBoardWrite
            | Self::FilesReadOtherMachine
            | Self::SessionType
            | Self::SessionStartOtherMachine
            | Self::SessionMove
            | Self::GitPush
            | Self::MachineProvision
            | Self::ChannelCreate
            | Self::ChannelRepoAdd
            | Self::SessionStopOther => Tier::AskMe,
            Self::NetworkReach
            | Self::DeskMoveToChannel
            | Self::ProviderImport
            | Self::SettingsChange => Tier::AskNoAllow,
            Self::SessionDeleteOther
            | Self::SessionsDeleteMany
            | Self::MachineSessionsStop
            | Self::ChannelChange
            | Self::MachineDestroy
            | Self::MachineScriptRun => Tier::NeverByDefault,
            Self::InboxAnswer
            | Self::AccessChange
            | Self::MarksChange
            | Self::PrSubmit
            | Self::UserReviewState
            | Self::TrashEmpty
            | Self::HostSecrets
            | Self::ChannelDelete
            | Self::MachineSettings
            | Self::TerminalInput => Tier::UserOnly,
            Self::PeerReport | Self::HostControl => Tier::Peer,
        }
    }

    /// The setting a new channel starts with, or `None` when the capability
    /// has no rule (user-only and peer).
    pub fn default_decision(self) -> Option<Decision> {
        match self.tier() {
            Tier::DefaultAllow => Some(Decision::Allow),
            Tier::AskMe | Tier::AskNoAllow => Some(Decision::Ask),
            Tier::NeverByDefault => Some(Decision::Never),
            Tier::UserOnly | Tier::Peer => None,
        }
    }

    /// The settings the user may give this capability's rule. Empty when it
    /// has no rule.
    pub fn settings(self) -> &'static [Decision] {
        match self.tier() {
            Tier::DefaultAllow | Tier::AskMe | Tier::NeverByDefault => SETTINGS_ALL,
            Tier::AskNoAllow => SETTINGS_NO_ALLOW,
            Tier::UserOnly | Tier::Peer => SETTINGS_NONE,
        }
    }

    /// True when a channel rule governs it, so agents may hold it.
    pub fn has_rule(self) -> bool {
        !self.settings().is_empty()
    }

    /// True when doing it always goes through the Mac: another machine,
    /// GitHub, the network bridge, or state only the Mac keeps. While the
    /// Mac is away such calls wait or, under Skip, fail (AW-7).
    pub fn needs_host(self) -> bool {
        match self {
            Self::GlobalRead
            | Self::GlobalBoardWrite
            | Self::FilesReadOtherMachine
            | Self::SessionStartOtherMachine
            | Self::SessionMove
            | Self::GitPush
            | Self::PrReviewStart
            | Self::NetworkReach
            | Self::MachineProvision
            | Self::MachineDestroy
            | Self::TrashRestore
            | Self::ChannelCreate
            | Self::ChannelRepoAdd
            | Self::DeskMoveToChannel
            | Self::ProviderImport
            | Self::ChannelChange
            | Self::MachineScriptRun
            | Self::SessionsDeleteMany
            | Self::AccessChange
            | Self::SettingsChange
            | Self::MarksChange
            | Self::PrSubmit
            | Self::UserReviewState
            | Self::TrashEmpty
            | Self::HostSecrets
            | Self::ChannelDelete
            | Self::MachineSettings => true,
            Self::OwnBoardWrite
            | Self::TodoSuggest
            | Self::TodoClose
            | Self::ChannelRead
            | Self::ChannelBoardAdd
            | Self::ChannelBoardEditOwn
            | Self::SharedWrite
            | Self::FilesFetchLocal
            | Self::ViewsPrepare
            | Self::ReviewDraft
            | Self::InboxAsk
            | Self::ReviewFocus
            | Self::ReviewNote
            | Self::ReviewOrder
            | Self::CommentReply
            | Self::CommentResolve
            | Self::ChannelBoardEditOthers
            | Self::FilesReadOtherSession
            | Self::FilesCopyIn
            | Self::SessionNote
            | Self::SessionType
            | Self::SessionStartSameMachine
            | Self::OwnSessionManage
            | Self::OwnSessionEnd
            | Self::SessionStopOther
            | Self::SessionDeleteOther
            | Self::MachineSessionsStop
            | Self::InboxAnswer
            | Self::TerminalInput
            | Self::PeerReport
            | Self::HostControl => false,
        }
    }

    /// True when a While I'm away "Allow low-risk kinds" rule may list it
    /// (AW-11): it has a rule that can be Allow, it never needs the Mac,
    /// it is not typing into another session and it is not Never by
    /// default. A listed kind whose call turns out to need another machine
    /// still waits. Ending another session's work is never low-risk either.
    pub fn may_be_low_risk(self) -> bool {
        matches!(self.tier(), Tier::DefaultAllow | Tier::AskMe)
            && !self.needs_host()
            && self != Self::SessionType
            && self != Self::SessionStopOther
    }

    /// True when "Always allow …" can be offered for it: IN-22 names the
    /// target such a grant covers.
    pub fn offers_always_let(self) -> bool {
        matches!(
            self,
            Self::FilesReadOtherSession
                | Self::FilesReadOtherMachine
                | Self::FilesCopyIn
                | Self::SessionNote
                | Self::SessionType
                | Self::SessionStartSameMachine
                | Self::SessionStartOtherMachine
                | Self::ChannelBoardEditOthers
                | Self::GlobalBoardWrite
                | Self::NetworkReach
                | Self::GitPush
                | Self::MachineProvision
                | Self::MachineSessionsStop
                | Self::MachineDestroy
        )
    }

    /// A short description, as the Agent access tab words it.
    pub fn label(self) -> &'static str {
        match self {
            Self::OwnBoardWrite => "change its own session board",
            Self::TodoSuggest => "suggest to-do items",
            Self::TodoClose => "close to-do items",
            Self::ChannelRead => "see its channel",
            Self::ChannelBoardAdd => "add items to the channel board",
            Self::ChannelBoardEditOwn => "edit channel board items it created",
            Self::SharedWrite => "write into shared/ on its machine",
            Self::FilesFetchLocal => "fetch files on its machine into its clone",
            Self::ViewsPrepare => "prepare views for you",
            Self::ReviewDraft => "draft PR review comments",
            Self::InboxAsk => "ask you a question or ask ahead for access",
            Self::ReviewFocus => "move your review's view to a file or lines",
            Self::ReviewNote => "leave you notes in a review",
            Self::ReviewOrder => "order a review's files",
            Self::CommentReply => "reply in the thread of comments you sent it",
            Self::CommentResolve => "resolve comments you sent it, with a note on what it did",
            Self::ChannelBoardEditOthers => "edit anyone else's board items",
            Self::GlobalRead => "use the global board or other channels",
            Self::GlobalBoardWrite => "change the global board or other channels' boards",
            Self::FilesReadOtherSession => "read another session's files",
            Self::FilesReadOtherMachine => "read files on another machine",
            Self::FilesCopyIn => "copy files in from another session or machine",
            Self::SessionNote => "send a note or question to another session",
            Self::SessionType => "type into another session's terminal",
            Self::SessionStartSameMachine => "start or fork sessions on its machine",
            Self::SessionStartOtherMachine => "start or fork sessions on another machine",
            Self::PrReviewStart => "start a PR review",
            Self::OwnSessionManage => "change its own session",
            Self::OwnSessionEnd => "stop, archive or delete its own session",
            Self::SessionMove => "move a session to another machine",
            Self::GitPush => "push its branch or open a pull request",
            Self::MachineProvision => "add, create, start or stop machines",
            Self::TrashRestore => "restore from trash",
            Self::ChannelCreate => "create a channel",
            Self::ChannelRepoAdd => "add a repo to a channel",
            Self::NetworkReach => "open a network connection through the Mac",
            Self::DeskMoveToChannel => "move a Desk session into a channel",
            Self::ProviderImport => "ask you to import a machine provider",
            Self::SessionStopOther => "stop or archive another session",
            Self::SessionDeleteOther => "delete another session",
            Self::SessionsDeleteMany => "delete many sessions at once",
            Self::MachineSessionsStop => "stop or start all sessions on a machine",
            Self::ChannelChange => "change channel settings, or remove a repo from a channel",
            Self::MachineDestroy => "remove, forget or delete machines",
            Self::MachineScriptRun => "run machine scripts",
            Self::InboxAnswer => "answer Inbox requests",
            Self::AccessChange => "change access rules or grants",
            Self::SettingsChange => "change Settings",
            Self::MarksChange => "change marks",
            Self::PrSubmit => "submit a PR review",
            Self::UserReviewState => "change your review state",
            Self::TrashEmpty => "empty the trash",
            Self::HostSecrets => "use the Mac's secrets",
            Self::ChannelDelete => "delete a channel",
            Self::MachineSettings => "change a machine's settings",
            Self::TerminalInput => "type into a terminal or open a shell",
            Self::PeerReport => "report to the Mac as a machine",
            Self::HostControl => "control a machine as its Mac",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One channel's rules and its While I'm away rule.
#[derive(Debug, Clone, Eq)]
pub struct ChannelRules {
    decisions: std::collections::BTreeMap<Capability, Decision>,
    away: AwayRule,
    owner_set: BTreeSet<Capability>,
}

impl PartialEq for ChannelRules {
    fn eq(&self, other: &Self) -> bool {
        self.decisions == other.decisions && self.away == other.away
    }
}

impl ChannelRules {
    /// Every rule at its default and the While I'm away rule at Wait.
    pub fn defaults() -> Self {
        let decisions = Capability::ALL
            .iter()
            .filter_map(|&capability| {
                capability
                    .default_decision()
                    .map(|decision| (capability, decision))
            })
            .collect();
        Self {
            decisions,
            away: AwayRule::Wait,
            owner_set: BTreeSet::new(),
        }
    }

    /// The rule for a capability, or `None` when it has no rule.
    pub fn decision(&self, capability: Capability) -> Option<Decision> {
        if !capability.has_rule() {
            return None;
        }
        self.decisions
            .get(&capability)
            .copied()
            .or_else(|| capability.default_decision())
    }

    pub fn away(&self) -> &AwayRule {
        &self.away
    }

    pub(crate) fn set_decision(&mut self, capability: Capability, decision: Decision) {
        self.decisions.insert(capability, decision);
    }

    pub(crate) fn set_decision_by_owner(&mut self, capability: Capability, decision: Decision) {
        self.decisions.insert(capability, decision);
        self.owner_set.insert(capability);
    }

    /// True when the owner explicitly set this capability's rule.
    pub fn is_owner_set(&self, capability: Capability) -> bool {
        self.owner_set.contains(&capability)
    }

    pub(crate) fn set_away(&mut self, away: AwayRule) {
        self.away = away;
    }

    /// The rules as `(capability, decision)` pairs, for the daemon to store
    /// or send in a policy snapshot.
    pub fn decisions(&self) -> impl Iterator<Item = (Capability, Decision)> + '_ {
        self.decisions.iter().map(|(&c, &d)| (c, d))
    }
}

impl Default for ChannelRules {
    fn default() -> Self {
        Self::defaults()
    }
}

#[cfg(test)]
mod todo_rules_tests {
    use super::*;

    #[test]
    fn suggesting_and_closing_start_allowed() {
        assert_eq!(Capability::TodoSuggest.default_decision(), Some(Decision::Allow));
        assert_eq!(Capability::TodoClose.default_decision(), Some(Decision::Allow));
        assert!(Capability::ALL.contains(&Capability::TodoSuggest));
        assert!(Capability::ALL.contains(&Capability::TodoClose));
    }

    #[test]
    fn settings_change_is_ask_no_allow_and_never_offers_always_let() {
        assert_eq!(Capability::SettingsChange.tier(), Tier::AskNoAllow);
        assert_eq!(Capability::SettingsChange.default_decision(), Some(Decision::Ask));
        assert_eq!(
            Capability::SettingsChange.settings(),
            &[Decision::Ask, Decision::Never]
        );
        assert!(!Capability::SettingsChange.offers_always_let());
        assert!(!Capability::SettingsChange.may_be_low_risk());
    }
}
