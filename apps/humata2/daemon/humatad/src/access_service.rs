//! AccessService: the grants "Always allow …" and approved ask-aheads made,
//! listed and revoked (IN-19, IN-23). Grants live in the policy, and the
//! Inbox saves each with the request its answer made, so a restart keeps
//! them (D96). A revoked grant goes on the event feed; the Inbox tells of
//! the ones answers make. The user also reads and sets each channel's
//! rules and the defaults new channels start with (IN-17, IN-21, ST-4),
//! saved before they apply and before the feed hears of them; anyone may
//! ask Explain what a session may do now. The While I'm away rule and the
//! approval passphrase aren't built yet.

use std::sync::Arc;
use std::time::SystemTime;

use humata2_access::{
    Call, Capability, ChannelId, CheckError, GrantId, Policy, PolicyError, PresentUser, Refusal,
    Rules, Source, Target, Verdict,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use tonic::{Code, Request, Response, Status};

use crate::events::{Feed, Scope};
use crate::grant_target;
use crate::guard::{self, Guard};
use crate::identity::{self, AgentCaller, Caller};
use crate::inbox::Inbox;
use crate::rules_kept::{self, Change, RulesKeptError, Unknown};
use crate::status;

/// How long a passphrase change waits for linked machines to keep the
/// policy copy that carries it before naming them pending.
const COPY_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

pub(crate) struct Access {
    guard: Arc<Guard>,
    /// Where revoked grants are told.
    feed: Arc<Feed>,
    /// Where grants are kept across a restart.
    inbox: Arc<Inbox>,
    /// Where rules are kept across a restart.
    store: Arc<Store>,
    /// The copy machines judge by, and which of them hold its newest.
    policy: Arc<crate::away::PolicySource>,
    /// Held while a rule changes, so two changes can't both pass the
    /// revision check.
    changing: tokio::sync::Mutex<()>,
}

impl Access {
    pub(crate) fn new(
        guard: Arc<Guard>,
        feed: Arc<Feed>,
        inbox: Arc<Inbox>,
        store: Arc<Store>,
        policy: Arc<crate::away::PolicySource>,
    ) -> Self {
        Self {
            guard,
            feed,
            inbox,
            store,
            policy,
            changing: tokio::sync::Mutex::new(()),
        }
    }
}

#[tonic::async_trait]
impl pb::access_service_server::AccessService for Access {
    /// The same list for every caller but a peer.
    async fn list_capabilities(
        &self,
        request: Request<pb::ListCapabilitiesRequest>,
    ) -> Result<Response<pb::ListCapabilitiesResponse>, Status> {
        if matches!(identity::caller(&request)?, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        let capabilities = Capability::ALL.into_iter().map(capability_info).collect();
        Ok(Response::new(pb::ListCapabilitiesResponse { capabilities }))
    }

    /// The user reads a channel's rules, with the grants under each. On a
    /// machine they are the policy copy's, the rules it judges by (D220).
    async fn get_channel_rules(
        &self,
        request: Request<pb::GetChannelRulesRequest>,
    ) -> Result<Response<pb::ChannelRules>, Status> {
        let caller = identity::caller(&request)?;
        if self.reads_copy(&caller) {
            let channel = channel_of(request.into_inner().channel)?;
            self.ensure_known(&channel).await?;
            let copy = self.copy_stamp().await?;
            let rules = self.guard.with_policy(|policy, _| {
                // A channel the copy doesn't hold is judged by the defaults;
                // reading it adds nothing to the policy.
                let rules = policy
                    .channel_rules(&ChannelId::new(channel.as_str()))
                    .unwrap_or_else(|| policy.new_channel_rules())
                    .clone();
                channel_rules_pb(policy, &channel, &rules, None, copy)
            });
            return Ok(Response::new(rules));
        }
        self.require_user(&caller)?;
        let channel = channel_of(request.into_inner().channel)?;
        self.ensure_known(&channel).await?;
        let revision = self.channel_revision(&channel).await?;
        let rules = self.guard.with_policy(|policy, _| {
            if policy
                .channel_rules(&ChannelId::new(channel.as_str()))
                .is_none()
            {
                policy
                    .add_channel(ChannelId::new(channel.as_str()))
                    .map_err(policy_failed)?;
            }
            Ok::<_, Status>(channel_pb(policy, &channel, revision))
        })?;
        Ok(Response::new(rules))
    }

    /// The user sets one rule; it applies from the next call (IN-21).
    async fn set_channel_rule(
        &self,
        request: Request<pb::SetChannelRuleRequest>,
    ) -> Result<Response<pb::ChannelRules>, Status> {
        let caller = identity::caller(&request)?;
        // An agent is refused here with IN-23 named, before anything else.
        self.require_user(&caller)?;
        let user = self.present_user(&caller)?;
        let request = request.into_inner();
        let channel = channel_of(request.channel)?;
        let capability = grant_target::capability_of(request.capability).ok_or_else(|| {
            unknown(
                "capability",
                &Unknown::Capability {
                    value: request.capability,
                },
            )
        })?;
        let decision = rules_kept::decision_of(request.decision)
            .map_err(|error| unknown("decision", &error))?;
        let change = Change {
            decisions: vec![(capability, decision)],
            away: None,
        };
        // Held until the rule is saved and applied.
        let _changing = self.changing.lock().await;
        self.ensure_known(&channel).await?;
        let actual = self.channel_revision(&channel).await?;
        check_revision(&channel, request.expected_revision, actual)?;
        let next = actual.saturating_add(1);
        let scope = Rules::Channel(ChannelId::new(channel.as_str()));
        let trial = self.guard.with_policy(|policy, _| {
            let mut trial = policy.clone();
            change.apply(&mut trial, &user, &scope)?;
            Ok::<_, Status>(channel_pb(&trial, &channel, next))
        })?;
        self.save(trial).await?;
        let rules = self.guard.with_policy(|policy, _| {
            change.apply(policy, &user, &scope)?;
            Ok::<_, Status>(channel_pb(policy, &channel, next))
        })?;
        self.feed.publish_in(
            pb::Change::Updated,
            pb::event::Payload::Rules(rules.clone()),
            Scope {
                channel,
                session: String::new(),
            },
            Some(identity::actor(&caller)),
        );
        Ok(Response::new(rules))
    }

    /// The user sets a channel's While I'm away rule (AW-2); machines get it
    /// with the next policy copy.
    async fn set_away_rule(
        &self,
        request: Request<pb::SetAwayRuleRequest>,
    ) -> Result<Response<pb::ChannelRules>, Status> {
        let caller = identity::caller(&request)?;
        self.require_user(&caller)?;
        let user = self.present_user(&caller)?;
        let request = request.into_inner();
        let channel = channel_of(request.channel)?;
        let away = rules_kept::away_of(request.away.as_ref())
            .map_err(|error| unknown("away", &error))?
            .ok_or_else(|| status::missing("away"))?;
        let change = Change {
            decisions: Vec::new(),
            away: Some(away),
        };
        // Held until the rule is saved and applied.
        let _changing = self.changing.lock().await;
        self.ensure_known(&channel).await?;
        let actual = self.channel_revision(&channel).await?;
        check_revision(&channel, request.expected_revision, actual)?;
        let next = actual.saturating_add(1);
        let scope = Rules::Channel(ChannelId::new(channel.as_str()));
        let trial = self.guard.with_policy(|policy, _| {
            let mut trial = policy.clone();
            change.apply(&mut trial, &user, &scope)?;
            Ok::<_, Status>(channel_pb(&trial, &channel, next))
        })?;
        self.save(trial).await?;
        let rules = self.guard.with_policy(|policy, _| {
            change.apply(policy, &user, &scope)?;
            Ok::<_, Status>(channel_pb(policy, &channel, next))
        })?;
        self.feed.publish_in(
            pb::Change::Updated,
            pb::event::Payload::Rules(rules.clone()),
            Scope {
                channel,
                session: String::new(),
            },
            Some(identity::actor(&caller)),
        );
        Ok(Response::new(rules))
    }

    /// The user reads what new channels start with.
    async fn get_default_rules(
        &self,
        request: Request<pb::GetDefaultRulesRequest>,
    ) -> Result<Response<pb::DefaultRules>, Status> {
        let caller = identity::caller(&request)?;
        if self.reads_copy(&caller) {
            let copy = self.copy_stamp().await?;
            let rules = self.guard.with_policy(|policy, _| pb::DefaultRules {
                revision: None,
                copy,
                ..default_pb(policy, 0)
            });
            return Ok(Response::new(rules));
        }
        self.require_user(&caller)?;
        let revision = self.default_revision().await?;
        let rules = self
            .guard
            .with_policy(|policy, _| default_pb(policy, revision));
        Ok(Response::new(rules))
    }

    /// The user sets what new channels start with (ST-4); channels that
    /// exist keep their rules.
    async fn set_default_rules(
        &self,
        request: Request<pb::SetDefaultRulesRequest>,
    ) -> Result<Response<pb::DefaultRules>, Status> {
        let caller = identity::caller(&request)?;
        self.require_user(&caller)?;
        let user = self.present_user(&caller)?;
        let request = request.into_inner();
        let change = Change {
            decisions: rules_kept::decisions_of(&request.rules)
                .map_err(|error| unknown("rules", &error))?,
            away: rules_kept::away_of(request.away.as_ref())
                .map_err(|error| unknown("away", &error))?,
        };
        let _changing = self.changing.lock().await;
        let actual = self.default_revision().await?;
        if let Some(expected) = request.expected_revision
            && expected.value != actual
        {
            return Err(revision_conflict(
                "default access rules",
                expected.value,
                actual,
            ));
        }
        let next = actual.saturating_add(1);
        let scope = Rules::NewChannels;
        let trial = self.guard.with_policy(|policy, _| {
            let mut trial = policy.clone();
            change.apply(&mut trial, &user, &scope)?;
            Ok::<_, Status>(default_pb(&trial, next))
        })?;
        let store = Arc::clone(&self.store);
        blocking(move || rules_kept::record_default(&store, &trial)).await?;
        let rules = self.guard.with_policy(|policy, _| {
            change.apply(policy, &user, &scope)?;
            Ok::<_, Status>(default_pb(policy, next))
        })?;
        Ok(Response::new(rules))
    }

    /// The user sees every grant; an agent only its own session's.
    async fn list_grants(
        &self,
        request: Request<pb::ListGrantsRequest>,
    ) -> Result<Response<pb::ListGrantsResponse>, Status> {
        let caller = identity::caller(&request)?;
        let own = match &caller {
            Caller::User(_) => None,
            Caller::Agent(agent) => Some(agent.session.id.clone()),
            Caller::Script(_) => {
                return Err(status::refused(&Refusal::UserOnly { capability: None }));
            }
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
        };
        let request = request.into_inner();
        let channel = request
            .channel
            .map(|channel| channel.value)
            .filter(|value| !value.is_empty());
        let requester = request
            .requester
            .map(|requester| requester.value)
            .filter(|value| !value.is_empty());
        let capability = match request.capability {
            0 => None,
            value => Some(grant_target::capability_of(value).ok_or_else(|| {
                status::invalid_field(
                    "capability",
                    pb::invalid_field::Problem::UnknownValue,
                    format!("capability {value} isn't one this humatad2 knows"),
                    "leave it unset to list every capability's grants, or update humatad2",
                )
            })?),
        };
        let grants = self.guard.with_policy(|policy, _| {
            let policy = &*policy;
            policy
                .grants()
                .filter(|grant| own.as_ref().is_none_or(|own| &grant.requester == own))
                .filter(|grant| {
                    channel
                        .as_deref()
                        .is_none_or(|channel| grant.channel.as_str() == channel)
                })
                .filter(|grant| {
                    requester
                        .as_deref()
                        .is_none_or(|requester| grant.requester.as_str() == requester)
                })
                .filter(|grant| capability.is_none_or(|capability| grant.capability == capability))
                .map(|grant| grant_target::grant(grant, policy.is_suspended(grant)))
                .collect()
        });
        Ok(Response::new(pb::ListGrantsResponse { grants, page: None }))
    }

    /// The user removes a grant; the next call it covered asks again.
    async fn revoke_grant(
        &self,
        request: Request<pb::RevokeGrantRequest>,
    ) -> Result<Response<pb::RevokeGrantResponse>, Status> {
        let caller = identity::caller(&request)?;
        // An agent is refused here with IN-23 named, before anything else.
        self.guard.require(
            &caller,
            &Call::Use {
                capability: Capability::AccessChange,
                target: Target::Global,
            },
        )?;
        let value = request
            .into_inner()
            .grant
            .map(|grant| grant.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| status::missing("grant"))?;
        let Some(id) = value.parse().ok().map(GrantId::new) else {
            return Err(no_grant(&value));
        };
        let principal = guard::principal(&caller);
        let revoked = self.guard.with_policy(|policy, context| {
            let Some(user) = principal.present_user(context) else {
                return Err(status::refused(&Refusal::UserOnly {
                    capability: Some(Capability::AccessChange),
                }));
            };
            let revoked = policy
                .revoke_grant(&user, id)
                .map_err(|error| match error {
                    PolicyError::UnknownGrant { .. } => no_grant(&value),
                    PolicyError::NeedsHost => status::refused(&Refusal::HostUnreachable),
                    other => status::internal(
                        format!("couldn't revoke grant {value}: {}", status::chain(&other)),
                        "",
                    ),
                })?;
            Ok(grant_target::grant(&revoked, policy.is_suspended(&revoked)))
        })?;
        let request = revoked
            .granted_from
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        self.inbox.revoked(&request, SystemTime::now()).await?;
        // The grant as it last stood, so the app can drop it.
        let scope = Scope {
            channel: revoked
                .channel
                .as_ref()
                .map(|channel| channel.value.clone())
                .unwrap_or_default(),
            session: revoked
                .requester
                .as_ref()
                .map(|session| session.value.clone())
                .unwrap_or_default(),
        };
        self.feed.publish_in(
            pb::Change::Deleted,
            pb::event::Payload::Grant(revoked),
            scope,
            Some(identity::actor(&caller)),
        );
        Ok(Response::new(pb::RevokeGrantResponse {}))
    }

    /// What the session would meet if it did this now, and why. An agent
    /// may ask only about itself; the user about any session.
    async fn explain(
        &self,
        request: Request<pb::ExplainRequest>,
    ) -> Result<Response<pb::ExplainResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let named = request
            .session
            .map(|session| session.value)
            .filter(|value| !value.is_empty());
        let here = self
            .guard
            .with_policy(|_, context| context.here.to_string());
        let agent = match &caller {
            Caller::Agent(agent) => {
                if named
                    .as_ref()
                    .is_some_and(|named| named.as_str() != agent.session.id.as_str())
                {
                    return Err(status::invalid_field(
                        "session",
                        pb::invalid_field::Problem::NotAllowedHere,
                        "an agent can ask what it may do, not what another session may",
                        "leave the session out to ask about this one",
                    ));
                }
                agent.clone()
            }
            Caller::User(_) => {
                let id = named.ok_or_else(|| status::missing("session"))?;
                let store = Arc::clone(&self.store);
                let looked = id.clone();
                let at = here.clone();
                let record = off_runtime(move || {
                    store
                        .session(&looked)
                        .map_err(|error| status::store(error, &at, "look up the session"))
                })
                .await?;
                let record = record.ok_or_else(|| {
                    status::not_found(
                        pb::EntityKind::Session,
                        &id,
                        format!("no session {id} on this machine"),
                        "check the session id; sessions on other machines can't be named here yet",
                    )
                })?;
                crate::identity::agent_caller_of(&record, &here)
            }
            Caller::Script(_) => {
                return Err(status::refused(&Refusal::ScriptTokenHasNoSession));
            }
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
        };
        let capability = grant_target::capability_of(request.capability).ok_or_else(|| {
            unknown(
                "capability",
                &Unknown::Capability {
                    value: request.capability,
                },
            )
        })?;
        let store = Arc::clone(&self.store);
        let call = match request.target {
            Some(target) => {
                let agent = agent.clone();
                let here = here.clone();
                off_runtime(move || {
                    grant_target::ask_ahead_call(&store, &here, &agent, capability, target)
                })
                .await?
            }
            None => Call::Use {
                capability,
                target: Target::Global,
            },
        };
        let principal = guard::principal(&Caller::Agent(agent.clone()));
        let channel = agent.session.channel.clone();
        // A Desk agent belongs to no channel; nothing is added for it (D182.3).
        let desk = matches!(agent.session_kind(), humata2_access::SessionKind::Desk(_));
        let explained = self.guard.with_policy(|policy, context| {
            if !desk && policy.channel_rules(&channel).is_none() {
                policy.add_channel(channel.clone()).map_err(policy_failed)?;
            }
            let mut checked = policy.check(&principal, &call, context);
            // A capability taken by a channel, not the whole of Humata.
            if matches!(checked, Err(CheckError::TargetDoesNotFit { .. }))
                && let Call::Use { capability, .. } = &call
            {
                let call = Call::Use {
                    capability: *capability,
                    target: Target::Channel(channel.clone()),
                };
                checked = policy.check(&principal, &call, context);
            }
            let verdict = checked.map_err(|error| {
                status::invalid_field(
                    "target",
                    pb::invalid_field::Problem::NotAllowedHere,
                    format!(
                        "\"{capability}\" can't be checked without the target it acts on: {error}"
                    ),
                    "name the target the capability acts on, as an ask-ahead does",
                )
            })?;
            Ok::<_, Status>(explanation(&verdict, policy, &channel))
        })?;
        Ok(Response::new(explained))
    }

    /// SEC-13, SEC-7: the user only, present, on the host.
    async fn set_approval_passphrase(
        &self,
        request: Request<pb::SetApprovalPassphraseRequest>,
    ) -> Result<Response<pb::SetApprovalPassphraseResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.require_user(&caller)?;
        self.present_user(&caller)?;
        let _changing = self.changing.lock().await;
        let set = crate::away::set_approval_passphrase(
            &self.store,
            self.guard.host_link().on_host(),
            self.guard.machine(),
            request.into_inner(),
        )
        .await?;
        // AW-5: machines get the new hash with the next policy copy; the
        // linked ones are given a moment to say they keep it.
        self.guard.policy_changed();
        let delivered = self.policy.delivered(COPY_WAIT).await;
        let ids = |machines: Vec<String>| {
            machines
                .into_iter()
                .map(|value| pb::MachineId { value })
                .collect()
        };
        Ok(Response::new(pb::SetApprovalPassphraseResponse {
            updated: ids(delivered.updated),
            pending: ids(delivered.pending),
            ..set
        }))
    }
}

