//! The Agent access rules the user set, kept across a restart (D96, ST-4)
//! and turned between the proto and the policy.
//!
//! Each channel's rules and the defaults new channels start with are saved
//! as the `humata.v1.ChannelRules` and `humata.v1.DefaultRules` the daemon
//! sends, grants left out (the Inbox keeps those), under the revision the
//! save made. A restart puts them back in the policy before anything else
//! uses it.

use std::collections::{BTreeMap, BTreeSet};

use humata2_access::{
    AwayRule, Capability, ChannelId, ChannelRules, Decision, Policy, PolicyError, RulesState,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SavedRules, Store, StoreError};
use prost::Message;

use crate::grant_target;
use crate::guard::{self, Guard};

/// Store setting key marking that the one-time PERMAUDIT migration (ATM-33)
/// has already run on this store, so subsequent restarts never overwrite an
/// owner's explicit `Ask` or `Never` setting on a promoted capability.
const PERMAUDIT_MIGRATED_KEY: &str = "permaudit_migrated";

/// Why saved rules couldn't be read back, or a change couldn't be saved.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RulesKeptError {
    #[error(
        "couldn't read the saved access rules from the store; the store may be damaged, so restore it from its last good copy before starting humatad2 again. Starting without them would put every rule back to its default, which may be looser than the ones you set"
    )]
    Read {
        #[source]
        source: StoreError,
    },

    #[error(
        "the saved access rules for {scope} can't be decoded; the store may be damaged, so restore it from its last good copy before starting humatad2 again. Starting without them would put every rule back to its default, which may be looser than the ones you set"
    )]
    Decode {
        scope: String,
        #[source]
        source: prost::DecodeError,
    },

    #[error(
        "the saved access rules for {scope} aren't ones this humatad2 knows; update humatad2 to the version that saved them, or restore the store from a copy made before"
    )]
    Unknown {
        scope: String,
        #[source]
        source: Unknown,
    },

    #[error(
        "the saved access rules don't fit the access policy; restore the store from a copy made before the rules were saved"
    )]
    Install {
        #[source]
        source: PolicyError,
    },

    #[error("couldn't save the access rules; check that the disk has room and try again")]
    Save {
        #[source]
        source: StoreError,
    },
}

/// A capability, setting or mode a message names that this build doesn't
/// know.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Unknown {
    #[error("capability {value} isn't one this humatad2 knows")]
    Capability { value: i32 },
    #[error("decision {value} isn't Allow, Ask me or Never")]
    Decision { value: i32 },
    #[error("while-away mode {value} isn't Wait, Skip or Allow low-risk kinds")]
    Away { value: i32 },
}

/// What a request changes in one set of rules.
#[derive(Debug, Clone, Default)]
pub(crate) struct Change {
    pub(crate) decisions: Vec<(Capability, Decision)>,
    /// Unset leaves the While I'm away rule as it is.
    pub(crate) away: Option<AwayRule>,
}

