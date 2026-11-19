//! The policy: channel rules, While I'm away rules and grants, and the
//! checks against them.
//!
//! Checks take `&self`: nothing a caller sends can change the policy. Rules
//! and grants change only through methods that take a [`PresentUser`], so
//! the only way a grant appears is the user approving a request with
//! "Always allow …" for exactly the target the request offered.

use std::collections::BTreeMap;
use std::time::SystemTime;

use crate::call::{
    BoardAct, BoardRef, Call, GrantTarget, MachineAction, MachineActionKind, PathScope, Place, Root,
    Target,
};
use crate::capability::{AwayRule, Capability as C, Capability, ChannelRules, Decision, Tier};
use crate::desk::{is_desk_channel, switch_allows};
use crate::error::{AnswerError, CheckError, PolicyError};
use crate::ids::{ChannelId, GrantId, MachineId, RequestId, SessionId};
use crate::principal::{
    Agent, Context, Host, PeerLink, PresenceMethod, PresentUser, Principal, ScriptToken,
    SessionKind,
};
use crate::verdict::{AccessRequest, AnswerAt, Effect, Refusal, Source, Verdict};

/// A standing approval made by "Always allow …" or an approved ask-ahead
/// (IN-14, IN-22, IN-26).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub id: GrantId,
    pub channel: ChannelId,
    pub capability: Capability,
    /// The one session it belongs to.
    pub requester: SessionId,
    pub target: GrantTarget,
    pub granted_from: RequestId,
    /// How the user showed presence; `None` for the app.
    pub granted_by: Option<PresenceMethod>,
    /// The machine the user answered on.
    pub granted_on: MachineId,
    /// False for a grant made on a machine while the Mac was away. It
    /// applies there at once and is replayed to the Mac (AW-12).
    pub on_host: bool,
    /// When the user answered. The daemon reads the clock; the policy
    /// only records it.
    pub granted_at: SystemTime,
}

/// The user's answer to a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Approve this call only. For an ask-ahead this still makes the
    /// grant it asked for.
    Once,
    /// Approve and grant the target the request offered. Any other target
    /// is refused.
    AlwaysLet(GrantTarget),
}

/// What approving did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    /// The grant made, if any.
    pub grant: Option<Grant>,
}

/// Which rules a change applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rules {
    Channel(ChannelId),
    /// The defaults new channels start with.
    NewChannels,
}

/// The policy as plain data, for the daemon to store and to send to a
/// machine as its policy snapshot. [`Policy::from_state`] checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyState {
    pub new_channels: RulesState,
    pub channels: BTreeMap<ChannelId, RulesState>,
    pub grants: Vec<Grant>,
    pub next_grant: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesState {
    pub decisions: Vec<(Capability, Decision)>,
    pub away: AwayRule,
}

/// Every channel's rules and every grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    new_channels: ChannelRules,
    channels: BTreeMap<ChannelId, ChannelRules>,
    grants: BTreeMap<GrantId, Grant>,
    next_grant: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Self::new()
    }
}

impl Policy {
    /// No channels and no grants; new channels start at the defaults.
    pub fn new() -> Self {
        Self {
            new_channels: ChannelRules::defaults(),
            channels: BTreeMap::new(),
            grants: BTreeMap::new(),
            next_grant: 1,
        }
    }

    // ----- Checks. -----

    /// Decides one call.
    pub fn check(
        &self,
        principal: &Principal,
        call: &Call,
        context: &Context,
    ) -> Result<Verdict, CheckError> {
        match principal {
            Principal::App => Ok(check_user(None, call, context)),
            Principal::Cli(method) => Ok(check_user(Some(*method), call, context)),
            Principal::Agent(agent) => self.check_agent(agent, call, context),
            Principal::Script(token) => Ok(check_script(token, call, context)),
            Principal::Peer(link) => Ok(check_peer(link, call)),
        }
    }

    /// Decides an agent asking ahead for access to a call it will make
    /// (IN-26). Approving the request makes the grant "Always allow …" would;
    /// a low-risk While I'm away rule never approves it.
    pub fn check_ask_ahead(
        &self,
        agent: &Agent,
        call: &Call,
        context: &Context,
    ) -> Result<Verdict, CheckError> {
        let channel = governing_channel(agent, call);
        let rules = self.rules(channel)?;
        if rules.decision(C::InboxAsk) == Some(Decision::Never) {
            return Ok(never(channel, C::InboxAsk));
        }
        let need = match resolve_agent(agent, call)? {
            Resolved::Refuse(refusal) => return Ok(Verdict::Refused(refusal)),
            Resolved::Owned => return Ok(allowed_now(Source::Owned)),
            Resolved::ReviewPush => return Ok(allowed_now(Source::ReviewPushSetting)),
            Resolved::Need(need) => need,
        };
        let decision = match rules.decision(need.capability) {
            Some(decision) => decision,
            None => return Ok(Verdict::Refused(no_rule(need.capability))),
        };
        if decision == Decision::Never {
            return Ok(never(channel, need.capability));
        }
        if switch_allows(&agent.kind, need.capability, call) {
            return Ok(allowed_now(Source::DeskSwitch));
        }
        if let Some(grant) = self.covering_grant(agent, &need) {
            return Ok(allowed_now(Source::Grant(grant)));
        }
        if need.offer.is_none() {
            return Ok(Verdict::Refused(Refusal::NothingToAskAhead {
                capability: need.capability,
            }));
        }
        let reach = reach(call, Some(need.capability), &context.here);
        Ok(ask(agent, call, need, reach, context, channel, rules, true))
    }

