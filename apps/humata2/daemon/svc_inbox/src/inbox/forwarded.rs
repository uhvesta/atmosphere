//! An agent on a machine waits on, or withdraws, an Inbox request the host
//! holds (AWAY, D268).
//!
//! An agent's board write that the host's rules ask about is made on the
//! host, so its request is in the host's Inbox, which the machine doesn't
//! hold. The agent still asks its own machine, as it always does. When the
//! machine has no such request and the host is linked, it sends the call up
//! as a `ForwardedCall` and the host runs it as that agent, so the host's
//! check that the request is the agent's own session's applies. A wait is
//! forwarded as one call that ends with the final answer; the machine sends
//! the agent its own keepalives meanwhile. If the agent goes away, the
//! forwarded call is cancelled.

use std::sync::Arc;

use humata2_daemon_ports::Caller;
use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::helpers as status;
use humata2_svc_away::{Forwarder, Method};
use prost::Message;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Status};

use super::{Inbox, KEEPALIVE};

/// Why a wait or withdrawal on a request the Mac holds didn't get its answer.
#[derive(Debug, thiserror::Error)]
enum HostHeld {
    #[error(
        "the link to the Mac dropped while {doing} request {id}, which the Mac holds; the request is still there, and if the user approves it there the change is still made"
    )]
    LinkLost {
        id: String,
        doing: &'static str,
        #[source]
        source: Status,
    },

    #[error("request {id} is held by the Mac, which this machine can't reach right now")]
    Away { id: String },

    #[error(
        "the Mac's answer about request {id} couldn't be read; update Humata on this machine so both ends speak the same version"
    )]
    Unreadable {
        id: String,
        #[source]
        source: prost::DecodeError,
    },

    #[error("the Mac ended its wait on request {id} without an answer; wait on it again")]
    NoAnswer { id: String },
}

impl From<HostHeld> for Status {
    fn from(error: HostHeld) -> Self {
        match error {
            HostHeld::LinkLost { .. } | HostHeld::Away { .. } => status::failure(
                Code::Unavailable,
                pb::Error {
                    message: status::chain(&error),
                    fix: "call inbox_wait again with the same request id once the Mac is linked; approving the request on the Mac makes the change whether or not this call waits".to_owned(),
                    retry: Some(pb::RetryAdvice {
                        after: None,
                        same_key: true,
                    }),
                    kind: Some(pb::error::Kind::HostUnreachable(pb::HostUnreachable { since: None })),
                },
            ),
            HostHeld::Unreadable { .. } | HostHeld::NoAnswer { .. } => status::internal(status::chain(&error), ""),
        }
    }
}

/// Where a call about a request this Inbox doesn't hold goes.
enum Route<'a> {
    /// The host is linked: the call goes up.
    Up { forwarder: &'a Arc<Forwarder>, session: String },
    /// The host holds the request and isn't linked.
    Away,
    /// Not a request of the host's, as far as this machine knows.
    Nowhere,
}

impl Inbox {
    /// Joins this machine's Inbox to the host's link, so that calls about a
    /// request the host holds can go up it.
    pub fn forwarding(&self, forwarder: Arc<Forwarder>) {
        let _ = self.forwarder.set(forwarder);
    }