/// Puts the saved rules in the policy, before the Inbox puts its grants
/// back (grants name channels the rules must already hold).
pub(crate) fn restore(store: &Store, guard: &Guard) -> Result<(), RulesKeptError> {
    let channels = store
        .channel_rules()
        .map_err(|source| RulesKeptError::Read { source })?;
    let defaults = store
        .default_rules()
        .map_err(|source| RulesKeptError::Read { source })?;
    let already_migrated = store
        .setting(PERMAUDIT_MIGRATED_KEY)
        .map_err(|source| RulesKeptError::Read { source })?
        .is_some();
    let default_revision = defaults.as_ref().map_or(0, |saved| saved.revision);
    let channel_revisions: BTreeMap<String, u64> = channels
        .iter()
        .map(|(id, saved)| (id.clone(), saved.revision))
        .collect();

    let migrated_to_save = guard.with_policy(|policy, _| {
        let mut state = policy.to_state();
        if let Some(saved) = defaults {
            let rules = pb::DefaultRules::decode(saved.rules.as_slice()).map_err(|source| {
                RulesKeptError::Decode {
                    scope: "new channels".to_owned(),
                    source,
                }
            })?;
            state.new_channels =
                rules_state(&rules.rules, rules.away.as_ref()).map_err(|source| {
                    RulesKeptError::Unknown {
                        scope: "new channels".to_owned(),
                        source,
                    }
                })?;
        }
        for (id, saved) in channels {
            let rules = pb::ChannelRules::decode(saved.rules.as_slice()).map_err(|source| {
                RulesKeptError::Decode {
                    scope: format!("channel {id}"),
                    source,
                }
            })?;
            let rules = rules_state(&rules.rules, rules.away.as_ref()).map_err(|source| {
                RulesKeptError::Unknown {
                    scope: format!("channel {id}"),
                    source,
                }
            })?;
            state.channels.insert(ChannelId::new(id), rules);
        }
        let mut restored =
            Policy::from_state(state).map_err(|source| RulesKeptError::Install { source })?;
        let to_save = if !already_migrated {
            let applied = restored.apply_permaudit_migration(&BTreeMap::new(), &BTreeSet::new());
            let mut defaults_changed = false;
            let mut changed_channels = BTreeSet::new();
            for (scope, _, _, _) in applied {
                match scope {
                    None => defaults_changed = true,
                    Some(channel_id) => {
                        changed_channels.insert(channel_id);
                    }
                }
            }
            let updated_defaults = defaults_changed.then(|| {
                let rules = restored.new_channel_rules();
                pb::DefaultRules {
                    rules: rules_pb(rules, &[]),
                    away: Some(away_pb(rules.away())),
                    revision: Some(pb::Revision {
                        value: default_revision,
                    }),
                    copy: None,
                }
            });
            let updated_channels: Vec<pb::ChannelRules> = changed_channels
                .into_iter()
                .filter_map(|channel_id| {
                    let rules = restored.channel_rules(&channel_id)?;
                    let revision = channel_revisions
                        .get(channel_id.as_str())
                        .copied()
                        .unwrap_or(0);
                    Some(pb::ChannelRules {
                        channel: Some(pb::ChannelId {
                            value: channel_id.as_str().to_owned(),
                        }),
                        rules: rules_pb(rules, &[]),
                        away: Some(away_pb(rules.away())),
                        revision: Some(pb::Revision { value: revision }),
                        copy: None,
                    })
                })
                .collect();
            Some((updated_defaults, updated_channels))
        } else {
            None
        };
        *policy = restored;
        Ok(to_save)
    })?;

    if let Some((updated_defaults, updated_channels)) = migrated_to_save {
        if let Some(defaults_pb) = updated_defaults {
            record_default(store, &defaults_pb)?;
        }
        for channel_pb in updated_channels {
            record_channel(store, &channel_pb)?;
        }
        store
            .put_setting(PERMAUDIT_MIGRATED_KEY, "1", 1)
            .map_err(|source| RulesKeptError::Save { source })?;
    }
    Ok(())
}

/// Saves a channel's rules, as `rules` says them, at `rules.revision`.
pub(crate) fn record_channel(
    store: &Store,
    rules: &pb::ChannelRules,
) -> Result<(), RulesKeptError> {
    let channel = rules
        .channel
        .as_ref()
        .map(|channel| channel.value.as_str())
        .unwrap_or_default();
    let saved = SavedRules {
        revision: rules.revision.map_or(0, |revision| revision.value),
        rules: rules.encode_to_vec(),
    };
    store
        .put_channel_rules(channel, &saved)
        .map_err(|source| RulesKeptError::Save { source })
}

/// Saves the rules new channels start with.
pub(crate) fn record_default(
    store: &Store,
    rules: &pb::DefaultRules,
) -> Result<(), RulesKeptError> {
    let saved = SavedRules {
        revision: rules.revision.map_or(0, |revision| revision.value),
        rules: rules.encode_to_vec(),
    };
    store
        .put_default_rules(&saved)
        .map_err(|source| RulesKeptError::Save { source })
}

/// The revision a channel's rules stand at; 0 until the user changes them.
pub(crate) fn channel_revision(store: &Store, channel: &str) -> Result<u64, RulesKeptError> {
    let saved = store
        .channel_rules()
        .map_err(|source| RulesKeptError::Read { source })?;
    Ok(saved
        .into_iter()
        .find(|(id, _)| id == channel)
        .map_or(0, |(_, saved)| saved.revision))
}

/// The revision the defaults stand at; 0 until the user changes them.
pub(crate) fn default_revision(store: &Store) -> Result<u64, RulesKeptError> {
    let saved = store
        .default_rules()
        .map_err(|source| RulesKeptError::Read { source })?;
    Ok(saved.map_or(0, |saved| saved.revision))
}

// ----- Between the proto and the policy. -----

/// The settings of `rules` as the proto lists them: one per capability
/// that has a rule. `grants` are the ones standing in the channel.
pub(crate) fn rules_pb(rules: &ChannelRules, grants: &[pb::Grant]) -> Vec<pb::Rule> {
    Capability::ALL
        .into_iter()
        .filter_map(|capability| {
            let decision = rules.decision(capability)?;
            let value = guard::capability(capability) as i32;
            Some(pb::Rule {
                capability: value,
                decision: decision_pb(decision) as i32,
                grants: grants
                    .iter()
                    .filter(|grant| grant.capability == value)
                    .cloned()
                    .collect(),
            })
        })
        .collect()
}

