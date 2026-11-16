//! `humata pin|unpin|vip|unvip <thing>` and `humata pinned [reorder]`
//! (NV-20, NV-21): each sends only its own mark, the thing resolves to
//! the right kind, and reorder sends the order against a fresh revision.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use pb::entity_ref::Entity;
use pb::error::Kind;
use serde_json::json;
use support::{FakeConsole, FakeDaemon, run};

fn entity_of(entity: Entity) -> Option<pb::EntityRef> {
    Some(pb::EntityRef {
        entity: Some(entity),
    })
}

fn session_ref(value: &str) -> Option<pb::EntityRef> {
    entity_of(Entity::Session(pb::SessionId { value: value.into() }))
}

fn channel_ref(value: &str) -> Option<pb::EntityRef> {
    entity_of(Entity::Channel(pb::ChannelId { value: value.into() }))
}

fn machine_ref(value: &str) -> Option<pb::EntityRef> {
    entity_of(Entity::Machine(pb::MachineId { value: value.into() }))
}

fn item_ref(value: &str) -> Option<pb::EntityRef> {
    entity_of(Entity::BoardItem(pb::BoardItemId { value: value.into() }))
}

fn pinned_list(revision: u64, entries: Vec<pb::PinnedEntry>) -> pb::ListPinnedResponse {
    pb::ListPinnedResponse {
        entries,
        revision: Some(pb::Revision { value: revision }),
    }
}

