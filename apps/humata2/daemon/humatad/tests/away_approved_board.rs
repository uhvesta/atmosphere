//! A write the host's rules ask about, made by an agent on a machine while
//! its host is away, is approved by the user (D268, BD-15, AW-9). A real
//! host and a real machine behind the rig's fake `ssh`: the user's rule for
//! the agent's own board asks, the host's link is cut, the agent's edit waits
//! as a request in the machine's Inbox, and the user approves it there with
//! the approval passphrase. Once the link is back the item has changed on
//! the host and the host's mirror of the request is Approved. In the other
//! case the user approves the mirror on the host after the link is back.

mod common;
#[path = "common/away.rs"]
mod away;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use away::{OK, PASSPHRASE, Terminal};
use humata2_grpc::humata::v1 as pb;
use prost::Message;
use remote_rig::{LONG, Rig, is_live};
use tonic::metadata::MetadataValue;
use tonic::{Request, Streaming};

const REPO: (&str, &str) = ("acme", "api");
const BOX: &str = "box-1";
const TOKEN_HEADER: &str = "humata-session-token";
const TITLE: &str = "Fix login properly";

fn as_agent<T>(token: &str, message: T) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    request
}

fn key(value: &str) -> Option<pb::IdempotencyKey> {
    Some(pb::IdempotencyKey {
        value: value.to_owned(),
    })
}

/// The token the shell printed between `TOKEN<` and `>`, if it has. The typed
/// command spells those markers as octal escapes, so its own echo never
/// matches them.
fn printed_token(screen: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(screen);
    let (_, rest) = text.split_once("TOKEN<")?;
    let (token, _) = rest.split_once('>')?;
    Some(token.to_owned())
}

/// The token the machine's daemon gave a shell tab of `session`, which the
/// session's agent holds.
async fn box_token(rig: &Rig, session: &pb::SessionId) -> String {
    let daemon = rig.the(BOX).daemon();
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(session.clone()),
            repo: None,
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut attach = common::Attach::start(daemon, common::open(&tab, 0)).await;
    attach
        .type_text("printf 'TOKEN\\074%s\\076\\n' \"$HUMATA_SESSION_TOKEN\"\r")
        .await;
    attach.wait_until(|attach| printed_token(&attach.screen).is_some()).await;
    printed_token(&attach.screen).unwrap()
}

/// The box's own change feed for its Machines topic: its host link, and each
/// policy copy it keeps. Opened before the calls that can change either.
async fn box_feed(rig: &Rig) -> Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(rig.the(BOX).daemon().channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Machines as i32],
            ..Default::default()
        })
        .await
        .expect("the box's event feed opens")
        .into_inner()
}

/// The host's feed of `topic`.
async fn host_feed(rig: &Rig, topic: pb::Topic) -> Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(rig.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![topic as i32],
            ..Default::default()
        })
        .await
        .expect("the host's event feed opens")
        .into_inner()
}

/// The policy copy the machine has kept.
fn kept_copy(rig: &Rig) -> Option<pb::PolicySnapshot> {
    let kept = rig
        .the(BOX)
        .store()
        .setting("policy_copy")
        .unwrap()
        .map(|saved| saved.value)
        .unwrap_or_default();
    let bytes: Vec<u8> = (0..kept.len() / 2)
        .filter_map(|at| u8::from_str_radix(kept.get(2 * at..2 * at + 2)?, 16).ok())
        .collect();
    pb::PolicySnapshot::decode(bytes.as_slice()).ok()
}

/// Waits until the policy copy the machine keeps satisfies `holds`. The copy
/// is read once, then read again each time the box's feed reports a change,
/// so a copy kept before the read is not missed.
async fn copy_holds(
    rig: &Rig,
    feed: &mut Streaming<pb::Event>,
    what: &str,
    holds: impl Fn(&pb::PolicySnapshot) -> bool,
) {
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        if kept_copy(rig).is_some_and(|copy| holds(&copy)) {
            return;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            Ok(ended) => panic!("the box's event feed ended ({ended:?}) before its copy held {what}"),
            Err(_) => panic!("the machine's policy copy never held {what} within {LONG:?}"),
        }
    }
}

/// Whether `event` is the box's host link going down.
fn host_lost_event(event: &pb::Event) -> bool {
    matches!(&event.payload, Some(pb::event::Payload::HostLink(changed)) if !changed.reachable)
}

/// Waits until the box shows its host link down.
async fn host_lost(rig: &Rig, feed: &mut Streaming<pb::Event>) {
    if !common::server_info(rig.the(BOX).daemon()).await.host_reachable {
        return;
    }
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(event))) if host_lost_event(&event) => return,
            Ok(Ok(Some(_))) => {}
            Ok(ended) => panic!("the box's event feed ended ({ended:?}) before its host link went down"),
            Err(_) => panic!("the box never showed its host link down within {LONG:?}"),
        }
    }
}