pub(crate) fn away_pb(away: &AwayRule) -> pb::AwayRule {
    match away {
        AwayRule::Wait => pb::AwayRule {
            mode: pb::AwayMode::Wait as i32,
            low_risk: Vec::new(),
        },
        AwayRule::Skip => pb::AwayRule {
            mode: pb::AwayMode::Skip as i32,
            low_risk: Vec::new(),
        },
        AwayRule::AllowLowRisk(kinds) => pb::AwayRule {
            mode: pb::AwayMode::AllowLowRisk as i32,
            low_risk: kinds
                .iter()
                .map(|kind| guard::capability(*kind) as i32)
                .collect(),
        },
    }
}

/// The While I'm away rule a message sets; `None` for an unset mode, which
/// leaves it as it is.
pub(crate) fn away_of(away: Option<&pb::AwayRule>) -> Result<Option<AwayRule>, Unknown> {
    let Some(away) = away else {
        return Ok(None);
    };
    let kinds = || -> Result<BTreeSet<Capability>, Unknown> {
        away.low_risk
            .iter()
            .map(|value| {
                grant_target::capability_of(*value).ok_or(Unknown::Capability { value: *value })
            })
            .collect()
    };
    match pb::AwayMode::try_from(away.mode) {
        Ok(pb::AwayMode::Unspecified) => Ok(None),
        Ok(pb::AwayMode::Wait) => Ok(Some(AwayRule::Wait)),
        Ok(pb::AwayMode::Skip) => Ok(Some(AwayRule::Skip)),
        Ok(pb::AwayMode::AllowLowRisk) => Ok(Some(AwayRule::AllowLowRisk(kinds()?))),
        Err(_) => Err(Unknown::Away { value: away.mode }),
    }
}

/// The settings a message lists.
pub(crate) fn decisions_of(rules: &[pb::Rule]) -> Result<Vec<(Capability, Decision)>, Unknown> {
    rules
        .iter()
        .map(|rule| {
            let capability =
                grant_target::capability_of(rule.capability).ok_or(Unknown::Capability {
                    value: rule.capability,
                })?;
            Ok((capability, decision_of(rule.decision)?))
        })
        .collect()
}

pub(crate) fn decision_of(value: i32) -> Result<Decision, Unknown> {
    match pb::Decision::try_from(value) {
        Ok(pb::Decision::Allow) => Ok(Decision::Allow),
        Ok(pb::Decision::Ask) => Ok(Decision::Ask),
        Ok(pb::Decision::Never) => Ok(Decision::Never),
        Ok(pb::Decision::Unspecified) | Err(_) => Err(Unknown::Decision { value }),
    }
}

pub(crate) fn decision_pb(decision: Decision) -> pb::Decision {
    match decision {
        Decision::Allow => pb::Decision::Allow,
        Decision::Ask => pb::Decision::Ask,
        Decision::Never => pb::Decision::Never,
    }
}

