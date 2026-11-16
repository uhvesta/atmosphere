//! Capabilities, their tiers, and the settings a channel rule can take.
//!
//! These mirror `humata.v1.Capability`, `Decision` and `AwayMode`; the
//! daemon converts. The two deprecated proto values (D21) are not here:
//! the daemon reads `GLOBAL_BOARD_WRITE` as [`Capability::GlobalRead`] and
//! `OWN_COMMENTS_READ` as a read of the caller's own session.
//!
//! `GlobalRead` is Default Allow (D127.4: reads are allowed by default), so
//! reading `GLOBAL_BOARD_WRITE` as it would let global board writes run
//! unasked. When the board is built, global board writes must get their own
//! Ask-tier capability (D127.4).

use std::collections::BTreeSet;
use std::fmt;

/// One kind of action the access rules know about (BOUNDARIES.md §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    // Default Allow.
    OwnBoardWrite,
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
    ChannelBoardEditOthers,
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
    // Ask me, no Allow (D20).
    NetworkReach,
    // Never by default.
    SessionStopOther,
    ChannelChange,
    MachineDestroy,
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
    pub const ALL: [Capability; 48] = [
        Self::OwnBoardWrite,
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
        Self::NetworkReach,
        Self::SessionStopOther,
        Self::ChannelChange,
        Self::MachineDestroy,
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

    pub fn tier(self) -> Tier {
        match self {
            Self::OwnBoardWrite
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
            | Self::GlobalRead => Tier::DefaultAllow,
            Self::ChannelBoardEditOthers
            | Self::FilesReadOtherSession
            | Self::FilesReadOtherMachine
            | Self::FilesCopyIn
            | Self::SessionNote
            | Self::SessionType
            | Self::SessionStartSameMachine
            | Self::SessionStartOtherMachine
            | Self::PrReviewStart
            | Self::OwnSessionManage
            | Self::OwnSessionEnd
            | Self::SessionMove
            | Self::GitPush
            | Self::MachineProvision
            | Self::TrashRestore
            | Self::ChannelCreate => Tier::AskMe,
            Self::NetworkReach => Tier::AskNoAllow,
            Self::SessionStopOther | Self::ChannelChange | Self::MachineDestroy => {
                Tier::NeverByDefault
            }
            Self::InboxAnswer
            | Self::AccessChange
            | Self::SettingsChange
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
            | Self::ChannelChange
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
    /// still waits.
    pub fn may_be_low_risk(self) -> bool {
        matches!(self.tier(), Tier::DefaultAllow | Tier::AskMe)
            && !self.needs_host()
            && self != Self::SessionType
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
                | Self::NetworkReach
                | Self::GitPush
                | Self::MachineProvision
                | Self::MachineDestroy
        )
    }

    /// A short description, as the Agent access tab words it.
    pub fn label(self) -> &'static str {
        match self {
            Self::OwnBoardWrite => "change its own session board",
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
            Self::NetworkReach => "open a network connection through the Mac",
            Self::SessionStopOther => "stop or delete another session",
            Self::ChannelChange => "change channel settings or repos",
            Self::MachineDestroy => "remove, forget or delete machines",
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRules {
    decisions: std::collections::BTreeMap<Capability, Decision>,
    away: AwayRule,
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
