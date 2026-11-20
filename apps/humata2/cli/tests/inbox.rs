//! `humata inbox` (CLI.md §5.4): listing, answering one request, and
//! answering all that are waiting.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use pb::inbox_scope::Scope;
use pb::request_answer::Answer;
use serde_json::json;
use support::{FakeConsole, FakeDaemon, run};

fn request(id: &str, state: pb::RequestState) -> pb::InboxRequest {
    pb::InboxRequest {
        id: Some(pb::RequestId { value: id.into() }),
        state: state as i32,
        action_title: format!("Do {id}"),
        ..Default::default()
    }
}

fn answer(sent: &pb::AnswerRequestRequest) -> Option<Answer> {
    sent.answer.clone().and_then(|answer| answer.answer)
}

#[tokio::test]
async fn the_inbox_lists_every_channel_by_default() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "inbox"]).await, exit::OK);
    let sent: pb::ListRequestsRequest = daemon.only(rpc::inbox::LIST_REQUESTS);
    assert!(matches!(sent.scope.and_then(|scope| scope.scope), Some(Scope::Global(_))));
    assert!(!sent.include_answered);
}

#[tokio::test]
async fn the_inbox_of_a_session_and_its_forks() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "inbox", "--all", "--session", "se_1", "--with-forks"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ListRequestsRequest = daemon.only(rpc::inbox::LIST_REQUESTS);
    assert_eq!(
        sent.scope.and_then(|scope| scope.scope),
        Some(Scope::Session(pb::SessionInbox {
            session: support::session("se_1"),
            with_forks: true,
        }))
    );
    assert!(sent.include_answered);
}

#[tokio::test]
async fn the_inbox_of_a_channel() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "--channel", "ch_1", "inbox"]).await, exit::OK);
    let sent: pb::ListRequestsRequest = daemon.only(rpc::inbox::LIST_REQUESTS);
    assert_eq!(
        sent.scope.and_then(|scope| scope.scope),
        Some(Scope::Channel(pb::ChannelId { value: "ch_1".into() }))
    );
}

#[tokio::test]
async fn approve_answers_approve() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::inbox::ANSWER_REQUEST,
        &request("rq_1", pb::RequestState::Approved),
    );
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "inbox", "approve", "rq_1"]).await, exit::OK);
    let sent: pb::AnswerRequestRequest = daemon.only(rpc::inbox::ANSWER_REQUEST);
    assert_eq!(sent.id, Some(pb::RequestId { value: "rq_1".into() }));
    assert_eq!(answer(&sent), Some(Answer::Approve(pb::Approve {})));
    assert_eq!(console.json()["result"]["state"], "approved");
}

#[tokio::test]
async fn approve_always_lets_what_the_request_offered() {
    let offered = pb::GrantTarget {
        target: Some(pb::grant_target::Target::Session(pb::SessionId {
            value: "se_1".into(),
        })),
    };
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::inbox::GET_REQUEST,
        &pb::InboxRequest {
            always: Some(pb::AlwaysOption {
                label: "Always for this session".into(),
                target: Some(offered.clone()),
            }),
            ..request("rq_1", pb::RequestState::Pending)
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "inbox", "approve", "rq_1", "--always"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::AnswerRequestRequest = daemon.only(rpc::inbox::ANSWER_REQUEST);
    assert_eq!(
        answer(&sent),
        Some(Answer::Always(pb::AlwaysLet {
            target: Some(offered)
        }))
    );
}

#[tokio::test]
async fn approve_always_is_refused_when_nothing_was_offered() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::inbox::GET_REQUEST,
        &request("rq_1", pb::RequestState::Pending),
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "inbox", "approve", "rq_1", "--always"]).await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::inbox::ANSWER_REQUEST));
}

#[tokio::test]
async fn an_approval_cant_carry_a_note() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "inbox", "approve", "rq_1", "--note", "thanks"],
    )
    .await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::inbox::ANSWER_REQUEST));
}

#[tokio::test]
async fn deny_carries_its_note_and_reply_its_text() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    run(&daemon, &console, &["--json", "inbox", "deny", "rq_1", "--note", "not today"]).await;
    let console = FakeConsole::pipe();
    run(&daemon, &console, &["--json", "inbox", "reply", "rq_2", "use the staging host"]).await;
    let sent: Vec<pb::AnswerRequestRequest> = daemon.sent(rpc::inbox::ANSWER_REQUEST);
    assert_eq!(
        answer(&sent[0]),
        Some(Answer::Deny(pb::Deny {
            note: "not today".into()
        }))
    );
    assert_eq!(
        answer(&sent[1]),
        Some(Answer::Reply(pb::Reply {
            text: "use the staging host".into(),
            selected_choices: Vec::new(),
        }))
    );
}

#[tokio::test]
async fn approve_all_answers_only_what_is_pending_now() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::inbox::LIST_REQUESTS,
        &pb::ListRequestsResponse {
            requests: vec![
                request("rq_1", pb::RequestState::Pending),
                request("rq_2", pb::RequestState::Approved),
                request("rq_3", pb::RequestState::Pending),
            ],
            pending_count: 2,
            page: None,
            allowed_without_asking_today: 0,
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "inbox", "--session", "se_1", "approve-all"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::AnswerAllRequest = daemon.only(rpc::inbox::ANSWER_ALL);
    let ids: Vec<_> = sent.ids.iter().map(|id| id.value.as_str()).collect();
    assert_eq!(ids, vec!["rq_1", "rq_3"]);
    assert!(sent.approve);
    assert!(matches!(
        sent.scope.and_then(|scope| scope.scope),
        Some(Scope::Session(_))
    ));
}

#[tokio::test]
async fn deny_all_with_nothing_waiting_changes_nothing() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "inbox", "deny-all"]).await, exit::OK);
    assert!(!daemon.called(rpc::inbox::ANSWER_ALL));
    assert_eq!(console.json()["result"], json!({ "answered": [], "skipped": [] }));
}

#[tokio::test]
async fn an_answer_someone_else_gave_first_is_a_conflict() {
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::inbox::ANSWER_REQUEST,
        pb::error::Kind::AlreadyAnswered(Default::default()),
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "inbox", "approve", "rq_1"]).await;
    assert_eq!(exit, exit::CONFLICT);
}