    fn check_agent(
        &self,
        agent: &Agent,
        call: &Call,
        context: &Context,
    ) -> Result<Verdict, CheckError> {
        let channel = governing_channel(agent, call);
        let rules = self.rules(channel)?;
        let away = Some(rules.away());
        let need = match resolve_agent(agent, call)? {
            Resolved::Refuse(refusal) => return Ok(Verdict::Refused(refusal)),
            Resolved::Owned => {
                let reach = reach(call, None, &context.here);
                return Ok(carry_out(Source::Owned, reach, context, away));
            }
            Resolved::ReviewPush => {
                return Ok(carry_out(
                    Source::ReviewPushSetting,
                    Reach::Host,
                    context,
                    away,
                ));
            }
            Resolved::Need(need) => need,
        };
        let reach = reach(call, Some(need.capability), &context.here);
        // Order: hard limits and user-only (in resolve_agent), then Never,
        // then grants, then the rule's Allow or Ask.
        let decision = match rules.decision(need.capability) {
            Some(decision) => decision,
            None => return Ok(Verdict::Refused(no_rule(need.capability))),
        };
        if decision == Decision::Never {
            return Ok(never(channel, need.capability));
        }
        if switch_allows(&agent.kind, need.capability, call) {
            return Ok(carry_out(Source::DeskSwitch, reach, context, away));
        }
        if let Some(grant) = self.covering_grant(agent, &need) {
            return Ok(carry_out(Source::Grant(grant), reach, context, away));
        }
        if decision == Decision::Allow {
            return Ok(carry_out(Source::Rule, reach, context, away));
        }
        Ok(ask(agent, call, need, reach, context, channel, rules, false))
    }

    fn rules(&self, channel: &ChannelId) -> Result<&ChannelRules, CheckError> {
        self.rules_of(channel)
            .ok_or_else(|| CheckError::UnknownChannel {
                channel: channel.clone(),
            })
    }

    /// The rules a channel id is governed by. The Desk channel id has no
    /// rules of its own: it takes the new-channel defaults (D182.3).
    fn rules_of(&self, channel: &ChannelId) -> Option<&ChannelRules> {
        if is_desk_channel(channel) {
            Some(&self.new_channels)
        } else {
            self.channels.get(channel)
        }
    }

    fn covering_grant(&self, agent: &Agent, need: &Need) -> Option<GrantId> {
        let asked = need.asked.as_ref()?;
        self.grants
            .values()
            .find(|grant| {
                grant.requester == agent.session.id
                    && grant.channel == agent.session.channel
                    && grant.capability == need.capability
                    && grant.target.covers(asked)
            })
            .map(|grant| grant.id)
    }

    // ----- Answers. -----

    /// Approves a request the policy asked for. The approval is checked
    /// against the rules as they are now (IN-21). A grant is made only for
    /// the target the request offered, and records `at` as when.
    pub fn approve(
        &mut self,
        user: &PresentUser,
        request_id: &RequestId,
        request: &AccessRequest,
        answer: &Answer,
        at: SystemTime,
    ) -> Result<Approval, AnswerError> {
        if !user.on_host() {
            if user.method() != Some(PresenceMethod::ApprovalPassphrase) {
                return Err(AnswerError::PassphraseNeeded);
            }
            if request.answer_at == AnswerAt::Host {
                return Err(AnswerError::NeedsHost);
            }
        }
        let rules = self
            .rules_of(&request.channel)
            .ok_or_else(|| AnswerError::UnknownChannel {
                channel: request.channel.clone(),
            })?;
        match rules.decision(request.capability) {
            None => {
                return Err(AnswerError::NotARule {
                    capability: request.capability,
                });
            }
            Some(Decision::Never) => {
                return Err(AnswerError::RuleIsNever {
                    channel: request.channel.clone(),
                    capability: request.capability,
                });
            }
            Some(Decision::Allow | Decision::Ask) => {}
        }
        let wanted = match answer {
            Answer::Once if !request.ahead => return Ok(Approval { grant: None }),
            Answer::Once => None,
            Answer::AlwaysLet(target) => Some(target),
        };
        let offered = request
            .always_let
            .as_ref()
            .ok_or(AnswerError::NoStandingApproval)?;
        if wanted.is_some_and(|target| target != offered) || !offered.fits(request.capability) {
            return Err(AnswerError::TargetNotOffered);
        }
        let grant = Grant {
            id: GrantId::new(self.next_grant),
            channel: request.channel.clone(),
            capability: request.capability,
            requester: request.requester.clone(),
            target: offered.clone(),
            granted_from: request_id.clone(),
            granted_by: user.method(),
            granted_on: user.machine().clone(),
            on_host: user.on_host(),
            granted_at: at,
        };
        self.next_grant += 1;
        self.grants.insert(grant.id, grant.clone());
        Ok(Approval { grant: Some(grant) })
    }

    /// Takes in a grant the user made on a machine while the Mac was away,
    /// when that machine's journal replays (AW-12). The grant gets a new id
    /// here; its target, requester and record stay as they were.
    pub fn replay_grant(&mut self, grant: &Grant) -> Result<Grant, PolicyError> {
        if grant.on_host || grant.granted_by != Some(PresenceMethod::ApprovalPassphrase) {
            return Err(PolicyError::NotAMachineGrant { grant: grant.id });
        }
        if self.rules_of(&grant.channel).is_none() {
            return Err(PolicyError::UnknownChannel {
                channel: grant.channel.clone(),
            });
        }
        if !grant.target.fits(grant.capability) {
            return Err(PolicyError::GrantDoesNotFit {
                grant: grant.id,
                capability: grant.capability,
            });
        }
        let replayed = Grant {
            id: GrantId::new(self.next_grant),
            ..grant.clone()
        };
        self.next_grant += 1;
        self.grants.insert(replayed.id, replayed.clone());
        Ok(replayed)
    }