impl Access {
    /// D220: on a machine the user reads the rules from the policy copy,
    /// whether the host is reachable or away. Everyone else, and every read
    /// on the host, goes through [`Access::require_user`].
    fn reads_copy(&self, caller: &Caller) -> bool {
        !self.guard.host_link().on_host() && matches!(caller, Caller::User(_))
    }

    /// Which copy a machine's rules come from; `None` when the host never
    /// sent one, so they are the defaults.
    async fn copy_stamp(&self) -> Result<Option<pb::PolicyCopyStamp>, Status> {
        let store = Arc::clone(&self.store);
        off_runtime(move || {
            crate::away::kept_policy_stamp(&store).map_err(|error| {
                status::internal(
                    format!(
                        "couldn't read the rules this machine keeps: {}",
                        status::chain(&error)
                    ),
                    "",
                )
            })
        })
        .await
    }

    /// Refuses anyone but the user, naming IN-23.
    fn require_user(&self, caller: &Caller) -> Result<(), Status> {
        self.guard.require(
            caller,
            &Call::Use {
                capability: Capability::AccessChange,
                target: Target::Global,
            },
        )
    }

    fn present_user(&self, caller: &Caller) -> Result<PresentUser, Status> {
        let principal = guard::principal(caller);
        self.guard
            .with_policy(|_, context| principal.present_user(context))
            .ok_or_else(|| {
                status::refused(&Refusal::UserOnly {
                    capability: Some(Capability::AccessChange),
                })
            })
    }

