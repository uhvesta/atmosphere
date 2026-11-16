//! A machine's agent uses its board while the host is away (AWAY T10c and
//! T10d, D227, AW-7 rows 2 and 3). A real role-machine daemon and a real
//! `serve --stdio` link; the test plays the agent and the host.
//!
//! Row 3: a write the rules allow and the policy copy proves the agent may
//! make is kept in the machine's journal and answered as journaled; any
//! other is host_unreachable and nothing is kept. One the rules ask about,
//! under Wait, waits as a request in the machine's Inbox and is journaled,
//! with that request's id, once the user approves it there (D268). Row 2: a read made while
//! the host was linked is answered while it is away from the host's last
//! answer, marked with its age; one never made is host_unreachable.
//! Assertions are on typed error kinds, never on wording.

#[path = "common/away.rs"]
mod away;
#[path = "common/away_machine.rs"]
mod away_machine;
mod common;

use away::{Host, OK, PASSPHRASE, Terminal, host_reachable_becomes};
use away_machine::{
    ASKER, AwayMachine, CHANNEL, Link, OTHER, board_request, kind, only, owned, policy,
    policy_owning, session,
};
use humata2_grpc::humata::v1 as pb;
use prost::Message;

/// The response header a read answered from the last copy carries.
const STALE: &str = "humata-stale-as-of";

fn channel_board() -> pb::BoardRef {
    pb::BoardRef {
        board: Some(pb::board_ref::Board::Channel(pb::ChannelId {
            value: CHANNEL.to_owned(),
        })),
    }
}

fn session_board(id: &str) -> pb::BoardRef {
    pb::BoardRef {
        board: Some(pb::board_ref::Board::Session(session(id))),
    }
}

fn item_id(key: &str) -> Option<pb::BoardItemId> {
    Some(pb::BoardItemId {
        value: key.to_owned(),
    })
}

fn key(value: &str) -> Option<pb::IdempotencyKey> {
    Some(pb::IdempotencyKey {
        value: value.to_owned(),
    })
}

async fn boards(
    machine: &AwayMachine,
) -> pb::board_service_client::BoardServiceClient<tonic::transport::Channel> {
    pb::board_service_client::BoardServiceClient::new(machine.daemon.channel().await)
}

fn add_to(board: pb::BoardRef, key_value: &str, title: &str) -> pb::CreateItemRequest {
    pb::CreateItemRequest {
        idempotency_key: key(key_value),
        board: Some(board),
        title: title.to_owned(),
        column: pb::BoardColumn::ToDo as i32,
        ..Default::default()
    }
}

fn move_to_review(id: &str, revision: u64) -> pb::MoveItemRequest {
    pb::MoveItemRequest {
        idempotency_key: key(&format!("move-{id}")),
        id: item_id(id),
        column: pb::BoardColumn::InReview as i32,
        expected_revision: Some(pb::Revision { value: revision }),
        ..Default::default()
    }
}

fn is_host_unreachable(status: &tonic::Status) -> bool {
    matches!(kind(status), Some(pb::error::Kind::HostUnreachable(_)))
}

fn is_never(status: &tonic::Status) -> bool {
    matches!(
        kind(status),
        Some(pb::error::Kind::Denied(denied)) if denied.source() == pb::denied::Source::NeverRule
    )
}

/// The channel's board as a list reads it.
fn list_channel() -> pb::ListItemsRequest {
    pb::ListItemsRequest {
        view: Some(pb::BoardView {
            view: Some(pb::board_view::View::Board(channel_board())),
        }),
        ..Default::default()
    }
}

/// The machine's agent has `PAY-7`, on its own board, at revision 3.
async fn owning() -> AwayMachine {
    let owned_items = vec![owned("PAY-7", ASKER, session_board(ASKER), 3)];
    AwayMachine::start(
        policy_owning(pb::AwayMode::Wait, &[], owned_items),
        Link::Away,
    )
    .await
}

/// What the machine journaled for the host, boards only.
async fn board_journal(machine: &AwayMachine) -> Vec<pb::JournalEntry> {
    only(machine.journal(2).await, "board.")
}

// ----- Row 3: board writes while away. -----

