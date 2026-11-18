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
                folder: String::new(),
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
    async fn ask_ahead(
        &self,
        capability: pb::Capability,
        target: pb::GrantTarget,
    ) -> pb::WaitResponse {
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
            .get_request(pb::GetRequestRequest {
                id: Some(id.clone()),
            })
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
        request.metadata_mut().insert(
            TOKEN_HEADER,
            MetadataValue::try_from(self.token.as_str()).unwrap(),
        );
        request
    }
}

fn session(id: &str) -> pb::SessionId {
    pb::SessionId {
        value: id.to_owned(),
    }
}

fn answer_of(id: &pb::RequestId, answer: pb::request_answer::Answer) -> pb::AnswerRequestRequest {
    pb::AnswerRequestRequest {
        id: Some(id.clone()),
        answer: Some(pb::RequestAnswer {
            answer: Some(answer),
        }),
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
    pb::request_answer::Answer::Reply(pb::Reply {
        text: text.to_owned(),
    })
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
        assert!(
            Instant::now() < deadline,
            "{} was never written",
            path.display()
        );
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
    assert_eq!(
        fixture.request(&id).await.state(),
        pb::RequestState::Pending
    );
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

    let answered: Vec<_> = response
        .answered
        .iter()
        .filter_map(|request| request.id.clone())
        .collect();
    assert_eq!(answered, vec![listed.clone()]);
    assert_eq!(response.skipped, vec![unknown]);
    assert_eq!(
        fixture.request(&listed).await.state(),
        pb::RequestState::Denied
    );
    assert_eq!(
        fixture.request(&unlisted).await.state(),
        pb::RequestState::Pending
    );
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
    assert_eq!(
        fixture.request(&ahead).await.state(),
        pb::RequestState::Pending
    );
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
        Some(pb::grant_target::Target::Network(pb::NetworkTarget {
            port: 5432,
            ..
        }))
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
        .ask_ahead(
            pb::Capability::FilesReadOtherSession,
            other_files("src/ledger"),
        )
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
    let offered = fixture
        .request(&id)
        .await
        .always
        .and_then(|always| always.target);

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
            pb::request_answer::Answer::Always(pb::AlwaysLet {
                target: offered.clone(),
            }),
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
async fn a_restart_keeps_the_inbox_and_the_grants_still_standing() {
    let mut fixture = Fixture::start().await;
    let question = fixture.ask("Which database?").await;
    let folder = fixture
        .ask_ahead(pb::Capability::FilesReadOtherSession, other_files("src"))
        .await
        .id
        .unwrap();
    fixture.answer(&folder, approve()).await.unwrap();
    let revoked = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("localhost", 5432))
        .await
        .id
        .unwrap();
    fixture.answer(&revoked, approve()).await.unwrap();
    let pending = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("example.com", 443))
        .await
        .id
        .unwrap();
    let before = fixture.grants().await;
    let gone = before
        .iter()
        .find(|grant| grant.granted_from.as_ref() == Some(&revoked))
        .and_then(|grant| grant.id.clone());
    fixture
        .access()
        .await
        .revoke_grant(pb::RevokeGrantRequest {
            grant: gone,
            ..Default::default()
        })
        .await
        .unwrap();
    let kept = fixture.grants().await;

    fixture.daemon.restart().await;

    let listed = fixture
        .inbox()
        .await
        .list_requests(pb::ListRequestsRequest {
            include_answered: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let states: Vec<_> = listed
        .requests
        .iter()
        .map(|request| (request.id.clone().unwrap(), request.state()))
        .collect();
    assert_eq!(
        states,
        vec![
            (question.clone(), pb::RequestState::Pending),
            (folder, pb::RequestState::Approved),
            (revoked, pb::RequestState::Approved),
            (pending.clone(), pb::RequestState::Pending),
        ]
    );
    assert_eq!(
        fixture.grants().await,
        kept,
        "only the grant still standing comes back"
    );
    // The folder grant covers what's under it and no more, as before.
    let inside = fixture
        .ask_ahead(
            pb::Capability::FilesReadOtherSession,
            other_files("src/ledger"),
        )
        .await;
    let parent = fixture
        .ask_ahead(pb::Capability::FilesReadOtherSession, other_files(""))
        .await;
    let again = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("localhost", 5432))
        .await;
    assert_eq!(inside.state(), pb::RequestState::Approved);
    assert_eq!(parent.state(), pb::RequestState::Pending);
    assert_eq!(
        again.state(),
        pb::RequestState::Pending,
        "a revoked grant stays revoked"
    );
    // What was pending can still be answered, and a new grant takes a new id.
    fixture.answer(&question, reply("Postgres")).await.unwrap();
    fixture.answer(&pending, approve()).await.unwrap();
    let new = grant_in(&fixture.wait(&pending).await);
    assert!(
        before.iter().all(|old| old.id != new.id),
        "grant {:?} was given out before the restart",
        new.id
    );
    assert_eq!(
        fixture.wait(&question).await.state(),
        pb::RequestState::Answered
    );
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
        Some(pb::error::Kind::AlreadyAnswered(pb::AlreadyAnswered {
            approved: false,
            ..
        }))
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
async fn feed(
    fixture: &Fixture,
    request: Request<pb::SubscribeRequest>,
) -> tonic::Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(fixture.daemon.channel().await)
        .subscribe(request)
        .await
        .unwrap()
        .into_inner()
}

/// The next event that isn't a session's or a Resync: the asking session's
/// own change (Needs you while its agent waits on the request, D158 item 7)
/// comes on the same feed, around the request's, and a feed opened before
/// the daemon's start finished gets its after-restart Resync (D341).
async fn next_not_session(events: &mut tonic::Streaming<pb::Event>) -> pb::Event {
    loop {
        let event = tokio::time::timeout(WAIT, events.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if !matches!(
            event.payload,
            Some(pb::event::Payload::Session(_) | pb::event::Payload::Resync(_))
        ) {
            return event;
        }
    }
}

/// Whether the feed tells of nothing but a Resync for a moment.
async fn quiet(events: &mut tonic::Streaming<pb::Event>) -> bool {
    tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            let event = events.message().await.unwrap().unwrap();
            if !matches!(event.payload, Some(pb::event::Payload::Resync(_))) {
                return event;
            }
        }
    })
    .await
    .is_err()
}

