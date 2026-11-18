//! The policy copy (AW-8). The host sends each machine its rules, the
//! grants of the sessions there, the approval passphrase's hash and Contain
//! agents, whenever the link comes up and whenever one of them changes. The
//! machine keeps the newest copy it was sent, judges calls by it while the
//! host is away, and loads it again when its daemon restarts. Nothing on the
//! machine changes the copy: access writes there need the host
//! (`PolicyError::NeedsHost`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_access::{
    Agent, Context, Grant, GrantId, MachineId, Policy, PolicyError, PolicyState, RequestId,
    RulesState, Verdict,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{BoardKey, Store, StoreError};
use prost::Message;
use tokio::sync::watch;

use super::journal;
use super::passphrase;
use crate::guard::Guard;
use crate::identity::AgentCaller;
use crate::rules_kept::{self, Unknown};
use crate::{contained, grant_target};

/// The settings key the machine keeps its copy under, as hex of the
/// snapshot's bytes. None on the host, or on a machine never sent one.
const COPY_KEY: &str = "policy_copy";

/// Why a policy copy couldn't be made, kept or put in force.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CopyError {
    #[error(
        "couldn't read or keep the policy copy in this machine's store; check that the disk has room and the state folder is writable. Until it is kept, this machine judges calls by the copy it had before"
    )]
    Store {
        #[source]
        source: StoreError,
    },

    #[error(
        "the policy copy names something this humatad2 doesn't know; update humatad2 on this machine to the host's version, then reconnect it. Until then it judges calls by the copy it had before"
    )]
    Unknown {
        #[source]
        source: Unknown,
    },

    #[error(
        "the policy copy's rules and grants don't fit together; reconnect this machine so the host sends a new copy. Until then it judges calls by the copy it had before"
    )]
    Policy {
        #[source]
        source: PolicyError,
    },

    #[error(
        "the kept policy copy is damaged, so this machine starts with the default rules (While I'm away: Wait); reconnect it to the Mac so the host sends a new copy"
    )]
    Damaged {
        #[source]
        source: prost::DecodeError,
    },

    #[error(
        "the kept policy copy is not hex, so this machine starts with the default rules (While I'm away: Wait); reconnect it to the Mac so the host sends a new copy"
    )]
    NotHex,
}

/// The revision of the copy this machine holds, 0 if it was never sent one.
pub(super) fn held_revision(store: &Store) -> Result<u64, StoreError> {
    Ok(store.setting(COPY_KEY)?.map_or(0, |saved| saved.revision))
}

/// What the host knows of one machine's copy: whether its link is up now
/// and the newest revision it said it keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Delivery {
    linked: bool,
    revision: u64,
}

/// Which machines took a copy and which still wait for it, as
/// SetApprovalPassphrase reports them.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Delivered {
    pub(crate) updated: Vec<String>,
    pub(crate) pending: Vec<String>,
}

/// Where the copy is made (on the host) or put in force (on a machine).
pub(crate) struct PolicySource {
    guard: Arc<Guard>,
    store: Arc<Store>,
    /// On the host: each machine that linked since this daemon started,
    /// with what it acked.
    deliveries: watch::Sender<BTreeMap<String, Delivery>>,
}

/// Marks a machine's link down when the link ends.
pub(crate) struct Linked {
    source: Arc<PolicySource>,
    machine: String,
}

impl Drop for Linked {
    fn drop(&mut self) {
        self.source.deliveries.send_modify(|deliveries| {
            if let Some(delivery) = deliveries.get_mut(&self.machine) {
                delivery.linked = false;
            }
        });
    }
}

impl PolicySource {
    pub(crate) fn new(guard: Arc<Guard>, store: Arc<Store>) -> Arc<Self> {
        Arc::new(Self {
            guard,
            store,
            deliveries: watch::Sender::new(BTreeMap::new()),
        })
    }

    /// Says `machine`'s link is up, until the returned value is dropped.
    pub(crate) fn linked(self: &Arc<Self>, machine: &str) -> Linked {
        self.deliveries.send_modify(|deliveries| {
            deliveries.entry(machine.to_owned()).or_default().linked = true;
        });
        Linked {
            source: Arc::clone(self),
            machine: machine.to_owned(),
        }
    }

    /// `machine` said it keeps the copy of `revision`.
    pub(crate) fn acked(&self, machine: &str, revision: u64) {
        self.deliveries.send_if_modified(|deliveries| {
            let delivery = deliveries.entry(machine.to_owned()).or_default();
            if revision > delivery.revision {
                delivery.revision = revision;
                true
            } else {
                false
            }
        });
    }