    async fn channel_revision(&self, channel: &str) -> Result<u64, Status> {
        let store = Arc::clone(&self.store);
        let channel = channel.to_owned();
        blocking(move || rules_kept::channel_revision(&store, &channel)).await
    }

    async fn default_revision(&self) -> Result<u64, Status> {
        let store = Arc::clone(&self.store);
        blocking(move || rules_kept::default_revision(&store)).await
    }

    /// Saves `rules`, a channel's.
    async fn save(&self, rules: pb::ChannelRules) -> Result<(), Status> {
        // Grants are the Inbox's to keep.
        let rules = pb::ChannelRules {
            rules: rules
                .rules
                .into_iter()
                .map(|rule| pb::Rule {
                    grants: Vec::new(),
                    ..rule
                })
                .collect(),
            ..rules
        };
        let store = Arc::clone(&self.store);
        blocking(move || rules_kept::record_channel(&store, &rules)).await
    }

    /// Refuses a channel nothing on this machine knows, so a typo doesn't
    /// make rules for it.
    async fn ensure_known(&self, channel: &str) -> Result<(), Status> {
        let (held, here) = self.guard.with_policy(|policy, context| {
            (
                policy.channel_rules(&ChannelId::new(channel)).is_some(),
                context.here.to_string(),
            )
        });
        if held {
            return Ok(());
        }
        let store = Arc::clone(&self.store);
        let id = channel.to_owned();
        let known = off_runtime(move || {
            let failed = |error| status::store(error, &here, "look up the channel");
            if store.channel(&id).map_err(failed)?.is_some() {
                return Ok(true);
            }
            Ok(store
                .sessions()
                .map_err(failed)?
                .iter()
                .any(|session| session.channel == id))
        })
        .await?;
        if known {
            Ok(())
        } else {
            Err(status::not_found(
                pb::EntityKind::Channel,
                channel,
                format!("no channel {channel} on this machine"),
                "check the channel id; list the channels and pick one from the list",
            ))
        }
    }
}

