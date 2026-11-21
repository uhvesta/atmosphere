//! A machine's agent calls its board while the host is linked (AWAY, D227):
//! the call goes up the Uplink as a `ForwardedCall` and is answered with the
//! host's `ForwardedResult`, and the machine's own store is left alone. A
//! real role-machine daemon and a real `serve --stdio` link; the test plays
//! the host. What the host does with such a call (its own checks, that it
//! runs as the agent under its rules) is the daemon's unit tests'.

use humatad_test_common as common;
use humatad_test_common::away;
use humatad_test_common::away_machine;

use away::Host;
use away_machine::{ASKER, AwayMachine, Link, OTHER, files_of, kind, policy, session};
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

// REPOS (RL-12, D568): the library stays on the host, so a machine's agent
// searches it there.

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_repo_search_on_a_machine_goes_up_and_the_hosts_answer_is_the_agents() {
    let (machine, mut host) = linked().await;
    let request = pb::SearchReposRequest {
        query: "ledger".to_owned(),
        ..Default::default()
    };
    let mut repos = pb::repo_service_client::RepoServiceClient::new(machine.daemon.channel().await);
    let call = machine.as_agent(request.clone());
    let searching = tokio::spawn(async move { repos.search_repos(call).await });

    let forwarded = next_call(&mut host).await;
    assert_eq!(forwarded.method, "/humata.v1.RepoService/SearchRepos");
    let Some(pb::forwarded_call::Principal::Agent(agent)) = &forwarded.principal else {
        panic!("an agent made the call: {forwarded:?}");
    };
    assert_eq!(agent.session, Some(session(ASKER)));
    assert_eq!(pb::SearchReposRequest::decode(forwarded.request.as_slice()).unwrap(), request);

    let found = pb::SearchReposResponse {
        library: vec![pb::LibraryRepo {
            repo: Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: "ledger".to_owned(),
            }),
            ..Default::default()
        }],
        ..Default::default()
    };
    host.send(answer(&forwarded, pb::forwarded_result::Outcome::Response(found.encode_to_vec())))
        .await;
    let got = tokio::time::timeout(WAIT, searching)
        .await
        .expect("the search was answered")
        .unwrap()
        .expect("the host's answer is the search's")
        .into_inner();
    assert_eq!(got, found);
}

#[tokio::test(flavor = "multi_thread")]
async fn away_an_agents_repo_search_is_host_unreachable_and_answers_nothing() {
    let (machine, host) = linked().await;
    drop(host);
    away::host_reachable_becomes(&machine.daemon, false).await;

    let mut repos = pb::repo_service_client::RepoServiceClient::new(machine.daemon.channel().await);
    let failed = repos
        .search_repos(machine.as_agent(pb::SearchReposRequest {
            query: "ledger".to_owned(),
            ..Default::default()
        }))
        .await
        .expect_err("the library is the host's");
    assert!(host_unreachable(&failed), "{failed:?}");
    let failed = repos
        .list_repo_library(machine.as_agent(pb::ListRepoLibraryRequest::default()))
        .await
        .expect_err("the library is the host's");
    assert!(host_unreachable(&failed), "{failed:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_never_changes_the_library_or_the_scope_itself() {
    let (machine, _host) = linked().await;
    let mut repos = pb::repo_service_client::RepoServiceClient::new(machine.daemon.channel().await);
    let removed = repos
        .remove_from_repo_library(machine.as_agent(pb::RemoveFromRepoLibraryRequest {
            repo: Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: "ledger".to_owned(),
            }),
        }))
        .await
        .expect_err("removing is the host's alone");
    assert_eq!(removed.code(), Code::FailedPrecondition);
    let saved = repos
        .update_repo_scope(machine.as_agent(pb::UpdateRepoScopeRequest::default()))
        .await
        .expect_err("saving the scope is the host's alone");
    assert_eq!(saved.code(), Code::FailedPrecondition);
}

// D768: only the host keeps machines and links them, so a machine's agent
// looks machines up, looks at other machines' files and copies between
// machines through it.

/// The machine's own id, as its daemon names itself.
async fn own_id(machine: &AwayMachine) -> pb::MachineId {
    pb::daemon_service_client::DaemonServiceClient::new(machine.daemon.channel().await)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap()
        .into_inner()
        .machine
        .expect("the daemon names its machine")
}

