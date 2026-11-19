//! A write the host's rules no longer allow at replay is not applied, and
//! the session says so (AW-9, D267). A real host and a real machine behind
//! the rig's fake `ssh`: the agent edits its own item while the host is away,
//! the host's rule is set to Never meanwhile, and once the link is back the
//! item is as it was, its history keeps the edit as lost, and the host's
//! session carries the summary the app draws under the agent's terminal.
//! Nothing is typed into the agent.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use humata2_grpc::humata::v1 as pb;
use prost::Message;
use remote_rig::{LONG, Rig, is_live};
use tonic::metadata::MetadataValue;
use tonic::{Request, Streaming};

const REPO: (&str, &str) = ("acme", "api");
const BOX: &str = "box-1";
const TOKEN_HEADER: &str = "humata-session-token";

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
/// session's agent holds. The shell prints it on its terminal, and the
/// attach is read until it does.
async fn box_token(rig: &Rig, session: &pb::SessionId) -> String {
    let daemon = rig.the(BOX).daemon();
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(session.clone()),
            repo: None,
            folder: String::new(),
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
/// policy copy it keeps. Opened before the calls that can change either, so
/// no change falls between a read and the next event.
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

/// Whether the policy copy the machine has kept lists `item` as a session's
/// own.
fn kept_copy_lists(rig: &Rig, item: &str) -> bool {
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
    pb::PolicySnapshot::decode(bytes.as_slice()).is_ok_and(|copy| {
        copy.owned_items
            .iter()
            .any(|owned| owned.item.as_ref().is_some_and(|id| id.value == item))
    })
}

/// Waits until the policy copy the machine keeps lists `item` as the
/// session's own, which is what lets it journal an edit while away. The copy
/// is read once, then read again each time the box's feed reports a copy
/// kept (PolicyCopyKept), so a copy kept before the read is not missed.
async fn copy_lists(rig: &Rig, feed: &mut Streaming<pb::Event>, item: &str) {
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        if kept_copy_lists(rig, item) {
            return;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            Ok(ended) => panic!("the box's event feed ended ({ended:?}) before its copy listed {item}"),
            Err(_) => panic!("the machine's policy copy never listed {item} within {LONG:?}"),
        }
    }
}

/// Whether `event` is the box's host link going down.
fn host_lost_event(event: &pb::Event) -> bool {
    matches!(&event.payload, Some(pb::event::Payload::HostLink(changed)) if !changed.reachable)
}

/// Waits until the box shows its host link down. The feed was opened before
/// the link was cut, so the event is there to wait for; when the box already
/// shows the link down, there is nothing to wait for.
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

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_the_host_now_refuses_is_kept_as_lost_and_the_session_says_so() {
    // The test reads the machine's own feed and server info as the user.
    let mut rig = Rig::start_seen(&[BOX], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id(BOX));
    let channel = rig.create_channel("Payments", &[REPO], &[host, one.clone()]).await;
    let session = rig.start_on(&channel, &one, "Ledger").await;
    rig.wait_session(&session, "live", is_live).await;
    let token = box_token(&rig, &session).await;
    let mut boards = pb::board_service_client::BoardServiceClient::new(rig.the(BOX).daemon().channel().await);
    let mut feed = box_feed(&rig).await;

    // The host is linked: the agent's new item on its own board is made on
    // the host, and the machine's copy of the rules learns it is the agent's.
    let made = boards
        .create_item(as_agent(
            &token,
            pb::CreateItemRequest {
                idempotency_key: key("away-left-out-create"),
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
    assert_eq!(made.delivery(), pb::Delivery::Applied);
    let item = made.item.expect("the host made the item");
    let id = item.id.clone().expect("the item has an id");
    copy_lists(&rig, &mut feed, &id.value).await;

    // The host goes away, and the box shows it: then the agent's edit is
    // journaled, on the one call.
    rig.link_lost(BOX).await;
    host_lost(&rig, &mut feed).await;
    let edit = pb::UpdateItemRequest {
        idempotency_key: key("away-left-out-edit"),
        id: Some(id.clone()),
        patch: Some(pb::ItemPatch {
            title: Some("Fix login properly".to_owned()),
            ..Default::default()
        }),
        expected_revision: item.revision.clone(),
        ..Default::default()
    };
    let written = boards
        .update_item(as_agent(&token, edit))
        .await
        .expect("the box takes the edit while its host is away")
        .into_inner();
    assert_eq!(written.delivery(), pb::Delivery::Journaled, "{written:?}");

    // While it is away, the user tightens the rule.
    pb::access_service_client::AccessServiceClient::new(rig.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: channel.id.clone(),
            capability: pb::Capability::OwnBoardWrite as i32,
            decision: pb::Decision::Never as i32,
            ..Default::default()
        })
        .await
        .unwrap();
    rig.link_back(BOX).await;

    let shown = rig
        .wait_session(&session, "showing a write the host left out", |shown| {
            shown.away_left_out.as_ref().is_some_and(|left| left.count == 1)
        })
        .await;
    let left = shown.away_left_out.expect("the summary");
    assert!(left.refused);
    assert_eq!(left.items, vec![id.clone()]);

    // The item is as it was; the edit is in its history as lost.
    let mut host_boards = pb::board_service_client::BoardServiceClient::new(rig.channel().await);
    let kept = host_boards
        .get_item(pb::GetItemRequest { id: Some(id.clone()) })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(kept.title, "Fix login");
    assert_eq!(kept.revision, item.revision);
    let history = host_boards
        .get_item_history(pb::GetItemHistoryRequest {
            id: Some(id.clone()),
            page: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(history.entries.iter().any(|entry| entry.lost_conflict), "{history:?}");

    // The host restarting keeps the summary.
    rig.restart_host(&[]).await;
    let again = rig.session(&session).await;
    assert_eq!(again.away_left_out.map(|left| left.count), Some(1));

    // Nothing was typed into the agent: its conversation holds no input.
    let conversation = rig.the(BOX).conversation(&rig.the(BOX).conversation_id(&session));
    assert_eq!(std::fs::read_to_string(conversation).unwrap_or_default(), "");
}