#[tokio::test(flavor = "multi_thread")]
async fn row_3_a_channel_board_add_while_away_is_journaled() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Away).await;
    let request = add_to(channel_board(), "aw7-add", "written from the road");
    let written = boards(&machine)
        .await
        .create_item(machine.as_agent(request.clone()))
        .await
        .expect("a write the rules allow is journaled while the host is away")
        .into_inner();
    assert_eq!(written.delivery(), pb::Delivery::Journaled);
    assert_eq!(
        written.item, None,
        "a journaled new item has no id until the host replays it"
    );
    let position = written
        .journal
        .expect("a journaled write says where it is in the journal");
    assert_eq!(
        position.machine,
        Some(common::machine_id(&machine.daemon).await),
        "the position names this machine"
    );

    let entries = board_journal(&machine).await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(
        entries[0].seq, position.seq,
        "the host is sent the write it was told of"
    );
    assert_eq!(entries[0].operation, "board.item_create");
    assert_eq!(board_request::<pb::CreateItemRequest>(&entries[0]), request);
    assert_eq!(
        entries[0]
            .principal
            .as_ref()
            .and_then(|principal| principal.session.clone()),
        Some(session(ASKER))
    );
    assert_eq!(
        entries[0].base_revision, None,
        "a new item was made against no revision"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_add_on_its_own_session_board_while_away_is_journaled() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Away).await;
    let written = boards(&machine)
        .await
        .create_item(machine.as_agent(add_to(session_board(ASKER), "own-add", "a note to self")))
        .await
        .expect("its own board takes a new item while the host is away")
        .into_inner();
    assert_eq!(written.delivery(), pb::Delivery::Journaled);
    assert_eq!(board_journal(&machine).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_of_its_own_item_while_away_is_journaled_against_the_revision_it_saw() {
    let machine = owning().await;
    let request = move_to_review("PAY-7", 3);
    let written = boards(&machine)
        .await
        .move_item(machine.as_agent(request.clone()))
        .await
        .expect("a change to an item the copy lists as its own is journaled")
        .into_inner();
    assert_eq!(written.delivery(), pb::Delivery::Journaled);
    let position = written
        .journal
        .expect("the write is said to be in the journal");

    let entries = board_journal(&machine).await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].seq, position.seq);
    assert_eq!(entries[0].operation, "board.item_move");
    assert_eq!(board_request::<pb::MoveItemRequest>(&entries[0]), request);
    assert_eq!(entries[0].base_revision, Some(pb::Revision { value: 3 }));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_write_sent_again_with_its_key_is_journaled_once() {
    let machine = owning().await;
    let request = move_to_review("PAY-7", 3);
    let mut told = Vec::new();
    for _ in 0..2 {
        let written = boards(&machine)
            .await
            .move_item(machine.as_agent(request.clone()))
            .await
            .expect("the write is journaled")
            .into_inner();
        told.push(written);
    }
    assert_eq!(told[0], told[1], "the repeat is told what the first was");
    assert_eq!(board_journal(&machine).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_item_the_copy_does_not_list_as_its_own_is_host_unreachable_and_nothing_is_kept() {
    let theirs = vec![
        owned("PAY-7", ASKER, session_board(ASKER), 3),
        owned("PAY-8", OTHER, session_board(OTHER), 1),
    ];
    let machine =
        AwayMachine::start(policy_owning(pb::AwayMode::Wait, &[], theirs), Link::Away).await;
    for id in ["PAY-8", "PAY-404"] {
        let failed = boards(&machine)
            .await
            .move_item(machine.as_agent(move_to_review(id, 1)))
            .await
            .expect_err("a change to an item that isn't its own was taken");
        assert!(is_host_unreachable(&failed), "{id}: {failed:?}");
    }
    let failed = boards(&machine)
        .await
        .delete_item(machine.as_agent(pb::DeleteItemRequest {
            idempotency_key: key("delete-8"),
            id: item_id("PAY-8"),
            ..Default::default()
        }))
        .await
        .expect_err("a delete of another session's item was taken");
    assert!(is_host_unreachable(&failed), "{failed:?}");
    let failed = boards(&machine)
        .await
        .create_item(machine.as_agent(add_to(
            session_board(OTHER),
            "other-add",
            "on someone else's board",
        )))
        .await
        .expect_err("an add on another session's board was taken");
    assert!(is_host_unreachable(&failed), "{failed:?}");

    assert!(
        board_journal(&machine).await.is_empty(),
        "a refused write leaves nothing for the host"
    );
}

// ----- Row 3, a write the rules ask about (D268). -----

/// The agent owns `PAY-7`, and the rule for changing its own board asks.
async fn owning_asking(mode: pb::AwayMode) -> AwayMachine {
    let owned_items = vec![owned("PAY-7", ASKER, session_board(ASKER), 3)];
    let asks = [(pb::Capability::OwnBoardWrite, pb::Decision::Ask)];
    AwayMachine::start(policy_owning(mode, &asks, owned_items), Link::Away).await
}

/// The request an `approval_pending` error names.
fn pending_request(status: &tonic::Status) -> pb::RequestId {
    match kind(status) {
        Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.expect("it names the request"),
        other => panic!("the call should wait on a request, not {other:?}: {status:?}"),
    }
}

/// The agent moves `PAY-7`, and is told to wait on the user's answer.
async fn ask_to_move(machine: &AwayMachine) -> pb::RequestId {
    let failed = boards(machine)
        .await
        .move_item(machine.as_agent(move_to_review("PAY-7", 3)))
        .await
        .expect_err("a write the rules ask about waits for the user");
    pending_request(&failed)
}

/// The user logs in with the approval passphrase on the machine, and
/// answers request `id` with `verb` (`approve` or `deny`).
async fn answer_on_machine(machine: &AwayMachine, verb: &str, id: &pb::RequestId) {
    let mut terminal = Terminal::open(&machine.daemon);
    assert_eq!(terminal.log_in(PASSPHRASE).await, OK);
    let line = format!(r#""$HUMATA" --json inbox {verb} {} > /dev/null 2>&1"#, id.value);
    assert_eq!(terminal.exit_code(&line, None).await, OK);
}

/// The last message of the agent's wait on request `id`, past keepalives.
async fn final_answer(machine: &AwayMachine, id: &pb::RequestId) -> pb::WaitResponse {
    let mut waiting = machine
        .inbox()
        .await
        .wait_for_answer(machine.as_agent(pb::WaitForAnswerRequest {
            id: Some(id.clone()),
            wait_seconds: 60,
        }))
        .await
        .expect("the wait opens")
        .into_inner();
    tokio::time::timeout(common::WAIT, async {
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

fn operations(entries: &[pb::JournalEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.operation.as_str()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_the_rules_ask_about_waits_in_the_machines_inbox_with_the_change_on_its_card() {
    let machine = owning_asking(pb::AwayMode::Wait).await;
    let id = ask_to_move(&machine).await;

    let card = machine
        .inbox()
        .await
        .get_request(machine.as_agent(pb::GetRequestRequest { id: Some(id.clone()) }))
        .await
        .expect("the agent's own request is on this machine")
        .into_inner();
    assert_eq!(card.state(), pb::RequestState::Pending);
    let Some(pb::request_preview::Preview::BoardChange(change)) = card.preview.and_then(|shown| shown.preview) else {
        panic!("the card shows the board change");
    };
    assert_eq!(change.item, item_id("PAY-7"));
    assert_eq!(change.changes.len(), 1, "{change:?}");
    assert_eq!(change.changes[0].after, "In review");
    assert_eq!(change.changes[0].before, "", "the host fills in the old value when it shows the card");

    // The write isn't kept until the user says yes: the host is sent the
    // request, and nothing else.
    let entries = machine.journal(2).await;
    assert_eq!(operations(&entries), ["inbox.request"], "{entries:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_the_request_on_the_machine_journals_the_write_with_its_approval_after_the_answer() {
    let machine = owning_asking(pb::AwayMode::Wait).await;
    let id = ask_to_move(&machine).await;
    answer_on_machine(&machine, "approve", &id).await;

    let answered = final_answer(&machine, &id).await;
    assert_eq!(answered.state(), pb::RequestState::Approved);
    let told = pb::WriteItemResponse::decode(answered.result.as_slice()).expect("the wait carries the write's answer");
    assert_eq!(told.delivery(), pb::Delivery::Journaled);
    let position = told.journal.expect("the write is said to be in the journal");

    let entries = machine.journal(2).await;
    assert_eq!(operations(&entries), ["inbox.request", "inbox.answer", "board.item_move"], "{entries:?}");
    assert!(
        entries[1].seq < entries[2].seq,
        "the host has the approval before it replays the write"
    );
    assert_eq!(entries[2].seq, position.seq);
    assert_eq!(entries[2].approved_request, Some(id));
    assert_eq!(board_request::<pb::MoveItemRequest>(&entries[2]), move_to_review("PAY-7", 3));
    assert_eq!(entries[2].base_revision, Some(pb::Revision { value: 3 }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_item_the_user_approves_on_the_machine_is_journaled_with_its_approval() {
    let asks = [(pb::Capability::ChannelBoardAdd, pb::Decision::Ask)];
    let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &asks), Link::Away).await;
    let failed = boards(&machine)
        .await
        .create_item(machine.as_agent(add_to(channel_board(), "ask-add", "needs the user")))
        .await
        .expect_err("a write the rules ask about waits for the user");
    let id = pending_request(&failed);
    answer_on_machine(&machine, "approve", &id).await;
    assert_eq!(final_answer(&machine, &id).await.state(), pb::RequestState::Approved);

    let entries = machine.journal(2).await;
    assert_eq!(operations(&entries), ["inbox.request", "inbox.answer", "board.item_create"], "{entries:?}");
    assert_eq!(entries[2].approved_request, Some(id));
}

#[tokio::test(flavor = "multi_thread")]
async fn denying_the_request_on_the_machine_journals_no_write() {
    let machine = owning_asking(pb::AwayMode::Wait).await;
    let id = ask_to_move(&machine).await;
    answer_on_machine(&machine, "deny", &id).await;
    assert_eq!(final_answer(&machine, &id).await.state(), pb::RequestState::Denied);

    let entries = machine.journal(2).await;
    assert_eq!(operations(&entries), ["inbox.request", "inbox.answer"], "{entries:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn under_skip_a_write_the_rules_ask_about_is_refused_as_the_user_being_away() {
    let machine = owning_asking(pb::AwayMode::Skip).await;
    let failed = boards(&machine)
        .await
        .move_item(machine.as_agent(move_to_review("PAY-7", 3)))
        .await
        .expect_err("a write the rules ask about was taken under Skip");
    assert!(matches!(kind(&failed), Some(pb::error::Kind::UserAway(_))), "{failed:?}");
    assert!(machine.journal(2).await.is_empty(), "no request, no write");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_it_may_not_journal_makes_no_request_even_when_the_rules_ask() {
    let theirs = vec![owned("PAY-8", OTHER, session_board(OTHER), 1)];
    let asks = [(pb::Capability::OwnBoardWrite, pb::Decision::Ask)];
    let machine = AwayMachine::start(policy_owning(pb::AwayMode::Wait, &asks, theirs), Link::Away).await;
    let failed = boards(&machine)
        .await
        .move_item(machine.as_agent(move_to_review("PAY-8", 1)))
        .await
        .expect_err("a change to another session's item was taken");
    assert!(is_host_unreachable(&failed), "{failed:?}");
    assert!(
        machine.journal(2).await.is_empty(),
        "nothing was kept, not even a request for the user"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_view_draft_while_away_is_host_unreachable_and_nothing_is_kept() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Away).await;
    let failed = pb::view_service_client::ViewServiceClient::new(machine.daemon.channel().await)
        .prepare_view(machine.as_agent(pb::PrepareViewRequest {
            idempotency_key: key("view-1"),
            title: "The board".to_owned(),
            summary: String::new(),
            content: Some(pb::prepare_view_request::Content::Board(
                pb::BoardViewOffer {
                    scope: Some(pb::board_view_offer::Scope::Channel(pb::ChannelId {
                        value: CHANNEL.to_owned(),
                    })),
                },
            )),
        }))
        .await
        .expect_err("a view draft was taken while the host is away");
    assert!(is_host_unreachable(&failed), "{failed:?}");
    assert!(
        machine.journal(2).await.is_empty(),
        "a refused draft leaves nothing for the host"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_items_the_copy_lists_are_still_the_agents_after_the_daemon_restarts() {
    let mut machine = owning().await;
    machine.daemon.restart().await;
    host_reachable_becomes(&machine.daemon, false).await;
    let written = boards(&machine)
        .await
        .move_item(machine.as_agent(move_to_review("PAY-7", 3)))
        .await
        .expect("the copy was kept, so its item is still the agent's")
        .into_inner();
    assert_eq!(written.delivery(), pb::Delivery::Journaled);
}

// ----- Row 2: reads while away. -----

/// The next call the machine sends up, skipping its other messages.
async fn next_call(host: &mut Host) -> pb::ForwardedCall {
    loop {
        if let pb::machine_to_host::Message::Call(call) = host.next().await {
            return call;
        }
    }
}

/// The agent reads the channel's board while the host is linked, and the
/// test, as the host, answers with `listed`.
async fn read_while_linked(
    machine: &AwayMachine,
    host: &mut Host,
    request: &pb::ListItemsRequest,
    listed: &pb::ListItemsResponse,
) {
    let mut client = boards(machine).await;
    let call = machine.as_agent(request.clone());
    let calling = tokio::spawn(async move { client.list_items(call).await });
    let forwarded = next_call(host).await;
    assert_eq!(forwarded.method, "/humata.v1.BoardService/ListItems");
    host.send(pb::host_to_machine::Message::Result(pb::ForwardedResult {
        call_id: forwarded.call_id,
        outcome: Some(pb::forwarded_result::Outcome::Response(
            listed.encode_to_vec(),
        )),
    }))
    .await;
    let live = tokio::time::timeout(common::WAIT, calling)
        .await
        .expect("the call was answered")
        .unwrap()
        .expect("the host's answer is the call's answer");
    assert!(
        live.metadata().get(STALE).is_none(),
        "an answer the host gave now carries no age"
    );
    assert_eq!(&live.into_inner(), listed);
}

fn hosts_answer() -> pb::ListItemsResponse {
    pb::ListItemsResponse {
        items: vec![pb::BoardItem {
            id: item_id("PAY-7"),
            board: Some(channel_board()),
            channel: Some(pb::ChannelId {
                value: CHANNEL.to_owned(),
            }),
            title: "Ship it".to_owned(),
            revision: Some(pb::Revision { value: 3 }),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn row_2_a_read_made_while_the_host_was_linked_is_answered_while_it_is_away_marked_with_its_age()
 {
    let mut machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Reachable).await;
    let mut host = machine.host.take().expect("the host is linked");
    let (request, listed) = (list_channel(), hosts_answer());
    read_while_linked(&machine, &mut host, &request, &listed).await;

    drop(host);
    host_reachable_becomes(&machine.daemon, false).await;
    let mut marks = Vec::new();
    for round in ["away", "away after a restart"] {
        let again = boards(&machine)
            .await
            .list_items(machine.as_agent(request.clone()))
            .await
            .unwrap_or_else(|status| {
                panic!("{round}: the read wasn't answered from the last copy: {status:?}")
            });
        let mark = again
            .metadata()
            .get(STALE)
            .unwrap_or_else(|| panic!("{round}: the answer doesn't say how old it is"))
            .to_str()
            .unwrap()
            .to_owned();
        assert!(
            mark.len() == 20 && mark.ends_with('Z'),
            "{round}: {mark} is not RFC 3339 in UTC to the second"
        );
        assert_eq!(again.into_inner(), listed, "{round}");
        marks.push(mark);
        machine.daemon.restart().await;
        host_reachable_becomes(&machine.daemon, false).await;
    }
    assert_eq!(
        marks[0], marks[1],
        "the age is when the host answered, not when it was read again"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_never_made_while_the_host_was_linked_is_host_unreachable() {
    let machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Away).await;
    let failed = boards(&machine)
        .await
        .list_items(machine.as_agent(list_channel()))
        .await
        .expect_err("a read the machine never had an answer to was answered");
    assert!(is_host_unreachable(&failed), "{failed:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_different_read_than_the_one_the_host_answered_is_not_answered_from_its_copy() {
    let mut machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Reachable).await;
    let mut host = machine.host.take().expect("the host is linked");
    read_while_linked(&machine, &mut host, &list_channel(), &hosts_answer()).await;
    drop(host);
    host_reachable_becomes(&machine.daemon, false).await;

    let smaller_page = pb::ListItemsRequest {
        page: Some(pb::PageRequest {
            page_size: 1,
            ..Default::default()
        }),
        ..list_channel()
    };
    let failed = boards(&machine)
        .await
        .list_items(machine.as_agent(smaller_page))
        .await
        .expect_err("another read was answered from a copy of this one");
    assert!(is_host_unreachable(&failed), "{failed:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_the_rules_now_say_never_is_not_served_from_the_copy() {
    let mut machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Reachable).await;
    let mut host = machine.host.take().expect("the host is linked");
    read_while_linked(&machine, &mut host, &list_channel(), &hosts_answer()).await;

    // The user turns the channel's read off; the machine takes the new copy.
    let never = policy(
        pb::AwayMode::Wait,
        &[(pb::Capability::ChannelRead, pb::Decision::Never)],
    );
    host.send_policy(2, never).await;
    host.journal_until_policy_ack(2).await;
    drop(host);
    host_reachable_becomes(&machine.daemon, false).await;

    let refused = boards(&machine)
        .await
        .list_items(machine.as_agent(list_channel()))
        .await
        .expect_err("a read that is Never now was served from the copy");
    assert!(is_never(&refused), "{refused:?}");
}