fn on_the_host(path: &str) -> pb::FileLocation {
    pb::FileLocation {
        machine: Some(pb::MachineId { value: "ma_host".to_owned() }),
        ..files_of(OTHER, path)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_machine_list_on_a_machine_goes_up_and_the_hosts_answer_is_the_agents() {
    let (machine, mut host) = linked().await;
    let mut machines = pb::machine_service_client::MachineServiceClient::new(machine.daemon.channel().await);
    let call = machine.as_agent(pb::ListMachinesRequest::default());
    let listing = tokio::spawn(async move { machines.list_machines(call).await });

    let forwarded = next_call(&mut host).await;
    assert_eq!(forwarded.method, "/humata.v1.MachineService/ListMachines");
    assert!(matches!(forwarded.principal, Some(pb::forwarded_call::Principal::Agent(_))));
    let listed = pb::ListMachinesResponse {
        machines: vec![pb::Machine {
            id: Some(pb::MachineId { value: "ma_host".to_owned() }),
            name: "This Mac".to_owned(),
            is_this_mac: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    host.send(answer(&forwarded, pb::forwarded_result::Outcome::Response(listed.encode_to_vec())))
        .await;
    let got = tokio::time::timeout(WAIT, listing)
        .await
        .expect("the list was answered")
        .unwrap()
        .expect("the host's answer is the agent's")
        .into_inner();
    assert_eq!(got, listed);
}

#[tokio::test(flavor = "multi_thread")]
async fn away_an_agents_machine_list_is_host_unreachable() {
    let (machine, host) = linked().await;
    drop(host);
    away::host_reachable_becomes(&machine.daemon, false).await;
    let failed = pb::machine_service_client::MachineServiceClient::new(machine.daemon.channel().await)
        .list_machines(machine.as_agent(pb::ListMachinesRequest::default()))
        .await
        .expect_err("the machines are the host's");
    assert!(host_unreachable(&failed), "{failed:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_look_at_another_machines_file_goes_up() {
    let (machine, mut host) = linked().await;
    let mut files = machine.files().await;
    let call = machine.as_agent(pb::StatRequest {
        location: Some(on_the_host("note.txt")),
    });
    let looking = tokio::spawn(async move { files.stat(call).await });

    let forwarded = next_call(&mut host).await;
    assert_eq!(forwarded.method, "/humata.v1.FileService/Stat");
    let entry = pb::FileEntry {
        location: Some(on_the_host("note.txt")),
        kind: pb::file_entry::Kind::File as i32,
        size: 4,
        ..Default::default()
    };
    host.send(answer(&forwarded, pb::forwarded_result::Outcome::Response(entry.encode_to_vec())))
        .await;
    let got = tokio::time::timeout(WAIT, looking)
        .await
        .expect("the look was answered")
        .unwrap()
        .expect("the host's answer is the agent's")
        .into_inner();
    assert_eq!(got, entry);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_pull_from_the_host_goes_up_with_its_own_side_left_for_the_host_to_name() {
    let (machine, mut host) = linked().await;
    let here = own_id(&machine).await;
    let mut transfers = pb::transfer_service_client::TransferServiceClient::new(machine.daemon.channel().await);
    let call = machine.as_agent(pb::StartTransferRequest {
        sources: vec![on_the_host("note.txt")],
        destination: Some(files_of(ASKER, "")),
        mode: pb::TransferMode::Copy as i32,
        on_existing: pb::OnExisting::Fail as i32,
        ..Default::default()
    });
    let pulling = tokio::spawn(async move { transfers.start_transfer(call).await });

    let forwarded = next_call(&mut host).await;
    assert_eq!(forwarded.method, "/humata.v1.TransferService/StartTransfer");
    let sent = pb::StartTransferRequest::decode(forwarded.request.as_slice()).unwrap();
    assert_eq!(sent.sources, vec![on_the_host("note.txt")]);
    // The host knows this machine by its own id for the link, not by the
    // id this daemon gives itself, so this side names none (D768).
    let named = sent.destination.and_then(|place| place.machine).filter(|machine| !machine.value.is_empty());
    assert_eq!(named, None, "this machine's id is {here:?}");
    let started = pb::Transfer {
        id: Some(pb::TransferId { value: "tr_1".to_owned() }),
        ..Default::default()
    };
    host.send(answer(&forwarded, pb::forwarded_result::Outcome::Response(started.encode_to_vec())))
        .await;
    let got = tokio::time::timeout(WAIT, pulling)
        .await
        .expect("the copy was answered")
        .unwrap()
        .expect("the host's answer is the agent's")
        .into_inner();
    assert_eq!(got, started);
}

// BROWSER (D891): host setup RPCs stay host-only on a machine daemon, and
// unsupported agents (`zsh`) are refused browser bridge access.

#[tokio::test(flavor = "multi_thread")]
async fn browser_setup_stays_host_only_and_unsupported_agents_are_refused_on_machine() {
    let (machine, _host) = linked().await;
    let mut bridge = pb::browser_bridge_service_client::BrowserBridgeServiceClient::new(
        machine.daemon.channel().await,
    );

    // Host-only management RPCs fail with FailedPrecondition on machine role.
    let list_err = bridge
        .list_detected_browsers(pb::ListDetectedBrowsersRequest {})
        .await
        .expect_err("ListDetectedBrowsers is host-only");
    assert_eq!(list_err.code(), Code::FailedPrecondition);

    let install_err = bridge
        .install_browser_host(pb::InstallBrowserHostRequest::default())
        .await
        .expect_err("InstallBrowserHost is host-only");
    assert_eq!(install_err.code(), Code::FailedPrecondition);

    let uninstall_err = bridge
        .uninstall_browser_host(pb::UninstallBrowserHostRequest::default())
        .await
        .expect_err("UninstallBrowserHost is host-only");
    assert_eq!(uninstall_err.code(), Code::FailedPrecondition);

    // ASKER is a `zsh` session, which is outside the 4-agent browser scope.
    let status_err = bridge
        .get_browser_bridge_status(machine.as_agent(pb::GetBrowserBridgeStatusRequest {}))
        .await
        .expect_err("zsh is outside the 4-agent browser scope");
    assert_eq!(status_err.code(), Code::PermissionDenied);
}

// PROVIDERCLI (D906): a remote machine forwards `RequestProviderImport` and
// `ListProviders` up the Uplink for both session agents and local/CLI callers,
// while direct provider mutations stay host-only.

#[tokio::test(flavor = "multi_thread")]
async fn remote_provider_import_request_forwards_for_agent_and_user_while_mutations_stay_host_only() {
    let (machine, mut host) = linked().await;
    let channel = machine.daemon.channel().await;

    // 1. Agent caller's RequestProviderImport forwards up the Uplink.
    let mut providers = pb::provider_service_client::ProviderServiceClient::new(channel.clone());
    let req = pb::RequestProviderImportRequest {
        text: "[providers.remote]\nlabel = \"Remote\"\nconnect = 'r {name} -- {cmd}'\n".to_owned(),
        file_name: "remote.toml".to_owned(),
        reason: "need remote provider".to_owned(),
        ..Default::default()
    };
    let call = machine.as_agent(req.clone());
    let asking = tokio::spawn(async move { providers.request_provider_import(call).await });

    let forwarded = next_call(&mut host).await;
    assert_eq!(forwarded.method, "/humata.v1.ProviderService/RequestProviderImport");
    assert!(matches!(forwarded.principal, Some(pb::forwarded_call::Principal::Agent(_))));
    assert_eq!(
        pb::RequestProviderImportRequest::decode(forwarded.request.as_slice()).unwrap(),
        req
    );
    let saved = pb::SetProvidersConfigResponse {
        problems: Vec::new(),
        revision: Some(pb::Revision { value: 42 }),
    };
    host.send(answer(&forwarded, pb::forwarded_result::Outcome::Response(saved.encode_to_vec())))
        .await;
    let got = tokio::time::timeout(WAIT, asking)
        .await
        .expect("agent request answered")
        .unwrap()
        .expect("host answer returned")
        .into_inner();
    assert_eq!(got, saved);

    // 2. Local/CLI user caller's RequestProviderImport forwards up as MachineUser.
    let mut providers = pb::provider_service_client::ProviderServiceClient::new(channel.clone());
    let req_user = req.clone();
    let asking_user = tokio::spawn(async move { providers.request_provider_import(req_user).await });

    let forwarded_user = next_call(&mut host).await;
    assert_eq!(forwarded_user.method, "/humata.v1.ProviderService/RequestProviderImport");
    assert!(matches!(
        forwarded_user.principal,
        Some(pb::forwarded_call::Principal::User(ref u)) if u.outside_sessions
    ));
    host.send(answer(&forwarded_user, pb::forwarded_result::Outcome::Response(saved.encode_to_vec())))
        .await;
    let got_user = tokio::time::timeout(WAIT, asking_user)
        .await
        .expect("user request answered")
        .unwrap()
        .expect("host answer returned")
        .into_inner();
    assert_eq!(got_user, saved);

    // 3. Direct host mutations stay host-only on a machine daemon.
    let mut providers = pb::provider_service_client::ProviderServiceClient::new(channel);
    let set_err = providers
        .set_providers_config(pb::SetProvidersConfigRequest::default())
        .await
        .expect_err("SetProvidersConfig is host-only");
    assert_eq!(set_err.code(), Code::FailedPrecondition);
}