    // ----- Changes the user makes. -----

    /// Sets one rule, from the next call on (IN-21). Returns the old
    /// setting.
    pub fn set_rule(
        &mut self,
        user: &PresentUser,
        rules: &Rules,
        capability: Capability,
        decision: Decision,
    ) -> Result<Decision, PolicyError> {
        require_host(user)?;
        check_setting(capability, decision)?;
        let rules = self.rules_mut(rules)?;
        let old = rules
            .decision(capability)
            .ok_or(PolicyError::NotARule { capability })?;
        rules.set_decision(capability, decision);
        Ok(old)
    }

    /// Sets the While I'm away rule (AW-4). Returns the old one.
    pub fn set_away_rule(
        &mut self,
        user: &PresentUser,
        rules: &Rules,
        away: AwayRule,
    ) -> Result<AwayRule, PolicyError> {
        require_host(user)?;
        check_away(&away)?;
        let rules = self.rules_mut(rules)?;
        let old = rules.away().clone();
        rules.set_away(away);
        Ok(old)
    }

    /// Removes a grant (IN-19). It stops applying at once.
    pub fn revoke_grant(&mut self, user: &PresentUser, grant: GrantId) -> Result<Grant, PolicyError> {
        require_host(user)?;
        self.grants
            .remove(&grant)
            .ok_or(PolicyError::UnknownGrant { grant })
    }

    // ----- Changes that follow other events. -----

    /// Adds a channel with the rules new channels start with.
    pub fn add_channel(&mut self, channel: ChannelId) -> Result<(), PolicyError> {
        if self.channels.contains_key(&channel) {
            return Err(PolicyError::ChannelExists { channel });
        }
        self.channels.insert(channel, self.new_channels.clone());
        Ok(())
    }

    /// Drops a deleted channel's rules and its grants.
    pub fn remove_channel(&mut self, channel: &ChannelId) -> Result<Vec<Grant>, PolicyError> {
        if self.channels.remove(channel).is_none() {
            return Err(PolicyError::UnknownChannel {
                channel: channel.clone(),
            });
        }
        Ok(self.take_grants(|grant| &grant.channel == channel))
    }

    /// Ends the grants of a session that was archived or deleted (IN-22).
    /// Returns them, for the record.
    pub fn end_grants_of(&mut self, session: &SessionId) -> Vec<Grant> {
        self.take_grants(|grant| &grant.requester == session)
    }

    fn take_grants(&mut self, mut ends: impl FnMut(&Grant) -> bool) -> Vec<Grant> {
        let ids: Vec<GrantId> = self
            .grants
            .values()
            .filter(|grant| ends(*grant))
            .map(|grant| grant.id)
            .collect();
        ids.iter()
            .filter_map(|id| self.grants.remove(id))
            .collect()
    }

    // ----- Reading the policy. -----

    pub fn channel_rules(&self, channel: &ChannelId) -> Option<&ChannelRules> {
        self.channels.get(channel)
    }

    pub fn new_channel_rules(&self) -> &ChannelRules {
        &self.new_channels
    }

    pub fn grants(&self) -> impl Iterator<Item = &Grant> + '_ {
        self.grants.values()
    }

    pub fn grant(&self, grant: GrantId) -> Option<&Grant> {
        self.grants.get(&grant)
    }

    /// True while the grant's rule is Never: it stays listed but does not
    /// apply until the rule changes back (IN-22).
    pub fn is_suspended(&self, grant: &Grant) -> bool {
        self.rules_of(&grant.channel)
            .and_then(|rules| rules.decision(grant.capability))
            .is_none_or(|decision| decision == Decision::Never)
    }

    // ----- Storing. -----

    pub fn to_state(&self) -> PolicyState {
        PolicyState {
            new_channels: rules_state(&self.new_channels),
            channels: self
                .channels
                .iter()
                .map(|(id, rules)| (id.clone(), rules_state(rules)))
                .collect(),
            grants: self.grants.values().cloned().collect(),
            next_grant: self.next_grant,
        }
    }

    /// Rebuilds a policy from stored state, refusing state that breaks the
    /// rules: settings a rule doesn't offer, low-risk kinds AW-11 forbids,
    /// grants whose target doesn't fit their capability.
    pub fn from_state(state: PolicyState) -> Result<Self, PolicyError> {
        let new_channels = rules_from_state(&state.new_channels)?;
        let mut channels = BTreeMap::new();
        for (id, rules) in &state.channels {
            channels.insert(id.clone(), rules_from_state(rules)?);
        }
        let mut grants = BTreeMap::new();
        let mut next_grant = state.next_grant.max(1);
        for grant in state.grants {
            if !is_desk_channel(&grant.channel) && !channels.contains_key(&grant.channel) {
                return Err(PolicyError::UnknownChannel {
                    channel: grant.channel,
                });
            }
            if !grant.target.fits(grant.capability) {
                return Err(PolicyError::GrantDoesNotFit {
                    grant: grant.id,
                    capability: grant.capability,
                });
            }
            let id = grant.id;
            next_grant = next_grant.max(id.get().saturating_add(1));
            if grants.insert(id, grant).is_some() {
                return Err(PolicyError::DuplicateGrant { grant: id });
            }
        }
        Ok(Self {
            new_channels,
            channels,
            grants,
            next_grant,
        })
    }

    fn rules_mut(&mut self, rules: &Rules) -> Result<&mut ChannelRules, PolicyError> {
        match rules {
            Rules::NewChannels => Ok(&mut self.new_channels),
            Rules::Channel(channel) => {
                self.channels
                    .get_mut(channel)
                    .ok_or_else(|| PolicyError::UnknownChannel {
                        channel: channel.clone(),
                    })
            }
        }
    }
}

