//! Terminal tabs through `humatad2`'s gRPC API: a real shell in a real
//! holder, typed into and read back over the socket.

mod common;

use common::{Attach, Daemon, folder, invalid_field, not_found, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use pb::attach_client_message::Message as ClientMessage;
use pb::terminal_state::Writable;

async fn machine_shell(daemon: &Daemon) -> pb::TabId {
    daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap()
}

async fn list(daemon: &Daemon) -> Vec<pb::Tab> {
    daemon
        .terminals()
        .await
        .list_tabs(pb::ListTabsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .tabs
}

// Quoting splits the word, so the shell's echo of the command line
// doesn't count as its output.
const SAY_HELLO: &str = "echo hel''lo\n";
const SAY_WORLD: &str = "echo wor''ld\n";

#[tokio::test(flavor = "multi_thread")]
async fn typed_input_comes_back_as_contiguous_output() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;

    let seq = attach.type_text(SAY_HELLO).await;
    attach.wait_for("hello").await;
    attach
        .wait_until(|attach| attach.accepted.contains(&seq))
        .await;

    assert_eq!(attach.snapshots, 1);
    let state = attach.states.first().unwrap();
    assert_eq!(state.writable(), Writable::Yes);
    assert_eq!(state.other_writers, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn reattaching_from_the_cursor_sends_only_newer_output() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut first = Attach::start(&daemon, open(&tab, 0)).await;
    first.type_text(SAY_HELLO).await;
    first.wait_for("hello").await;
    let cursor = first.cursor;
    drop(first);

    let mut again = Attach::start(&daemon, open(&tab, cursor)).await;
    again.type_text(SAY_WORLD).await;
    again.wait_for("world").await;

    assert_eq!(again.snapshots, 0);
    assert!(!again.shows("hello"));
}

#[tokio::test(flavor = "multi_thread")]
async fn attaching_from_zero_again_replays_what_was_kept() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut first = Attach::start(&daemon, open(&tab, 0)).await;
    first.type_text(SAY_HELLO).await;
    first.wait_for("hello").await;
    drop(first);

    let mut again = Attach::start(&daemon, open(&tab, 0)).await;
    again.read().await.unwrap();

    assert_eq!(again.snapshots, 1);
    assert!(again.shows("hello"));
}

/// Prints about 2 MiB of repetitive lines, then "done".
const SAY_A_LOT: &str = "yes humata-compresses-this-line | head -n 90000; echo do''ne\n";

/// An attach that decodes every codec and holds all the output it's sent.
fn decoding(tab: &pb::TabId, from_offset: u64) -> pb::AttachOpen {
    pb::AttachOpen {
        accept_codecs: vec![pb::ChunkCodec::Lz4 as i32, pb::ChunkCodec::Zstd as i32],
        window_bytes: 64 << 20,
        ..open(tab, from_offset)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn output_comes_compressed_to_a_client_that_decodes_it() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut attach = Attach::start(&daemon, decoding(&tab, 0)).await;
    attach.type_text(SAY_A_LOT).await;
    attach.wait_for("done").await;

    assert!(attach.compressed > 0);
    assert!(attach.shows("humata-compresses-this-line"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_big_snapshot_comes_in_pieces_to_a_client_that_decodes() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut first = Attach::start(&daemon, decoding(&tab, 0)).await;
    first.type_text(SAY_A_LOT).await;
    first.wait_for("done").await;
    let end = first.cursor;
    drop(first);

    let mut again = Attach::start(&daemon, decoding(&tab, 0)).await;
    again.wait_until(|again| again.snapshots == 1).await;

    assert!(again.snapshot_pieces > 1);
    assert!(again.compressed > 0);
    assert!(again.cursor >= end);
    assert!(again.shows("done"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_lists_no_codecs_gets_its_snapshot_whole_and_raw() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut first = Attach::start(&daemon, decoding(&tab, 0)).await;
    first.type_text(SAY_A_LOT).await;
    first.wait_for("done").await;
    drop(first);

    let mut again = Attach::start(&daemon, open(&tab, 0)).await;
    again.wait_until(|again| again.snapshots == 1).await;

    assert_eq!(again.snapshot_pieces, 1);
    assert_eq!(again.compressed, 0);
    assert!(again.shows("done"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_size_a_writer_attaches_with_reaches_the_shell() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut attach = Attach::start(
        &daemon,
        pb::AttachOpen {
            cols: 100,
            rows: 30,
            ..open(&tab, 0)
        },
    )
    .await;
    attach.type_text("stty size\n").await;
    attach.wait_for("30 100").await;

    attach
        .send(ClientMessage::Resize(pb::TerminalResize {
            cols: 90,
            rows: 20,
        }))
        .await;
    attach.type_text("stty size\n").await;
    attach.wait_for("20 90").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn machine_shells_are_listed_with_their_offsets() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach.type_text(SAY_HELLO).await;
    attach.wait_for("hello").await;

    // The listing gives what the daemon's link to the holder has heard,
    // which can trail output still in flight; it catches up.
    let mut tabs = list(&daemon).await;
    for _ in 0..50 {
        if tabs[0].end_offset >= attach.cursor {
            break;
        }
        tabs = list(&daemon).await;
    }

    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].id.as_ref(), Some(&tab));
    assert_eq!(tabs[0].kind(), pb::tab::Kind::Shell);
    assert!(tabs[0].machine.is_some());
    assert!(tabs[0].end_offset >= attach.cursor);
    assert!(tabs[0].earliest_offset <= tabs[0].end_offset);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tab_outlives_its_daemon() {
    let mut daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut before = Attach::start(&daemon, open(&tab, 0)).await;
    before.type_text(SAY_HELLO).await;
    before.wait_for("hello").await;
    drop(before);

    daemon.restart().await;

    let tabs = list(&daemon).await;
    assert_eq!(tabs.len(), 1);
    let mut after = Attach::start(&daemon, open(&tab, 0)).await;
    after.type_text(SAY_WORLD).await;
    after.wait_for("world").await;
    assert!(after.shows("hello"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_tab_is_gone() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;

    daemon
        .terminals()
        .await
        .close_tab(pb::CloseTabRequest {
            tab: Some(tab.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert!(list(&daemon).await.is_empty());
    let refused = Attach::try_start(&daemon, open(&tab, 0))
        .await
        .err()
        .unwrap();
    assert_eq!(not_found(&refused), pb::EntityKind::Tab);
}

#[tokio::test(flavor = "multi_thread")]
async fn attaching_past_the_end_is_refused() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;

    let refused = Attach::try_start(&daemon, open(&tab, u64::MAX))
        .await
        .err()
        .unwrap();

    let (field, problem) = invalid_field(&refused);
    assert_eq!(field, "open.from_offset");
    assert_eq!(problem, pb::invalid_field::Problem::OutOfRange);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_attach_watches_but_cannot_type() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut writer = Attach::start(&daemon, open(&tab, 0)).await;
    let mut watcher = Attach::start(
        &daemon,
        pb::AttachOpen {
            read_only: true,
            ..open(&tab, 0)
        },
    )
    .await;

    writer.type_text(SAY_HELLO).await;
    watcher.wait_for("hello").await;
    let state = watcher.states.first().unwrap();
    assert_eq!(state.writable(), Writable::ReadOnlyAttach);
    assert_eq!(state.other_writers, 1);

    watcher.type_text(SAY_WORLD).await;
    let refused = watcher.wait_for_error().await;
    let (field, problem) = invalid_field(&refused);
    assert_eq!(field, "input");
    assert_eq!(problem, pb::invalid_field::Problem::NotAllowedHere);
}

#[tokio::test(flavor = "multi_thread")]
async fn writers_hear_about_each_other() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut first = Attach::start(&daemon, open(&tab, 0)).await;
    first.wait_until(|attach| !attach.states.is_empty()).await;

    let mut second = Attach::start(&daemon, open(&tab, 0)).await;
    second.wait_until(|attach| !attach.states.is_empty()).await;
    first
        .wait_until(|attach| {
            attach
                .states
                .last()
                .is_some_and(|state| state.other_writers == 1)
        })
        .await;

    assert_eq!(second.states[0].other_writers, 1);
    drop(second);
    first
        .wait_until(|attach| {
            attach
                .states
                .last()
                .is_some_and(|state| state.other_writers == 0)
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_that_exits_says_so() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;

    attach.type_text("exit 3\n").await;
    attach
        .wait_until(|attach| {
            attach
                .states
                .last()
                .is_some_and(|state| state.writable() == Writable::Exited)
        })
        .await;

    let state = attach.states.last().unwrap();
    assert_eq!(state.exit_code, 3);
    assert_eq!(state.status(), pb::SessionStatus::Exited);
    attach.wait_until(|attach| !attach.notices.is_empty()).await;
    assert_eq!(attach.notices[0].kind(), pb::terminal_notice::Kind::Exited);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_tab_is_not_found() {
    let daemon = Daemon::start().await;

    let refused = Attach::try_start(
        &daemon,
        open(
            &pb::TabId {
                value: "tb_missing".to_owned(),
            },
            0,
        ),
    )
    .await
    .err()
    .unwrap();

    assert_eq!(not_found(&refused), pb::EntityKind::Tab);
}

const TYPING_SESSION: &str = "se_typing";

/// A daemon with one session and a shell tab open in it.
async fn session_shell() -> (Daemon, pb::TabId) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: TYPING_SESSION.to_owned(),
            channel: "ch_typing".to_owned(),
            machine: String::new(),
            title: "typing".to_owned(),
            agent: "sh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let daemon = Daemon::start_in(dir).await;
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(typing_session()),
            repo: None,
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    (daemon, tab)
}

fn typing_session() -> pb::SessionId {
    pb::SessionId {
        value: TYPING_SESSION.to_owned(),
    }
}

async fn type_into_session(daemon: &Daemon, text: &str) -> Result<pb::TypeResponse, tonic::Status> {
    daemon
        .terminals()
        .await
        .r#type(pb::TypeRequest {
            session: Some(typing_session()),
            text: text.to_owned(),
            ..Default::default()
        })
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn each_text_typed_into_a_session_reaches_its_shell() {
    let (daemon, tab) = session_shell().await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;

    let first = type_into_session(&daemon, SAY_HELLO).await.unwrap();
    attach.wait_for("hello").await;
    let second = type_into_session(&daemon, SAY_WORLD).await.unwrap();
    attach.wait_for("world").await;

    assert!(first.typed);
    assert!(second.typed);
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_into_a_shell_that_exited_is_refused() {
    let (daemon, tab) = session_shell().await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    type_into_session(&daemon, "exit 3\n").await.unwrap();
    attach
        .wait_until(|attach| {
            attach
                .states
                .last()
                .is_some_and(|state| state.writable() == Writable::Exited)
        })
        .await;

    let refused = type_into_session(&daemon, SAY_HELLO).await.unwrap_err();

    assert_ne!(refused.code(), tonic::Code::Ok);
}

/// The user answers `id` with an approval, and the agent waits for what the
/// approved call returned: its type name and encoded response.
async fn approve_and_wait(
    daemon: &common::Daemon,
    token: &str,
    id: &pb::RequestId,
) -> pb::WaitResponse {
    let mut inbox = pb::inbox_service_client::InboxServiceClient::new(daemon.channel().await);
    inbox
        .answer_request(pb::AnswerRequestRequest {
            id: Some(id.clone()),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut stream = inbox
        .wait_for_answer(agent_request(
            pb::WaitForAnswerRequest {
                id: Some(id.clone()),
                wait_seconds: 10,
            },
            token,
        ))
        .await
        .unwrap()
        .into_inner();
    loop {
        let message = tokio::time::timeout(common::WAIT, stream.message())
            .await
            .expect("the wait ended in time")
            .unwrap()
            .expect("the wait ended with an answer");
        if !message.keepalive {
            return message;
        }
    }
}

/// The request an agent's call is waiting on, from the pending error.
fn pending_request(refused: &tonic::Status) -> pb::RequestId {
    match common::error_of(refused).kind {
        Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.unwrap(),
        other => panic!("expected approval_pending, got {other:?}"),
    }
}

fn agent_request<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).unwrap(),
    );
    request
}

/// Makes the channel's rule for `capability` Ask me.
async fn ask_me(daemon: &common::Daemon, channel: &pb::ChannelId, capability: pb::Capability) {
    pb::access_service_client::AccessServiceClient::new(daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: Some(channel.clone()),
            capability: capability as i32,
            decision: pb::Decision::Ask as i32,
            ..Default::default()
        })
        .await
        .unwrap();
}

/// The session token a shell tab of `session` was given.
async fn session_token(daemon: &common::Daemon, session: &pb::SessionId) -> String {
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
    let file = daemon.dir.path().join(format!("token-{}", session.value));
    let mut attach = common::Attach::start(daemon, common::open(&tab, 0)).await;
    attach
        .type_text(&format!(
            "printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r",
            file.display()
        ))
        .await;
    let deadline = std::time::Instant::now() + common::WAIT;
    loop {
        if let Ok(text) = std::fs::read_to_string(&file)
            && !text.is_empty()
        {
            return text;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the token was never written"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_typing_into_another_session_that_a_rule_sends_to_ask_runs_once_approved() {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    for (id, title) in [(TYPING_SESSION, "typing"), ("se_typist", "typist")] {
        store
            .put_session(&SessionRecord {
                id: id.to_owned(),
                channel: "ch_typing".to_owned(),
                machine: String::new(),
                title: title.to_owned(),
                agent: "sh".to_owned(),
                folder: dir.path().to_owned(),
                created_unix_ms: 0,
            })
            .unwrap();
    }
    drop(store);
    let daemon = Daemon::start_in(dir).await;
    let target = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(typing_session()),
            repo: None,
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut watching = Attach::start(&daemon, open(&target, 0)).await;
    let token = session_token(
        &daemon,
        &pb::SessionId {
            value: "se_typist".to_owned(),
        },
    )
    .await;
    ask_me(
        &daemon,
        &pb::ChannelId {
            value: "ch_typing".to_owned(),
        },
        pb::Capability::SessionType,
    )
    .await;

    let refused = daemon
        .terminals()
        .await
        .r#type(agent_request(
            pb::TypeRequest {
                session: Some(typing_session()),
                text: SAY_HELLO.to_owned(),
                ..Default::default()
            },
            &token,
        ))
        .await
        .unwrap_err();
    let waited = approve_and_wait(&daemon, &token, &pending_request(&refused)).await;

    assert_eq!(waited.result_type, "humata.v1.TypeResponse");
    let typed = <pb::TypeResponse as prost::Message>::decode(waited.result.as_slice()).unwrap();
    assert!(typed.typed);
    watching.wait_for("hello").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_tab_is_told_the_socket_its_daemon_serves_on() {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: TYPING_SESSION.to_owned(),
            channel: "ch_typing".to_owned(),
            machine: String::new(),
            title: "typing".to_owned(),
            agent: "sh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let daemon = Daemon::start_in(dir).await;
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(typing_session()),
            repo: None,
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let file = daemon.dir.path().join("socket-seen");
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach
        .type_text(&format!(
            "printf '%s' \"$HUMATA_SOCKET\" > {}\r",
            file.display()
        ))
        .await;
    let deadline = std::time::Instant::now() + common::WAIT;
    let seen = loop {
        if let Ok(text) = std::fs::read_to_string(&file)
            && !text.is_empty()
        {
            break text;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the tab never wrote HUMATA_SOCKET"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };

    let expected = std::path::absolute(&daemon.socket).unwrap();
    assert_eq!(std::path::PathBuf::from(seen), expected);
}
