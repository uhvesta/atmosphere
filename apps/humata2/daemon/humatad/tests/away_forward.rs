//! A machine's agent calls its board while the host is linked (AWAY, D227):
//! the call goes up the Uplink as a `ForwardedCall` and is answered with the
//! host's `ForwardedResult`, and the machine's own store is left alone. A
//! real role-machine daemon and a real `serve --stdio` link; the test plays
//! the host. What the host does with such a call (its own checks, that it
//! runs as the agent under its rules) is the daemon's unit tests'.

mod common;
#[path = "common/away.rs"]
mod away;
#[path = "common/away_machine.rs"]
mod away_machine;

use away::Host;
use away_machine::{ASKER, AwayMachine, Link, kind, policy, session};
use common::WAIT;
use humata2_grpc::humata::v1 as pb;
use humata2_store::{BoardKey, Store};
use prost::Message;
use tonic::{Code, Status, Streaming};

/// The agent's own session board.
fn own_board() -> Option<pb::BoardRef> {
    Some(pb::BoardRef {
        board: Some(pb::board_ref::Board::Session(session(ASKER))),
    })
}

/// The next call the machine sends up, skipping its other messages.
async fn next_call(host: &mut Host) -> pb::ForwardedCall {
    loop {
        if let pb::machine_to_host::Message::Call(call) = host.next().await {
            return call;
        }
    }
}

fn answer(call: &pb::ForwardedCall, outcome: pb::forwarded_result::Outcome) -> pb::host_to_machine::Message {
    pb::host_to_machine::Message::Result(pb::ForwardedResult {
        call_id: call.call_id,
        outcome: Some(outcome),
    })
}

