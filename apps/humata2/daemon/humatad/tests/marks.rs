//! Pins and VIP marks on the host (NV-20, NV-21, NV-24, NV-25; D170)
//! through a real `humatad2`: MarkService's three calls, what a restart
//! keeps, what the change feed tells (TOPIC_MARKS) and who may change a
//! mark. Needs FIND T4a's seams (the service registered, the feed's marks
//! filter); moves up from tests/pending with them.

use humatad_test_common as common;

use common::{
    Daemon, WAIT, error_of, folder, get_session, invalid_field, machine_id, shell_token, state_folder,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use pb::entity_ref::Entity;
use pb::mark_service_client::MarkServiceClient;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request, Status, Streaming};

const CHANNEL: &str = "ch_marks";
const ALPHA: &str = "se_alpha";
const BETA: &str = "se_beta";
const GAMMA: &str = "se_gamma";
/// The sessions every daemon here starts with, and their titles.
const SESSIONS: [(&str, &str); 3] = [
    (ALPHA, "Fix login"),
    (BETA, "Rate limiter"),
    (GAMMA, "Docs pass"),
];
const TOKEN_HEADER: &str = "humata-session-token";

/// A daemon that knows the three sessions, with nothing marked.
async fn start() -> Daemon {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    for (id, title) in SESSIONS {
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
    Daemon::start_in(dir).await
}

fn session(id: &str) -> pb::EntityRef {
    pb::EntityRef {
        entity: Some(Entity::Session(pb::SessionId {
            value: id.to_owned(),
        })),
    }
}

fn machine(id: &pb::MachineId) -> pb::EntityRef {
    pb::EntityRef {
        entity: Some(Entity::Machine(id.clone())),
    }
}

async fn client(daemon: &Daemon) -> MarkServiceClient<Channel> {
    MarkServiceClient::new(daemon.channel().await)
}

fn set_request(entity: &pb::EntityRef, pinned: Option<bool>, vip: Option<bool>) -> pb::SetMarksRequest {
    pb::SetMarksRequest {
        idempotency_key: None,
        entity: Some(entity.clone()),
        pinned,
        vip,
    }
}

fn reorder_request(order: &[pb::EntityRef], expected: Option<u64>) -> pb::ReorderPinnedRequest {
    pb::ReorderPinnedRequest {
        idempotency_key: None,
        order: order.to_vec(),
        expected_revision: expected.map(|value| pb::Revision { value }),
    }
}

/// The user sets `entity`'s marks; `None` leaves a flag as it is.
async fn set(daemon: &Daemon, entity: &pb::EntityRef, pinned: Option<bool>, vip: Option<bool>) -> pb::Marks {
    client(daemon)
        .await
        .set_marks(set_request(entity, pinned, vip))
        .await
        .unwrap()
        .into_inner()
}

/// `entity`'s marks as they are: a set that names no flag changes nothing.
async fn marks_of(daemon: &Daemon, entity: &pb::EntityRef) -> pb::Marks {
    set(daemon, entity, None, None).await
}

async fn pin(daemon: &Daemon, entity: &pb::EntityRef) -> pb::Marks {
    set(daemon, entity, Some(true), None).await
}

async fn list(daemon: &Daemon) -> pb::ListPinnedResponse {
    client(daemon)
        .await
        .list_pinned(pb::ListPinnedRequest {})
        .await
        .unwrap()
        .into_inner()
}

async fn reorder(
    daemon: &Daemon,
    order: &[pb::EntityRef],
    expected: Option<u64>,
) -> Result<pb::ListPinnedResponse, Status> {
    client(daemon)
        .await
        .reorder_pinned(reorder_request(order, expected))
        .await
        .map(tonic::Response::into_inner)
}

/// A revision as a number; one never set counts as 0.
fn number(revision: &Option<pb::Revision>) -> u64 {
    revision.as_ref().map_or(0, |revision| revision.value)
}

/// The pinned things of `response`, in its order.
fn order_of(response: &pb::ListPinnedResponse) -> Vec<pb::EntityRef> {
    response
        .entries
        .iter()
        .map(|entry| entry.entity.clone().expect("a pinned entry names its thing"))
        .collect()
}

/// The pinned things in order, each with its VIP flag.
fn flags_of(response: &pb::ListPinnedResponse) -> Vec<(pb::EntityRef, bool)> {
    response
        .entries
        .iter()
        .map(|entry| (entry.entity.clone().expect("a pinned entry names its thing"), entry.vip))
        .collect()
}

/// What a client keeps of the list: the pinned things in order, each with
/// its VIP flag, and the revision.
fn kept(response: &pb::ListPinnedResponse) -> (Vec<(pb::EntityRef, bool)>, u64) {
    (flags_of(response), number(&response.revision))
}

/// A request that carries `token`, an agent's identity.
fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    request
}