    /// Waits up to `within` for every linked machine to keep the copy of
    /// the rules in force now, then says which did. A machine whose link
    /// is down, or that didn't ack in time, gets it with a later copy.
    pub(crate) async fn delivered(&self, within: Duration) -> Delivered {
        let revision = self.guard.policy_revision();
        let mut deliveries = self.deliveries.subscribe();
        let all_kept = |deliveries: &BTreeMap<String, Delivery>| {
            deliveries
                .values()
                .all(|delivery| !delivery.linked || delivery.revision >= revision)
        };
        let _ = tokio::time::timeout(within, deliveries.wait_for(all_kept)).await;
        let mut delivered = Delivered::default();
        for (machine, delivery) in deliveries.borrow().iter() {
            if delivery.revision >= revision {
                delivered.updated.push(machine.clone());
            } else {
                delivered.pending.push(machine.clone());
            }
        }
        delivered
    }

    /// The store the copy is kept in.
    pub(super) fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Changes each time the copy would.
    pub(crate) fn changes(&self) -> watch::Receiver<u64> {
        self.guard.policy_changes()
    }

    /// The copy for `machine`, as the host has it now. Grants of sessions on
    /// other machines are left out.
    pub(crate) fn snapshot(&self, machine: &str) -> Result<pb::PolicySnapshot, CopyError> {
        let store_error = |source| CopyError::Store { source };
        let revision = self.guard.policy_revision();
        let policy = self.guard.with_policy(|policy, _| policy.clone());
        let state = policy.to_state();
        let mut channels = Vec::new();
        let mut channel_contain = Vec::new();
        for id in state.channels.keys() {
            let Some(rules) = policy.channel_rules(id) else {
                continue;
            };
            channels.push(pb::ChannelRules {
                channel: Some(pb::ChannelId { value: id.to_string() }),
                rules: rules_kept::rules_pb(rules, &[]),
                away: Some(rules_kept::away_pb(rules.away())),
                revision: None,
                copy: None,
            });
            if let Some(on) = contained::channel(&self.store, id.as_str()).map_err(store_error)? {
                channel_contain.push(pb::ChannelContain {
                    channel: Some(pb::ChannelId { value: id.to_string() }),
                    contain_agents: on,
                });
            }
        }
        let mut grants = Vec::new();
        for grant in policy.grants() {
            let here = self.store.session(grant.requester.as_str()).map_err(store_error)?;
            if here.is_some_and(|session| session.machine == machine) {
                grants.push(grant_target::grant(grant, policy.is_suspended(grant)));
            }
        }
        let defaults = policy.new_channel_rules();
        Ok(pb::PolicySnapshot {
            revision,
            at: Some(grant_target::timestamp(SystemTime::now())),
            channels,
            grants,
            owned_items: self.owned_items(machine).map_err(store_error)?,
            low_risk: Vec::new(),
            defaults: Some(pb::DefaultRules {
                rules: rules_kept::rules_pb(defaults, &[]),
                away: Some(rules_kept::away_pb(defaults.away())),
                revision: None,
                copy: None,
            }),
            approval_verifier: passphrase::verifier(&self.store).map_err(store_error)?,
            contain_agents: contained::global(&self.store).map_err(store_error)?.0,
            channel_contain,
        })
    }

    /// The board items `machine`'s sessions may change while the host is
    /// away (D227): those they made, and those on their own session boards,
    /// each with its owner, board and revision. The machine judges a
    /// journaled edit against this list. Deleted items are left out.
    fn owned_items(&self, machine: &str) -> Result<Vec<pb::OwnedItem>, StoreError> {
        let here: BTreeMap<String, String> = self
            .store
            .sessions()?
            .into_iter()
            .filter(|session| session.machine == machine)
            .map(|session| (session.id, session.channel))
            .collect();
        let channels: BTreeSet<&str> = here.values().map(String::as_str).collect();
        let mut owned = Vec::new();
        for channel in channels {
            for item in self.store.board_items_in_channel(channel)? {
                if item.deleted {
                    continue;
                }
                let owner = if here.contains_key(&item.created_by) {
                    item.created_by.clone()
                } else if let BoardKey::Session(id) = &item.board
                    && here.contains_key(id)
                {
                    id.clone()
                } else {
                    continue;
                };
                owned.push(pb::OwnedItem {
                    item: Some(pb::BoardItemId { value: item.key.clone() }),
                    owner: Some(pb::SessionId { value: owner }),
                    revision: Some(pb::Revision { value: item.revision }),
                    board: Some(board_ref(&item.board)),
                });
            }
        }
        Ok(owned)
    }

    /// Keeps `snapshot` on this machine and puts it in force, returning its
    /// revision for the host's ack. On an error the copy before stays in
    /// force.
    pub(crate) fn apply(&self, snapshot: &pb::PolicySnapshot) -> Result<u64, CopyError> {
        self.kept(snapshot).map(|_| snapshot.revision)
    }

