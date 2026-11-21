//! The policy copy (AW-8). The host sends each machine its rules, the
//! grants of the sessions there, the approval passphrase's hash and Contain
//! agents, whenever the link comes up and whenever one of them changes. The
//! machine keeps the newest copy it was sent, judges calls by it while the
//! host is away, and loads it again when its daemon restarts. Nothing on the
//! machine changes the copy: access writes there need the host
//! (`PolicyError::NeedsHost`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use humata2_access::{ChannelRules, Grant, Policy, PolicyError, PolicyState, RulesState};
use crate::pb;
use humata2_rpc_kit::helpers as status;
use humata2_store::{BoardKey, Store, StoreError};
use prost::Message;
use tokio::sync::watch;

use super::journal;
use super::passphrase;

/// The settings key the machine keeps its copy under, as hex of the
/// snapshot's bytes. None on the host, or on a machine never sent one.
const COPY_KEY: &str = "policy_copy";

/// Narrow policy/guard bridge used by [`PolicySource`] so `svc_away` does not
/// depend on `humatad`'s `Guard`, `contained`, `rules_kept`, or `grant_target`.
pub trait PolicyGuard: Send + Sync {
    fn policy_revision(&self) -> u64;
    fn policy_changes(&self) -> watch::Receiver<u64>;
    fn policy_clone(&self) -> Policy;
    fn replace_policy(&self, policy: Policy);
    fn host_rules_arrived(&self);
    fn drop_local_grants(&self, answered: &BTreeSet<String>) -> Result<(), PolicyError>;
    fn encode_rules(&self, rules: &ChannelRules) -> (Vec<pb::Rule>, pb::AwayRule);
    fn decode_rules(&self, rules: &[pb::Rule], away: Option<&pb::AwayRule>) -> Result<RulesState, String>;
    fn encode_grant(&self, grant: &Grant, suspended: bool) -> pb::Grant;
    fn decode_grant(&self, store: &Store, rules: &Policy, grant: &pb::Grant) -> Result<Option<Grant>, StoreError>;
    fn global_contain(&self, store: &Store) -> Result<bool, StoreError>;
    fn put_global_contain(&self, store: &Store, on: bool, revision: u64) -> Result<(), StoreError>;
    fn channel_contain(&self, store: &Store, channel: &str) -> Result<Option<bool>, StoreError>;
    fn put_channel_contain(
        &self,
        store: &Store,
        channel: &str,
        on: Option<bool>,
        revision: u64,
    ) -> Result<(), StoreError>;
    fn use_worktrees(&self, store: &Store) -> Result<bool, StoreError>;
    fn put_use_worktrees(&self, store: &Store, on: bool, revision: u64) -> Result<(), StoreError>;
}

/// Why a policy copy couldn't be made, kept or put in force.
#[derive(Debug, thiserror::Error)]
pub enum CopyError {
    #[error(
        "couldn't read or keep the policy copy in this machine's store; check that the disk has room and the state folder is writable. Until it is kept, this machine judges calls by the copy it had before"
    )]
    Store {
        #[source]
        source: StoreError,
    },

    #[error(
        "the policy copy names something this humatad2 doesn't know ({detail}); update humatad2 on this machine to the host's version, then reconnect it. Until then it judges calls by the copy it had before"
    )]
    Unknown { detail: String },

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
pub struct Delivered {
    pub updated: Vec<String>,
    pub pending: Vec<String>,
}

/// Where the copy is made (on the host) or put in force (on a machine).
pub struct PolicySource {
    guard: Arc<dyn PolicyGuard>,
    store: Arc<Store>,
    /// On the host: each machine that linked since this daemon started,
    /// with what it acked.
    deliveries: watch::Sender<BTreeMap<String, Delivery>>,
}

/// Marks a machine's link down when the link ends.
pub struct Linked {
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
    pub fn new(guard: Arc<dyn PolicyGuard>, store: Arc<Store>) -> Arc<Self> {
        Arc::new(Self {
            guard,
            store,
            deliveries: watch::Sender::new(BTreeMap::new()),
        })
    }

    /// Says `machine`'s link is up, until the returned value is dropped.
    pub fn linked(self: &Arc<Self>, machine: &str) -> Linked {
        self.deliveries.send_modify(|deliveries| {
            deliveries.entry(machine.to_owned()).or_default().linked = true;
        });
        Linked {
            source: Arc::clone(self),
            machine: machine.to_owned(),
        }
    }

