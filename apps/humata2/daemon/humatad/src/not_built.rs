//! The services this build doesn't serve yet. Each answers every method
//! with UNIMPLEMENTED and an Error naming the method, rather than the bare
//! status an unknown route gets. PeerService is never on a socket
//! (GRPC.md §2); the services the peer link relays are `relay`'s (D139).

use std::convert::Infallible;
use std::future::{Ready, ready};
use std::task::{Context, Poll};

use humata2_grpc::humata::v1 as pb;
use tonic::body::Body;
use tonic::codegen::{Service, http};
use tonic::server::NamedService;
use tonic::transport::server::Router;

use crate::status;

macro_rules! not_built {
    ($($service:ident => $module:ident),* $(,)?) => {
        $(
            #[derive(Clone, Copy)]
            struct $service;

            impl NamedService for $service {
                const NAME: &'static str = pb::$module::SERVICE_NAME;
            }

            impl Service<http::Request<Body>> for $service {
                type Response = http::Response<Body>;
                type Error = Infallible;
                type Future = Ready<Result<Self::Response, Infallible>>;

                fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
                    Poll::Ready(Ok(()))
                }

                fn call(&mut self, request: http::Request<Body>) -> Self::Future {
                    ready(Ok(answer(&request)))
                }
            }
        )*
    };
}

fn answer(request: &http::Request<Body>) -> http::Response<Body> {
    let method = request
        .uri()
        .path()
        .trim_start_matches('/')
        .replace('/', ".");
    status::not_built(&method).into_http()
}

not_built! {
    AuditService => audit_service_server,
    ActivityService => activity_service_server,
    MachineService => machine_service_server,
    ProviderService => provider_service_server,
    SkillsService => skills_service_server,
    BridgeService => bridge_service_server,
    AgentService => agent_service_server,
    ViewStateService => view_state_service_server,
    PullRequestReviewService => pull_request_review_service_server,
}

/// Adds to the socket's `router` every client service not built yet.
pub(crate) fn register<L>(router: Router<L>) -> Router<L> {
    router
        .add_service(AuditService)
        .add_service(ActivityService)
        .add_service(BridgeService)
        .add_service(AgentService)
        .add_service(ViewStateService)
        .add_service(PullRequestReviewService)
}

/// Adds MachineService and ProviderService to a machine's `router`: only
/// the host keeps machines and their providers, and the machine's daemon
/// doesn't forward to them yet.
pub(crate) fn register_host_only<L>(router: Router<L>) -> Router<L> {
    router
        .add_service(MachineService)
        .add_service(ProviderService)
        .add_service(SkillsService)
}