async fn subscribe(daemon: &Daemon, topics: &[pb::Topic]) -> Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: topics.iter().map(|topic| *topic as i32).collect(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
}

/// The next event of a feed that asked for marks only, within `WAIT`. It
/// must be a marks event: the feed was asked for nothing else. The one
/// `Resync` a start sends every feed once it has brought things back
/// (D341) is passed over, since it can land after the subscribe.
async fn next_marks(events: &mut Streaming<pb::Event>) -> (pb::Event, pb::Marks) {
    tokio::time::timeout(WAIT, async {
        loop {
            let event = events.message().await.unwrap().expect("the feed ended");
            match event.payload.clone() {
                Some(pb::event::Payload::Marks(marks)) => return (event, marks),
                Some(pb::event::Payload::Resync(_)) => continue,
                other => panic!("a feed narrowed to marks sent {other:?}"),
            }
        }
    })
    .await
    .expect("no marks event arrived")
}

/// The next marks event of a feed that carries everything, within `WAIT`.
async fn marks_among(events: &mut Streaming<pb::Event>) -> pb::Marks {
    tokio::time::timeout(WAIT, async {
        loop {
            let event = events.message().await.unwrap().expect("the feed ended");
            if let Some(pb::event::Payload::Marks(marks)) = event.payload {
                return marks;
            }
        }
    })
    .await
    .expect("no marks event arrived")
}

#[track_caller]
fn assert_user_only(refused: &Status) {
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    match error_of(refused).kind {
        Some(pb::error::Kind::UserOnly(only)) => {
            assert_eq!(only.capability(), pb::Capability::MarksChange);
        }
        other => panic!("expected user_only, got {other:?}"),
    }
}

