//! The Inbox and grants against the real daemon: an agent asks, only the
//! user answers, an approved ask-ahead makes a grant that covers exactly
//! what was asked, and a revoked grant asks again.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{Attach, Daemon, WAIT, error_of, folder, invalid_field, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use prost::Message;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

const CHANNEL: &str = "ch_inbox";
/// The session whose tab's agent asks.
const ASKER: &str = "se_asker";
/// Another session in the same channel, whose files the asker wants.
const OTHER: &str = "se_other";
const TOKEN_HEADER: &str = "humata-session-token";

/// A daemon that knows `ASKER` and `OTHER`, with a shell tab open in
/// `ASKER`, and the token that tab's shell was given.
struct Fixture {
    daemon: Daemon,
    token: String,
    _attach: Attach,
}

impl Fixture {
    async fn start() -> Self {
        let dir = folder();
        let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
        for (id, title) in [(ASKER, "asker"), (OTHER, "other")] {
            store
                .put_session(&SessionRecord {
                    id: id.to_owned(),
                    channel: CHANNEL.to_owned(),
                    machine: String::new(),
                    title: title.to_owned(),
                    agent: "zsh".to_owned(),
                    folder: dir.path().to_owned(),
                    created_unix_ms: 0,
                })
                .unwrap();
        }
        drop(store);
        let daemon = Daemon::start_in(dir).await;

        let tab = daemon
            .terminals()
            .await
            .open_shell_tab(pb::OpenShellTabRequest {
                idempotency_key: None,
                session: Some(session(ASKER)),
                repo: None,
            })
            .await
            .unwrap()
            .into_inner()
            .id
            .unwrap();
        let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
        let file = daemon.dir.path().join("token");
        attach
            .type_text(&format!(
                "printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r",
                file.display()
            ))
            .await;
        let token = read_when_written(&file).await;

        Self {
            daemon,
            token,
            _attach: attach,
        }
    }

    async fn inbox(&self) -> pb::inbox_service_client::InboxServiceClient<Channel> {
        pb::inbox_service_client::InboxServiceClient::new(self.daemon.channel().await)
    }

    async fn access(&self) -> pb::access_service_client::AccessServiceClient<Channel> {
        pb::access_service_client::AccessServiceClient::new(self.daemon.channel().await)
    }

