//! The policy copy (AW-8). The host sends each machine its rules, the
//! grants of the sessions there, the approval passphrase's hash and Contain
//! agents, whenever the link comes up and whenever one of them changes. The
//! machine keeps the newest copy it was sent, judges calls by it while the
//! host is away, and loads it again when its daemon restarts. Nothing on the
//! machine changes the copy: access writes there need the host
//! (`PolicyError::NeedsHost`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_access::{
    Agent, Context, Grant, GrantId, MachineId, Policy, PolicyError, PolicyState, RequestId,
    RulesState, Verdict,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{Store, StoreError};
use prost::Message;
use tokio::sync::watch;

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

/// Where the copy is made (on the host) or put in force (on a machine).
pub(crate) struct PolicySource {
    guard: Arc<Guard>,
    store: Arc<Store>,
}

impl PolicySource {
    pub(crate) fn new(guard: Arc<Guard>, store: Arc<Store>) -> Arc<Self> {
        Arc::new(Self { guard, store })
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
            owned_items: Vec::new(),
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

    /// Keeps `snapshot` on this machine and puts it in force, returning its
    /// revision for the host's ack. On an error the copy before stays in
    /// force.
    pub(crate) fn apply(&self, snapshot: &pb::PolicySnapshot) -> Result<u64, CopyError> {
        // Copies apply only forward: an older one, from a link that was
        // replaced, is acked and dropped.
        let kept = self
            .store
            .setting(COPY_KEY)
            .map_err(|source| CopyError::Store { source })?
            .map_or(0, |saved| saved.revision);
        if snapshot.revision < kept {
            return Ok(snapshot.revision);
        }
        let policy = self.policy_of(snapshot)?;
        keep(&self.store, snapshot)?;
        self.guard.with_policy(|held, _| *held = policy);
        Ok(snapshot.revision)
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
        let local: Vec<Grant> = held.grants.into_iter().filter(|grant| !grant.on_host).collect();
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
        machine.apply(&newer).unwrap();
        machine.apply(&older).unwrap();
        assert_eq!(away_rule(&machine_guard, "ch_a"), Some(AwayRule::Skip));
    }

    #[test]
    fn hex_comes_back_as_the_bytes() {
        let bytes = vec![0, 1, 0xab, 0xff];
        assert_eq!(from_hex(&to_hex(&bytes)), Some(bytes));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
    }
}