fn require_host(user: &PresentUser) -> Result<(), PolicyError> {
    if user.on_host() {
        Ok(())
    } else {
        Err(PolicyError::NeedsHost)
    }
}

fn check_setting(capability: Capability, decision: Decision) -> Result<(), PolicyError> {
    if !capability.has_rule() {
        return Err(PolicyError::NotARule { capability });
    }
    if !capability.settings().contains(&decision) {
        return Err(PolicyError::SettingNotOffered {
            capability,
            decision,
        });
    }
    Ok(())
}

fn check_away(away: &AwayRule) -> Result<(), PolicyError> {
    if let AwayRule::AllowLowRisk(kinds) = away {
        if let Some(&capability) = kinds.iter().find(|kind| !kind.may_be_low_risk()) {
            return Err(PolicyError::NotLowRisk { capability });
        }
    }
    Ok(())
}

fn rules_state(rules: &ChannelRules) -> RulesState {
    RulesState {
        decisions: rules.decisions().collect(),
        away: rules.away().clone(),
    }
}

fn rules_from_state(state: &RulesState) -> Result<ChannelRules, PolicyError> {
    let mut rules = ChannelRules::defaults();
    for &(capability, decision) in &state.decisions {
        check_setting(capability, decision)?;
        rules.set_decision(capability, decision);
    }
    check_away(&state.away)?;
    rules.set_away(state.away.clone());
    Ok(rules)
}

// ----- Working out what a call needs. -----

/// What an agent's call comes down to.
enum Resolved {
    /// Its own things: no rule (IN-16).
    Owned,
    /// A capability, checked against the rules and grants.
    Need(Need),
    /// A review session's push the user allowed (PR-8).
    ReviewPush,
    /// Refused whatever the rules say.
    Refuse(Refusal),
}

struct Need {
    capability: Capability,
    /// The call's exact target, for matching grants.
    asked: Option<GrantTarget>,
    /// What "Always allow …" would grant.
    offer: Option<GrantTarget>,
}

fn need(capability: Capability) -> Resolved {
    Resolved::Need(Need {
        capability,
        asked: None,
        offer: None,
    })
}

fn need_exactly(capability: Capability, target: GrantTarget) -> Resolved {
    Resolved::Need(Need {
        capability,
        asked: Some(target.clone()),
        offer: Some(target),
    })
}

/// A files capability: a grant on the folder asked for covers this path.
fn need_files(capability: Capability, place: &Place) -> Resolved {
    let scope = |prefix| {
        GrantTarget::Path(PathScope {
            machine: place.machine.clone(),
            root: place.root.clone(),
            prefix,
        })
    };
    Resolved::Need(Need {
        capability,
        asked: Some(scope(place.path.clone())),
        offer: Some(scope(place.folder())),
    })
}

fn refuse(refusal: Refusal) -> Resolved {
    Resolved::Refuse(refusal)
}

fn other_channel(channel: &ChannelId) -> Resolved {
    refuse(Refusal::OtherChannel {
        channel: channel.clone(),
    })
}

/// True when `channel` is the agent's own. A Desk session belongs to no
/// channel, so no channel is its own and two Desk sessions are not
/// channel-mates (D182.3).
fn same_channel(agent: &Agent, channel: &ChannelId) -> bool {
    !matches!(agent.kind, SessionKind::Desk(_)) && channel == &agent.session.channel
}

/// `resolved` if `channel` is the agent's own, else refused.
fn in_channel(agent: &Agent, channel: &ChannelId, resolved: Resolved) -> Resolved {
    if same_channel(agent, channel) {
        resolved
    } else {
        other_channel(channel)
    }
}

/// True when `session` is the parent of a context-only fork, whose clones
/// the fork works in as its own (D185, D187).
fn is_parent(agent: &Agent, session: &SessionId) -> bool {
    matches!(&agent.kind, SessionKind::ContextOnlyFork { parent } if parent == session)
}

fn resolve_agent(agent: &Agent, call: &Call) -> Result<Resolved, CheckError> {
    let me = &agent.session;
    Ok(match call {
        Call::ReadFiles(place) => read_place(agent, place),
        Call::WriteFiles(place) => write_place(agent, place),
        Call::CopyFiles { from, into } => copy_files(agent, from, into),
        Call::ReadSession(session) => {
            if session.id == me.id {
                Resolved::Owned
            } else if same_channel(agent, &session.channel) {
                need(C::ChannelRead)
            } else {
                need(C::GlobalRead)
            }
        }
        Call::Board { board, act } => board_need(agent, board, act),
        Call::Use { capability, target } => return use_need(agent, *capability, target),
    })
}