/// `refused` says the request's `field` is wrong.
#[track_caller]
fn assert_bad_field(refused: &Status, field: &str) {
    let (named, _) = invalid_field(refused);
    assert!(
        named.starts_with(field),
        "the failure names {named:?}, not {field:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_pinned_or_vip_at_first() {
    let daemon = start().await;

    let listed = list(&daemon).await;
    assert!(listed.entries.is_empty(), "{listed:?}");

    let unmarked = marks_of(&daemon, &session(ALPHA)).await;
    assert_eq!(unmarked.entity, Some(session(ALPHA)));
    assert!(!unmarked.pinned);
    assert!(!unmarked.vip);
    // Reading changed nothing.
    assert!(list(&daemon).await.entries.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_pin_goes_at_the_end() {
    let daemon = start().await;

    for id in [BETA, ALPHA, GAMMA] {
        let pinned = pin(&daemon, &session(id)).await;
        assert_eq!(pinned.entity, Some(session(id)));
        assert!(pinned.pinned);
        assert!(!pinned.vip);
    }
    let order = [session(BETA), session(ALPHA), session(GAMMA)];
    assert_eq!(order_of(&list(&daemon).await), order);

    // Pinning what is pinned already doesn't move it.
    assert!(pin(&daemon, &session(BETA)).await.pinned);
    assert_eq!(order_of(&list(&daemon).await), order);
}

#[tokio::test(flavor = "multi_thread")]
async fn unpinning_keeps_the_rest_in_order_and_a_pin_put_back_goes_last() {
    let daemon = start().await;
    for id in [ALPHA, BETA, GAMMA] {
        pin(&daemon, &session(id)).await;
    }

    let unpinned = set(&daemon, &session(BETA), Some(false), None).await;
    assert!(!unpinned.pinned);
    assert_eq!(
        order_of(&list(&daemon).await),
        [session(ALPHA), session(GAMMA)]
    );

    pin(&daemon, &session(BETA)).await;
    assert_eq!(
        order_of(&list(&daemon).await),
        [session(ALPHA), session(GAMMA), session(BETA)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn vip_and_pinned_are_set_apart() {
    let daemon = start().await;

    // VIP alone doesn't pin.
    let vip = set(&daemon, &session(ALPHA), None, Some(true)).await;
    assert!(vip.vip);
    assert!(!vip.pinned);
    assert!(list(&daemon).await.entries.is_empty());

    // Pinning leaves VIP as it was, on and off.
    let both = pin(&daemon, &session(ALPHA)).await;
    assert!(both.pinned && both.vip, "{both:?}");
    let plain = pin(&daemon, &session(BETA)).await;
    assert!(plain.pinned && !plain.vip, "{plain:?}");
    assert_eq!(
        flags_of(&list(&daemon).await),
        [(session(ALPHA), true), (session(BETA), false)]
    );

    // Unpinning keeps VIP, and clearing VIP keeps the pin and its place.
    let unpinned = set(&daemon, &session(ALPHA), Some(false), None).await;
    assert!(!unpinned.pinned && unpinned.vip, "{unpinned:?}");
    set(&daemon, &session(BETA), None, Some(true)).await;
    let cleared = set(&daemon, &session(BETA), None, Some(false)).await;
    assert!(cleared.pinned && !cleared.vip, "{cleared:?}");
    assert_eq!(flags_of(&list(&daemon).await), [(session(BETA), false)]);
    assert!(marks_of(&daemon, &session(ALPHA)).await.vip);

    // Both flags in one call.
    let both = set(&daemon, &session(GAMMA), Some(true), Some(true)).await;
    assert!(both.pinned && both.vip, "{both:?}");
    assert_eq!(
        flags_of(&list(&daemon).await),
        [(session(BETA), false), (session(GAMMA), true)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_revision_rises_with_a_change_and_only_then() {
    let daemon = start().await;
    let fresh = number(&list(&daemon).await.revision);

    let pinned = pin(&daemon, &session(ALPHA)).await;
    let first = number(&pinned.revision);
    assert!(first > fresh, "{first} after {fresh}");
    assert_eq!(number(&list(&daemon).await.revision), first);

    // Calls that change nothing: a pin already there, a set naming no
    // flag, unpinning what was never pinned, the order as it is.
    assert_eq!(number(&pin(&daemon, &session(ALPHA)).await.revision), first);
    assert_eq!(number(&marks_of(&daemon, &session(ALPHA)).await.revision), first);
    set(&daemon, &session(BETA), Some(false), Some(false)).await;
    let same = reorder(&daemon, &[session(ALPHA)], Some(first)).await.unwrap();
    assert_eq!(number(&same.revision), first);
    assert_eq!(number(&list(&daemon).await.revision), first);

    let vip = set(&daemon, &session(ALPHA), None, Some(true)).await;
    let second = number(&vip.revision);
    assert!(second > first, "{second} after {first}");

    pin(&daemon, &session(BETA)).await;
    let third = number(&list(&daemon).await.revision);
    assert!(third > second, "{third} after {second}");
    let moved = reorder(&daemon, &[session(BETA), session(ALPHA)], Some(third))
        .await
        .unwrap();
    assert!(number(&moved.revision) > third, "{moved:?}");
    assert_eq!(number(&list(&daemon).await.revision), number(&moved.revision));
}

/// NV-20's five kinds. The ids of the channel, the PR review and the
/// board item name nothing the daemon knows: a mark is kept by kind and
/// id, and the daemon doesn't look the thing up first.
#[tokio::test(flavor = "multi_thread")]
async fn each_of_the_five_kinds_is_pinned_as_its_own_kind() {
    let daemon = start().await;
    let host = machine_id(&daemon).await;
    let things = [
        session(ALPHA),
        pb::EntityRef {
            entity: Some(Entity::Channel(pb::ChannelId {
                value: CHANNEL.to_owned(),
            })),
        },
        machine(&host),
        pb::EntityRef {
            entity: Some(Entity::PrReview(pb::SessionId {
                value: "se_review".to_owned(),
            })),
        },
        pb::EntityRef {
            entity: Some(Entity::BoardItem(pb::BoardItemId {
                value: "item_1".to_owned(),
            })),
        },
    ];

    for thing in &things {
        let pinned = set(&daemon, thing, Some(true), Some(true)).await;
        assert_eq!(pinned.entity.as_ref(), Some(thing));
        assert!(pinned.pinned && pinned.vip, "{pinned:?}");
    }

    let listed = list(&daemon).await;
    assert_eq!(order_of(&listed), things);
    assert!(listed.entries.iter().all(|entry| entry.vip), "{listed:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thing_that_cant_be_marked_is_refused() {
    let daemon = start().await;
    pin(&daemon, &session(ALPHA)).await;
    let before = list(&daemon).await;
    let unmarkable = [
        Entity::Request(pb::RequestId {
            value: "rq_1".to_owned(),
        }),
        Entity::Comment(pb::CommentId {
            value: "co_1".to_owned(),
        }),
        Entity::View(pb::ViewId {
            value: "vw_1".to_owned(),
        }),
    ];
    let mut marks = client(&daemon).await;

    for entity in unmarkable {
        let entity = pb::EntityRef {
            entity: Some(entity),
        };
        let refused = marks
            .set_marks(set_request(&entity, Some(true), Some(true)))
            .await
            .unwrap_err();
        assert_bad_field(&refused, "entity");
    }
    // No thing at all, and a reference that names none.
    let refused = marks
        .set_marks(pb::SetMarksRequest {
            pinned: Some(true),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_bad_field(&refused, "entity");
    let refused = marks
        .set_marks(set_request(&pb::EntityRef { entity: None }, Some(true), None))
        .await
        .unwrap_err();
    assert_bad_field(&refused, "entity");

    assert_eq!(kept(&list(&daemon).await), kept(&before));
}

#[tokio::test(flavor = "multi_thread")]
async fn pinned_things_take_the_order_the_user_gives() {
    let daemon = start().await;
    let host = machine(&machine_id(&daemon).await);
    pin(&daemon, &session(ALPHA)).await;
    set(&daemon, &session(BETA), Some(true), Some(true)).await;
    pin(&daemon, &host).await;
    let read = list(&daemon).await;

    // Made against the revision just read.
    let order = [host.clone(), session(ALPHA), session(BETA)];
    let moved = reorder(&daemon, &order, Some(number(&read.revision)))
        .await
        .unwrap();
    assert_eq!(order_of(&moved), order);
    assert!(number(&moved.revision) > number(&read.revision));
    // A VIP mark moves with its thing, and the list read again agrees.
    let flags = [
        (host.clone(), false),
        (session(ALPHA), false),
        (session(BETA), true),
    ];
    assert_eq!(flags_of(&moved), flags);
    let listed = list(&daemon).await;
    assert_eq!(flags_of(&listed), flags);
    assert_eq!(listed.revision, moved.revision);

    // With no revision named, the order is applied to what is there now
    // (GRPC.md §5.3).
    let order = [session(BETA), host.clone(), session(ALPHA)];
    let moved = reorder(&daemon, &order, None).await.unwrap();
    assert_eq!(order_of(&moved), order);
    assert_eq!(order_of(&list(&daemon).await), order);

    // A pin made after a reorder still goes at the end.
    pin(&daemon, &session(GAMMA)).await;
    assert_eq!(
        order_of(&list(&daemon).await),
        [session(BETA), host, session(ALPHA), session(GAMMA)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reorder_made_against_a_list_that_changed_is_refused_and_changes_nothing() {
    let daemon = start().await;
    pin(&daemon, &session(ALPHA)).await;
    pin(&daemon, &session(BETA)).await;
    let read = list(&daemon).await;
    // The list changes under the client that read it.
    pin(&daemon, &session(GAMMA)).await;
    let now = list(&daemon).await;

    // An order that was right for the list as read, and one that would be
    // right for the list as it is now: the revision alone refuses both.
    for order in [
        vec![session(BETA), session(ALPHA)],
        vec![session(GAMMA), session(BETA), session(ALPHA)],
    ] {
        let refused = reorder(&daemon, &order, Some(number(&read.revision)))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::Aborted, "{refused:?}");
        match error_of(&refused).kind {
            Some(pb::error::Kind::RevisionConflict(conflict)) => {
                // It carries the revision to read again at.
                assert_eq!(number(&conflict.actual), number(&now.revision));
            }
            other => panic!("expected revision_conflict, got {other:?}"),
        }
        assert_eq!(kept(&list(&daemon).await), kept(&now));
    }

    // Reading again and asking against what was read works.
    let order = [session(GAMMA), session(BETA), session(ALPHA)];
    let moved = reorder(&daemon, &order, Some(number(&now.revision)))
        .await
        .unwrap();
    assert_eq!(order_of(&moved), order);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reorder_that_doesnt_name_every_pin_once_is_refused_and_changes_nothing() {
    let daemon = start().await;
    for id in [ALPHA, BETA, GAMMA] {
        pin(&daemon, &session(id)).await;
    }
    // VIP, but not pinned: it has no place in the order.
    let host = machine(&machine_id(&daemon).await);
    set(&daemon, &host, None, Some(true)).await;
    let before = list(&daemon).await;
    let current = number(&before.revision);

    let wrong = [
        // A pin named twice, with every pin there.
        vec![session(ALPHA), session(BETA), session(GAMMA), session(ALPHA)],
        // A pin named twice in place of another.
        vec![session(ALPHA), session(ALPHA), session(GAMMA)],
        // A pin left out.
        vec![session(GAMMA), session(ALPHA)],
        // A thing that isn't pinned.
        vec![session(ALPHA), session(BETA), session(GAMMA), host.clone()],
        // Nothing, while things are pinned.
        vec![],
    ];
    for order in wrong {
        // Against the current revision and against none: the order is
        // what's wrong.
        for expected in [Some(current), None] {
            let refused = reorder(&daemon, &order, expected).await.unwrap_err();
            assert_bad_field(&refused, "order");
            assert_eq!(kept(&list(&daemon).await), kept(&before), "after {order:?}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_sessions_pin_is_gone_from_the_pinned_list() {
    let mut daemon = start().await;
    pin(&daemon, &session(ALPHA)).await;
    pin(&daemon, &session(BETA)).await;

    // ALPHA is deleted while the daemon is down.
    daemon.kill();
    let store = Store::open(daemon.state.join("store")).unwrap();
    store.remove_session(ALPHA).unwrap();
    drop(store);
    daemon.restart().await;

    let titles: Vec<String> = list(&daemon)
        .await
        .entries
        .into_iter()
        .map(|entry| entry.title)
        .collect();
    assert_eq!(titles, ["Rate limiter"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn marks_and_their_order_survive_a_restart() {
    let mut daemon = start().await;
    pin(&daemon, &session(ALPHA)).await;
    set(&daemon, &session(BETA), Some(true), Some(true)).await;
    // VIP only.
    set(&daemon, &session(GAMMA), None, Some(true)).await;
    reorder(&daemon, &[session(BETA), session(ALPHA)], None)
        .await
        .unwrap();
    let before = list(&daemon).await;
    assert_eq!(
        flags_of(&before),
        [(session(BETA), true), (session(ALPHA), false)]
    );

    // Killed, as a crash would, and started on the same state.
    daemon.restart().await;

    assert_eq!(kept(&list(&daemon).await), kept(&before));
    let vip_only = marks_of(&daemon, &session(GAMMA)).await;
    assert!(vip_only.vip && !vip_only.pinned, "{vip_only:?}");

    // The revision goes on from where it was, so a list read before the
    // restart is not taken for a newer one.
    let pinned = pin(&daemon, &session(GAMMA)).await;
    assert!(number(&pinned.revision) > number(&before.revision));
    assert_eq!(
        flags_of(&list(&daemon).await),
        [
            (session(BETA), true),
            (session(ALPHA), false),
            (session(GAMMA), true)
        ]
    );
}

/// NV-24: the app and `humata pinned` are two clients of the one host.
#[tokio::test(flavor = "multi_thread")]
async fn two_clients_see_the_same_pins_in_the_same_order() {
    let daemon = start().await;
    let mut one = client(&daemon).await;
    let mut two = client(&daemon).await;

    for id in [ALPHA, BETA] {
        one.set_marks(set_request(&session(id), Some(true), None))
            .await
            .unwrap();
    }
    let seen = two
        .list_pinned(pb::ListPinnedRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(order_of(&seen), [session(ALPHA), session(BETA)]);

    let order = [session(BETA), session(ALPHA)];
    two.reorder_pinned(reorder_request(&order, Some(number(&seen.revision))))
        .await
        .unwrap();
    let seen = one
        .list_pinned(pb::ListPinnedRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(order_of(&seen), order);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_change_reaches_a_subscriber_and_a_call_that_changes_nothing_sends_none() {
    let daemon = start().await;
    let mut only_marks = subscribe(&daemon, &[pb::Topic::Marks]).await;
    // A feed that asked for no topic gets everything the user may see.
    let mut everything = subscribe(&daemon, &[]).await;

    // A pin: the event is the thing's marks as the call returned them,
    // made by the user.
    let pinned = pin(&daemon, &session(ALPHA)).await;
    let (event, told) = next_marks(&mut only_marks).await;
    assert_eq!(told, pinned);
    assert!(
        matches!(
            event.by.as_ref().and_then(|by| by.actor.as_ref()),
            Some(pb::actor::Actor::User(_))
        ),
        "{event:?}"
    );
    assert_eq!(marks_among(&mut everything).await, pinned);

    // A call that changes nothing, then a VIP mark on a thing that isn't
    // pinned: the next event is the VIP mark's.
    pin(&daemon, &session(ALPHA)).await;
    marks_of(&daemon, &session(GAMMA)).await;
    let vip = set(&daemon, &session(BETA), None, Some(true)).await;
    assert_eq!(next_marks(&mut only_marks).await.1, vip);
    assert_eq!(marks_among(&mut everything).await, vip);

    // Clearing a mark is a change too, down to a thing with no mark left.
    let cleared = set(&daemon, &session(BETA), None, Some(false)).await;
    assert!(!cleared.pinned && !cleared.vip, "{cleared:?}");
    assert_eq!(next_marks(&mut only_marks).await.1, cleared);
    let unpinned = set(&daemon, &session(ALPHA), Some(false), None).await;
    assert_eq!(next_marks(&mut only_marks).await.1, unpinned);
}

/// Which thing a reorder's event names is the daemon's to choose: its
/// revision is what tells a client that the list it shows is out of date.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_order_reaches_a_subscriber_and_a_refused_one_sends_nothing() {
    let daemon = start().await;
    pin(&daemon, &session(ALPHA)).await;
    let stale = number(&list(&daemon).await.revision);
    pin(&daemon, &session(BETA)).await;
    let current = number(&list(&daemon).await.revision);
    let mut events = subscribe(&daemon, &[pb::Topic::Marks]).await;

    // Refused twice, then the order as it is: nothing to tell.
    reorder(&daemon, &[session(BETA), session(ALPHA)], Some(stale))
        .await
        .unwrap_err();
    reorder(&daemon, &[session(BETA)], Some(current))
        .await
        .unwrap_err();
    reorder(&daemon, &[session(ALPHA), session(BETA)], Some(current))
        .await
        .unwrap();

    let moved = reorder(&daemon, &[session(BETA), session(ALPHA)], Some(current))
        .await
        .unwrap();
    assert!(number(&moved.revision) > current);
    let (_, told) = next_marks(&mut events).await;
    assert_eq!(number(&told.revision), number(&moved.revision));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_can_neither_mark_nor_reorder() {
    let daemon = start().await;
    // The agent of ALPHA: the token its session's shell was given.
    let (token, _attach) = shell_token(
        &daemon,
        &pb::SessionId {
            value: ALPHA.to_owned(),
        },
    )
    .await;
    pin(&daemon, &session(ALPHA)).await;
    pin(&daemon, &session(BETA)).await;
    let before = list(&daemon).await;
    let mut events = subscribe(&daemon, &[pb::Topic::Marks]).await;
    let mut marks = client(&daemon).await;

    // Setting, on another session and on its own, and clearing.
    for request in [
        set_request(&session(GAMMA), Some(true), None),
        set_request(&session(ALPHA), None, Some(true)),
        set_request(&session(BETA), Some(false), None),
    ] {
        let refused = marks
            .set_marks(as_agent(request, &token))
            .await
            .unwrap_err();
        assert_user_only(&refused);
    }
    let refused = marks
        .reorder_pinned(as_agent(
            reorder_request(&[session(BETA), session(ALPHA)], None),
            &token,
        ))
        .await
        .unwrap_err();
    assert_user_only(&refused);

    // Nothing changed, and nothing was told: the next event is the
    // user's own change.
    assert_eq!(kept(&list(&daemon).await), kept(&before));
    assert!(!marks_of(&daemon, &session(ALPHA)).await.vip);
    let by_user = pin(&daemon, &session(GAMMA)).await;
    assert_eq!(next_marks(&mut events).await.1, by_user);
}

/// NV-21: the Pinned section shows each thing by its name, so the list
/// carries it. Sessions only here; the other kinds' names come with the
/// lanes that keep them.
#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_session_is_listed_with_its_title() {
    let daemon = start().await;
    pin(&daemon, &session(BETA)).await;
    pin(&daemon, &session(ALPHA)).await;

    let listed = list(&daemon).await;
    let titles: Vec<&str> = listed
        .entries
        .iter()
        .map(|entry| entry.title.as_str())
        .collect();
    assert_eq!(titles, ["Rate limiter", "Fix login"]);
}

fn session_id(id: &str) -> pb::SessionId {
    pb::SessionId {
        value: id.to_owned(),
    }
}

/// The next Session event for `id` that `wanted` accepts, within `WAIT`.
async fn next_session(
    events: &mut Streaming<pb::Event>,
    id: &str,
    wanted: impl Fn(&pb::Session) -> bool,
) -> pb::Session {
    tokio::time::timeout(WAIT, async {
        loop {
            let event = events.message().await.unwrap().expect("the feed ended");
            if let Some(pb::event::Payload::Session(shown)) = event.payload
                && shown.id.as_ref().map(|shown| shown.value.as_str()) == Some(id)
                && wanted(&shown)
            {
                return shown;
            }
        }
    })
    .await
    .expect("the feed never told the session with its marks")
}

/// D387: a session shows its pin and its VIP mark, each apart from the
/// other, and shows them cleared again.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_shows_its_pin_and_vip_mark() {
    let daemon = start().await;
    let id = session_id(ALPHA);
    let shown = get_session(&daemon, &id).await;
    assert!(!shown.pinned && !shown.vip);

    set(&daemon, &session(ALPHA), Some(true), None).await;
    let shown = get_session(&daemon, &id).await;
    assert!(shown.pinned && !shown.vip);

    set(&daemon, &session(ALPHA), None, Some(true)).await;
    let shown = get_session(&daemon, &id).await;
    assert!(shown.pinned && shown.vip);
    let other = get_session(&daemon, &session_id(BETA)).await;
    assert!(!other.pinned && !other.vip, "another session is not marked");

    set(&daemon, &session(ALPHA), Some(false), Some(false)).await;
    let shown = get_session(&daemon, &id).await;
    assert!(!shown.pinned && !shown.vip);
}

/// D387: a client holding the session list hears of a mark change on the
/// feed, with the session as it now shows.
#[tokio::test(flavor = "multi_thread")]
async fn a_pin_or_vip_change_is_told_as_a_session_event() {
    let daemon = start().await;
    let mut events = subscribe(&daemon, &[pb::Topic::Sessions]).await;

    set(&daemon, &session(BETA), Some(true), Some(true)).await;
    let shown = next_session(&mut events, BETA, |shown| shown.pinned && shown.vip).await;
    assert!(shown.pinned && shown.vip);

    set(&daemon, &session(BETA), Some(false), None).await;
    let shown = next_session(&mut events, BETA, |shown| !shown.pinned).await;
    assert!(shown.vip, "VIP stays when only the pin is cleared");
}

/// D387: the marks come back from the file after a restart.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_shows_its_marks_after_a_restart() {
    let mut daemon = start().await;
    set(&daemon, &session(GAMMA), Some(true), Some(true)).await;
    daemon.restart().await;
    let shown = get_session(&daemon, &session_id(GAMMA)).await;
    assert!(shown.pinned && shown.vip);
}