/// Waits until the host's Boards feed shows an item `id` titled `title`.
async fn item_titled(feed: &mut Streaming<pb::Event>, id: &pb::BoardItemId, title: &str) -> pb::BoardItem {
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(event))) => {
                if let Some(pb::event::Payload::BoardItem(item)) = event.payload
                    && item.id.as_ref() == Some(id)
                    && item.title == title
                {
                    return item;
                }
            }
            Ok(ended) => panic!("the host's Boards feed ended ({ended:?}) before {id:?} was titled {title:?}"),
            Err(_) => panic!("the host never showed {id:?} titled {title:?} within {LONG:?}"),
        }
    }
}

/// The host's Inbox request `id`, once `ready` holds of it. The Inbox is
/// listed, then listed again each time `feed` tells of a change.
async fn host_request(
    rig: &Rig,
    id: &str,
    feed: &mut Streaming<pb::Event>,
    ready: impl Fn(&pb::InboxRequest) -> bool,
) -> pb::InboxRequest {
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        let listed = pb::inbox_service_client::InboxServiceClient::new(rig.channel().await)
            .list_requests(pb::ListRequestsRequest {
                include_answered: true,
                ..Default::default()
            })
            .await
            .expect("the host lists its Inbox")
            .into_inner()
            .requests;
        let found = listed
            .into_iter()
            .find(|request| request.id.as_ref().is_some_and(|found| found.value == id) && ready(request));
        if let Some(found) = found {
            return found;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            Ok(ended) => panic!("the host's Inbox feed ended ({ended:?}) before request {id} was ready"),
            Err(_) => panic!("the host's Inbox never showed request {id} ready within {LONG:?}"),
        }
    }
}

/// The agent's wait on request `id`, on the box, up to its last message.
async fn final_answer(rig: &Rig, token: &str, id: &pb::RequestId) -> pb::WaitResponse {
    let mut waiting = pb::inbox_service_client::InboxServiceClient::new(rig.the(BOX).daemon().channel().await)
        .wait_for_answer(as_agent(
            token,
            pb::WaitForAnswerRequest {
                id: Some(id.clone()),
                wait_seconds: 60,
            },
        ))
        .await
        .expect("the wait opens")
        .into_inner();
    tokio::time::timeout(LONG, async {
        loop {
            match waiting.message().await.expect("the wait goes on") {
                Some(message) if message.keepalive => {}
                Some(message) => return message,
                None => panic!("the wait ended without an answer"),
            }
        }
    })
    .await
    .expect("the wait was answered")
}

/// A host and a box with an agent's item, the user's rule for the agent's
/// own board asking, and the box's host link cut and seen to be.
struct Away {
    rig: Rig,
    session: pb::SessionId,
    token: String,
    item: pb::BoardItem,
    id: pb::BoardItemId,
    boards: pb::board_service_client::BoardServiceClient<tonic::transport::Channel>,
}

impl Away {
    async fn start(idempotency: &str) -> Self {
        // The test reads the machine's own feed as the user.
        let rig = Rig::start_seen(&[BOX], &[REPO]).await;
        let (host, one) = (rig.host_id().await, Rig::box_id(BOX));
        let channel = rig.create_channel("Payments", &[REPO], &[host, one.clone()]).await;
        let session = rig.start_on(&channel, &one, "Ledger").await;
        rig.wait_session(&session, "live", is_live).await;
        let token = box_token(&rig, &session).await;
        let mut boards = pb::board_service_client::BoardServiceClient::new(rig.the(BOX).daemon().channel().await);
        let mut feed = box_feed(&rig).await;

        // The host is linked: the agent's new item on its own board is made
        // on the host, and the machine's copy learns it is the agent's.
        let made = boards
            .create_item(as_agent(
                &token,
                pb::CreateItemRequest {
                    idempotency_key: key(&format!("{idempotency}-create")),
                    board: Some(pb::BoardRef {
                        board: Some(pb::board_ref::Board::Session(session.clone())),
                    }),
                    title: "Fix login".to_owned(),
                    ..Default::default()
                },
            ))
            .await
            .expect("a linked machine's board call is made on the host")
            .into_inner();
        let item = made.item.expect("the host made the item");
        let id = item.id.clone().expect("the item has an id");
        let listed = id.value.clone();
        copy_holds(&rig, &mut feed, "the agent's item", |copy| {
            copy.owned_items
                .iter()
                .any(|owned| owned.item.as_ref().is_some_and(|item| item.value == listed))
        })
        .await;

        // The user sets the approval passphrase and makes the rule for the
        // agent's own board ask; the machine takes both in its copy.
        let mut access = pb::access_service_client::AccessServiceClient::new(rig.channel().await);
        access
            .set_approval_passphrase(pb::SetApprovalPassphraseRequest {
                passphrase: PASSPHRASE.to_owned(),
                ..Default::default()
            })
            .await
            .expect("the approval passphrase is set");
        access
            .set_channel_rule(pb::SetChannelRuleRequest {
                channel: channel.id.clone(),
                capability: pb::Capability::OwnBoardWrite as i32,
                decision: pb::Decision::Ask as i32,
                ..Default::default()
            })
            .await
            .expect("the channel's rule is set");
        let channel_id = channel.id.clone().map(|id| id.value).unwrap_or_default();
        copy_holds(&rig, &mut feed, "the Ask rule and the passphrase", |copy| {
            !copy.approval_verifier.is_empty()
                && copy.channels.iter().any(|rules| {
                    rules.channel.as_ref().is_some_and(|found| found.value == channel_id)
                        && rules.rules.iter().any(|rule| {
                            rule.capability == pb::Capability::OwnBoardWrite as i32
                                && rule.decision == pb::Decision::Ask as i32
                        })
                })
        })
        .await;

        // The host goes away, and the box shows it.
        rig.link_lost(BOX).await;
        host_lost(&rig, &mut feed).await;
        Self {
            rig,
            session,
            token,
            item,
            id,
            boards,
        }
    }