fn read_place(agent: &Agent, place: &Place) -> Resolved {
    let me = &agent.session;
    match &place.root {
        Root::Outside | Root::Home | Root::Pastes(_) | Root::Received => refuse(Refusal::OutsideHumataFolders {
            machine: place.machine.clone(),
        }),
        Root::Clone(owner) => {
            if owner.id == me.id || is_parent(agent, &owner.id) {
                Resolved::Owned
            } else if !same_channel(agent, &owner.channel) {
                other_channel(&owner.channel)
            } else if place.machine == me.machine {
                need_files(C::FilesReadOtherSession, place)
            } else {
                need_files(C::FilesReadOtherMachine, place)
            }
        }
        Root::Shared(channel) | Root::ChannelFolder(channel) | Root::MainClone { channel, .. } => {
            if !same_channel(agent, channel) {
                other_channel(channel)
            } else if place.machine != me.machine {
                need_files(C::FilesReadOtherMachine, place)
            } else if matches!(place.root, Root::Shared(_)) {
                Resolved::Owned
            } else {
                need(C::FilesFetchLocal)
            }
        }
    }
}

fn write_place(agent: &Agent, place: &Place) -> Resolved {
    let me = &agent.session;
    let not_its = || {
        refuse(Refusal::NotItsFolder {
            machine: place.machine.clone(),
        })
    };
    match &place.root {
        // A send to another machine by name lands in its received/ folder;
        // as with shared/ there, the session on that machine pulls (D755).
        Root::Received if place.machine != me.machine => refuse(Refusal::SharedOnOtherMachine {
            machine: place.machine.clone(),
            here: me.machine.clone(),
        }),
        Root::Outside | Root::Home | Root::Pastes(_) | Root::Received => refuse(Refusal::OutsideHumataFolders {
            machine: place.machine.clone(),
        }),
        Root::MainClone { repo, .. } => refuse(Refusal::MainCloneReadOnly {
            repo: repo.clone(),
            machine: place.machine.clone(),
        }),
        Root::Clone(owner) => {
            if owner.id == me.id || is_parent(agent, &owner.id) {
                Resolved::Owned
            } else {
                not_its()
            }
        }
        Root::Shared(channel) => {
            if !same_channel(agent, channel) {
                other_channel(channel)
            } else if place.machine == me.machine {
                need(C::SharedWrite)
            } else {
                // D755: a hard limit, not a rule; the session there pulls.
                refuse(Refusal::SharedOnOtherMachine {
                    machine: place.machine.clone(),
                    here: me.machine.clone(),
                })
            }
        }
        Root::ChannelFolder(_) => not_its(),
    }
}

/// A copy lands like a write to `into`. What it needs comes from where the
/// files come from: its own things need only the landing, a channel folder
/// or main clone on its machine is a fetch, and anything of another session
/// or machine is a copy-in, which IN-17 lets land in the clone or `shared/`.
fn copy_files(agent: &Agent, from: &Place, into: &Place) -> Resolved {
    let landing = write_place(agent, into);
    if matches!(landing, Resolved::Refuse(_)) {
        return landing;
    }
    match read_place(agent, from) {
        Resolved::Refuse(refusal) => refuse(refusal),
        Resolved::Owned | Resolved::ReviewPush => landing,
        Resolved::Need(need) => match need.capability {
            C::FilesReadOtherSession | C::FilesReadOtherMachine => need_files(C::FilesCopyIn, from),
            _ => Resolved::Need(need),
        },
    }
}

fn board_need(agent: &Agent, board: &BoardRef, act: &BoardAct) -> Resolved {
    let me = &agent.session;
    let created_it = |created_by: &Option<SessionId>| created_by.as_ref() == Some(&me.id);
    let others = || need_exactly(C::ChannelBoardEditOthers, GrantTarget::Board(board.clone()));
    // Outside its channel: reading is GlobalRead, any write asks (D155).
    let elsewhere = || match act {
        BoardAct::Read => need(C::GlobalRead),
        BoardAct::Add | BoardAct::Edit { .. } => {
            need_exactly(C::GlobalBoardWrite, GrantTarget::Board(board.clone()))
        }
    };
    match board {
        BoardRef::Global => elsewhere(),
        BoardRef::Channel(channel) if !same_channel(agent, channel) => elsewhere(),
        BoardRef::Session(session) if !same_channel(agent, &session.channel) => elsewhere(),
        BoardRef::Channel(_) => match act {
            BoardAct::Read => need(C::ChannelRead),
            BoardAct::Add => need(C::ChannelBoardAdd),
            BoardAct::Edit { created_by } if created_it(created_by) => need(C::ChannelBoardEditOwn),
            BoardAct::Edit { .. } => others(),
        },
        BoardRef::Session(session) if session.id == me.id => match act {
            BoardAct::Read => Resolved::Owned,
            BoardAct::Add | BoardAct::Edit { .. } => need(C::OwnBoardWrite),
        },
        BoardRef::Session(_) => match act {
            BoardAct::Read => need(C::ChannelRead),
            BoardAct::Edit { created_by } if created_it(created_by) => need(C::ChannelBoardEditOwn),
            BoardAct::Add | BoardAct::Edit { .. } => others(),
        },
    }
}