impl Change {
    /// Applies the change to `policy`'s rules in `scope`, adding the
    /// channel with the defaults first if the policy doesn't hold it yet.
    fn apply(&self, policy: &mut Policy, user: &PresentUser, scope: &Rules) -> Result<(), Status> {
        if let Rules::Channel(channel) = scope
            && policy.channel_rules(channel).is_none()
        {
            policy.add_channel(channel.clone()).map_err(policy_failed)?;
        }
        for &(capability, decision) in &self.decisions {
            policy
                .set_rule(user, scope, capability, decision)
                .map_err(policy_failed)?;
        }
        if let Some(away) = &self.away {
            policy
                .set_away_rule(user, scope, away.clone())
                .map_err(policy_failed)?;
        }
        Ok(())
    }
}

fn channel_of(channel: Option<pb::ChannelId>) -> Result<String, Status> {
    channel
        .map(|channel| channel.value)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| status::missing("channel"))
}

/// The channel's rules and the grants under them, as clients see them.
fn channel_pb(policy: &Policy, channel: &str, revision: u64) -> pb::ChannelRules {
    let rules = policy
        .channel_rules(&ChannelId::new(channel))
        .cloned()
        .unwrap_or_default();
    channel_rules_pb(
        policy,
        channel,
        &rules,
        Some(pb::Revision { value: revision }),
        None,
    )
}