    /// The agent edits its item's title, and is told to wait on the user.
    async fn ask_to_retitle(&mut self, idempotency: &str) -> pb::RequestId {
        let edit = pb::UpdateItemRequest {
            idempotency_key: key(&format!("{idempotency}-edit")),
            id: Some(self.id.clone()),
            patch: Some(pb::ItemPatch {
                title: Some(TITLE.to_owned()),
                ..Default::default()
            }),
            expected_revision: self.item.revision.clone(),
            ..Default::default()
        };
        let failed = self
            .boards
            .update_item(as_agent(&self.token, edit))
            .await
            .expect_err("the box asks the user about the edit while its host is away");
        match common::error_of(&failed).kind {
            Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.expect("it names the request"),
            other => panic!("the edit should wait on a request, not {other:?}: {failed:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_approved_on_the_box_while_the_host_is_away_is_made_when_the_link_is_back() {
    let mut away = Away::start("approved-on-box").await;
    let id = away.ask_to_retitle("approved-on-box").await;

    // The user logs in on the box with the passphrase and approves it.
    let mut terminal = Terminal::open(away.rig.the(BOX).daemon());
    assert_eq!(terminal.log_in(PASSPHRASE).await, OK);
    let approve = format!(r#""$HUMATA" --json inbox approve {} > /dev/null 2>&1"#, id.value);
    assert_eq!(terminal.exit_code(&approve, None).await, OK);
    let answered = final_answer(&away.rig, &away.token, &id).await;
    assert_eq!(answered.state(), pb::RequestState::Approved);
    let told = pb::WriteItemResponse::decode(answered.result.as_slice()).expect("the wait carries the write's answer");
    assert_eq!(told.delivery(), pb::Delivery::Journaled);

    // The link comes back: the host takes the answer, then the write.
    let mut items = host_feed(&away.rig, pb::Topic::Boards).await;
    let mut inbox = host_feed(&away.rig, pb::Topic::Inbox).await;
    away.rig.link_back(BOX).await;
    let changed = item_titled(&mut items, &away.id, TITLE).await;
    assert_eq!(
        changed.revision.map(|revision| revision.value),
        away.item.revision.as_ref().map(|revision| revision.value + 1)
    );
    let mirror = host_request(&away.rig, &id.value, &mut inbox, |request| {
        request.state() == pb::RequestState::Approved
    })
    .await;
    let by = mirror.answered.and_then(|answered| answered.by).and_then(|actor| actor.actor);
    match by {
        Some(pb::actor::Actor::User(user)) => {
            assert_eq!(user.via(), pb::user_actor::Via::CliOnMachine);
            assert_eq!(user.machine, Some(Rig::box_id(BOX)));
        }
        other => panic!("the approval is credited to the user on the box, not {other:?}"),
    }
    let shown = away.rig.session(&away.session).await;
    assert_eq!(
        shown.away_left_out, None,
        "an approved write is not one the host left out"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mirror_approved_on_the_host_after_the_link_is_back_makes_the_edit() {
    let mut away = Away::start("approved-on-host").await;
    let id = away.ask_to_retitle("approved-on-host").await;

    let mut items = host_feed(&away.rig, pb::Topic::Boards).await;
    let mut inbox = host_feed(&away.rig, pb::Topic::Inbox).await;
    away.rig.link_back(BOX).await;
    host_request(&away.rig, &id.value, &mut inbox, |request| {
        request.state() == pb::RequestState::Pending
    })
    .await;

    // The host's user approves the mirror: the answer goes down to the box,
    // which journals the write with the approval, and the host applies it.
    let settled = pb::inbox_service_client::InboxServiceClient::new(away.rig.channel().await)
        .answer_request(pb::AnswerRequestRequest {
            id: Some(id.clone()),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            ..Default::default()
        })
        .await
        .expect("the host's answer reaches the box")
        .into_inner();
    assert_eq!(settled.state(), pb::RequestState::Approved);
    let changed = item_titled(&mut items, &away.id, TITLE).await;
    assert_eq!(
        changed.revision.map(|revision| revision.value),
        away.item.revision.as_ref().map(|revision| revision.value + 1)
    );
    host_request(&away.rig, &id.value, &mut inbox, |request| {
        request.state() == pb::RequestState::Approved
    })
    .await;
}