fn use_need(agent: &Agent, capability: Capability, target: &Target) -> Result<Resolved, CheckError> {
    match capability.tier() {
        Tier::UserOnly | Tier::Peer => return Ok(refuse(no_rule(capability))),
        Tier::DefaultAllow | Tier::AskMe | Tier::AskNoAllow | Tier::NeverByDefault => {}
    }
    let me = &agent.session;
    let resolved = match (capability, target) {
        (C::ChannelRead, Target::Channel(channel)) if same_channel(agent, channel) => {
            need(C::ChannelRead)
        }
        (C::ChannelRead | C::GlobalRead, Target::Channel(_) | Target::Global) => need(C::GlobalRead),
        // A Desk agent belongs to no channel: its asks are filed under the
        // Desk channel id it carries, which is its own (D182.3).
        (C::InboxAsk, Target::Channel(channel))
            if matches!(agent.kind, SessionKind::Desk(_)) && channel == &me.channel =>
        {
            need(C::InboxAsk)
        }
        (
            C::ViewsPrepare | C::ReviewDraft | C::InboxAsk | C::PrReviewStart | C::TrashRestore
            | C::ChannelChange,
            Target::Channel(channel),
        ) => in_channel(agent, channel, need(capability)),
        // A channel agent proposes for its own channel only; a Desk agent
        // belongs to no channel and may propose for any (CH-26, D304).
        (C::ChannelRepoAdd, Target::Channel(channel)) => {
            if matches!(agent.kind, SessionKind::Desk(_)) {
                need(C::ChannelRepoAdd)
            } else {
                in_channel(agent, channel, need(C::ChannelRepoAdd))
            }
        }
        // Only a Desk session's agent asks to move its own session into a
        // channel, and the target channel's rule decides (DK-15, D305). A
        // channel session is already in one.
        (C::DeskMoveToChannel, Target::Channel(channel)) => {
            if matches!(agent.kind, SessionKind::Desk(_)) {
                need(C::DeskMoveToChannel)
            } else {
                other_channel(channel)
            }
        }
        (C::ChannelCreate, Target::Global) => need(C::ChannelCreate),
        (C::SessionNote | C::SessionType, Target::Session(session)) => {
            if session.id == me.id {
                Resolved::Owned
            } else {
                in_channel(
                    agent,
                    &session.channel,
                    need_exactly(capability, GrantTarget::Session(session.id.clone())),
                )
            }
        }
        (C::OwnSessionEnd | C::SessionStopOther | C::SessionDeleteOther, Target::Session(session)) => {
            if session.id == me.id {
                need(C::OwnSessionEnd)
            } else {
                in_channel(agent, &session.channel, need(capability))
            }
        }
        (C::OwnSessionManage, Target::Session(session)) if session.id == me.id => {
            need(C::OwnSessionManage)
        }
        // The review chat acts on its own review only. The session kind
        // isn't required: the daemon checks the review is open, and the
        // user's per-review switch decides past this rule.
        (C::ReviewFocus | C::ReviewNote | C::ReviewOrder, Target::Session(session))
            if session.id == me.id =>
        {
            need(capability)
        }
        // Only on its own session's comments; any other target doesn't fit.
        (C::CommentReply | C::CommentResolve, Target::Session(session))
            if session.id == me.id =>
        {
            need(capability)
        }
        // A to-do suggest or close by an agent: its own session's rule.
        (C::TodoSuggest | C::TodoClose, Target::Session(session)) if session.id == me.id => {
            need(capability)
        }
        (C::SessionMove, Target::Session(session)) => {
            in_channel(agent, &session.channel, need(C::SessionMove))
        }
        (
            C::SessionStartSameMachine | C::SessionStartOtherMachine,
            Target::Start {
                channel,
                machine,
                agent: kind,
                fork_of: _,
            },
        ) => {
            let capability = if machine == &me.machine {
                C::SessionStartSameMachine
            } else {
                C::SessionStartOtherMachine
            };
            let target = GrantTarget::AgentOnMachine {
                machine: machine.clone(),
                agent: kind.clone(),
            };
            in_channel(agent, channel, need_exactly(capability, target))
        }
        (C::GitPush, Target::Push(branch)) => match agent.kind {
            SessionKind::Review { push_allowed: true } => Resolved::ReviewPush,
            SessionKind::Review { push_allowed: false } => refuse(Refusal::ReviewSessionPush {
                session: me.id.clone(),
            }),
            SessionKind::Ordinary
            | SessionKind::ContextOnlyFork { .. }
            | SessionKind::Desk(_)
            | SessionKind::ChannelDesk(_) => {
                need_exactly(C::GitPush, GrantTarget::RepoBranch(branch.clone()))
            }
        },
        // HO-11: a forward opens from the requester's own machine.
        (C::NetworkReach, Target::Network(network)) if network.machine == me.machine => {
            need_exactly(C::NetworkReach, GrantTarget::Network(network.clone()))
        }
        // MP-23, D792: the providers file is the Mac's, so the ask names no machine.
        (C::ProviderImport, Target::Global) => need(C::ProviderImport),
        // MACHINESCRIPTS (D688): run now has no per-machine action kind; the channel's switch decides.
        (C::MachineScriptRun, Target::Global | Target::Machine(_)) => need(C::MachineScriptRun),
        (C::MachineProvision | C::MachineDestroy | C::MachineSessionsStop, Target::Machine(action)) => need_exactly(
            action.action.capability(),
            GrantTarget::MachineAction(action.clone()),
        ),
        _ => return Err(CheckError::TargetDoesNotFit { capability }),
    };
    Ok(resolved)
}

/// The refusal for a capability that has no rule.
fn no_rule(capability: Capability) -> Refusal {
    match capability.tier() {
        Tier::Peer => Refusal::PeerOnly { capability },
        Tier::UserOnly
        | Tier::DefaultAllow
        | Tier::AskMe
        | Tier::AskNoAllow
        | Tier::NeverByDefault => Refusal::UserOnly {
            capability: Some(capability),
        },
    }
}

/// The channel whose rules decide `call`: the target channel when a Desk
/// agent asks to move into it (DK-15, D305), else the agent's own.
fn governing_channel<'a>(agent: &'a Agent, call: &'a Call) -> &'a ChannelId {
    match call {
        Call::Use {
            capability: C::DeskMoveToChannel,
            target: Target::Channel(channel),
        } if matches!(agent.kind, SessionKind::Desk(_)) => channel,
        _ => &agent.session.channel,
    }
}