/// The next request event's change, request id and state.
/// The next event's change, request, its state, and who made the change:
/// the asking session's agent, the user, or no one (the daemon).
async fn next_request(
    events: &mut tonic::Streaming<pb::Event>,
) -> (i32, pb::RequestId, i32, &'static str) {
    let event = next_not_session(events).await;
    let by = match event.by.and_then(|by| by.actor) {
        Some(pb::actor::Actor::Agent(agent)) if agent.session == Some(session(ASKER)) => "asker",
        Some(pb::actor::Actor::User(_)) => "user",
        None => "daemon",
        Some(other) => panic!("unexpected actor {other:?}"),
    };
    match event.payload {
        Some(pb::event::Payload::Request(request)) => {
            (event.change, request.id.unwrap(), request.state, by)
        }
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
    fixture
        .inbox()
        .await
        .withdraw_request(fixture.as_agent(withdraw.clone()))
        .await
        .unwrap();
    fixture
        .inbox()
        .await
        .withdraw_request(fixture.as_agent(withdraw))
        .await
        .unwrap();
    // Marks the end: nothing after the second withdrawal was told.
    let last = fixture.ask("Done?").await;

    let (created, updated) = (pb::Change::Created as i32, pb::Change::Updated as i32);
    let pending = pb::RequestState::Pending as i32;
    let expected = vec![
        (created, answered.clone(), pending, "asker"),
        (updated, answered, pb::RequestState::Answered as i32, "user"),
        (created, withdrawn.clone(), pending, "asker"),
        (
            updated,
            withdrawn,
            pb::RequestState::Withdrawn as i32,
            "asker",
        ),
        (created, last, pending, "asker"),
    ];
    for events in [&mut users, &mut agents, &mut inbox_only] {
        let mut told = Vec::new();
        for _ in 0..expected.len() {
            told.push(next_request(events).await);
        }
        assert_eq!(
            told, expected,
            "a repeated withdrawal changes nothing, so it isn't told"
        );
    }
    for (events, why) in [
        (&mut other_session, "a feed narrowed to another session"),
        (&mut machines_only, "a feed of machines only"),
    ] {
        assert!(quiet(events).await, "{why} hears of no request");
    }
}

/// The next event's change and what it tells of: `Ok` a grant's id, `Err`
/// a request's state.
async fn next_told(events: &mut tonic::Streaming<pb::Event>) -> (i32, Result<pb::GrantId, i32>) {
    let event = next_not_session(events).await;
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
    assert_eq!(
        next_told(&mut access_only).await,
        (created, Ok(grant.clone()))
    );
    assert_eq!(next_told(&mut access_only).await, (deleted, Ok(grant)));
    assert!(
        quiet(&mut other_session).await,
        "a feed narrowed to another session hears of no grant"
    );
}

// ----- Calls a rule sends to Ask wait in the Inbox (D38.6). The default
// rule for reading other channels is Allow (D127.4), so these tests set it
// to Ask first. ListChannels has no idempotency
// key, so the same-key repeat (TP-14) and a rule turned Never after the
// ask (IN-21) are not covered here: the first needs a deferred call with a
// key, the second needs SetChannelRule. -----

impl Fixture {
    /// The user sets the channel's rule for reading other channels.
    async fn set_global_read(&self, decision: pb::Decision) {
        self.access()
            .await
            .set_channel_rule(pb::SetChannelRuleRequest {
                channel: Some(pb::ChannelId {
                    value: CHANNEL.to_owned(),
                }),
                capability: pb::Capability::GlobalRead as i32,
                decision: decision as i32,
                ..Default::default()
            })
            .await
            .unwrap();
    }

    /// With the rule set to Ask, the agent lists channels; the call is
    /// deferred and this returns the request it waits on.
    async fn list_channels_deferred(&self) -> pb::RequestId {
        self.set_global_read(pb::Decision::Ask).await;
        let refused =
            pb::channel_service_client::ChannelServiceClient::new(self.daemon.channel().await)
                .list_channels(self.as_agent(pb::ListChannelsRequest::default()))
                .await
                .unwrap_err();
        assert_eq!(refused.code(), Code::FailedPrecondition);
        match error_of(&refused).kind {
            Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.unwrap(),
            other => panic!("expected approval_pending, got {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approved_deferred_call_runs_and_its_result_is_waited_for() {
    let fixture = Fixture::start().await;
    let id = fixture.list_channels_deferred().await;
    assert_eq!(
        fixture.request(&id).await.state(),
        pb::RequestState::Pending
    );

    fixture.answer(&id, approve()).await.unwrap();
    let waited = fixture.wait(&id).await;

    assert_eq!(waited.state(), pb::RequestState::Approved);
    assert!(waited.error.is_none());
    assert_eq!(waited.result_type, "humata.v1.ListChannelsResponse");
    pb::ListChannelsResponse::decode(waited.result.as_slice()).unwrap();
    assert!(waited.result_kept_until.is_some());
    // A second wait gets the same result (TP-12).
    assert_eq!(fixture.wait(&id).await.result, waited.result);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_lists_channels_at_once_while_the_rule_is_left_at_its_default() {
    let fixture = Fixture::start().await;

    let listed =
        pb::channel_service_client::ChannelServiceClient::new(fixture.daemon.channel().await)
            .list_channels(fixture.as_agent(pb::ListChannelsRequest::default()))
            .await;

    assert!(listed.is_ok(), "{listed:?}");
    assert!(common::pending_requests(&fixture.daemon).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_denied_deferred_call_never_runs() {
    let fixture = Fixture::start().await;
    let id = fixture.list_channels_deferred().await;

    fixture.answer(&id, deny()).await.unwrap();
    let waited = fixture.wait(&id).await;

    assert_eq!(waited.state(), pb::RequestState::Denied);
    assert!(waited.result.is_empty());
    assert!(matches!(
        waited.error.and_then(|error| error.kind),
        Some(pb::error::Kind::Denied(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_user_lists_channels_without_asking() {
    let fixture = Fixture::start().await;

    let listed =
        pb::channel_service_client::ChannelServiceClient::new(fixture.daemon.channel().await)
            .list_channels(pb::ListChannelsRequest::default())
            .await;

    assert!(listed.is_ok());
}

// GlobalRead, the capability behind ListChannels, has no grant target
// (IN-22), so its card offers no Always allow. A positive Always-allow test
// needs a deferred call on a capability that has one; it is a unit test in
// src/inbox/deferred.rs, which defers a network reach.
#[tokio::test(flavor = "multi_thread")]
async fn a_deferred_card_has_a_preview_and_offers_no_always_allow_without_a_target() {
    let fixture = Fixture::start().await;
    let id = fixture.list_channels_deferred().await;

    let card = fixture.request(&id).await;
    assert!(card.preview.is_some());
    assert!(card.always.is_none());

    let refused = fixture
        .answer(
            &id,
            pb::request_answer::Answer::Always(pb::AlwaysLet {
                target: Some(network("example.com", 443)),
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert_eq!(
        fixture.request(&id).await.state(),
        pb::RequestState::Pending,
        "a refused answer leaves the request waiting"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_a_deferred_call_once_makes_no_grant_so_the_next_call_asks_again() {
    let fixture = Fixture::start().await;
    let first = fixture.list_channels_deferred().await;
    fixture.answer(&first, approve()).await.unwrap();
    fixture.wait(&first).await;

    let second = fixture.list_channels_deferred().await;

    assert_ne!(first, second);
    assert!(fixture.grants().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_restart_a_pending_deferred_request_is_withdrawn_and_an_ask_ahead_still_waits() {
    let mut fixture = Fixture::start().await;
    let deferred = fixture.list_channels_deferred().await;
    let ahead = fixture
        .ask_ahead(pb::Capability::NetworkReach, network("example.com", 443))
        .await
        .id
        .unwrap();

    fixture.daemon.restart().await;

    let deferred = fixture.request(&deferred).await;
    assert_eq!(deferred.state(), pb::RequestState::Withdrawn);
    assert_eq!(deferred.withdraw_reason(), pb::WithdrawReason::CallEnded);
    assert_eq!(
        fixture.request(&ahead).await.state(),
        pb::RequestState::Pending
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_listing_another_sessions_turns_waits_in_the_inbox_when_the_rule_asks() {
    let fixture = Fixture::start().await;
    fixture
        .access()
        .await
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: Some(pb::ChannelId {
                value: CHANNEL.to_owned(),
            }),
            capability: pb::Capability::ChannelRead as i32,
            decision: pb::Decision::Ask as i32,
            ..Default::default()
        })
        .await
        .unwrap();

    let deferred = pb::session_service_client::SessionServiceClient::new(fixture.daemon.channel().await)
        .list_turns(fixture.as_agent(pb::ListTurnsRequest {
            session: Some(session(OTHER)),
            page: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(deferred.code(), Code::FailedPrecondition, "{deferred:?}");
    let Some(pb::error::Kind::ApprovalPending(pending)) = error_of(&deferred).kind else {
        panic!("expected approval_pending, got {deferred:?}");
    };
    let id = pending.request.unwrap();

    fixture.answer(&id, approve()).await.unwrap();
    let waited = fixture.wait(&id).await;

    assert_eq!(waited.state(), pb::RequestState::Approved);
    assert!(waited.error.is_none(), "{:?}", waited.error);
    assert_eq!(waited.result_type, "humata.v1.ListTurnsResponse");
    pb::ListTurnsResponse::decode(waited.result.as_slice()).unwrap();
}

// ----- D768: an agent's file read the rule asks about waits in the Inbox.
// Reading another session's clone is Ask by default
// (FilesReadOtherSession). One card per session, capability, path and
// operation; approving it runs the read, and the wait returns its result. -----

/// The root of `OTHER`'s clone of acme/ledger, or `path` in it.
fn ledger(path: &str) -> pb::FileLocation {
    pb::FileLocation {
        machine: None,
        root: Some(pb::file_location::Root::SessionClone(pb::SessionClone {
            session: Some(session(OTHER)),
            repo: Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: "ledger".to_owned(),
            }),
        })),
        path: path.to_owned(),
    }
}

/// The request an approval-pending refusal names.
fn pending_of(refused: &tonic::Status) -> pb::RequestId {
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    match error_of(refused).kind {
        Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.unwrap(),
        other => panic!("expected approval_pending, got {other:?}"),
    }
}

impl Fixture {
    /// `OTHER`'s clone of acme/ledger on disk, holding `a.txt`.
    fn ledger_on_disk(&self) {
        let folder = self.daemon.dir.path().join("ledger");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.txt"), b"ledger\n").unwrap();
    }

    async fn files(&self) -> pb::file_service_client::FileServiceClient<Channel> {
        pb::file_service_client::FileServiceClient::new(self.daemon.channel().await)
    }

    /// The agent lists `OTHER`'s clone.
    async fn list_ledger(&self) -> Result<pb::ListDirectoryResponse, tonic::Status> {
        self.files()
            .await
            .list_directory(self.as_agent(pb::ListDirectoryRequest {
                location: Some(ledger("")),
                page: None,
            }))
            .await
            .map(tonic::Response::into_inner)
    }

    /// The agent stats `path` in `OTHER`'s clone.
    async fn stat_ledger(&self, path: &str) -> Result<pb::FileEntry, tonic::Status> {
        self.files()
            .await
            .stat(self.as_agent(pb::StatRequest {
                location: Some(ledger(path)),
            }))
            .await
            .map(tonic::Response::into_inner)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_asked_read_raises_one_card_and_runs_once_approved() {
    let fixture = Fixture::start().await;
    fixture.ledger_on_disk();

    let first = pending_of(&fixture.list_ledger().await.unwrap_err());
    let again = pending_of(&fixture.list_ledger().await.unwrap_err());

    assert_eq!(first, again, "a repeated read reuses its card");
    assert_eq!(common::pending_requests(&fixture.daemon).await, [first.clone()]);
    assert_eq!(fixture.request(&first).await.state(), pb::RequestState::Pending);

    fixture.answer(&first, approve()).await.unwrap();
    fixture.wait(&first).await;
    let listed = fixture.list_ledger().await.unwrap();

    let names: Vec<String> = listed
        .entries
        .iter()
        .filter_map(|entry| entry.location.as_ref())
        .map(|location| location.path.clone())
        .collect();
    assert!(names.iter().any(|name| name.ends_with("a.txt")), "{names:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn another_path_or_another_read_of_one_file_gets_its_own_card() {
    let fixture = Fixture::start().await;
    fixture.ledger_on_disk();

    let listing = pending_of(&fixture.list_ledger().await.unwrap_err());
    let stat = pending_of(&fixture.stat_ledger("a.txt").await.unwrap_err());
    let read = fixture
        .files()
        .await
        .read_file(fixture.as_agent(pb::ReadFileRequest {
            location: Some(ledger("a.txt")),
            ..Default::default()
        }))
        .await;
    let read = match read {
        Ok(_) => panic!("an asked read streamed the file"),
        Err(refused) => pending_of(&refused),
    };

    assert_ne!(listing, stat);
    assert_ne!(stat, read, "each read waits for its own result");
    assert_eq!(common::pending_requests(&fixture.daemon).await.len(), 3);

    fixture.answer(&stat, approve()).await.unwrap();
    fixture.wait(&stat).await;
    assert!(fixture.stat_ledger("a.txt").await.is_ok());
    fixture.answer(&read, approve()).await.unwrap();
    fixture.wait(&read).await;
    let mut chunks = fixture
        .files()
        .await
        .read_file(fixture.as_agent(pb::ReadFileRequest {
            location: Some(ledger("a.txt")),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(chunks.message().await.unwrap().is_some(), "the approved read streams the file");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_denied_read_is_reported_as_denied() {
    let fixture = Fixture::start().await;
    fixture.ledger_on_disk();
    let id = pending_of(&fixture.list_ledger().await.unwrap_err());

    fixture.answer(&id, deny()).await.unwrap();
    let refused = fixture.list_ledger().await.unwrap_err();

    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    assert!(
        matches!(error_of(&refused).kind, Some(pb::error::Kind::Denied(denied)) if denied.request == Some(id.clone())),
        "{refused:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn waiting_on_an_approved_read_says_what_was_approved_not_a_bare_approval() {
    let fixture = Fixture::start().await;
    fixture.ledger_on_disk();
    let id = pending_of(&fixture.list_ledger().await.unwrap_err());

    fixture.answer(&id, approve()).await.unwrap();
    let waited = fixture.wait(&id).await;

    assert_eq!(waited.state(), pb::RequestState::Approved);
    assert!(waited.error.is_none(), "{waited:?}");
    assert!(
        !waited.result_type.is_empty(),
        "an approved read's wait names what it holds, so a client can tell it to repeat the read: {waited:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn waiting_on_an_approved_listing_returns_the_listing() {
    let fixture = Fixture::start().await;
    fixture.ledger_on_disk();
    let id = pending_of(&fixture.list_ledger().await.unwrap_err());

    fixture.answer(&id, approve()).await.unwrap();
    let waited = fixture.wait(&id).await;

    assert!(waited.error.is_none(), "{waited:?}");
    assert_eq!(waited.result_type, "humata.v1.ListDirectoryResponse");
    let listed = pb::ListDirectoryResponse::decode(waited.result.as_slice()).unwrap();
    let names: Vec<String> = listed
        .entries
        .iter()
        .filter_map(|entry| entry.location.as_ref())
        .map(|location| location.path.clone())
        .collect();
    assert!(names.iter().any(|name| name.ends_with("a.txt")), "{names:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn waiting_on_an_approved_file_read_returns_the_files_bytes() {
    let fixture = Fixture::start().await;
    fixture.ledger_on_disk();
    let refused = fixture
        .files()
        .await
        .read_file(fixture.as_agent(pb::ReadFileRequest {
            location: Some(ledger("a.txt")),
            ..Default::default()
        }))
        .await;
    let id = match refused {
        Ok(_) => panic!("an asked read streamed the file"),
        Err(refused) => pending_of(&refused),
    };

    fixture.answer(&id, approve()).await.unwrap();
    let waited = fixture.wait(&id).await;

    assert!(waited.error.is_none(), "{waited:?}");
    assert_eq!(waited.result_type, "humata.v1.FileChunk");
    let chunk = pb::FileChunk::decode(waited.result.as_slice()).unwrap();
    assert_eq!(chunk.data, b"ledger\n");
    assert!(chunk.last, "the whole file was held: {chunk:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn waiting_on_an_approved_stat_returns_the_entry() {
    let fixture = Fixture::start().await;
    fixture.ledger_on_disk();
    let id = pending_of(&fixture.stat_ledger("a.txt").await.unwrap_err());

    fixture.answer(&id, approve()).await.unwrap();
    let waited = fixture.wait(&id).await;

    assert!(waited.error.is_none(), "{waited:?}");
    assert_eq!(waited.result_type, "humata.v1.FileEntry");
    let entry = pb::FileEntry::decode(waited.result.as_slice()).unwrap();
    assert!(entry.location.unwrap().path.ends_with("a.txt"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approved_reads_bytes_go_to_the_first_wait_and_later_ones_repeat_the_read() {
    let fixture = Fixture::start().await;
    fixture.ledger_on_disk();
    let id = pending_of(&fixture.list_ledger().await.unwrap_err());
    fixture.answer(&id, approve()).await.unwrap();

    let first = fixture.wait(&id).await;
    let second = fixture.wait(&id).await;

    assert_eq!(first.result_type, "humata.v1.ListDirectoryResponse");
    assert!(second.result.is_empty(), "the held bytes were dropped once delivered: {second:?}");
    assert_ne!(second.result_type, first.result_type, "a later wait is told to repeat the read");
    assert!(second.error.is_none(), "{second:?}");
    assert!(fixture.list_ledger().await.is_ok(), "the repeat goes through");
}
#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_approved_read_drops_its_held_bytes() {
    let fixture = Fixture::start().await;
    fixture.ledger_on_disk();
    let id = pending_of(&fixture.list_ledger().await.unwrap_err());
    fixture.answer(&id, approve()).await.unwrap();

    // The agent repeats the read instead of waiting (exit 12, then again).
    assert!(fixture.list_ledger().await.is_ok(), "the repeat goes through");
    let waited = fixture.wait(&id).await;

    assert!(waited.error.is_none(), "{waited:?}");
    assert!(waited.result.is_empty(), "the repeat took the read; its bytes aren't held: {waited:?}");
    assert_ne!(waited.result_type, "humata.v1.ListDirectoryResponse");
}