/// `rules`, a channel's, and the grants under them, as clients see them.
fn channel_rules_pb(
    policy: &Policy,
    channel: &str,
    rules: &humata2_access::ChannelRules,
    revision: Option<pb::Revision>,
    copy: Option<pb::PolicyCopyStamp>,
) -> pb::ChannelRules {
    let id = ChannelId::new(channel);
    let grants: Vec<pb::Grant> = policy
        .grants()
        .filter(|grant| grant.channel == id)
        .map(|grant| grant_target::grant(grant, policy.is_suspended(grant)))
        .collect();
    pb::ChannelRules {
        channel: Some(pb::ChannelId {
            value: channel.to_owned(),
        }),
        rules: rules_kept::rules_pb(rules, &grants),
        away: Some(rules_kept::away_pb(rules.away())),
        revision,
        copy,
    }
}

fn default_pb(policy: &Policy, revision: u64) -> pb::DefaultRules {
    let rules = policy.new_channel_rules();
    pb::DefaultRules {
        rules: rules_kept::rules_pb(rules, &[]),
        away: Some(rules_kept::away_pb(rules.away())),
        revision: Some(pb::Revision { value: revision }),
        copy: None,
    }
}

fn capability_info(capability: Capability) -> pb::CapabilityInfo {
    use Capability as C;
    use pb::CapabilityScope as S;
    let scope = match capability {
        C::GlobalRead
        | C::GlobalBoardWrite
        | C::ChannelCreate
        | C::TrashRestore
        | C::InboxAnswer
        | C::AccessChange
        | C::SettingsChange
        | C::MarksChange
        | C::TrashEmpty
        | C::HostSecrets
        | C::ProviderImport
        | C::ChannelDelete => S::Global,
        C::SharedWrite
        | C::FilesFetchLocal
        | C::FilesReadOtherMachine
        | C::SessionStartOtherMachine
        | C::NetworkReach
        | C::MachineProvision
        | C::MachineSessionsStop
        | C::MachineDestroy
        | C::MachineScriptRun
        | C::MachineSettings
        | C::PeerReport
        | C::HostControl => S::Machine,
        C::OwnBoardWrite
        | C::TodoSuggest
        | C::TodoClose
        | C::ViewsPrepare
        | C::ReviewDraft
        | C::InboxAsk
        | C::ReviewFocus
        | C::ReviewNote
        | C::ReviewOrder
        | C::CommentReply
        | C::CommentResolve
        | C::FilesReadOtherSession
        | C::FilesCopyIn
        | C::SessionNote
        | C::SessionType
        | C::OwnSessionManage
        | C::OwnSessionEnd
        | C::SessionMove
        | C::SessionStopOther
        | C::SessionDeleteOther
        | C::SessionsDeleteMany
        | C::PrSubmit
        | C::UserReviewState
        | C::TerminalInput => S::Session,
        C::ChannelRead
        | C::ChannelBoardAdd
        | C::ChannelBoardEditOwn
        | C::ChannelBoardEditOthers
        | C::SessionStartSameMachine
        | C::PrReviewStart
        | C::GitPush
        | C::ChannelRepoAdd
        | C::DeskMoveToChannel
        | C::ChannelChange => S::Channel,
    };
    pb::CapabilityInfo {
        capability: guard::capability(capability) as i32,
        scope: scope as i32,
        title: capability.label().to_owned(),
        default_decision: capability
            .default_decision()
            .map_or(pb::Decision::Unspecified, rules_kept::decision_pb)
            as i32,
        agents_may_hold: capability.has_rule(),
        rule_editable: capability.has_rule(),
        may_be_low_risk: capability.may_be_low_risk(),
    }
}

