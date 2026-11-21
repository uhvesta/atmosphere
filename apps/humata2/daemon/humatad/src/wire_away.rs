//! Wiring adapter connecting `humata2_svc_away` (`PolicySource`, `Uplinks`) to
//! `humatad`'s concrete services (`Guard`, `Feed`, `Inbox`, `Store`,
//! `rules_kept`, `grant_target`, `contained`).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_access::{
    Agent, ChannelRules, Context, Grant, GrantId, MachineId, Policy, PolicyError, RequestId,
    RulesState, Verdict,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{Store, StoreError};
use humata2_svc_away::{EventSink, HostLink, PolicyGuard, PolicySource, UplinkInbox, Uplinks};
use tokio::sync::watch;
use tonic::Status;

use crate::contained;
use crate::events::Feed;
use crate::grant_target;
use crate::guard::Guard;
use crate::identity::AgentCaller;
use crate::inbox::Inbox;
use crate::rules_kept;

// ---------------------------------------------------------------------------
// EventSink adapter
// ---------------------------------------------------------------------------

struct FeedAdapter(Arc<Feed>);

impl EventSink for FeedAdapter {
    fn publish(&self, change: pb::Change, payload: pb::event::Payload, actor: Option<pb::Actor>) {
        self.0.publish(change, payload, actor);
    }
}

// ---------------------------------------------------------------------------
// UplinkInbox adapter
// ---------------------------------------------------------------------------

struct InboxAdapter(Arc<Inbox>);

#[tonic::async_trait]
impl UplinkInbox for InboxAdapter {
    async fn answer_relayed(
        &self,
        relayed: &pb::InboxAnswerRelay,
    ) -> Option<pb::InboxAnswerRefused> {
        self.0.answer_relayed(relayed).await
    }

    async fn revoked(&self, request: &str, at: SystemTime) -> Result<(), Status> {
        self.0.revoked(request, at).await
    }
}

// ---------------------------------------------------------------------------
// PolicyGuard adapter
// ---------------------------------------------------------------------------

struct PolicyGuardAdapter(Arc<Guard>);

impl PolicyGuard for PolicyGuardAdapter {
    fn policy_revision(&self) -> u64 {
        self.0.policy_revision()
    }

    fn policy_changes(&self) -> watch::Receiver<u64> {
        self.0.policy_changes()
    }

    fn policy_clone(&self) -> Policy {
        self.0.with_policy(|policy, _| policy.clone())
    }

    fn replace_policy(&self, policy: Policy) {
        self.0.with_policy(|held, _| *held = policy);
    }

    fn host_rules_arrived(&self) {
        self.0.host_rules_arrived();
    }

    fn drop_local_grants(&self, answered: &BTreeSet<String>) -> Result<(), PolicyError> {
        self.0.with_policy(|policy, _| {
            let mut state = policy.to_state();
            let before = state.grants.len();
            state.grants.retain(|grant| {
                grant.on_host || !answered.contains(&grant.granted_from.to_string())
            });
            if state.grants.len() == before {
                return Ok(());
            }
            Policy::from_state(state).map(|kept| *policy = kept)
        })
    }

    fn encode_rules(&self, rules: &ChannelRules) -> (Vec<pb::Rule>, pb::AwayRule) {
        (
            rules_kept::rules_pb(rules, &[]),
            rules_kept::away_pb(rules.away()),
        )
    }

    fn decode_rules(
        &self,
        rules: &[pb::Rule],
        away: Option<&pb::AwayRule>,
    ) -> Result<RulesState, String> {
        let mut decisions = std::collections::BTreeMap::new();
        for rule in rules {
            let capability = grant_target::capability_of(rule.capability)
                .ok_or_else(|| format!("unknown capability {}", rule.capability))?;
            let decision =
                rules_kept::decision_of(rule.decision).map_err(|err| err.to_string())?;
            decisions.insert(capability, decision);
        }
        let away = rules_kept::away_of(away)
            .map_err(|err| err.to_string())?
            .unwrap_or(humata2_access::AwayRule::Wait);
        Ok(RulesState { decisions: decisions.into_iter().collect(), away })
    }

    fn encode_grant(&self, grant: &Grant, suspended: bool) -> pb::Grant {
        grant_target::grant(grant, suspended)
    }

