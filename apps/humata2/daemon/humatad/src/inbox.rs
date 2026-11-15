//! InboxService: questions an agent asks the user, and waiting for the
//! answer (MCP.md §5). Requests live in memory until the daemon stops.
//! Answering them, and access requests, aren't built yet.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use humata2_access::{Call, Capability, Target};
use humata2_grpc::humata::v1 as pb;
use pb::invalid_field::Problem;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::attach;
use crate::guard::Guard;
use crate::identity::{self, AgentCaller, Caller};
use crate::status;

/// How often a wait says it is still waiting (MCP.md §5).
const KEEPALIVE: Duration = Duration::from_secs(15);

/// The most choices a question offers.
const CHOICES_MAX: usize = 4;

pub(crate) struct Inbox {
    guard: Arc<Guard>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    requests: Vec<Entry>,
    /// The request each asking session's idempotency key made.
    keys: HashMap<String, String>,
}

struct Entry {
    id: String,
    /// The session that asked.
    session: String,
    request: watch::Sender<pb::InboxRequest>,
}

impl Inbox {
    pub(crate) fn new(guard: Arc<Guard>) -> Self {
        Self {
            guard,
            state: Mutex::new(State::default()),
        }
    }

    /// The request `id`, if `caller` may see it: the user sees every
    /// request, an agent its own session's.
    fn find(&self, caller: &Caller, id: &str) -> Result<watch::Sender<pb::InboxRequest>, Status> {
        lock(&self.state)
            .requests
            .iter()
            .find(|entry| entry.id == id && sees(caller, entry))
            .map(|entry| entry.request.clone())
            .ok_or_else(|| no_request(id))
    }
}

fn sees(caller: &Caller, entry: &Entry) -> bool {
    match caller {
        Caller::User => true,
        Caller::Agent(agent) => agent.session.id.as_str() == entry.session,
        // The Inbox isn't on the peer link; relayed answers come over Uplink.
        Caller::Peer => false,
    }
}