    /// The agent asks the user `question`; returns the request's id.
    async fn ask(&self, question: &str) -> pb::RequestId {
        let asked = self
            .inbox()
            .await
            .ask_user(self.as_agent(pb::AskUserRequest {
                question: question.to_owned(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(asked.state(), pb::RequestState::Pending);
        asked.id.unwrap()
    }

    /// The agent asks ahead for `capability` on `target`.
    async fn ask_ahead(&self, capability: pb::Capability, target: pb::GrantTarget) -> pb::WaitResponse {
        self.inbox()
            .await
            .request_access(self.as_agent(pb::RequestAccessRequest {
                capability: capability as i32,
                target: Some(target),
                reason: "to run the tests".to_owned(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner()
    }

    /// The user answers request `id` with `answer`.
    async fn answer(
        &self,
        id: &pb::RequestId,
        answer: pb::request_answer::Answer,
    ) -> Result<pb::InboxRequest, tonic::Status> {
        self.inbox()
            .await
            .answer_request(answer_of(id, answer))
            .await
            .map(tonic::Response::into_inner)
    }

    /// What the agent's wait on request `id` ends with.
    async fn wait(&self, id: &pb::RequestId) -> pb::WaitResponse {
        let mut stream = self
            .inbox()
            .await
            .wait_for_answer(self.as_agent(pb::WaitForAnswerRequest {
                id: Some(id.clone()),
                wait_seconds: 10,
            }))
            .await
            .unwrap()
            .into_inner();
        loop {
            let message = tokio::time::timeout(WAIT, stream.message())
                .await
                .expect("the wait ended in time")
                .unwrap()
                .expect("the wait ended with an answer");
            if !message.keepalive {
                return message;
            }
        }
    }

    /// The request `id` as the user sees it.
    async fn request(&self, id: &pb::RequestId) -> pb::InboxRequest {
        self.inbox()
            .await
            .get_request(pb::GetRequestRequest { id: Some(id.clone()) })
            .await
            .unwrap()
            .into_inner()
    }

    /// Every grant, as the user sees them.
    async fn grants(&self) -> Vec<pb::Grant> {
        self.access()
            .await
            .list_grants(pb::ListGrantsRequest::default())
            .await
            .unwrap()
            .into_inner()
            .grants
    }

    fn as_agent<T>(&self, message: T) -> Request<T> {
        let mut request = Request::new(message);
        request
            .metadata_mut()
            .insert(TOKEN_HEADER, MetadataValue::try_from(self.token.as_str()).unwrap());
        request
    }
}

fn session(id: &str) -> pb::SessionId {
    pb::SessionId { value: id.to_owned() }
}

fn answer_of(id: &pb::RequestId, answer: pb::request_answer::Answer) -> pb::AnswerRequestRequest {
    pb::AnswerRequestRequest {
        id: Some(id.clone()),
        answer: Some(pb::RequestAnswer { answer: Some(answer) }),
        ..Default::default()
    }
}

fn approve() -> pb::request_answer::Answer {
    pb::request_answer::Answer::Approve(pb::Approve {})
}

fn deny() -> pb::request_answer::Answer {
    pb::request_answer::Answer::Deny(pb::Deny::default())
}

fn reply(text: &str) -> pb::request_answer::Answer {
    pb::request_answer::Answer::Reply(pb::Reply { text: text.to_owned() })
}

fn network(host: &str, port: u32) -> pb::GrantTarget {
    pb::GrantTarget {
        target: Some(pb::grant_target::Target::Network(pb::NetworkTarget {
            host: host.to_owned(),
            port,
            machine: None,
        })),
    }
}

/// `path` in the acme/ledger repo of `OTHER`'s clone.
fn other_files(path: &str) -> pb::GrantTarget {
    pb::GrantTarget {
        target: Some(pb::grant_target::Target::Path(pb::PathScope {
            machine: None,
            session: Some(session(OTHER)),
            prefix: Some(pb::RepoPath {
                repo: Some(pb::RepoRef {
                    owner: "acme".to_owned(),
                    name: "ledger".to_owned(),
                }),
                path: path.to_owned(),
            }),
        })),
    }
}

/// The grant a wait's result carries.
fn grant_in(waited: &pb::WaitResponse) -> pb::Grant {
    assert_eq!(waited.result_type, "humata.v1.Grant");
    pb::Grant::decode(waited.result.as_slice()).unwrap()
}

/// `path`'s contents once something non-empty is written there.
async fn read_when_written(path: &Path) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        match std::fs::read_to_string(path) {
            Ok(text) if !text.is_empty() => return text,
            _ => {}
        }
        assert!(Instant::now() < deadline, "{} was never written", path.display());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_users_reply_ends_the_agents_wait() {
    let fixture = Fixture::start().await;
    let id = fixture.ask("Ship it?").await;

    let answered = fixture.answer(&id, reply("yes")).await.unwrap();
    let waited = fixture.wait(&id).await;

    assert_eq!(answered.state(), pb::RequestState::Answered);
    assert_eq!(waited.state(), pb::RequestState::Answered);
    assert_eq!(
        waited.answer.and_then(|answer| answer.answer),
        Some(reply("yes"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_answer_its_own_request() {
    let fixture = Fixture::start().await;
    let id = fixture.ask("May I?").await;

    let refused = fixture
        .inbox()
        .await
        .answer_request(fixture.as_agent(answer_of(&id, reply("yes"))))
        .await
        .unwrap_err();

    assert!(matches!(
        error_of(&refused).kind,
        Some(pb::error::Kind::UserOnly(_))
    ));
    assert_eq!(fixture.request(&id).await.state(), pb::RequestState::Pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn answer_all_answers_only_the_listed_pending_requests() {
    let fixture = Fixture::start().await;
    let listed = fixture.ask("First?").await;
    let unlisted = fixture.ask("Second?").await;
    let unknown = pb::RequestId {
        value: "rq_nobody".to_owned(),
    };

    let response = fixture
        .inbox()
        .await
        .answer_all(pb::AnswerAllRequest {
            ids: vec![listed.clone(), unknown.clone()],
            approve: false,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    let answered: Vec<_> = response.answered.iter().filter_map(|request| request.id.clone()).collect();
    assert_eq!(answered, vec![listed.clone()]);
    assert_eq!(response.skipped, vec![unknown]);
    assert_eq!(fixture.request(&listed).await.state(), pb::RequestState::Denied);
    assert_eq!(fixture.request(&unlisted).await.state(), pb::RequestState::Pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_all_never_approves_an_ask_ahead() {
    let fixture = Fixture::start().await;
    let ahead = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("localhost", 5432))
        .await
        .id
        .unwrap();

    let response = fixture
        .inbox()
        .await
        .answer_all(pb::AnswerAllRequest {
            ids: vec![ahead.clone()],
            approve: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    assert!(response.answered.is_empty());
    assert_eq!(response.skipped, vec![ahead.clone()]);
    assert_eq!(fixture.request(&ahead).await.state(), pb::RequestState::Pending);
    assert!(fixture.grants().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approved_ask_ahead_covers_the_same_ask_and_nothing_wider() {
    let fixture = Fixture::start().await;
    let first = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("localhost", 5432))
        .await;
    let id = first.id.clone().unwrap();

    fixture.answer(&id, approve()).await.unwrap();
    let waited = fixture.wait(&id).await;
    let again = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("localhost", 5432))
        .await;
    let other_port = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("localhost", 5433))
        .await;
    let other_host = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("example.com", 5432))
        .await;

    assert_eq!(first.state(), pb::RequestState::Pending);
    assert_eq!(waited.state(), pb::RequestState::Approved);
    let grant = grant_in(&waited);
    assert_eq!(grant.capability(), pb::Capability::NetworkReach);
    assert_eq!(grant.requester, Some(session(ASKER)));
    assert_eq!(grant.granted_from, Some(id));
    assert!(grant.granted_at.is_some());
    assert!(matches!(
        grant.target.and_then(|target| target.target),
        Some(pb::grant_target::Target::Network(pb::NetworkTarget { port: 5432, .. }))
    ));
    // Covered: approved at once, with no new request for the user.
    assert_eq!(again.state(), pb::RequestState::Approved);
    assert!(again.id.is_none());
    assert_eq!(other_port.state(), pb::RequestState::Pending);
    assert_eq!(other_host.state(), pb::RequestState::Pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_grant_covers_what_is_under_it_and_not_its_parent() {
    let fixture = Fixture::start().await;
    let asked = fixture
        .ask_ahead(pb::Capability::FilesReadOtherSession, other_files("src"))
        .await;
    let id = asked.id.unwrap();

    fixture.answer(&id, approve()).await.unwrap();
    let inside = fixture
        .ask_ahead(pb::Capability::FilesReadOtherSession, other_files("src/ledger"))
        .await;
    let parent = fixture
        .ask_ahead(pb::Capability::FilesReadOtherSession, other_files(""))
        .await;
    let beside = fixture
        .ask_ahead(pb::Capability::FilesReadOtherSession, other_files("docs"))
        .await;

    assert_eq!(inside.state(), pb::RequestState::Approved);
    assert_eq!(parent.state(), pb::RequestState::Pending);
    assert_eq!(beside.state(), pb::RequestState::Pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn always_let_grants_only_the_target_the_card_offered() {
    let fixture = Fixture::start().await;
    let id = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("localhost", 5432))
        .await
        .id
        .unwrap();
    let offered = fixture.request(&id).await.always.and_then(|always| always.target);

    let wider = fixture
        .answer(
            &id,
            pb::request_answer::Answer::Always(pb::AlwaysLet {
                target: Some(network("localhost", 1)),
            }),
        )
        .await
        .unwrap_err();
    let still = fixture.request(&id).await;
    let answered = fixture
        .answer(
            &id,
            pb::request_answer::Answer::Always(pb::AlwaysLet { target: offered.clone() }),
        )
        .await
        .unwrap();

    assert_eq!(invalid_field(&wider).0, "answer.always.target");
    assert_eq!(still.state(), pb::RequestState::Pending);
    assert_eq!(answered.state(), pb::RequestState::Approved);
    let grants = fixture.grants().await;
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].target, offered);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_grant_makes_the_next_ask_wait_for_the_user() {
    let fixture = Fixture::start().await;
    let id = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("localhost", 5432))
        .await
        .id
        .unwrap();
    fixture.answer(&id, approve()).await.unwrap();
    let listed = fixture.grants().await;
    let grant = listed[0].id.clone();

    let by_agent = fixture
        .access()
        .await
        .revoke_grant(fixture.as_agent(pb::RevokeGrantRequest {
            grant: grant.clone(),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    let kept = fixture.grants().await;
    fixture
        .access()
        .await
        .revoke_grant(pb::RevokeGrantRequest {
            grant: grant.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    let again = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("localhost", 5432))
        .await;

    assert_eq!(listed.len(), 1);
    assert!(matches!(
        error_of(&by_agent).kind,
        Some(pb::error::Kind::UserOnly(_))
    ));
    assert_eq!(kept.len(), 1);
    assert!(fixture.grants().await.is_empty());
    assert_eq!(again.state(), pb::RequestState::Pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_grant_that_isnt_there_is_not_found() {
    let fixture = Fixture::start().await;

    let missing = fixture
        .access()
        .await
        .revoke_grant(pb::RevokeGrantRequest {
            grant: Some(pb::GrantId {
                value: "41".to_owned(),
            }),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(common::not_found(&missing), pb::EntityKind::Grant);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_takes_only_its_first_answer() {
    let fixture = Fixture::start().await;
    let id = fixture.ask("Which branch?").await;
    fixture.answer(&id, deny()).await.unwrap();

    let second = fixture.answer(&id, reply("main")).await.unwrap_err();

    assert_eq!(second.code(), Code::Aborted);
    assert!(matches!(
        error_of(&second).kind,
        Some(pb::error::Kind::AlreadyAnswered(pb::AlreadyAnswered { approved: false, .. }))
    ));
    assert_eq!(fixture.request(&id).await.state(), pb::RequestState::Denied);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_withdrawn_request_cannot_be_answered() {
    let fixture = Fixture::start().await;
    let id = fixture.ask("Still there?").await;
    fixture
        .inbox()
        .await
        .withdraw_request(fixture.as_agent(pb::WithdrawRequestRequest {
            idempotency_key: None,
            id: Some(id.clone()),
        }))
        .await
        .unwrap();

    let late = fixture.answer(&id, reply("yes")).await.unwrap_err();

    assert_eq!(late.code(), Code::FailedPrecondition);
    assert!(matches!(
        error_of(&late).kind,
        Some(pb::error::Kind::RequestWithdrawn(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_answer_to_a_request_seen_at_an_older_revision_is_refused() {
    let fixture = Fixture::start().await;
    let id = fixture.ask("Now?").await;
    let seen = fixture.request(&id).await.revision.unwrap();

    let stale = fixture
        .inbox()
        .await
        .answer_request(pb::AnswerRequestRequest {
            expected_revision: Some(pb::Revision {
                value: seen.value + 1,
            }),
            ..answer_of(&id, reply("yes"))
        })
        .await
        .unwrap_err();
    let current = fixture
        .inbox()
        .await
        .answer_request(pb::AnswerRequestRequest {
            expected_revision: Some(seen),
            ..answer_of(&id, reply("yes"))
        })
        .await
        .unwrap()
        .into_inner();

    assert!(matches!(
        error_of(&stale).kind,
        Some(pb::error::Kind::RevisionConflict(_))
    ));
    assert_eq!(current.state(), pb::RequestState::Answered);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_lists_only_its_own_sessions_grants() {
    let fixture = Fixture::start().await;
    let id = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("localhost", 5432))
        .await
        .id
        .unwrap();
    fixture.answer(&id, approve()).await.unwrap();

    let own = fixture
        .access()
        .await
        .list_grants(fixture.as_agent(pb::ListGrantsRequest::default()))
        .await
        .unwrap()
        .into_inner()
        .grants;
    let others = fixture
        .access()
        .await
        .list_grants(pb::ListGrantsRequest {
            requester: Some(session(OTHER)),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
        .grants;

    assert_eq!(own.len(), 1);
    assert!(others.is_empty());
}

/// A feed subscribed with `request`, from now.
async fn feed(fixture: &Fixture, request: Request<pb::SubscribeRequest>) -> tonic::Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(fixture.daemon.channel().await)
        .subscribe(request)
        .await
        .unwrap()
        .into_inner()
}

/// The next request event's change, request id and state.
/// The next event's change, request, its state, and who made the change:
/// the asking session's agent, the user, or no one (the daemon).
async fn next_request(events: &mut tonic::Streaming<pb::Event>) -> (i32, pb::RequestId, i32, &'static str) {
    let event = tokio::time::timeout(WAIT, events.message()).await.unwrap().unwrap().unwrap();
    let by = match event.by.and_then(|by| by.actor) {
        Some(pb::actor::Actor::Agent(agent)) if agent.session == Some(session(ASKER)) => "asker",
        Some(pb::actor::Actor::User(_)) => "user",
        None => "daemon",
        Some(other) => panic!("unexpected actor {other:?}"),
    };
    match event.payload {
        Some(pb::event::Payload::Request(request)) => (event.change, request.id.unwrap(), request.state, by),
        other => panic!("expected an Inbox request, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_feed_tells_of_each_request_and_its_answer() {
    let fixture = Fixture::start().await;
    let mut users = feed(&fixture, Request::new(pb::SubscribeRequest::default())).await;
    let mut agents = feed(&fixture, fixture.as_agent(pb::SubscribeRequest::default())).await;
    let mut inbox_only = feed(
        &fixture,
        Request::new(pb::SubscribeRequest {
            topics: vec![pb::Topic::Inbox as i32],
            ..Default::default()
        }),
    )
    .await;
    let mut other_session = feed(
        &fixture,
        Request::new(pb::SubscribeRequest {
            sessions: vec![session(OTHER)],
            ..Default::default()
        }),
    )
    .await;
    let mut machines_only = feed(
        &fixture,
        Request::new(pb::SubscribeRequest {
            topics: vec![pb::Topic::Machines as i32],
            ..Default::default()
        }),
    )
    .await;

    let answered = fixture.ask("Ship it?").await;
    fixture.answer(&answered, reply("yes")).await.unwrap();
    let withdrawn = fixture.ask("Still there?").await;
    let withdraw = pb::WithdrawRequestRequest {
        idempotency_key: None,
        id: Some(withdrawn.clone()),
    };
    fixture.inbox().await.withdraw_request(fixture.as_agent(withdraw.clone())).await.unwrap();
    fixture.inbox().await.withdraw_request(fixture.as_agent(withdraw)).await.unwrap();
    // Marks the end: nothing after the second withdrawal was told.
    let last = fixture.ask("Done?").await;

    let (created, updated) = (pb::Change::Created as i32, pb::Change::Updated as i32);
    let pending = pb::RequestState::Pending as i32;
    let expected = vec![
        (created, answered.clone(), pending, "asker"),
        (updated, answered, pb::RequestState::Answered as i32, "user"),
        (created, withdrawn.clone(), pending, "asker"),
        (updated, withdrawn, pb::RequestState::Withdrawn as i32, "asker"),
        (created, last, pending, "asker"),
    ];
    for events in [&mut users, &mut agents, &mut inbox_only] {
        let mut told = Vec::new();
        for _ in 0..expected.len() {
            told.push(next_request(events).await);
        }
        assert_eq!(told, expected, "a repeated withdrawal changes nothing, so it isn't told");
    }
    for (events, why) in [
        (&mut other_session, "a feed narrowed to another session"),
        (&mut machines_only, "a feed of machines only"),
    ] {
        assert!(
            tokio::time::timeout(Duration::from_millis(200), events.message()).await.is_err(),
            "{why} hears of no request"
        );
    }
}

/// The next event's change and what it tells of: `Ok` a grant's id, `Err`
/// a request's state.
async fn next_told(events: &mut tonic::Streaming<pb::Event>) -> (i32, Result<pb::GrantId, i32>) {
    let event = tokio::time::timeout(WAIT, events.message()).await.unwrap().unwrap().unwrap();
    match event.payload {
        Some(pb::event::Payload::Grant(grant)) => (event.change, Ok(grant.id.unwrap())),
        Some(pb::event::Payload::Request(request)) => (event.change, Err(request.state)),
        other => panic!("expected a grant or a request, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_feed_tells_of_the_grant_an_approval_makes_and_its_revoking() {
    let fixture = Fixture::start().await;
    let mut users = feed(&fixture, Request::new(pb::SubscribeRequest::default())).await;
    let mut agents = feed(&fixture, fixture.as_agent(pb::SubscribeRequest::default())).await;
    let mut access_only = feed(
        &fixture,
        Request::new(pb::SubscribeRequest {
            topics: vec![pb::Topic::Access as i32],
            ..Default::default()
        }),
    )
    .await;
    let mut other_session = feed(
        &fixture,
        Request::new(pb::SubscribeRequest {
            sessions: vec![session(OTHER)],
            ..Default::default()
        }),
    )
    .await;

    let id = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("localhost", 5432))
        .await
        .id
        .unwrap();
    fixture.answer(&id, approve()).await.unwrap();
    let grant = fixture.grants().await[0].id.clone().unwrap();
    // Covered by the grant: approved at once, so nothing new is told.
    fixture
        .ask_ahead(pb::Capability::NetworkReach, network("localhost", 5432))
        .await;
    fixture
        .access()
        .await
        .revoke_grant(pb::RevokeGrantRequest {
            grant: Some(grant.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

    let (created, updated, deleted) = (
        pb::Change::Created as i32,
        pb::Change::Updated as i32,
        pb::Change::Deleted as i32,
    );
    let expected = vec![
        (created, Err(pb::RequestState::Pending as i32)),
        (created, Ok(grant.clone())),
        (updated, Err(pb::RequestState::Approved as i32)),
        (deleted, Ok(grant.clone())),
    ];
    for events in [&mut users, &mut agents] {
        let mut told = Vec::new();
        for _ in 0..expected.len() {
            told.push(next_told(events).await);
        }
        assert_eq!(told, expected);
    }
    assert_eq!(next_told(&mut access_only).await, (created, Ok(grant.clone())));
    assert_eq!(next_told(&mut access_only).await, (deleted, Ok(grant)));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), other_session.message()).await.is_err(),
        "a feed narrowed to another session hears of no grant"
    );
}