/// The machine whose host is linked, and the host's end of the link.
async fn linked() -> (AwayMachine, Host) {
    let mut machine = AwayMachine::start(policy(pb::AwayMode::Wait, &[]), Link::Reachable).await;
    let host = machine.host.take().expect("the host is linked");
    (machine, host)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_create_item_while_the_host_is_linked_goes_up_and_is_answered_by_the_host() {
    let (machine, mut host) = linked().await;
    let request = pb::CreateItemRequest {
        idempotency_key: Some(pb::IdempotencyKey {
            value: "create-1".to_owned(),
        }),
        board: own_board(),
        title: "Ship it".to_owned(),
        ..Default::default()
    };
    let mut boards = pb::board_service_client::BoardServiceClient::new(machine.daemon.channel().await);
    let call = machine.as_agent(request.clone());
    let calling = tokio::spawn(async move { boards.create_item(call).await });

    let forwarded = next_call(&mut host).await;
    assert_eq!(forwarded.method, "/humata.v1.BoardService/CreateItem");
    let Some(pb::forwarded_call::Principal::Agent(agent)) = &forwarded.principal else {
        panic!("an agent made the call: {forwarded:?}");
    };
    assert_eq!(agent.session, Some(session(ASKER)));
    assert_eq!(forwarded.idempotency_key, request.idempotency_key);
    assert_eq!(pb::CreateItemRequest::decode(forwarded.request.as_slice()).unwrap(), request);

    let response = pb::WriteItemResponse {
        delivery: pb::Delivery::Applied as i32,
        item: Some(pb::BoardItem {
            id: Some(pb::BoardItemId {
                value: "PAY-7".to_owned(),
            }),
            title: "Ship it".to_owned(),
            ..Default::default()
        }),
        ..Default::default()
    };
    host.send(answer(&forwarded, pb::forwarded_result::Outcome::Response(response.encode_to_vec())))
        .await;
    let got = tokio::time::timeout(common::WAIT, calling)
        .await
        .expect("the call was answered")
        .unwrap()
        .expect("the host's answer is the call's answer")
        .into_inner();
    assert_eq!(got, response);

    // The machine's own board took nothing: the host's store is the truth.
    let store = Store::open(machine.daemon.state.join("store")).unwrap();
    assert!(store.board_items(&[BoardKey::Session(ASKER.to_owned())]).unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_error_the_host_answers_with_is_the_agents_error() {
    let (machine, mut host) = linked().await;
    let mut boards = pb::board_service_client::BoardServiceClient::new(machine.daemon.channel().await);
    let call = machine.as_agent(pb::GetItemRequest {
        id: Some(pb::BoardItemId {
            value: "PAY-404".to_owned(),
        }),
    });
    let calling = tokio::spawn(async move { boards.get_item(call).await });

    let forwarded = next_call(&mut host).await;
    assert_eq!(forwarded.method, "/humata.v1.BoardService/GetItem");
    let error = pb::Error {
        message: "there is no board item PAY-404 here".to_owned(),
        fix: "check the id".to_owned(),
        retry: None,
        kind: Some(pb::error::Kind::NotFound(pb::NotFound {
            kind: pb::EntityKind::BoardItem as i32,
            id: "PAY-404".to_owned(),
            ..Default::default()
        })),
    };
    host.send(answer(&forwarded, pb::forwarded_result::Outcome::Error(error))).await;
    let failed = tokio::time::timeout(common::WAIT, calling)
        .await
        .expect("the call was answered")
        .unwrap()
        .expect_err("the host's refusal is the call's");
    assert_eq!(failed.code(), Code::NotFound);
    assert!(matches!(kind(&failed), Some(pb::error::Kind::NotFound(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_that_drops_while_the_call_waits_is_host_unreachable_and_retryable() {
    let (machine, mut host) = linked().await;
    let mut boards = pb::board_service_client::BoardServiceClient::new(machine.daemon.channel().await);
    let call = machine.as_agent(pb::CreateItemRequest {
        idempotency_key: Some(pb::IdempotencyKey {
            value: "create-2".to_owned(),
        }),
        board: own_board(),
        title: "Lost in transit".to_owned(),
        ..Default::default()
    });
    let calling = tokio::spawn(async move { boards.create_item(call).await });

    next_call(&mut host).await;
    // The host goes away without answering.
    drop(host);
    let failed = tokio::time::timeout(common::WAIT, calling)
        .await
        .expect("the call ended with the link")
        .unwrap()
        .expect_err("the call can't have been answered");
    assert!(matches!(kind(&failed), Some(pb::error::Kind::HostUnreachable(_))), "{failed:?}");
    let retry = common::error_of(&failed).retry.expect("the caller is told it can retry");
    assert!(retry.same_key, "the first try may have been done, so the key is kept");
    away::host_reachable_becomes(&machine.daemon, false).await;
}

// ---- Waiting on a request the host holds (D268). An agent's write the
// host's rules ask about is made on the host, so the request is in the
// host's Inbox: the machine forwards the agent's wait and withdrawal on it.

const WAIT_METHOD: &str = "/humata.v1.InboxService/WaitForAnswer";
const WITHDRAW_METHOD: &str = "/humata.v1.InboxService/WithdrawRequest";
/// The host's request for the agent's edit.
const REQUEST: &str = "rq_host_1";

fn request_id(id: &str) -> Option<pb::RequestId> {
    Some(pb::RequestId { value: id.to_owned() })
}

fn waiting_on(id: &str) -> pb::WaitForAnswerRequest {
    pb::WaitForAnswerRequest {
        id: request_id(id),
        wait_seconds: 60,
    }
}

/// The agent edits an item and the host, which it was sent to, asks the user:
/// the agent now holds the host's request id.
async fn asked_by_the_host(machine: &AwayMachine, host: &mut Host) {
    let mut boards = pb::board_service_client::BoardServiceClient::new(machine.daemon.channel().await);
    let call = machine.as_agent(pb::UpdateItemRequest {
        idempotency_key: Some(pb::IdempotencyKey {
            value: "update-1".to_owned(),
        }),
        id: Some(pb::BoardItemId {
            value: "PAY-1".to_owned(),
        }),
        ..Default::default()
    });
    let calling = tokio::spawn(async move { boards.update_item(call).await });
    let forwarded = next_call(host).await;
    assert_eq!(forwarded.method, "/humata.v1.BoardService/UpdateItem");
    let pending = pb::Error {
        message: "waiting for the user".to_owned(),
        fix: "wait on the request".to_owned(),
        retry: None,
        kind: Some(pb::error::Kind::ApprovalPending(pb::ApprovalPending {
            request: request_id(REQUEST),
        })),
    };
    host.send(answer(&forwarded, pb::forwarded_result::Outcome::Error(pending))).await;
    let failed = tokio::time::timeout(WAIT, calling)
        .await
        .expect("the call was answered")
        .unwrap()
        .expect_err("the host asked the user");
    assert!(matches!(kind(&failed), Some(pb::error::Kind::ApprovalPending(_))));
}

/// The agent's wait on `id`, as the machine's daemon serves it.
async fn wait_on(machine: &AwayMachine, id: &str) -> Result<Streaming<pb::WaitResponse>, Status> {
    machine
        .inbox()
        .await
        .wait_for_answer(machine.as_agent(waiting_on(id)))
        .await
        .map(tonic::Response::into_inner)
}

/// The wait's last message, past its keepalives.
async fn final_answer(waiting: &mut Streaming<pb::WaitResponse>) -> Result<pb::WaitResponse, Status> {
    tokio::time::timeout(WAIT, async {
        loop {
            match waiting.message().await? {
                Some(message) if message.keepalive => {}
                Some(message) => return Ok::<_, Status>(message),
                None => panic!("the wait ended without an answer"),
            }
        }
    })
    .await
    .expect("the wait was answered")
}

/// The next cancel the machine sends up, skipping its other messages.
async fn next_cancel(host: &mut Host) -> pb::ForwardedCallCancel {
    loop {
        if let pb::machine_to_host::Message::Cancel(cancel) = host.next().await {
            return cancel;
        }
    }
}

fn host_unreachable(failed: &Status) -> bool {
    matches!(kind(failed), Some(pb::error::Kind::HostUnreachable(_)))
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_wait_on_a_request_the_host_holds_goes_up_and_returns_the_hosts_answer() {
    let (machine, mut host) = linked().await;
    asked_by_the_host(&machine, &mut host).await;

    let mut waiting = wait_on(&machine, REQUEST).await.expect("the wait is taken");
    let forwarded = next_call(&mut host).await;
    assert_eq!(forwarded.method, WAIT_METHOD);
    let Some(pb::forwarded_call::Principal::Agent(agent)) = &forwarded.principal else {
        panic!("the agent made the call: {forwarded:?}");
    };
    assert_eq!(agent.session, Some(session(ASKER)));
    assert_eq!(
        pb::WaitForAnswerRequest::decode(forwarded.request.as_slice()).unwrap().id,
        request_id(REQUEST)
    );

    // The user approved it on the host, which ran the write.
    let written = pb::WriteItemResponse {
        delivery: pb::Delivery::Applied as i32,
        ..Default::default()
    };
    let approved = pb::WaitResponse {
        id: request_id(REQUEST),
        state: pb::RequestState::Approved as i32,
        result: written.encode_to_vec(),
        result_type: "humata.v1.WriteItemResponse".to_owned(),
        ..Default::default()
    };
    host.send(answer(&forwarded, pb::forwarded_result::Outcome::Response(approved.encode_to_vec())))
        .await;
    let got = final_answer(&mut waiting).await.expect("the host's answer is the wait's");
    assert_eq!(got.state, pb::RequestState::Approved as i32);
    assert_eq!(pb::WriteItemResponse::decode(got.result.as_slice()).unwrap(), written);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wait_on_an_id_the_host_doesnt_know_is_the_hosts_not_found() {
    let (machine, mut host) = linked().await;
    let mut waiting = wait_on(&machine, "rq_nobody").await.expect("the wait is taken");
    let forwarded = next_call(&mut host).await;
    assert_eq!(forwarded.method, WAIT_METHOD);
    let error = pb::Error {
        message: "no Inbox request rq_nobody is yours".to_owned(),
        fix: "check the request id".to_owned(),
        retry: None,
        kind: Some(pb::error::Kind::NotFound(pb::NotFound {
            kind: pb::EntityKind::Request as i32,
            id: "rq_nobody".to_owned(),
            ..Default::default()
        })),
    };
    host.send(answer(&forwarded, pb::forwarded_result::Outcome::Error(error))).await;
    let failed = final_answer(&mut waiting).await.expect_err("the host has no such request");
    assert_eq!(failed.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_that_drops_while_the_agent_waits_is_host_unreachable_and_the_wait_can_be_made_again() {
    let (machine, mut host) = linked().await;
    asked_by_the_host(&machine, &mut host).await;
    let mut waiting = wait_on(&machine, REQUEST).await.expect("the wait is taken");
    next_call(&mut host).await;
    drop(host);
    let failed = final_answer(&mut waiting).await.expect_err("the link ended the wait");
    assert!(host_unreachable(&failed), "{failed:?}");
    let retry = common::error_of(&failed).retry.expect("the agent is told it can wait again");
    assert!(retry.same_key);
    away::host_reachable_becomes(&machine.daemon, false).await;

    // Away, the request is still the host's: it can't be waited on until the link is back.
    let again = wait_on(&machine, REQUEST).await.expect_err("the host isn't reachable");
    assert!(host_unreachable(&again), "{again:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn away_a_request_the_host_holds_is_host_unreachable_and_an_unknown_one_is_not_found() {
    let (machine, mut host) = linked().await;
    asked_by_the_host(&machine, &mut host).await;
    drop(host);
    away::host_reachable_becomes(&machine.daemon, false).await;

    let held = wait_on(&machine, REQUEST).await.expect_err("the host isn't reachable");
    assert!(host_unreachable(&held), "{held:?}");
    let unknown = wait_on(&machine, "rq_nobody").await.expect_err("no such request");
    assert_eq!(unknown.code(), Code::NotFound);

    let mut inbox = machine.inbox().await;
    let withdrawn = |id: &str| pb::WithdrawRequestRequest {
        idempotency_key: None,
        id: request_id(id),
    };
    let held = inbox
        .withdraw_request(machine.as_agent(withdrawn(REQUEST)))
        .await
        .expect_err("the host isn't reachable");
    assert!(host_unreachable(&held), "{held:?}");
    let unknown = inbox
        .withdraw_request(machine.as_agent(withdrawn("rq_nobody")))
        .await
        .expect_err("no such request");
    assert_eq!(unknown.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_that_stops_waiting_has_its_forwarded_wait_cancelled() {
    let (machine, mut host) = linked().await;
    asked_by_the_host(&machine, &mut host).await;
    let waiting = wait_on(&machine, REQUEST).await.expect("the wait is taken");
    let forwarded = next_call(&mut host).await;
    assert_eq!(forwarded.method, WAIT_METHOD);
    drop(waiting);
    let cancel = tokio::time::timeout(WAIT, next_cancel(&mut host))
        .await
        .expect("the host was told");
    assert_eq!(cancel.call_id, forwarded.call_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_withdrawal_of_a_request_the_host_holds_goes_up_and_returns_the_hosts_answer() {
    let (machine, mut host) = linked().await;
    asked_by_the_host(&machine, &mut host).await;
    let mut inbox = machine.inbox().await;
    let key = pb::IdempotencyKey {
        value: "withdraw-1".to_owned(),
    };
    let call = machine.as_agent(pb::WithdrawRequestRequest {
        idempotency_key: Some(key.clone()),
        id: request_id(REQUEST),
    });
    let withdrawing = tokio::spawn(async move { inbox.withdraw_request(call).await });

    let forwarded = next_call(&mut host).await;
    assert_eq!(forwarded.method, WITHDRAW_METHOD);
    assert_eq!(forwarded.idempotency_key, Some(key));
    assert_eq!(
        pb::WithdrawRequestRequest::decode(forwarded.request.as_slice()).unwrap().id,
        request_id(REQUEST)
    );
    let withdrawn = pb::InboxRequest {
        id: request_id(REQUEST),
        state: pb::RequestState::Withdrawn as i32,
        ..Default::default()
    };
    host.send(answer(&forwarded, pb::forwarded_result::Outcome::Response(withdrawn.encode_to_vec())))
        .await;
    let got = tokio::time::timeout(WAIT, withdrawing)
        .await
        .expect("the withdrawal was answered")
        .unwrap()
        .expect("the host's answer is the withdrawal's")
        .into_inner();
    assert_eq!(got, withdrawn);
}