#[tonic::async_trait]
impl pb::inbox_service_server::InboxService for Inbox {
    async fn list_requests(
        &self,
        request: Request<pb::ListRequestsRequest>,
    ) -> Result<Response<pb::ListRequestsResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let scope = request.scope.and_then(|scope| scope.scope);
        let state = lock(&self.state);
        let mut requests = Vec::new();
        let mut pending_count = 0;
        for entry in state.requests.iter().filter(|entry| sees(&caller, entry)) {
            let current = entry.request.borrow().clone();
            let in_scope = match &scope {
                None | Some(pb::inbox_scope::Scope::Global(_)) => true,
                Some(pb::inbox_scope::Scope::Channel(channel)) => {
                    current.channel.as_ref() == Some(channel)
                }
                Some(pb::inbox_scope::Scope::Session(inbox)) => inbox
                    .session
                    .as_ref()
                    .is_some_and(|session| session.value == entry.session),
            };
            if !in_scope {
                continue;
            }
            let pending = current.state == pb::RequestState::Pending as i32;
            if pending {
                pending_count += 1;
            }
            if pending || request.include_answered {
                requests.push(current);
            }
        }
        Ok(Response::new(pb::ListRequestsResponse {
            requests,
            pending_count,
            page: None,
        }))
    }

    async fn get_request(
        &self,
        request: Request<pb::GetRequestRequest>,
    ) -> Result<Response<pb::InboxRequest>, Status> {
        let caller = identity::caller(&request)?;
        let id = id_of(request.into_inner().id.map(|id| id.value)).ok_or_else(|| status::missing("id"))?;
        let found = self.find(&caller, &id)?;
        let current = found.borrow().clone();
        Ok(Response::new(current))
    }

    async fn answer_request(
        &self,
        request: Request<pb::AnswerRequestRequest>,
    ) -> Result<Response<pb::InboxRequest>, Status> {
        self.guard.require(&identity::caller(&request)?, &answer())?;
        Err(status::not_built("humata.v1.InboxService.AnswerRequest"))
    }

    async fn answer_all(
        &self,
        request: Request<pb::AnswerAllRequest>,
    ) -> Result<Response<pb::AnswerAllResponse>, Status> {
        self.guard.require(&identity::caller(&request)?, &answer())?;
        Err(status::not_built("humata.v1.InboxService.AnswerAll"))
    }

    /// Puts an agent's question in the Inbox and answers pending at once;
    /// the agent then waits with WaitForAnswer.
    async fn ask_user(
        &self,
        request: Request<pb::AskUserRequest>,
    ) -> Result<Response<pb::WaitResponse>, Status> {
        let caller = identity::caller(&request)?;
        let Caller::Agent(agent) = &caller else {
            return Err(status::invalid_field(
                "question",
                Problem::NotAllowedHere,
                "only a session's agent asks the user a question; this connection is the user",
                "ask from a Humata session tab, where HUMATA_SESSION_TOKEN is set",
            ));
        };
        // An Ask verdict is fine too: the question is the request.
        self.guard.check(
            &caller,
            &Call::Use {
                capability: Capability::InboxAsk,
                target: Target::Channel(agent.session.channel.clone()),
            },
        )?;
        let request = request.into_inner();
        if request.question.trim().is_empty() {
            return Err(status::missing("question"));
        }
        if request.choices.len() > CHOICES_MAX {
            return Err(status::invalid_field(
                "choices",
                Problem::OutOfRange,
                format!(
                    "the question offers {} choices, more than the {CHOICES_MAX} the Inbox shows",
                    request.choices.len()
                ),
                format!("offer at most {CHOICES_MAX} choices, or none for a free reply"),
            ));
        }
        let session = agent.session.id.to_string();
        let key = id_of(request.idempotency_key.map(|key| key.value))
            .map(|key| format!("{session}/{key}"));
        let mut state = lock(&self.state);
        if let Some(id) = key.as_ref().and_then(|key| state.keys.get(key)) {
            let made = state.requests.iter().find(|entry| &entry.id == id);
            if let Some(entry) = made {
                return Ok(Response::new(settled(&entry.request.borrow())));
            }
        }
        let id = format!("rq_{}", uuid::Uuid::new_v4().simple());
        let made = question(&id, agent, request.question, request.choices);
        let reply = settled(&made);
        state.requests.push(Entry {
            id: id.clone(),
            session,
            request: watch::channel(made).0,
        });
        if let Some(key) = key {
            state.keys.insert(key, id);
        }
        Ok(Response::new(reply))
    }

    async fn request_access(
        &self,
        request: Request<pb::RequestAccessRequest>,
    ) -> Result<Response<pb::WaitResponse>, Status> {
        identity::caller(&request)?;
        Err(status::not_built("humata.v1.InboxService.RequestAccess"))
    }

    type WaitForAnswerStream = ReceiverStream<Result<pb::WaitResponse, Status>>;

    /// Sends a keepalive at once and then about every 15 seconds, and ends
    /// with the request's state once it's answered or the wait is over.
    async fn wait_for_answer(
        &self,
        request: Request<pb::WaitForAnswerRequest>,
    ) -> Result<Response<Self::WaitForAnswerStream>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let id = id_of(request.id.map(|id| id.value)).ok_or_else(|| status::missing("id"))?;
        let mut answers = self.find(&caller, &id)?.subscribe();
        let wait = Duration::from_secs(u64::from(request.wait_seconds));
        let (sender, receiver) = mpsc::channel(4);
        tokio::spawn(async move {
            let deadline = tokio::time::sleep(wait);
            tokio::pin!(deadline);
            let mut keepalive = tokio::time::interval(KEEPALIVE);
            loop {
                let current = answers.borrow_and_update().clone();
                if current.state != pb::RequestState::Pending as i32 {
                    let _ = sender.send(Ok(settled(&current))).await;
                    return;
                }
                tokio::select! {
                    biased;
                    () = sender.closed() => return,
                    () = &mut deadline => {
                        let _ = sender.send(Ok(settled(&current))).await;
                        return;
                    }
                    changed = answers.changed() => {
                        if changed.is_err() {
                            let _ = sender.send(Ok(settled(&current))).await;
                            return;
                        }
                    }
                    _ = keepalive.tick() => {
                        let still = pb::WaitResponse {
                            id: current.id.clone(),
                            state: current.state,
                            keepalive: true,
                            ..Default::default()
                        };
                        if sender.send(Ok(still)).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(Response::new(ReceiverStream::new(receiver)))
    }

    /// The asking agent stopped waiting (TP-15). Withdrawing a request
    /// that's no longer pending changes nothing.
    async fn withdraw_request(
        &self,
        request: Request<pb::WithdrawRequestRequest>,
    ) -> Result<Response<pb::InboxRequest>, Status> {
        let caller = identity::caller(&request)?;
        if let Caller::User = caller {
            return Err(status::invalid_field(
                "id",
                Problem::NotAllowedHere,
                "only the agent that made a request withdraws it",
                "answer or deny the request in the Inbox instead",
            ));
        }
        let id = id_of(request.into_inner().id.map(|id| id.value)).ok_or_else(|| status::missing("id"))?;
        let found = self.find(&caller, &id)?;
        found.send_modify(|current| {
            if current.state == pb::RequestState::Pending as i32 {
                current.state = pb::RequestState::Withdrawn as i32;
                // The RPC carries no reason; the MCP server calls it only
                // when the agent's call ended.
                current.withdraw_reason = pb::WithdrawReason::CallEnded as i32;
            }
        });
        let current = found.borrow().clone();
        Ok(Response::new(current))
    }
}

fn answer() -> Call {
    Call::Use {
        capability: Capability::InboxAnswer,
        target: Target::Global,
    }
}

fn question(id: &str, agent: &AgentCaller, question: String, choices: Vec<String>) -> pb::InboxRequest {
    pb::InboxRequest {
        id: Some(pb::RequestId {
            value: id.to_owned(),
        }),
        channel: Some(pb::ChannelId {
            value: agent.session.channel.to_string(),
        }),
        kind: pb::RequestKind::Question as i32,
        state: pb::RequestState::Pending as i32,
        requester: Some(agent.actor()),
        action_title: format!("{} asks you a question", agent.title),
        preview: Some(pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::Question(pb::QuestionPreview {
                question,
                choices,
            })),
        }),
        created_at: Some(attach::now()),
        ..Default::default()
    }
}

/// What a wait ends with for `request` as it stands.
fn settled(request: &pb::InboxRequest) -> pb::WaitResponse {
    let id = request.id.clone();
    let value = id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
    let error = match pb::RequestState::try_from(request.state) {
        Ok(pb::RequestState::Withdrawn) => Some(pb::Error {
            message: format!("request {value} was withdrawn"),
            fix: "ask again if it still matters".to_owned(),
            retry: None,
            kind: Some(pb::error::Kind::RequestWithdrawn(pb::RequestWithdrawn {
                request: id.clone(),
            })),
        }),
        Ok(pb::RequestState::Denied) => Some(pb::Error {
            message: format!("the user denied request {value}"),
            fix: "don't ask again for the same thing; ask the user what they want instead".to_owned(),
            retry: None,
            kind: Some(pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::UserDenied as i32,
                capability: request.capability,
                request: id.clone(),
                user_note: match &request.answer {
                    Some(pb::RequestAnswer {
                        answer: Some(pb::request_answer::Answer::Deny(deny)),
                    }) => deny.note.clone(),
                    _ => String::new(),
                },
            })),
        }),
        _ => None,
    };
    pb::WaitResponse {
        id,
        state: request.state,
        answer: request.answer.clone(),
        error,
        ..Default::default()
    }
}

fn no_request(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Request,
        id,
        format!("no Inbox request {id} is yours on this machine"),
        "check the request id; requests are kept only until humatad2 stops",
    )
}

fn id_of(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