    fn route(&self, caller: &Caller, id: &str) -> Route<'_> {
        let (Caller::Agent(agent), Some(forwarder)) = (caller, self.forwarder.get()) else {
            return Route::Nowhere;
        };
        if forwarder.forwards() {
            Route::Up {
                forwarder,
                session: agent.session.id.to_string(),
            }
        } else if forwarder.host_holds(id) {
            Route::Away
        } else {
            Route::Nowhere
        }
    }

    /// An agent waits on request `id`, which this Inbox doesn't hold
    /// (`missing` is why). On a machine whose host is linked the wait goes
    /// up to the host; away, a request the host is known to hold can't be
    /// waited on; any other is `missing`.
    pub(super) fn wait_on_host(
        &self,
        caller: &Caller,
        request: pb::WaitForAnswerRequest,
        id: &str,
        missing: Status,
    ) -> Result<ReceiverStream<Result<pb::WaitResponse, Status>>, Status> {
        let (forwarder, session) = match self.route(caller, id) {
            Route::Up { forwarder, session } => (Arc::clone(forwarder), session),
            Route::Away => return Err(HostHeld::Away { id: id.to_owned() }.into()),
            Route::Nowhere => return Err(missing),
        };
        let id = id.to_owned();
        let still = pb::WaitResponse {
            id: request.id.clone(),
            state: pb::RequestState::Pending as i32,
            keepalive: true,
            ..Default::default()
        };
        let (sender, receiver) = mpsc::channel(4);
        tokio::spawn(async move {
            let call = forwarder.call(&session, Method::WaitForAnswer.row(), request.encode_to_vec(), None);
            tokio::pin!(call);
            let mut keepalive = tokio::time::interval(KEEPALIVE);
            loop {
                tokio::select! {
                    biased;
                    // The agent stopped waiting: dropping the call tells the host to drop it.
                    () = sender.closed() => return,
                    done = &mut call => {
                        let answer = done
                            .map_err(|failed| lost(&forwarder, &id, "waiting on", failed))
                            .and_then(|bytes| {
                                pb::WaitResponse::decode(bytes.as_slice())
                                    .map_err(|source| HostHeld::Unreadable { id: id.clone(), source }.into())
                            });
                        let _ = sender.send(answer).await;
                        return;
                    }
                    _ = keepalive.tick() => {
                        if sender.send(Ok(still.clone())).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(ReceiverStream::new(receiver))
    }

    /// An agent withdraws request `id`, which this Inbox doesn't hold
    /// (`missing` is why); as for [`Inbox::wait_on_host`].
    pub(super) async fn withdraw_on_host(
        &self,
        caller: &Caller,
        request: pb::WithdrawRequestRequest,
        id: &str,
        missing: Status,
    ) -> Result<pb::InboxRequest, Status> {
        let (forwarder, session) = match self.route(caller, id) {
            Route::Up { forwarder, session } => (Arc::clone(forwarder), session),
            Route::Away => return Err(HostHeld::Away { id: id.to_owned() }.into()),
            Route::Nowhere => return Err(missing),
        };
        let key = request.idempotency_key.clone();
        let bytes = forwarder
            .call(&session, Method::WithdrawRequest.row(), request.encode_to_vec(), key)
            .await
            .map_err(|failed| lost(&forwarder, id, "withdrawing", failed))?;
        pb::InboxRequest::decode(bytes.as_slice()).map_err(|source| {
            HostHeld::Unreadable {
                id: id.to_owned(),
                source,
            }
            .into()
        })
    }

    /// The host's side of a forwarded wait: `request` waited on as `caller`,
    /// the forwarding agent, so only its own session's requests are found.
    /// Skips the keepalives and returns the final answer.
    pub async fn wait_final(
        &self,
        caller: Caller,
        request: pb::WaitForAnswerRequest,
    ) -> Result<pb::WaitResponse, Status> {
        use pb::inbox_service_server::InboxService;
        let id = request.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let mut asked = Request::new(request);
        asked.extensions_mut().insert(caller);
        let mut stream = self.wait_for_answer(asked).await?.into_inner();
        while let Some(next) = stream.next().await {
            let response = next?;
            if !response.keepalive {
                return Ok(response);
            }
        }
        Err(HostHeld::NoAnswer { id }.into())
    }

    /// A user on a machine asks why on request `id`, which this Inbox doesn't
    /// hold (`missing` is why). While the host is linked the call goes up as a
    /// trusted peer user; away, a request the host is known to hold is
    /// host_unreachable; any other is `missing`.
    pub(super) async fn ask_why_on_host(
        &self,
        caller: &Caller,
        request: pb::AskWhyRequest,
        id: &str,
        missing: Status,
    ) -> Result<pb::InboxRequest, Status> {
        let (Caller::User(_), Some(forwarder)) = (caller, self.forwarder.get()) else {
            return Err(missing);
        };
        if !forwarder.forwards() {
            return if forwarder.host_holds(id) {
                Err(HostHeld::Away { id: id.to_owned() }.into())
            } else {
                Err(missing)
            };
        }
        let key = request.idempotency_key.clone();
        let bytes = forwarder
            .call_as_user(Method::AskWhy.row(), request.encode_to_vec(), key)
            .await
            .map_err(|failed| lost(forwarder, id, "asking why on", failed))?;
        pb::InboxRequest::decode(bytes.as_slice()).map_err(|source| {
            HostHeld::Unreadable {
                id: id.to_owned(),
                source,
            }
            .into()
        })
    }

    /// An agent explains request `id`, which this Inbox doesn't hold
    /// (`missing` is why); as for [`Inbox::withdraw_on_host`].
    pub(super) async fn explain_on_host(
        &self,
        caller: &Caller,
        request: pb::ExplainAskRequest,
        id: &str,
        missing: Status,
    ) -> Result<pb::InboxRequest, Status> {
        let (forwarder, session) = match self.route(caller, id) {
            Route::Up { forwarder, session } => (Arc::clone(forwarder), session),
            Route::Away => return Err(HostHeld::Away { id: id.to_owned() }.into()),
            Route::Nowhere => return Err(missing),
        };
        let bytes = forwarder
            .call(&session, Method::ExplainAsk.row(), request.encode_to_vec(), None)
            .await
            .map_err(|failed| lost(&forwarder, id, "explaining", failed))?;
        pb::InboxRequest::decode(bytes.as_slice()).map_err(|source| {
            HostHeld::Unreadable {
                id: id.to_owned(),
                source,
            }
            .into()
        })
    }

    /// The host's side of a forwarded withdrawal: `request` as `caller`.
    pub async fn withdraw_as(
        &self,
        caller: Caller,
        request: pb::WithdrawRequestRequest,
    ) -> Result<pb::InboxRequest, Status> {
        use pb::inbox_service_server::InboxService;
        let mut asked = Request::new(request);
        asked.extensions_mut().insert(caller);
        self.withdraw_request(asked).await.map(tonic::Response::into_inner)
    }

    /// The host's side of a forwarded Ask why: `request` as `caller`.
    pub async fn ask_why_as(
        &self,
        caller: Caller,
        request: pb::AskWhyRequest,
    ) -> Result<pb::InboxRequest, Status> {
        use pb::inbox_service_server::InboxService;
        let mut asked = Request::new(request);
        asked.extensions_mut().insert(caller);
        self.ask_why(asked).await.map(tonic::Response::into_inner)
    }

    /// The host's side of a forwarded ExplainAsk: `request` as `caller`.
    pub async fn explain_as(
        &self,
        caller: Caller,
        request: pb::ExplainAskRequest,
    ) -> Result<pb::InboxRequest, Status> {
        use pb::inbox_service_server::InboxService;
        let mut asked = Request::new(request);
        asked.extensions_mut().insert(caller);
        self.explain_ask(asked).await.map(tonic::Response::into_inner)
    }
}

/// What a forwarded call's failure is to the agent: the Mac's own error as
/// it came, and a link that didn't reach it as `host_unreachable`, noting
/// the request is the host's for the time the host is away.
fn lost(forwarder: &Forwarder, id: &str, doing: &'static str, failed: Status) -> Status {
    let unreached = matches!(
        status::status_error_of(&failed).and_then(|error| error.kind),
        Some(pb::error::Kind::HostUnreachable(_))
    );
    if !unreached {
        return failed;
    }
    forwarder.remember_held(id);
    HostHeld::LinkLost {
        id: id.to_owned(),
        doing,
        source: failed,
    }
    .into()
}