    /// `machine` said it keeps the copy of `revision`.
    pub fn acked(&self, machine: &str, revision: u64) {
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
    pub async fn delivered(&self, within: Duration) -> Delivered {
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
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.guard.policy_changes()
    }

    /// The copy for `machine`, as the host has it now. Grants of sessions on
    /// other machines are left out.
    pub fn snapshot(&self, machine: &str) -> Result<pb::PolicySnapshot, CopyError> {
        let store_error = |source| CopyError::Store { source };
        let revision = self.guard.policy_revision();
        let policy = self.guard.policy_clone();
        let state = policy.to_state();
        let mut channels = Vec::new();
        let mut channel_contain = Vec::new();
        for id in state.channels.keys() {
            let Some(rules) = policy.channel_rules(id) else {
                continue;
            };
            let (rules_pb, away_pb) = self.guard.encode_rules(rules);
            channels.push(pb::ChannelRules {
                channel: Some(pb::ChannelId { value: id.to_string() }),
                rules: rules_pb,
                away: Some(away_pb),
                revision: None,
                copy: None,
            });
            if let Some(on) = self.guard.channel_contain(&self.store, id.as_str()).map_err(store_error)? {
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
                grants.push(self.guard.encode_grant(grant, policy.is_suspended(grant)));
            }
        }
        let (default_rules, default_away) = self.guard.encode_rules(policy.new_channel_rules());
        Ok(pb::PolicySnapshot {
            revision,
            at: Some(journal::timestamp(SystemTime::now())),
            channels,
            grants,
            owned_items: self.owned_items(machine).map_err(store_error)?,
            low_risk: Vec::new(),
            defaults: Some(pb::DefaultRules {
                rules: default_rules,
                away: Some(default_away),
                revision: None,
                copy: None,
            }),
            approval_verifier: passphrase::verifier(&self.store).map_err(store_error)?,
            contain_agents: self.guard.global_contain(&self.store).map_err(store_error)?,
            channel_contain,
            use_worktrees: Some(self.guard.use_worktrees(&self.store).map_err(store_error)?),
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
    pub fn apply(&self, snapshot: &pb::PolicySnapshot) -> Result<u64, CopyError> {
        self.kept(snapshot).map(|_| snapshot.revision)
    }

    /// Puts `snapshot` in force and gives its revision, or `None` when an
    /// older copy, from a link that was replaced, is dropped: that one is
    /// still acked but was never kept.
    pub fn kept(&self, snapshot: &pb::PolicySnapshot) -> Result<Option<u64>, CopyError> {
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
        self.keep(snapshot)?;
        self.guard.replace_policy(policy);
        self.guard.host_rules_arrived();
        Ok(Some(snapshot.revision))
    }

    /// The policy `snapshot` holds, with the grants this machine made while
    /// the host was away (AW-12) kept.
    fn policy_of(&self, snapshot: &pb::PolicySnapshot) -> Result<Policy, CopyError> {
        let unknown = |detail| CopyError::Unknown { detail };
        let new_channels = match &snapshot.defaults {
            Some(defaults) => self
                .guard
                .decode_rules(&defaults.rules, defaults.away.as_ref())
                .map_err(unknown)?,
            None => Policy::new().to_state().new_channels,
        };
        let mut channels = BTreeMap::new();
        for rules in &snapshot.channels {
            let Some(id) = &rules.channel else {
                continue;
            };
            channels.insert(
                humata2_access::ChannelId::new(&id.value),
                self.guard
                    .decode_rules(&rules.rules, rules.away.as_ref())
                    .map_err(unknown)?,
            );
        }
        let held = self.guard.policy_clone().to_state();
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
            if let Some(grant) = self
                .guard
                .decode_grant(&self.store, &rules_only, grant)
                .map_err(|source| CopyError::Store { source })?
            {
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
        self.guard
            .drop_local_grants(answered)
            .map_err(|source| CopyError::Policy { source })
    }

    /// The requests whose answers the host hasn't acked, which is how long
    /// a grant made for one on this machine is kept. `None` when the
    /// journal can't be read: every grant stays then.
    fn unacked_answers(&self) -> Option<BTreeSet<String>> {
        match journal::answered_through(&self.store, u64::MAX) {
            Ok(unacked) => Some(unacked),
            Err(error) => {
                humata2_rpc_kit::log::line(&format!(
                    "humatad2: warning: couldn't tell which of this machine's grants the host has taken over, so they all stay: {}",
                    status::chain(&error)
                ));
                None
            }
        }
    }

    /// Keeps the copy and the settings it carries.
    fn keep(&self, snapshot: &pb::PolicySnapshot) -> Result<(), CopyError> {
        let store_error = |source| CopyError::Store { source };
        let revision = snapshot.revision;
        passphrase::put_verifier(&self.store, &snapshot.approval_verifier, revision).map_err(store_error)?;
        self.guard
            .put_global_contain(&self.store, snapshot.contain_agents, revision)
            .map_err(store_error)?;
        if let Some(on) = snapshot.use_worktrees {
            self.guard
                .put_use_worktrees(&self.store, on, revision)
                .map_err(store_error)?;
        }
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
            self.guard
                .put_channel_contain(&self.store, &id.value, on, revision)
                .map_err(store_error)?;
        }
        self.store
            .put_setting(COPY_KEY, &to_hex(&snapshot.encode_to_vec()), revision)
            .map_err(store_error)
    }
}

/// Puts the copy this machine kept back in force, after the saved rules and
/// before the Inbox puts its grants back. Nothing kept leaves the default
/// rules, with While I'm away at Wait.
pub fn restore(store: &Arc<Store>, guard: &Arc<dyn PolicyGuard>) -> Result<(), CopyError> {
    let Some(snapshot) = kept(store)? else {
        return Ok(());
    };
    PolicySource::new(Arc::clone(guard), Arc::clone(store))
        .apply(&snapshot)
        .map(drop)
}

/// Which copy this machine's rules come from (D220): its revision and the
/// host's time when it made it. `None` when the host never sent one.
pub fn kept_stamp(store: &Store) -> Result<Option<pb::PolicyCopyStamp>, CopyError> {
    Ok(kept(store)?.map(|snapshot| pb::PolicyCopyStamp {
        revision: snapshot.revision,
        at: snapshot.at,
    }))
}

/// `item` as the copy this machine keeps lists it, when it says `session`
/// owns it: what a journaled change to it is judged against (D227). `None`
/// when the copy doesn't list it for that session, or none was ever sent.
pub fn owned_item(
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
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_odd_length() {
        let raw = b"\x00\x0f\xa5\xffhello";
        let hex = to_hex(raw);
        assert_eq!(from_hex(&hex).as_deref(), Some(raw.as_slice()));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
    }

    #[test]
    fn kept_stamp_is_none_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(kept_stamp(&store).unwrap(), None);
        assert_eq!(owned_item(&store, "se_1", "it_1").unwrap(), None);
    }
}