fn never(channel: &ChannelId, capability: Capability) -> Verdict {
    Verdict::Refused(Refusal::NeverRule {
        channel: channel.clone(),
        capability,
    })
}

// ----- Where a call runs, and the Mac being away. -----

/// What a call touches, which decides what happens while the Mac is away
/// (AW-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// This machine's own files and sessions: done on the machine.
    Local,
    /// Channel state the Mac keeps, read: answered from the last copy.
    ChannelRead,
    /// Channel state the Mac keeps, written: journaled.
    ChannelWrite,
    /// Another machine, GitHub, the network or state only the Mac has:
    /// waits for the Mac.
    Host,
}

fn reach(call: &Call, capability: Option<Capability>, here: &MachineId) -> Reach {
    if capability.is_some_and(Capability::needs_host) {
        return Reach::Host;
    }
    let local_if = |machine: &MachineId, otherwise: Reach| {
        if machine == here { Reach::Local } else { otherwise }
    };
    match call {
        Call::ReadFiles(place) | Call::WriteFiles(place) => local_if(&place.machine, Reach::Host),
        Call::CopyFiles { from, into } => {
            if &from.machine == here && &into.machine == here {
                Reach::Local
            } else {
                Reach::Host
            }
        }
        Call::ReadSession(session) => local_if(&session.machine, Reach::ChannelRead),
        Call::Board { board, act } => match (board, act) {
            (BoardRef::Global, _) => Reach::Host,
            (_, BoardAct::Read) => Reach::ChannelRead,
            (_, BoardAct::Add | BoardAct::Edit { .. }) => Reach::ChannelWrite,
        },
        Call::Use { target, .. } => match target {
            Target::Machine(MachineAction {
                machine,
                action: MachineActionKind::StopSessions | MachineActionKind::StartSessions,
            }) => local_if(machine, Reach::Host),
            Target::Global | Target::Network(_) | Target::Push(_) | Target::Machine(_) => {
                Reach::Host
            }
            Target::Channel(_) if capability == Some(C::ChannelRead) => Reach::ChannelRead,
            Target::Channel(_) => Reach::ChannelWrite,
            Target::Session(session) => local_if(&session.machine, Reach::Host),
            Target::Start { machine, .. } => local_if(machine, Reach::Host),
        },
    }
}

fn allowed_now(source: Source) -> Verdict {
    Verdict::Allowed {
        source,
        effect: Effect::Apply,
    }
}

/// An allowed call: done now while the Mac can be reached, else as AW-7
/// says. `away` is the channel's While I'm away rule, when the caller has a
/// channel.
fn carry_out(source: Source, reach: Reach, context: &Context, away: Option<&AwayRule>) -> Verdict {
    if !context.host_away() {
        return allowed_now(source);
    }
    let effect = match reach {
        Reach::Local => Effect::Apply,
        Reach::ChannelRead => Effect::LastCopy,
        Reach::ChannelWrite => Effect::Journal,
        Reach::Host => {
            return if matches!(away, Some(AwayRule::Skip)) {
                Verdict::Refused(Refusal::HostUnreachable)
            } else {
                Verdict::WaitForHost
            };
        }
    };
    Verdict::Allowed { source, effect }
}

/// A call the rules say to ask about: an Inbox request, or while the Mac
/// is away, what the While I'm away rule says (AW-4, AW-7).
fn ask(
    agent: &Agent,
    call: &Call,
    need: Need,
    reach: Reach,
    context: &Context,
    channel: &ChannelId,
    rules: &ChannelRules,
    ahead: bool,
) -> Verdict {
    let channel = channel.clone();
    let capability = need.capability;
    let request = |answer_at| AccessRequest {
        capability,
        channel: channel.clone(),
        requester: agent.session.id.clone(),
        call: call.clone(),
        always_let: need.offer.clone(),
        ahead,
        answer_at,
    };
    if context.host != Host::Away {
        return Verdict::Ask(request(AnswerAt::Host));
    }
    let away = rules.away();
    if reach == Reach::Host {
        // Never low-risk and never answered on the machine: the Mac must
        // route it (AW-11, HO-11).
        return carry_out(Source::Rule, Reach::Host, context, Some(away));
    }
    let on_machine = || Verdict::Ask(request(AnswerAt::Machine(context.here.clone())));
    match away {
        AwayRule::Wait => on_machine(),
        AwayRule::Skip => Verdict::Refused(Refusal::UserAway {
            channel: channel.clone(),
        }),
        AwayRule::AllowLowRisk(kinds) => {
            if !ahead && kinds.contains(&capability) && capability.may_be_low_risk() {
                carry_out(Source::AwayRule, reach, context, Some(away))
            } else {
                on_machine()
            }
        }
    }
}

// ----- Callers without a session. -----