fn rules_state(rules: &[pb::Rule], away: Option<&pb::AwayRule>) -> Result<RulesState, Unknown> {
    Ok(RulesState {
        decisions: decisions_of(rules)?,
        away: away_of(away)?.unwrap_or(AwayRule::Wait),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_listed_as_the_proto_come_back_the_same() {
        let rules = ChannelRules::defaults();
        let listed = rules_pb(&rules, &[]);
        let state = rules_state(&listed, Some(&away_pb(&AwayRule::Skip))).unwrap();
        assert_eq!(state.away, AwayRule::Skip);
        let mut decisions = state.decisions;
        decisions.sort();
        let mut expected: Vec<_> = rules.decisions().collect();
        expected.sort();
        assert_eq!(decisions, expected);
    }

    #[test]
    fn a_decision_left_unset_is_unknown() {
        assert_eq!(decision_of(0), Err(Unknown::Decision { value: 0 }));
        assert_eq!(decision_of(pb::Decision::Never as i32), Ok(Decision::Never));
    }

    #[test]
    fn user_only_capabilities_have_no_rule_listed() {
        let listed = rules_pb(&ChannelRules::defaults(), &[]);
        let access_change = guard::capability(Capability::AccessChange) as i32;
        assert!(listed.iter().all(|rule| rule.capability != access_change));
    }

    fn rules_with_overrides(overrides: &[(Capability, Decision)]) -> Vec<pb::Rule> {
        let mut listed = rules_pb(&ChannelRules::defaults(), &[]);
        for &(capability, decision) in overrides {
            let cap_pb = guard::capability(capability) as i32;
            let dec_pb = decision_pb(decision) as i32;
            if let Some(rule) = listed.iter_mut().find(|rule| rule.capability == cap_pb) {
                rule.decision = dec_pb;
            }
        }
        listed
    }

    fn channel_rules_pb(
        channel: &str,
        overrides: &[(Capability, Decision)],
        revision: u64,
    ) -> pb::ChannelRules {
        pb::ChannelRules {
            channel: Some(pb::ChannelId {
                value: channel.to_owned(),
            }),
            rules: rules_with_overrides(overrides),
            away: Some(away_pb(&AwayRule::Wait)),
            revision: Some(pb::Revision { value: revision }),
            copy: None,
        }
    }

    fn default_rules_pb(overrides: &[(Capability, Decision)], revision: u64) -> pb::DefaultRules {
        pb::DefaultRules {
            rules: rules_with_overrides(overrides),
            away: Some(away_pb(&AwayRule::Wait)),
            revision: Some(pb::Revision { value: revision }),
            copy: None,
        }
    }

    #[test]
    fn owner_ask_on_promoted_capability_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();

        // First startup on a fresh store marks PERMAUDIT migration done.
        let guard = Guard::new("host", pb::DaemonRole::Host);
        restore(&store, &guard).unwrap();

        // Owner explicitly tightens SessionNote back to Ask on a channel and in defaults.
        let overrides = &[(Capability::SessionNote, Decision::Ask)];
        record_channel(&store, &channel_rules_pb("ch-1", overrides, 1)).unwrap();
        record_default(&store, &default_rules_pb(overrides, 1)).unwrap();

        // Daemon restarts and restores rules from the store.
        let restarted = Guard::new("host", pb::DaemonRole::Host);
        restore(&store, &restarted).unwrap();

        restarted.with_policy(|policy, _| {
            assert_eq!(
                policy
                    .channel_rules(&ChannelId::new("ch-1"))
                    .and_then(|rules| rules.decision(Capability::SessionNote)),
                Some(Decision::Ask),
                "owner's explicit Ask on channel SessionNote must survive restart"
            );
            assert_eq!(
                policy
                    .new_channel_rules()
                    .decision(Capability::SessionNote),
                Some(Decision::Ask),
                "owner's explicit Ask on default SessionNote must survive restart"
            );
        });
    }

    #[test]
    fn pre_permaudit_channel_migrates_once_and_persists_across_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();

        // Simulate a pre-PERMAUDIT store (no permaudit_migrated marker yet)
        // where SessionNote was at the old Ask default and TrashRestore was Never.
        let legacy = &[
            (Capability::SessionNote, Decision::Ask),
            (Capability::TrashRestore, Decision::Never),
        ];
        record_channel(&store, &channel_rules_pb("ch-legacy", legacy, 1)).unwrap();

        // Startup 1: migration promotes Ask -> Allow and leaves Never alone.
        let guard1 = Guard::new("host", pb::DaemonRole::Host);
        restore(&store, &guard1).unwrap();
        guard1.with_policy(|policy, _| {
            let rules = policy.channel_rules(&ChannelId::new("ch-legacy")).unwrap();
            assert_eq!(rules.decision(Capability::SessionNote), Some(Decision::Allow));
            assert_eq!(rules.decision(Capability::TrashRestore), Some(Decision::Never));
        });

        // Startup 2 without edits: migrated Allow stays Allow.
        let guard2 = Guard::new("host", pb::DaemonRole::Host);
        restore(&store, &guard2).unwrap();
        guard2.with_policy(|policy, _| {
            let rules = policy.channel_rules(&ChannelId::new("ch-legacy")).unwrap();
            assert_eq!(rules.decision(Capability::SessionNote), Some(Decision::Allow));
            assert_eq!(rules.decision(Capability::TrashRestore), Some(Decision::Never));
        });

        // Owner later tightens SessionNote back to Ask; startup 3 must keep Ask.
        record_channel(&store, &channel_rules_pb("ch-legacy", legacy, 2)).unwrap();

        let guard3 = Guard::new("host", pb::DaemonRole::Host);
        restore(&store, &guard3).unwrap();
        guard3.with_policy(|policy, _| {
            let rules = policy.channel_rules(&ChannelId::new("ch-legacy")).unwrap();
            assert_eq!(rules.decision(Capability::SessionNote), Some(Decision::Ask));
            assert_eq!(rules.decision(Capability::TrashRestore), Some(Decision::Never));
        });
    }
}
