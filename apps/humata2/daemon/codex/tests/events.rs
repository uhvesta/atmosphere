//! What each message of Codex's app-server means for Humata (D795): the
//! thread a conversation is in, a turn starting and ending (an interrupted
//! one is stopped by you), an approval or a question for the Inbox, and an
//! ask answered elsewhere. A message Humata doesn't know is passed over,
//! never fatal, since the protocol is experimental.

use std::path::PathBuf;

use humata2_codex::{AskKind, Event, read_frame};

const FRAMES: &str = include_str!("../testdata/frames.jsonl");
const THREAD: &str = "019b2c4d-7e8f-7a01-b234-56789abcdef0";

fn frame(n: usize) -> &'static str {
    FRAMES.lines().nth(n).expect("a recorded frame")
}

fn event(n: usize) -> Event {
    read_frame(frame(n))
        .expect("a frame Humata can read")
        .expect("a frame that means something")
}

#[test]
fn a_started_thread_names_the_conversation_and_its_file() {
    match event(0) {
        Event::ThreadStarted { thread, path } => {
            assert_eq!(thread, THREAD);
            assert_eq!(
                path,
                Some(PathBuf::from(format!(
                    "/home/u/.codex/sessions/2026/11/18/rollout-2026-11-18T09-30-00-{THREAD}.jsonl"
                )))
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_subagents_thread_names_no_conversation_of_the_session() {
    assert!(read_frame(frame(10)).expect("readable").is_none());
}

#[test]
fn turns_start_and_end_and_an_interrupted_one_is_stopped() {
    assert!(matches!(event(1), Event::TurnStarted { thread } if thread == THREAD));
    assert!(matches!(
        event(8),
        Event::TurnCompleted { thread, interrupted: false } if thread == THREAD
    ));
    assert!(matches!(
        event(9),
        Event::TurnCompleted { interrupted: true, .. }
    ));
}

#[test]
fn approvals_and_questions_are_asks_keyed_by_thread_and_item() {
    let Event::Ask { thread, item, kind, .. } = event(2) else {
        panic!("a command approval is an ask");
    };
    assert_eq!((thread.as_str(), item.as_str(), kind), (THREAD, "call_a1", AskKind::Command));
    let Event::Ask { item, kind, .. } = event(4) else {
        panic!("a file change approval is an ask");
    };
    assert_eq!((item.as_str(), kind), ("call_b2", AskKind::FileChange));
    let Event::Ask { item, kind, params, .. } = event(5) else {
        panic!("a question is an ask");
    };
    assert_eq!((item.as_str(), kind), ("call_9f8e", AskKind::Question));
    assert_eq!(params["questions"][0]["id"], "deploy_target");
}

#[test]
fn an_ask_answered_elsewhere_is_resolved_by_the_same_request() {
    let Event::Ask { request: asked, .. } = event(2) else {
        panic!("an ask");
    };
    let Event::Resolved { request, thread } = event(3) else {
        panic!("a resolution");
    };
    assert_eq!(request, asked);
    assert_eq!(thread, THREAD);
    let Event::Ask { request: other, .. } = event(4) else {
        panic!("an ask");
    };
    assert_ne!(request, other);
}

#[test]
fn what_humata_doesnt_know_is_passed_over() {
    // A delta, a method from a later Codex, and a reply to our own request.
    for n in [6, 7, 11] {
        assert!(read_frame(frame(n)).expect("readable").is_none(), "frame {n}");
    }
    // Fields Humata doesn't know never stop it reading one it does.
    let extra = frame(1).replacen("\"params\":{", "\"params\":{\"newField\":{\"x\":1},", 1);
    assert!(matches!(read_frame(&extra), Ok(Some(Event::TurnStarted { .. }))));
}

#[test]
fn what_isnt_json_rpc_is_an_error() {
    for text in ["not json", "[1,2]", "{\"jsonrpc\":\"2.0\",\"method\":\"turn/started\",\"params\":{}}"] {
        assert!(read_frame(text).is_err(), "{text:?}");
    }
}
