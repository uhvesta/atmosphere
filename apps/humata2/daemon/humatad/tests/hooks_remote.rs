//! A permission hook of a session on a machine (HOOKS, D191, D225, D226): the
//! agent's hook asks its own machine's daemon, as it does on the Mac, and
//! the card shows in the host's Inbox. Approving it there releases the hook
//! on the machine; the hook letting go withdraws the host's card too.
//!
//! A real host and a real machine behind the rig's fake `ssh`. Each wait
//! reads the host's Inbox again each time its Inbox feed says something
//! changed, so nothing sleeps.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use humata2_grpc::humata::v1 as pb;
use remote_rig::{LONG, Rig, is_live};
use serde_json::Value;
use tonic::metadata::MetadataValue;
use tonic::{Request, Streaming};

const REPO: (&str, &str) = ("acme", "api");
const BOX: &str = "box-1";
/// Copilot's permissionRequest payload (agent_hooks/testdata/copilot).
const COPILOT_SHELL: &str = r#"{"sessionId":"c0p1","timestamp":1763251200000,"cwd":"/work/api","hook_event_name":"permissionRequest","toolName":"bash","toolInput":{"command":"rm -rf build","description":"Clean the build folder"}}"#;

fn as_agent<T>(token: &str, message: T) -> Request<T> {
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        MetadataValue::try_from(token).expect("a token is ASCII"),
    );
    request
}

/// A host and box-1, with a live session on box-1 and its agent's token.
struct Remote {
    rig: Rig,
    session: pb::SessionId,
    token: String,
}

impl Remote {
    async fn start() -> Self {
        let rig = Rig::start_seen(&[BOX], &[REPO]).await;
        let (host, one) = (rig.host_id().await, Rig::box_id(BOX));
        let channel = rig.create_channel("Payments", &[REPO], &[host, one.clone()]).await;
        let session = rig.start_on(&channel, &one, "Ledger").await;
        rig.wait_session(&session, "live", is_live).await;
        let token = rig.agent_token(BOX, &session).await;
        Self { rig, session, token }
    }

    /// The session's agent's hook holds the Hook stream of its own
    /// machine's daemon, as `humata hook copilot permission` does there.
    async fn hold(&self) -> Streaming<pb::HookReply> {
        pb::session_service_client::SessionServiceClient::new(self.rig.the(BOX).daemon().channel().await)
            .hook(as_agent(
                &self.token,
                pb::HookRequest {
                    agent: "copilot".to_owned(),
                    event: "permission".to_owned(),
                    payload: COPILOT_SHELL.as_bytes().to_vec(),
                },
            ))
            .await
            .expect("the machine opens the Hook stream")
            .into_inner()
    }

    async fn inbox_feed(&self) -> Streaming<pb::Event> {
        pb::event_service_client::EventServiceClient::new(self.rig.channel().await)
            .subscribe(pb::SubscribeRequest {
                topics: vec![pb::Topic::Inbox as i32],
                ..Default::default()
            })
            .await
            .expect("the host's Inbox feed opens")
            .into_inner()
    }

    /// The host's Inbox requests of the session, settled ones too.
    async fn host_requests(&self) -> Vec<pb::InboxRequest> {
        self.rig
            .host
            .inbox_service()
            .await
            .list_requests(pb::ListRequestsRequest {
                include_answered: true,
                ..Default::default()
            })
            .await
            .expect("the host lists its Inbox")
            .into_inner()
            .requests
            .into_iter()
            .filter(|request| {
                request.requester.as_ref().and_then(|agent| agent.session.as_ref()) == Some(&self.session)
            })
            .collect()
    }

    /// The session's host request once `want` holds for it.
    async fn until(
        &self,
        feed: &mut Streaming<pb::Event>,
        what: &str,
        want: impl Fn(&pb::InboxRequest) -> bool,
    ) -> pb::InboxRequest {
        let deadline = tokio::time::Instant::now() + LONG;
        loop {
            let now = self.host_requests().await;
            if let Some(found) = now.iter().find(|request| want(request)) {
                return found.clone();
            }
            match tokio::time::timeout_at(deadline, feed.message()).await {
                Ok(Ok(Some(_))) => {}
                Ok(ended) => panic!("the host's Inbox feed ended ({ended:?}) before {what}; it showed {now:?}"),
                Err(_) => panic!("never {what} within {LONG:?}; the host's Inbox showed {now:?}"),
            }
        }
    }
}

fn pending(request: &pb::InboxRequest) -> bool {
    request.state() == pb::RequestState::Pending
}

fn command_of(request: &pb::InboxRequest) -> String {
    match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
        Some(pb::request_preview::Preview::Command(command)) => command.command.clone(),
        other => panic!("not a command card: {other:?}"),
    }
}

/// The bytes the hook prints, once the stream ends its keepalives.
async fn printed(stream: &mut Streaming<pb::HookReply>) -> Vec<u8> {
    tokio::time::timeout(LONG, async {
        while let Some(reply) = stream.message().await.expect("the Hook stream yields a message") {
            if !reply.keepalive {
                return reply.stdout;
            }
        }
        panic!("the Hook stream ended with no reply");
    })
    .await
    .expect("no reply on the Hook stream")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_sessions_permission_ask_waits_in_the_hosts_inbox_and_its_approval_releases_the_hook() {
    let remote = Remote::start().await;
    let mut feed = remote.inbox_feed().await;

    let mut hook = remote.hold().await;
    let card = remote.until(&mut feed, "the ask showed in the host's Inbox", pending).await;
    assert_eq!(command_of(&card), "rm -rf build");

    remote
        .rig
        .host
        .inbox_service()
        .await
        .answer_request(pb::AnswerRequestRequest {
            id: card.id.clone(),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            ..Default::default()
        })
        .await
        .expect("the host's user approves the ask");

    let out: Value = serde_json::from_slice(&printed(&mut hook).await).expect("the hook prints JSON");
    assert_eq!(out["behavior"], "allow", "{out}");
    let settled = remote
        .until(&mut feed, "the host showed the ask approved", |request| request.id == card.id && !pending(request))
        .await;
    assert_eq!(settled.state(), pb::RequestState::Approved);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_hook_that_lets_go_withdraws_the_hosts_card() {
    let remote = Remote::start().await;
    let mut feed = remote.inbox_feed().await;

    let hook = remote.hold().await;
    let card = remote.until(&mut feed, "the ask showed in the host's Inbox", pending).await;
    drop(hook);

    let gone = remote
        .until(&mut feed, "the host's card was withdrawn", |request| request.id == card.id && !pending(request))
        .await;
    assert_eq!(gone.state(), pb::RequestState::Withdrawn);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_restart_withdraws_the_hosts_card_for_the_hook_it_ended() {
    let mut remote = Remote::start().await;
    let mut feed = remote.inbox_feed().await;

    let hook = remote.hold().await;
    let card = remote.until(&mut feed, "the ask showed in the host's Inbox", pending).await;
    remote.rig.the_mut(BOX).kill();
    drop(hook);
    remote.rig.the_mut(BOX).relaunch(&[]).await;
    remote.rig.link_back(BOX).await;

    let gone = remote
        .until(&mut feed, "the host's card was withdrawn", |request| request.id == card.id && !pending(request))
        .await;
    assert_eq!(gone.state(), pb::RequestState::Withdrawn);
}