fn pinned_entry(entity: Option<pb::EntityRef>, title: &str) -> pb::PinnedEntry {
    pb::PinnedEntry {
        entity,
        title: title.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn each_command_sends_only_its_own_mark() {
    for (word, pinned, vip) in [
        ("pin", Some(true), None),
        ("unpin", Some(false), None),
        ("vip", None, Some(true)),
        ("unvip", None, Some(false)),
    ] {
        let daemon = FakeDaemon::new();
        let console = FakeConsole::pipe();
        let exit = run(&daemon, &console, &["--json", word, "se_1"]).await;
        assert_eq!(exit, exit::OK, "{word}");
        let sent: pb::SetMarksRequest = daemon.only(rpc::marks::SET_MARKS);
        assert_eq!(sent.pinned, pinned, "{word}");
        assert_eq!(sent.vip, vip, "{word}");
        assert_eq!(sent.entity, session_ref("se_1"), "{word}");
        assert!(sent.idempotency_key.is_some(), "{word}");
    }
}

#[tokio::test]
async fn a_bare_id_resolves_by_its_prefix() {
    for (thing, expected) in [
        ("se_1", session_ref("se_1")),
        ("ch_pay", channel_ref("ch_pay")),
        ("ma_laptop", machine_ref("ma_laptop")),
        ("PAY-3", item_ref("PAY-3")),
        (
            "rq_9",
            entity_of(Entity::Request(pb::RequestId { value: "rq_9".into() })),
        ),
    ] {
        let daemon = FakeDaemon::new();
        let console = FakeConsole::pipe();
        assert_eq!(run(&daemon, &console, &["--json", "pin", thing]).await, exit::OK, "{thing}");
        let sent: pb::SetMarksRequest = daemon.only(rpc::marks::SET_MARKS);
        assert_eq!(sent.entity, expected, "{thing}");
    }
}

#[tokio::test]
async fn a_kind_prefix_and_a_name_resolve_through_the_daemon() {
    let daemon = FakeDaemon::new();
    daemon
        .reply(
            rpc::session::LIST_SESSIONS,
            &pb::ListSessionsResponse {
                sessions: vec![pb::Session {
                    id: support::session("se_7"),
                    title: "Fix login".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .reply(
            rpc::channel::LIST_CHANNELS,
            &pb::ListChannelsResponse {
                channels: vec![pb::Channel {
                    id: support::channel("ch_pay"),
                    name: "payments".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .reply(
            rpc::machine::LIST_MACHINES,
            &pb::ListMachinesResponse {
                machines: vec![pb::Machine {
                    id: support::machine("ma_laptop"),
                    name: "laptop".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
    for thing in [
        "session:fix login",
        "channel:payments",
        "machine:laptop",
        "item:GLB-4",
    ] {
        let console = FakeConsole::pipe();
        assert_eq!(run(&daemon, &console, &["--json", "vip", thing]).await, exit::OK, "{thing}");
    }
    let sent: Vec<pb::SetMarksRequest> = daemon.sent(rpc::marks::SET_MARKS);
    let entities: Vec<_> = sent.into_iter().map(|request| request.entity).collect();
    assert_eq!(
        entities,
        vec![
            session_ref("se_7"),
            channel_ref("ch_pay"),
            machine_ref("ma_laptop"),
            item_ref("GLB-4"),
        ]
    );
}

#[tokio::test]
async fn an_unknown_kind_prefix_is_a_usage_error_with_no_call() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["pin", "planet:mars"]).await, exit::USAGE);
    assert!(!daemon.called(rpc::marks::SET_MARKS));
}

#[tokio::test]
async fn a_name_that_matches_nothing_is_not_found_with_no_call() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::channel::LIST_CHANNELS, &pb::ListChannelsResponse::default());
    let console = FakeConsole::pipe();
    assert_eq!(
        run(&daemon, &console, &["pin", "channel:nowhere"]).await,
        exit::NOT_FOUND
    );
    assert!(!daemon.called(rpc::marks::SET_MARKS));
}

#[tokio::test]
async fn pinned_lists_every_entry() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::marks::LIST_PINNED,
        &pinned_list(
            4,
            vec![
                pinned_entry(session_ref("se_1"), "Fix login"),
                pinned_entry(channel_ref("ch_pay"), "payments"),
            ],
        ),
    );
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "pinned"]).await, exit::OK);
    let result = &console.json()["result"];
    let titles: Vec<_> = result["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["title"].clone())
        .collect();
    assert_eq!(titles, vec![json!("Fix login"), json!("payments")]);
    assert!(!daemon.called(rpc::marks::REORDER_PINNED));
}

#[tokio::test]
async fn reorder_sends_the_order_against_a_fresh_revision() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::marks::LIST_PINNED,
        &pinned_list(
            7,
            vec![
                pinned_entry(session_ref("se_1"), "Fix login"),
                pinned_entry(channel_ref("ch_pay"), "payments"),
            ],
        ),
    );
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "pinned", "reorder", "ch_pay", "se_1", "PAY-3"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    assert_eq!(daemon.sent::<pb::ListPinnedRequest>(rpc::marks::LIST_PINNED).len(), 1);
    let sent: pb::ReorderPinnedRequest = daemon.only(rpc::marks::REORDER_PINNED);
    assert_eq!(
        sent.order.into_iter().map(Some).collect::<Vec<_>>(),
        vec![channel_ref("ch_pay"), session_ref("se_1"), item_ref("PAY-3")]
    );
    assert_eq!(sent.expected_revision, Some(pb::Revision { value: 7 }));
    assert!(sent.idempotency_key.is_some());
}

#[tokio::test]
async fn a_stale_reorder_exits_as_a_conflict() {
    let daemon = FakeDaemon::new();
    daemon
        .reply(rpc::marks::LIST_PINNED, &pinned_list(7, vec![]))
        .refuse(
            rpc::marks::REORDER_PINNED,
            Kind::RevisionConflict(Default::default()),
        );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["pinned", "reorder", "se_1"]).await;
    assert_eq!(exit, exit::CONFLICT);
}

#[tokio::test]
async fn a_refused_mark_exits_with_that_kinds_code() {
    for (kind, code) in [
        (Kind::Denied(Default::default()), exit::DENIED),
        (Kind::InvalidField(Default::default()), exit::USAGE),
    ] {
        let daemon = FakeDaemon::new();
        daemon.refuse(rpc::marks::SET_MARKS, kind);
        let console = FakeConsole::pipe();
        let exit = run(&daemon, &console, &["--json", "pin", "ch_other"]).await;
        assert_eq!(exit, code);
        assert_eq!(console.json()["ok"], false);
    }
}

#[tokio::test]
async fn a_bare_permission_denied_status_exits_as_denied() {
    let daemon = FakeDaemon::new();
    daemon.status(rpc::marks::SET_MARKS, 7);
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["unvip", "se_1"]).await, exit::DENIED);
}