/// What a verdict means to the person asking Explain.
fn explanation(verdict: &Verdict, policy: &Policy, channel: &ChannelId) -> pb::ExplainResponse {
    let away_mode = policy
        .channel_rules(channel)
        .map(|rules| rules_kept::away_pb(rules.away()).mode)
        .unwrap_or_default();
    let mut explained = pb::ExplainResponse::default();
    match verdict {
        Verdict::Allowed { source, .. } => {
            explained.decision = pb::Decision::Allow as i32;
            explained.source = match source {
                Source::Owned => pb::DecisionSource::Owned,
                Source::Grant(id) => {
                    explained.grant = Some(pb::GrantId {
                        value: id.get().to_string(),
                    });
                    pb::DecisionSource::Grant
                }
                Source::AwayRule => pb::DecisionSource::AwayRule,
                _ => pb::DecisionSource::Rule,
            } as i32;
        }
        Verdict::Ask(_) => {
            explained.decision = pb::Decision::Ask as i32;
            explained.source = pb::DecisionSource::Rule as i32;
        }
        Verdict::WaitForHost => {
            explained.decision = pb::Decision::Ask as i32;
            explained.source = pb::DecisionSource::AwayRule as i32;
            explained.host_away = true;
            explained.away_mode = away_mode;
        }
        Verdict::Refused(refusal) => {
            explained.decision = pb::Decision::Never as i32;
            explained.source = match refusal {
                Refusal::NeverRule { .. } => pb::DecisionSource::Rule,
                Refusal::UserOnly { .. } => pb::DecisionSource::UserOnly,
                Refusal::UserAway { .. } => {
                    explained.host_away = true;
                    explained.away_mode = away_mode;
                    pb::DecisionSource::AwayRule
                }
                _ => pb::DecisionSource::HardLimit,
            } as i32;
        }
    }
    explained
}