    /// Puts `snapshot` in force and gives its revision, or `None` when an
    /// older copy, from a link that was replaced, is dropped: that one is
    /// still acked but was never kept.
    pub(crate) fn kept(&self, snapshot: &pb::PolicySnapshot) -> Result<Option<u64>, CopyError> {
        // Copies apply only forward.
        let kept = self
            .store
            .setting(COPY_KEY)
            .map_err(|source| CopyError::Store { source })?
            .map_or(0, |saved| saved.revision);
        if snapshot.revision < kept {
            return Ok(None);
        }
        let policy = self.policy_of(snapshot)?;
        keep(&self.store, snapshot)?;
        self.guard.with_policy(|held, _| *held = policy);
        self.guard.host_rules_arrived();
        Ok(Some(snapshot.revision))
    }

    /// The policy `snapshot` holds, with the grants this machine made while
    /// the host was away (AW-12) kept.
    fn policy_of(&self, snapshot: &pb::PolicySnapshot) -> Result<Policy, CopyError> {
        let unknown = |source| CopyError::Unknown { source };
        let new_channels = match &snapshot.defaults {
            Some(defaults) => rules_state(&defaults.rules, defaults.away.as_ref()).map_err(unknown)?,
            None => Policy::new().to_state().new_channels,
        };
        let mut channels = BTreeMap::new();
        for rules in &snapshot.channels {
            let Some(id) = &rules.channel else {
                continue;
            };
            channels.insert(
                humata2_access::ChannelId::new(&id.value),
                rules_state(&rules.rules, rules.away.as_ref()).map_err(unknown)?,
            );
        }
        let held = self.guard.with_policy(|policy, _| policy.to_state());
        // A grant made here is kept only until the host holds its answer:
        // acked, or taken over by a grant of the host's own (D226).
        let unacked = self.unacked_answers();
        let taken: BTreeSet<&str> = snapshot
            .grants
            .iter()
            .filter_map(|grant| grant.granted_from.as_ref().map(|id| id.value.as_str()))
            .collect();
        let local: Vec<Grant> = held
            .grants
            .into_iter()
            .filter(|grant| !grant.on_host)
            .filter(|grant| {
                let request = grant.granted_from.to_string();
                !taken.contains(request.as_str())
                    && unacked.as_ref().is_none_or(|unacked| unacked.contains(&request))
            })
            .collect();
        let rules_only = Policy::from_state(PolicyState {
            new_channels: new_channels.clone(),
            channels: channels.clone(),
            grants: Vec::new(),
            next_grant: held.next_grant,
        })
        .map_err(|source| CopyError::Policy { source })?;
        let mut grants = Vec::new();
        for grant in &snapshot.grants {
            if let Some(grant) = self.grant_of(&rules_only, grant)? {
                grants.push(grant);
            }
        }
        let next_grant = grants
            .iter()
            .chain(&local)
            .map(|grant| grant.id.get().saturating_add(1))
            .fold(held.next_grant, u64::max);
        grants.extend(local);
        Policy::from_state(PolicyState {
            new_channels,
            channels,
            grants,
            next_grant,
        })
        .map_err(|source| CopyError::Policy { source })
    }

    /// Drops the grants this machine made for the requests in `answered`,
    /// which the host now holds the answers to (D226): it made its own
    /// grants from them. The host's grants stay.
    pub(super) fn drop_made_for(&self, answered: &BTreeSet<String>) -> Result<(), CopyError> {
        self.guard.with_policy(|policy, _| {
            let mut state = policy.to_state();
            let before = state.grants.len();
            state
                .grants
                .retain(|grant| grant.on_host || !answered.contains(&grant.granted_from.to_string()));
            if state.grants.len() == before {
                return Ok(());
            }
            Policy::from_state(state)
                .map(|kept| *policy = kept)
                .map_err(|source| CopyError::Policy { source })
        })
    }

    /// The requests whose answers the host hasn't acked, which is how long
    /// a grant made for one on this machine is kept. `None` when the
    /// journal can't be read: every grant stays then.
    fn unacked_answers(&self) -> Option<BTreeSet<String>> {
        match journal::answered_through(&self.store, u64::MAX) {
            Ok(unacked) => Some(unacked),
            Err(error) => {
                crate::log::say!(
                    "humatad2: warning: couldn't tell which of this machine's grants the host has taken over, so they all stay: {}",
                    crate::status::chain(&error)
                );
                None
            }
        }
    }