/// Limits no one passes: paths outside Humata's folders, writes to a main
/// clone.
fn file_hard_limit(call: &Call) -> Option<Refusal> {
    let outside = |place: &Place| {
        (place.root == Root::Outside).then(|| Refusal::OutsideHumataFolders {
            machine: place.machine.clone(),
        })
    };
    let main_clone = |place: &Place| match &place.root {
        Root::MainClone { repo, .. } => Some(Refusal::MainCloneReadOnly {
            repo: repo.clone(),
            machine: place.machine.clone(),
        }),
        Root::Clone(_)
        | Root::Shared(_)
        | Root::ChannelFolder(_)
        | Root::Home
        | Root::Pastes(_)
        | Root::Received
        | Root::Outside => None,
    };
    match call {
        Call::ReadFiles(place) => outside(place),
        Call::WriteFiles(place) => outside(place).or_else(|| main_clone(place)),
        Call::CopyFiles { from, into } => outside(from)
            .or_else(|| outside(into))
            .or_else(|| main_clone(into)),
        Call::ReadSession(_) | Call::Board { .. } | Call::Use { .. } => None,
    }
}

fn capability_of(call: &Call) -> Option<Capability> {
    match call {
        Call::Use { capability, .. } => Some(*capability),
        Call::ReadFiles(_)
        | Call::WriteFiles(_)
        | Call::CopyFiles { .. }
        | Call::ReadSession(_)
        | Call::Board { .. } => None,
    }
}

/// The user, in the app or with presence shown: everything, except peer
/// calls and the limits no one passes. While the Mac is away only what the
/// machine can do itself, and of the user-only calls only answering
/// requests (with the passphrase) and typing into a terminal here.
fn check_user(method: Option<PresenceMethod>, call: &Call, context: &Context) -> Verdict {
    if let Some(refusal) = file_hard_limit(call) {
        return Verdict::Refused(refusal);
    }
    if let Call::Use { capability, target } = call {
        match capability.tier() {
            Tier::Peer => {
                return Verdict::Refused(Refusal::PeerOnly {
                    capability: *capability,
                });
            }
            Tier::UserOnly if context.host_away() => {
                return user_only_while_away(method, *capability, target, context);
            }
            Tier::UserOnly
            | Tier::DefaultAllow
            | Tier::AskMe
            | Tier::AskNoAllow
            | Tier::NeverByDefault => {}
        }
    }
    let reach = reach(call, capability_of(call), &context.here);
    carry_out(Source::User, reach, context, None)
}

fn user_only_while_away(
    method: Option<PresenceMethod>,
    capability: Capability,
    target: &Target,
    context: &Context,
) -> Verdict {
    match (capability, target) {
        (C::TerminalInput, Target::Session(session)) if session.machine == context.here => {
            allowed_now(Source::User)
        }
        (C::InboxAnswer, _) if method == Some(PresenceMethod::ApprovalPassphrase) => {
            allowed_now(Source::User)
        }
        (C::InboxAnswer, _) => Verdict::Refused(Refusal::PresenceRequired {
            accepted: vec![PresenceMethod::ApprovalPassphrase],
        }),
        _ => Verdict::Refused(Refusal::HostUnreachable),
    }
}

/// A script token: exactly what it lists, in its channels, never asking
/// (SEC-14). Channel rules don't apply: the user chose the list when
/// making the token.
fn check_script(token: &ScriptToken, call: &Call, context: &Context) -> Verdict {
    let (capability, channel) = match call {
        Call::ReadFiles(_) | Call::WriteFiles(_) | Call::CopyFiles { .. } => {
            return Verdict::Refused(Refusal::ScriptTokenHasNoSession);
        }
        Call::ReadSession(session) => (C::ChannelRead, Some(&session.channel)),
        Call::Board { board, act } => {
            let capability = match (board, act) {
                (BoardRef::Global, BoardAct::Read) => C::GlobalRead,
                (BoardRef::Global, _) => C::GlobalBoardWrite,
                (_, BoardAct::Read) => C::ChannelRead,
                (BoardRef::Channel(_), BoardAct::Add) => C::ChannelBoardAdd,
                (_, BoardAct::Add | BoardAct::Edit { .. }) => C::ChannelBoardEditOthers,
            };
            let channel = match board {
                BoardRef::Global => None,
                BoardRef::Channel(channel) => Some(channel),
                BoardRef::Session(session) => Some(&session.channel),
            };
            (capability, channel)
        }
        Call::Use { capability, target } => {
            let channel = match target {
                Target::Channel(channel) | Target::Start { channel, .. } => Some(channel),
                Target::Session(session) => Some(&session.channel),
                Target::Global | Target::Network(_) | Target::Push(_) | Target::Machine(_) => None,
            };
            (*capability, channel)
        }
    };
    match capability.tier() {
        Tier::UserOnly | Tier::Peer => return Verdict::Refused(no_rule(capability)),
        Tier::AskNoAllow => return Verdict::Refused(Refusal::ScriptTokenCannotAsk { capability }),
        Tier::DefaultAllow | Tier::AskMe | Tier::NeverByDefault => {}
    }
    if !token.capabilities.contains(&capability) {
        return Verdict::Refused(Refusal::ScriptTokenLacks { capability });
    }
    if let Some(channel) = channel {
        if !token.channels.contains(channel) {
            return Verdict::Refused(Refusal::ScriptTokenOtherChannel {
                channel: channel.clone(),
            });
        }
    }
    let reach = reach(call, Some(capability), &context.here);
    carry_out(Source::ScriptToken, reach, context, None)
}

/// The peer link carries only the two daemons' own calls, each one way.
fn check_peer(link: &PeerLink, call: &Call) -> Verdict {
    match (link, call) {
        (
            PeerLink::DaemonToHost { .. },
            Call::Use {
                capability: C::PeerReport,
                ..
            },
        )
        | (
            PeerLink::HostToDaemon,
            Call::Use {
                capability: C::HostControl,
                ..
            },
        ) => allowed_now(Source::PeerLink),
        _ => Verdict::Refused(Refusal::PeerCannotCall),
    }
}