fn unknown(field: &str, error: &Unknown) -> Status {
    status::invalid_field(
        field,
        pb::invalid_field::Problem::UnknownValue,
        error.to_string(),
        "pick one of the values the catalog lists (ListCapabilities), or update humatad2",
    )
}

fn policy_failed(error: PolicyError) -> Status {
    match error {
        PolicyError::NeedsHost => status::refused(&Refusal::HostUnreachable),
        PolicyError::NotARule { .. } => status::invalid_field(
            "capability",
            pb::invalid_field::Problem::NotAllowedHere,
            error.to_string(),
            "pick a capability the catalog lists as rule_editable",
        ),
        PolicyError::SettingNotOffered { .. } => status::invalid_field(
            "decision",
            pb::invalid_field::Problem::NotAllowedHere,
            error.to_string(),
            "pick Ask me or Never for it",
        ),
        PolicyError::NotLowRisk { .. } => status::invalid_field(
            "away.low_risk",
            pb::invalid_field::Problem::NotAllowedHere,
            error.to_string(),
            "remove it from the low-risk kinds; the catalog marks which may be listed",
        ),
        PolicyError::UnknownChannel { ref channel } => status::not_found(
            pb::EntityKind::Channel,
            channel.as_str(),
            error.to_string(),
            "check the channel id; list the channels and pick one from the list",
        ),
        other => status::internal(
            format!(
                "couldn't change the access rules: {}",
                status::chain(&other)
            ),
            "",
        ),
    }
}