    /// The policy's form of a grant the host sent, worked out again from
    /// its target as the host's Inbox does on restart. `None` when its
    /// session isn't on this machine, or the rules no longer ask for it.
    fn grant_of(&self, rules: &Policy, grant: &pb::Grant) -> Result<Option<Grant>, CopyError> {
        let (Some(number), Some(requester), Some(target)) =
            (grant_target::grant_number(grant), &grant.requester, &grant.target)
        else {
            return Ok(None);
        };
        let Some(capability) = grant_target::capability_of(grant.capability) else {
            return Ok(None);
        };
        let here = self.guard.machine();
        let Some(record) = self
            .store
            .session(&requester.value)
            .map_err(|source| CopyError::Store { source })?
        else {
            return Ok(None);
        };
        let agent = AgentCaller::of(&record, here)
            .resolve_fork(&self.store)
            .map_err(|source| CopyError::Store { source })?;
        let Ok(call) = grant_target::ask_ahead_call(&self.store, here, &agent, capability, target.clone())
        else {
            return Ok(None);
        };
        let asker = Agent {
            session: agent.session.clone(),
            kind: agent.session_kind(),
        };
        // Judged as if the host were reachable: the copy holds what it granted.
        let context = Context::machine(MachineId::new(here), true);
        let Ok(Verdict::Ask(asked)) = rules.check_ask_ahead(&asker, &call, &context) else {
            return Ok(None);
        };
        let Some(target) = asked.always_let else {
            return Ok(None);
        };
        let granted_at = grant.granted_at.as_ref().map_or(UNIX_EPOCH, |at| {
            let seconds = u64::try_from(at.seconds).unwrap_or(0);
            let nanos = u32::try_from(at.nanos).unwrap_or(0);
            UNIX_EPOCH + Duration::new(seconds, nanos)
        });
        Ok(Some(Grant {
            id: GrantId::new(number),
            channel: asked.channel,
            capability: asked.capability,
            requester: asked.requester,
            target,
            granted_from: RequestId::new(grant.granted_from.as_ref().map_or("", |id| id.value.as_str())),
            // The copy doesn't say how the user showed presence, or where.
            granted_by: None,
            granted_on: MachineId::new(here),
            on_host: true,
            granted_at,
        }))
    }
}

/// Puts the copy this machine kept back in force, after the saved rules and
/// before the Inbox puts its grants back. Nothing kept leaves the default
/// rules, with While I'm away at Wait.
pub(crate) fn restore(store: &Arc<Store>, guard: &Arc<Guard>) -> Result<(), CopyError> {
    let Some(snapshot) = kept(store)? else {
        return Ok(());
    };
    PolicySource::new(guard.clone(), store.clone()).apply(&snapshot).map(drop)
}

/// Which copy this machine's rules come from (D220): its revision and the
/// host's time when it made it. `None` when the host never sent one.
pub(crate) fn kept_stamp(store: &Store) -> Result<Option<pb::PolicyCopyStamp>, CopyError> {
    Ok(kept(store)?.map(|snapshot| pb::PolicyCopyStamp {
        revision: snapshot.revision,
        at: snapshot.at,
    }))
}

/// `item` as the copy this machine keeps lists it, when it says `session`
/// owns it: what a journaled change to it is judged against (D227). `None`
/// when the copy doesn't list it for that session, or none was ever sent.
pub(crate) fn owned_item(
    store: &Store,
    session: &str,
    item: &str,
) -> Result<Option<pb::OwnedItem>, CopyError> {
    Ok(kept(store)?.and_then(|snapshot| {
        snapshot.owned_items.into_iter().find(|owned| {
            owned.item.as_ref().is_some_and(|id| id.value == item)
                && owned.owner.as_ref().is_some_and(|owner| owner.value == session)
        })
    }))
}

/// The API's name for a board.
fn board_ref(board: &BoardKey) -> pb::BoardRef {
    pb::BoardRef {
        board: Some(match board {
            BoardKey::Global => pb::board_ref::Board::Global(pb::GlobalBoard {}),
            BoardKey::Channel(id) => {
                pb::board_ref::Board::Channel(pb::ChannelId { value: id.clone() })
            }
            BoardKey::Session(id) => {
                pb::board_ref::Board::Session(pb::SessionId { value: id.clone() })
            }
        }),
    }
}

/// The copy this machine keeps, if the host ever sent one.
fn kept(store: &Store) -> Result<Option<pb::PolicySnapshot>, CopyError> {
    let Some(saved) = store.setting(COPY_KEY).map_err(|source| CopyError::Store { source })? else {
        return Ok(None);
    };
    if saved.value.is_empty() {
        return Ok(None);
    }
    let bytes = from_hex(&saved.value).ok_or(CopyError::NotHex)?;
    pb::PolicySnapshot::decode(bytes.as_slice())
        .map(Some)
        .map_err(|source| CopyError::Damaged { source })
}

