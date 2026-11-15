//! The services this build doesn't serve yet. Each answers every method
//! with UNIMPLEMENTED and an Error naming the method, rather than the bare
//! status an unknown route gets. PeerService is never served on a socket
//! (GRPC.md §2), so it isn't here.

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

        /// Adds every service not built yet to `router`.
        pub(crate) fn register<L>(router: Router<L>) -> Router<L> {
            router$(.add_service($service))*
        }
    };
}

fn answer(request: &http::Request<Body>) -> http::Response<Body> {
    let method = request.uri().path().trim_start_matches('/').replace('/', ".");
    status::not_built(&method).into_http()
}

not_built! {
    AccessService => access_service_server,
    CliAuthService => cli_auth_service_server,
    AuditService => audit_service_server,
    BoardService => board_service_server,
    ChannelService => channel_service_server,
    ActivityService => activity_service_server,
    FileService => file_service_server,
    TransferService => transfer_service_server,
    TrashService => trash_service_server,
    EventService => event_service_server,
    OperationService => operation_service_server,
    MachineService => machine_service_server,
    ProviderService => provider_service_server,
    SkillsService => skills_service_server,
    BridgeService => bridge_service_server,
    SessionService => session_service_server,
    SettingsService => settings_service_server,
    AgentService => agent_service_server,
    MarkService => mark_service_server,
    ViewStateService => view_state_service_server,
    DiffService => diff_service_server,
    CommentService => comment_service_server,
    PullRequestReviewService => pull_request_review_service_server,
    ViewService => view_service_server,
}