fn check_revision(
    channel: &str,
    expected: Option<pb::Revision>,
    actual: u64,
) -> Result<(), Status> {
    match expected {
        Some(expected) if expected.value != actual => Err(revision_conflict(
            &format!("channel {channel}'s access rules"),
            expected.value,
            actual,
        )),
        _ => Ok(()),
    }
}

fn revision_conflict(what: &str, expected: u64, actual: u64) -> Status {
    status::failure(
        Code::Aborted,
        status::error(
            format!("{what} changed since you saw them (revision {expected}, now {actual})"),
            "read the rules again and make the change on what they are now",
            pb::error::Kind::RevisionConflict(pb::RevisionConflict {
                kind: pb::EntityKind::Channel as i32,
                id: what.to_owned(),
                expected: Some(pb::Revision { value: expected }),
                actual: Some(pb::Revision { value: actual }),
            }),
        ),
    )
}

/// Runs work that blocks off the runtime.
async fn off_runtime<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Status> + Send + 'static,
) -> Result<T, Status> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))?
}

/// Runs the rules' store work off the runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, RulesKeptError> + Send + 'static,
) -> Result<T, Status> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))?
        .map_err(|error| {
            status::internal(
                format!("couldn't keep the access rules: {}", status::chain(&error)),
                "",
            )
        })
}

fn no_grant(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Grant,
        id,
        format!(
            "no grant {id} on this machine; it may have been revoked or ended with its session"
        ),
        "list the grants again and pick one from the list; a grant ends when it is revoked or its session is deleted",
    )
}