/// Keeps the copy and the settings it carries.
fn keep(store: &Store, snapshot: &pb::PolicySnapshot) -> Result<(), CopyError> {
    let store_error = |source| CopyError::Store { source };
    let revision = snapshot.revision;
    passphrase::put_verifier(store, &snapshot.approval_verifier, revision).map_err(store_error)?;
    contained::put_global(store, snapshot.contain_agents, revision).map_err(store_error)?;
    let overrides: BTreeMap<&str, bool> = snapshot
        .channel_contain
        .iter()
        .filter_map(|choice| Some((choice.channel.as_ref()?.value.as_str(), choice.contain_agents)))
        .collect();
    for rules in &snapshot.channels {
        let Some(id) = &rules.channel else {
            continue;
        };
        let on = overrides.get(id.value.as_str()).copied();
        contained::put_channel(store, &id.value, on, revision).map_err(store_error)?;
    }
    store
        .put_setting(COPY_KEY, &to_hex(&snapshot.encode_to_vec()), revision)
        .map_err(store_error)
}

fn rules_state(rules: &[pb::Rule], away: Option<&pb::AwayRule>) -> Result<RulesState, Unknown> {
    Ok(RulesState {
        decisions: rules_kept::decisions_of(rules)?,
        away: rules_kept::away_of(away)?.unwrap_or(humata2_access::AwayRule::Wait),
    })
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use humata2_access::{AwayRule, Capability, ChannelId, Decision, PresentUser, Rules};

    use super::*;
    use crate::identity::{Caller, UserVia};

    const MACHINE: &str = "m1";

    fn store() -> (tempfile::TempDir, Arc<Store>) {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        (directory, store)
    }

    fn host() -> (tempfile::TempDir, Arc<PolicySource>, Arc<Guard>, Arc<Store>) {
        let (directory, store) = store();
        let guard = Arc::new(Guard::new("mac", pb::DaemonRole::Host));
        (directory, PolicySource::new(guard.clone(), store.clone()), guard, store)
    }

    fn machine() -> (tempfile::TempDir, Arc<PolicySource>, Arc<Guard>, Arc<Store>) {
        let (directory, store) = store();
        let guard = Arc::new(Guard::new(MACHINE, pb::DaemonRole::Machine));
        (directory, PolicySource::new(guard.clone(), store.clone()), guard, store)
    }

    /// The user in the app.
    fn app() -> humata2_access::Principal {
        crate::guard::principal(&Caller::User(UserVia::App))
    }

    /// The user, present on the host.
    fn user(guard: &Guard) -> PresentUser {
        guard
            .with_policy(|_, context| app().present_user(context))
            .expect("the user is present on the host")
    }

    fn away_rule(guard: &Guard, channel: &str) -> Option<AwayRule> {
        guard.with_policy(|policy, _| {
            policy
                .channel_rules(&ChannelId::new(channel))
                .map(|rules| rules.away().clone())
        })
    }

    #[test]
    fn a_machine_judges_by_the_rules_the_host_sent() {
        let (_h, host, host_guard, _) = host();
        let present = user(&host_guard);
        host_guard.with_policy(|policy, _| {
            policy.add_channel(ChannelId::new("ch_a")).unwrap();
            policy
                .set_away_rule(&present, &Rules::Channel(ChannelId::new("ch_a")), AwayRule::Skip)
                .unwrap();
        });
        let snapshot = host.snapshot(MACHINE).unwrap();

        let (_m, machine, machine_guard, _) = machine();
        assert_eq!(machine.apply(&snapshot).unwrap(), snapshot.revision);
        assert_eq!(away_rule(&machine_guard, "ch_a"), Some(AwayRule::Skip));
    }

    #[test]
    fn a_machine_restarted_comes_back_with_the_copy_it_kept() {
        let (_h, host, host_guard, host_store) = host();
        host_guard.with_policy(|policy, _| policy.add_channel(ChannelId::new("ch_a")).unwrap());
        contained::put_global(&host_store, true, 1).unwrap();
        let snapshot = host.snapshot(MACHINE).unwrap();
        let (_m, machine, _, machine_store) = machine();
        machine.apply(&snapshot).unwrap();

        let restarted = Arc::new(Guard::new(MACHINE, pb::DaemonRole::Machine));
        restore(&machine_store, &restarted).unwrap();
        assert_eq!(away_rule(&restarted, "ch_a"), Some(AwayRule::Wait));
        assert!(contained::global(&machine_store).unwrap().0);
    }

    #[test]
    fn a_machine_has_the_hosts_rules_once_a_copy_is_kept_and_again_after_a_restart() {
        let (_h, host, _, _) = host();
        let snapshot = host.snapshot(MACHINE).unwrap();
        let (_m, machine, guard, store) = machine();
        assert!(!guard.has_host_rules());
        machine.apply(&snapshot).unwrap();
        assert!(guard.has_host_rules());

        let restarted = Arc::new(Guard::new(MACHINE, pb::DaemonRole::Machine));
        restore(&store, &restarted).unwrap();
        assert!(restarted.has_host_rules());
    }

    #[test]
    fn a_machine_never_sent_a_copy_still_has_no_rules_after_a_restart() {
        let (_m, _, guard, store) = machine();
        restore(&store, &guard).unwrap();
        assert!(!guard.has_host_rules());
    }

    #[test]
    fn a_machine_never_sent_a_copy_starts_at_the_defaults() {
        let (_m, _, guard, store) = machine();
        restore(&store, &guard).unwrap();
        let away = guard.with_policy(|policy, _| policy.new_channel_rules().away().clone());
        assert_eq!(away, AwayRule::Wait);
    }

    #[test]
    fn the_copy_carries_the_passphrase_hash_and_contain_choices() {
        let (_h, host, host_guard, host_store) = host();
        host_guard.with_policy(|policy, _| {
            policy.add_channel(ChannelId::new("ch_a")).unwrap();
            policy.add_channel(ChannelId::new("ch_b")).unwrap();
        });
        passphrase::put_verifier(&host_store, "$argon2id$hash", 1).unwrap();
        contained::put_channel(&host_store, "ch_a", Some(false), 1).unwrap();
        let (_m, machine, _, machine_store) = machine();
        contained::put_channel(&machine_store, "ch_b", Some(true), 1).unwrap();

        machine.apply(&host.snapshot(MACHINE).unwrap()).unwrap();
        assert_eq!(passphrase::verifier(&machine_store).unwrap(), "$argon2id$hash");
        assert_eq!(contained::channel(&machine_store, "ch_a").unwrap(), Some(false));
        // The host has no choice for ch_b, so the machine's own one is gone.
        assert_eq!(contained::channel(&machine_store, "ch_b").unwrap(), None);
    }

    #[test]
    fn a_copy_that_does_not_fit_leaves_the_one_before_in_force() {
        let (_h, host, host_guard, _) = host();
        host_guard.with_policy(|policy, _| policy.add_channel(ChannelId::new("ch_a")).unwrap());
        let (_m, machine, machine_guard, _) = machine();
        machine.apply(&host.snapshot(MACHINE).unwrap()).unwrap();

        let mut broken = host.snapshot(MACHINE).unwrap();
        broken.channels[0].rules.push(pb::Rule {
            capability: 9999,
            decision: pb::Decision::Allow as i32,
            grants: Vec::new(),
        });
        assert!(matches!(machine.apply(&broken), Err(CopyError::Unknown { .. })));
        assert_eq!(away_rule(&machine_guard, "ch_a"), Some(AwayRule::Wait));
    }

    #[test]
    fn rules_cannot_be_changed_on_a_machine() {
        let (_m, _, guard, _) = machine();
        let _opened = guard.host_link().open();
        let refused = guard.with_policy(|policy, context| {
            let user = app().present_user(context)?;
            Some(policy.set_rule(&user, &Rules::NewChannels, Capability::NetworkReach, Decision::Allow))
        });
        if let Some(result) = refused {
            assert!(matches!(result, Err(PolicyError::NeedsHost)));
        }
    }

    #[test]
    fn an_older_copy_does_not_replace_a_newer_one() {
        let (_h, host, host_guard, _) = host();
        let present = user(&host_guard);
        host_guard.with_policy(|policy, _| policy.add_channel(ChannelId::new("ch_a")).unwrap());
        let older = host.snapshot(MACHINE).unwrap();
        host_guard.with_policy(|policy, _| {
            policy
                .set_away_rule(&present, &Rules::Channel(ChannelId::new("ch_a")), AwayRule::Skip)
                .unwrap();
        });
        let newer = host.snapshot(MACHINE).unwrap();
        let (_m, machine, machine_guard, _) = machine();
        assert_eq!(machine.kept(&newer).unwrap(), Some(newer.revision));
        assert_eq!(machine.kept(&older).unwrap(), None);
        assert_eq!(away_rule(&machine_guard, "ch_a"), Some(AwayRule::Skip));
    }

    #[test]
    fn hex_comes_back_as_the_bytes() {
        let bytes = vec![0, 1, 0xab, 0xff];
        assert_eq!(from_hex(&to_hex(&bytes)), Some(bytes));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
    }

    /// A grant made on this machine for `request`, as a passphrase answer
    /// makes one.
    fn session_on(id: &str, machine: &str) -> humata2_store::SessionRecord {
        humata2_store::SessionRecord {
            id: id.to_owned(),
            channel: "ch_a".to_owned(),
            machine: machine.to_owned(),
            title: "t".to_owned(),
            agent: "claude".to_owned(),
            folder: std::path::PathBuf::new(),
            created_unix_ms: 0,
        }
    }

    fn make_item(store: &Store, board: BoardKey, created_by: &str) -> String {
        store
            .create_board_item(&humata2_store::NewBoardItem {
                board,
                channel: "ch_a".to_owned(),
                title: "t".to_owned(),
                description: String::new(),
                column: 1,
                rank: "m".to_owned(),
                created_by: created_by.to_owned(),
                at_unix_ms: 1,
                sessions: Vec::new(),
                depends_on: Vec::new(),
            })
            .unwrap()
            .key
    }

    /// A host that places `se_a` on this machine and `se_b` on another, and
    /// an item of each kind on its boards. Returns the keys of the items the
    /// machine's `se_a` owns.
    fn host_with_items() -> (tempfile::TempDir, Arc<PolicySource>, Arc<Store>, Vec<String>) {
        let (directory, host, _guard, store) = host();
        store.claim_board_prefix("ch_a", "PAY").unwrap();
        store.put_session(&session_on("se_a", MACHINE)).unwrap();
        store.put_session(&session_on("se_b", "m2")).unwrap();
        let channel = || BoardKey::Channel("ch_a".to_owned());
        let made_by_a = make_item(&store, channel(), "se_a");
        let on_a_board = make_item(&store, BoardKey::Session("se_a".to_owned()), "");
        make_item(&store, channel(), "");
        make_item(&store, channel(), "se_b");
        make_item(&store, BoardKey::Session("se_b".to_owned()), "");
        let deleted = make_item(&store, channel(), "se_a");
        store
            .change_board_item(&humata2_store::BoardItemChange {
                key: deleted,
                expected_revision: None,
                by: String::new(),
                at_unix_ms: 2,
                title: None,
                description: None,
                column: None,
                rank: None,
                sessions: None,
                depends_on: None,
                delete: true,
                cause: String::new(),
            })
            .unwrap();
        (directory, host, store, vec![made_by_a, on_a_board])
    }

    #[test]
    fn the_copy_lists_what_a_machines_sessions_made_and_what_is_on_their_own_boards() {
        let (_h, host, _store, mine) = host_with_items();
        let owned = host.snapshot(MACHINE).unwrap().owned_items;
        let listed: BTreeMap<String, String> = owned
            .iter()
            .map(|owned| {
                (
                    owned.item.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
                    owned.owner.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
                )
            })
            .collect();
        let expected: BTreeMap<String, String> =
            mine.iter().map(|key| (key.clone(), "se_a".to_owned())).collect();
        assert_eq!(
            listed, expected,
            "the user's items, another machine's and deleted ones are not listed"
        );
        let on_own_board = owned
            .iter()
            .find(|owned| owned.item.as_ref().is_some_and(|id| id.value == mine[1]))
            .and_then(|owned| owned.board.clone());
        assert_eq!(on_own_board, Some(board_ref(&BoardKey::Session("se_a".to_owned()))));
        assert!(owned.iter().all(|owned| owned.revision == Some(pb::Revision { value: 1 })));
    }

    #[test]
    fn a_machine_finds_an_item_in_its_copy_only_for_the_session_that_owns_it() {
        let (_h, host, _store, mine) = host_with_items();
        let (_m, machine, _, machine_store) = machine();
        assert!(
            owned_item(&machine_store, "se_a", &mine[0]).unwrap().is_none(),
            "a machine never sent a copy owns nothing"
        );
        machine.apply(&host.snapshot(MACHINE).unwrap()).unwrap();
        assert!(owned_item(&machine_store, "se_a", &mine[0]).unwrap().is_some());
        assert!(owned_item(&machine_store, "se_a", &mine[1]).unwrap().is_some());
        assert!(owned_item(&machine_store, "se_other", &mine[0]).unwrap().is_none());
        assert!(owned_item(&machine_store, "se_a", "PAY-404").unwrap().is_none());
    }

    fn made_here(guard: &Guard, id: u64, request: &str, on_host: bool) {
        use humata2_access::{GrantTarget, PresenceMethod, SessionId};
        guard.with_policy(|policy, _| {
            let _ = policy.add_channel(ChannelId::new("ch_a"));
            let mut state = policy.to_state();
            state.grants.push(Grant {
                id: GrantId::new(id),
                channel: ChannelId::new("ch_a"),
                capability: Capability::SessionNote,
                requester: SessionId::new("se_a"),
                target: GrantTarget::Session(SessionId::new("se_b")),
                granted_from: RequestId::new(request),
                granted_by: Some(PresenceMethod::ApprovalPassphrase),
                granted_on: MachineId::new(MACHINE),
                on_host,
                granted_at: UNIX_EPOCH,
            });
            *policy = Policy::from_state(state).unwrap();
        });
    }

    fn grants_held(guard: &Guard) -> Vec<u64> {
        guard.with_policy(|policy, _| policy.grants().map(|grant| grant.id.get()).collect())
    }

    /// A copy from the host with `ch_a`, at `revision`.
    fn copy(revision: u64) -> pb::PolicySnapshot {
        pb::PolicySnapshot {
            revision,
            channels: vec![pb::ChannelRules {
                channel: Some(pb::ChannelId {
                    value: "ch_a".to_owned(),
                }),
                rules: Vec::new(),
                away: None,
                revision: None,
                copy: None,
            }],
            ..Default::default()
        }
    }

    /// The journal entry of the answer to `request`, as the Inbox keeps it.
    async fn answered(store: &Arc<Store>, request: &str) -> u64 {
        let link = crate::away::HostLink::new(false);
        let approved = pb::InboxRequest {
            id: Some(pb::RequestId {
                value: request.to_owned(),
            }),
            state: pb::RequestState::Approved as i32,
            ..Default::default()
        };
        crate::away::record_journal(store, &link, "se_a", pb::RequestState::Pending, &approved).await;
        journal::entries_after(store, 0)
            .unwrap()
            .into_iter()
            .filter(|entry| entry.operation == journal::ANSWER)
            .map(|entry| entry.seq)
            .max()
            .unwrap()
    }

    #[tokio::test]
    async fn a_grant_made_on_a_machine_stays_through_the_hosts_copies_while_its_answer_is_unacked() {
        let (_m, machine, guard, store) = machine();
        machine.apply(&copy(1)).unwrap();
        made_here(&guard, 4, "rq_1", false);
        answered(&store, "rq_1").await;
        machine.apply(&copy(2)).unwrap();
        assert_eq!(grants_held(&guard), [4]);
    }

    #[tokio::test]
    async fn a_grant_made_on_a_machine_is_gone_from_the_next_copy_once_its_answer_is_acked() {
        let (_m, machine, guard, store) = machine();
        machine.apply(&copy(1)).unwrap();
        made_here(&guard, 4, "rq_1", false);
        let seq = answered(&store, "rq_1").await;
        journal::acked(&store, seq).unwrap();
        machine.apply(&copy(2)).unwrap();
        assert!(grants_held(&guard).is_empty());
    }

    #[tokio::test]
    async fn dropping_what_an_answer_made_leaves_other_grants_and_the_hosts() {
        let (_m, machine, guard, _store) = machine();
        machine.apply(&copy(1)).unwrap();
        made_here(&guard, 4, "rq_1", false);
        made_here(&guard, 5, "rq_2", false);
        made_here(&guard, 6, "rq_1", true);
        machine
            .drop_made_for(&BTreeSet::from(["rq_1".to_owned()]))
            .unwrap();
        assert_eq!(grants_held(&guard), [5, 6]);
    }

    #[tokio::test]
    async fn a_grant_the_host_made_for_the_same_answer_takes_the_place_of_the_one_made_here() {
        let (_m, machine, guard, store) = machine();
        machine.apply(&copy(1)).unwrap();
        made_here(&guard, 4, "rq_1", false);
        // Still unacked, but the host's copy already carries its own grant
        // for that request, so this machine's isn't kept beside it.
        answered(&store, "rq_1").await;
        let mut theirs = copy(2);
        theirs.grants.push(pb::Grant {
            granted_from: Some(pb::RequestId {
                value: "rq_1".to_owned(),
            }),
            ..Default::default()
        });
        // The copy's grant names no session on this machine, so it isn't
        // taken; what matters is that the local one is out of the way.
        machine.apply(&theirs).unwrap();
        assert!(grants_held(&guard).is_empty());
    }

    #[tokio::test]
    async fn a_change_names_the_machines_that_kept_its_copy_and_those_still_waiting() {
        let (_h, host, guard, _store) = host();
        let _box_1 = host.linked("box-1");
        let box_2 = host.linked("box-2");
        drop(host.linked("box-3"));
        guard.policy_changed();
        let revision = guard.policy_revision();
        host.acked("box-1", revision);
        drop(box_2);
        let delivered = host.delivered(Duration::from_secs(30)).await;
        assert_eq!(delivered.updated, ["box-1"]);
        assert_eq!(delivered.pending, ["box-2", "box-3"]);
    }

    #[tokio::test]
    async fn a_linked_machine_that_never_acks_is_pending_once_the_wait_ends() {
        let (_h, host, guard, _store) = host();
        let _linked = host.linked("box-1");
        guard.policy_changed();
        let delivered = host.delivered(Duration::from_millis(50)).await;
        assert!(delivered.updated.is_empty());
        assert_eq!(delivered.pending, ["box-1"]);
    }
}