    fn decode_grant(
        &self,
        store: &Store,
        rules: &Policy,
        grant: &pb::Grant,
    ) -> Result<Option<Grant>, StoreError> {
        let (Some(number), Some(requester), Some(target)) = (
            grant_target::grant_number(grant),
            &grant.requester,
            &grant.target,
        ) else {
            return Ok(None);
        };
        let Some(capability) = grant_target::capability_of(grant.capability) else {
            return Ok(None);
        };
        let here = self.0.machine();
        let Some(record) = store.session(&requester.value)? else {
            return Ok(None);
        };
        let agent = crate::identity::resolve_fork(crate::identity::agent_caller_of(&record, here), store)?;
        let Ok(call) =
            grant_target::ask_ahead_call(store, here, &agent, capability, target.clone())
        else {
            return Ok(None);
        };
        let asker = Agent {
            session: agent.session.clone(),
            kind: agent.session_kind(),
        };
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
            granted_from: RequestId::new(
                grant.granted_from.as_ref().map_or("", |id| id.value.as_str()),
            ),
            granted_by: None,
            granted_on: MachineId::new(here),
            on_host: true,
            granted_at,
        }))
    }

    fn global_contain(&self, store: &Store) -> Result<bool, StoreError> {
        Ok(contained::global(store)?.0)
    }

    fn put_global_contain(&self, store: &Store, on: bool, revision: u64) -> Result<(), StoreError> {
        contained::put_global(store, on, revision)
    }

    fn channel_contain(&self, store: &Store, channel: &str) -> Result<Option<bool>, StoreError> {
        contained::channel(store, channel)
    }

    fn put_channel_contain(
        &self,
        store: &Store,
        channel: &str,
        on: Option<bool>,
        revision: u64,
    ) -> Result<(), StoreError> {
        contained::put_channel(store, channel, on, revision)
    }

    fn use_worktrees(&self, store: &Store) -> Result<bool, StoreError> {
        Ok(store
            .setting(humata2_sessions::USE_WORKTREES_KEY)?
            .map_or(true, |saved| saved.value != "false"))
    }

    fn put_use_worktrees(&self, store: &Store, on: bool, revision: u64) -> Result<(), StoreError> {
        store.put_setting(
            humata2_sessions::USE_WORKTREES_KEY,
            if on { "true" } else { "false" },
            revision,
        )
    }
}

// ---------------------------------------------------------------------------
// Public constructors for `serve.rs`
// ---------------------------------------------------------------------------

/// Restores the kept policy copy into `guard` on a machine daemon.
pub(crate) fn restore_policy_copy(
    store: &Arc<Store>,
    guard: &Arc<Guard>,
) -> Result<(), humata2_svc_away::CopyError> {
    let adapter: Arc<dyn PolicyGuard> = Arc::new(PolicyGuardAdapter(Arc::clone(guard)));
    humata2_svc_away::restore_policy_copy(store, &adapter)
}

/// Builds a [`PolicySource`] wired to `humatad`'s `Guard` and `Store`.
pub(crate) fn build_policy_source(guard: Arc<Guard>, store: Arc<Store>) -> Arc<PolicySource> {
    PolicySource::new(Arc::new(PolicyGuardAdapter(guard)), store)
}

/// Builds [`Uplinks`] wired to `humatad`'s `Feed`, `PolicySource`, and `Inbox`.
pub(crate) fn build_uplinks(
    link: Arc<HostLink>,
    feed: Arc<Feed>,
    policy: Arc<PolicySource>,
    inbox: Arc<Inbox>,
) -> Arc<Uplinks> {
    Arc::new(Uplinks::new(
        link,
        Arc::new(FeedAdapter(feed)),
        policy,
        Arc::new(InboxAdapter(inbox)),
    ))
}

// ---------------------------------------------------------------------------
// The machine's agents' file looks and copies, sent up to the host (D768)
// ---------------------------------------------------------------------------

/// A machine's end of the Uplink for its agents' calls naming another
/// machine's files: the host holds the links and answers as that agent.
pub(crate) struct HostAsk(pub(crate) Arc<humata2_svc_away::Forwarder>);

impl HostAsk {
    async fn send(&self, session: &str, method: &'static str, request: Vec<u8>) -> Result<Vec<u8>, Status> {
        let row = humata2_svc_away::by_name(method).ok_or_else(|| {
            crate::status::internal(
                format!("{method} has no row in the forwarded-call table, so it can't go to the host; this is a bug in Humata"),
                "",
            )
        })?;
        // While the host is away the forwarder answers host_unreachable,
        // retryable with the same key.
        self.0.call(session, row, request, None).await
    }
}

#[tonic::async_trait]
impl humata2_svc_files::ports::HostAskPort for HostAsk {
    async fn ask(&self, session: &str, method: &'static str, request: Vec<u8>) -> Result<Vec<u8>, Status> {
        self.send(session, method, request).await
    }
}

#[tonic::async_trait]
impl humata2_svc_transfers::ports::HostAskPort for HostAsk {
    async fn ask(&self, session: &str, method: &'static str, request: Vec<u8>) -> Result<Vec<u8>, Status> {
        self.send(session, method, request).await
    }
}
