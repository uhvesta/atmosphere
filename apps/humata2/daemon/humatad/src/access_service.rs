//! AccessService: the grants "Always let …" and approved ask-aheads made,
//! listed and revoked (IN-19, IN-23). Grants live in the policy, in memory
//! until the daemon stops. A revoked grant goes on the event feed; the
//! Inbox tells of the ones answers make. Rules, defaults and Explain
//! aren't built yet.

use std::sync::Arc;

use humata2_access::{Call, Capability, GrantId, PolicyError, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use tonic::{Request, Response, Status};

use crate::events::{Feed, Scope};
use crate::grant_target;
use crate::guard::{self, Guard};
use crate::identity::{self, Caller};
use crate::status;

pub(crate) struct Access {
    guard: Arc<Guard>,
    /// Where revoked grants are told.
    feed: Arc<Feed>,
}

impl Access {
    pub(crate) fn new(guard: Arc<Guard>, feed: Arc<Feed>) -> Self {
        Self { guard, feed }
    }
}

#[tonic::async_trait]
impl pb::access_service_server::AccessService for Access {
    async fn list_capabilities(
        &self,
        _request: Request<pb::ListCapabilitiesRequest>,
    ) -> Result<Response<pb::ListCapabilitiesResponse>, Status> {
        Err(status::not_built("humata.v1.AccessService.ListCapabilities"))
    }

    async fn get_channel_rules(
        &self,
        _request: Request<pb::GetChannelRulesRequest>,
    ) -> Result<Response<pb::ChannelRules>, Status> {
        Err(status::not_built("humata.v1.AccessService.GetChannelRules"))
    }

    async fn set_channel_rule(
        &self,
        _request: Request<pb::SetChannelRuleRequest>,
    ) -> Result<Response<pb::ChannelRules>, Status> {
        Err(status::not_built("humata.v1.AccessService.SetChannelRule"))
    }

    async fn set_away_rule(
        &self,
        _request: Request<pb::SetAwayRuleRequest>,
    ) -> Result<Response<pb::ChannelRules>, Status> {
        Err(status::not_built("humata.v1.AccessService.SetAwayRule"))
    }

    async fn get_default_rules(
        &self,
        _request: Request<pb::GetDefaultRulesRequest>,
    ) -> Result<Response<pb::DefaultRules>, Status> {
        Err(status::not_built("humata.v1.AccessService.GetDefaultRules"))
    }

    async fn set_default_rules(
        &self,
        _request: Request<pb::SetDefaultRulesRequest>,
    ) -> Result<Response<pb::DefaultRules>, Status> {
        Err(status::not_built("humata.v1.AccessService.SetDefaultRules"))
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
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
        };
        let request = request.into_inner();
        let channel = request.channel.map(|channel| channel.value).filter(|value| !value.is_empty());
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
                .filter(|grant| channel.as_deref().is_none_or(|channel| grant.channel.as_str() == channel))
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
            let revoked = policy.revoke_grant(&user, id).map_err(|error| match error {
                PolicyError::UnknownGrant { .. } => no_grant(&value),
                PolicyError::NeedsHost => status::refused(&Refusal::HostUnreachable),
                other => status::internal(
                    format!("couldn't revoke grant {value}: {}", status::chain(&other)),
                    "",
                ),
            })?;
            Ok(grant_target::grant(&revoked, policy.is_suspended(&revoked)))
        })?;
        // The grant as it last stood, so the app can drop it.
        let scope = Scope {
            channel: revoked.channel.as_ref().map(|channel| channel.value.clone()).unwrap_or_default(),
            session: revoked.requester.as_ref().map(|session| session.value.clone()).unwrap_or_default(),
        };
        self.feed
            .publish_in(pb::Change::Deleted, pb::event::Payload::Grant(revoked), scope);
        Ok(Response::new(pb::RevokeGrantResponse {}))
    }

    async fn explain(
        &self,
        _request: Request<pb::ExplainRequest>,
    ) -> Result<Response<pb::ExplainResponse>, Status> {
        Err(status::not_built("humata.v1.AccessService.Explain"))
    }

    async fn set_approval_passphrase(
        &self,
        _request: Request<pb::SetApprovalPassphraseRequest>,
    ) -> Result<Response<pb::SetApprovalPassphraseResponse>, Status> {
        Err(status::not_built("humata.v1.AccessService.SetApprovalPassphrase"))
    }
}

fn no_grant(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Grant,
        id,
        format!("no grant {id} on this machine; it may have been revoked or ended with its session"),
        "list the grants again and pick one from the list; grants are kept only until humatad2 stops",
    )
}
