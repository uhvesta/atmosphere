//! Terminal tabs through `humatad2`'s gRPC API: a real shell in a real
//! holder, typed into and read back over the socket.

mod common;

use common::{Attach, Daemon, invalid_field, not_found, open};
use humata2_grpc::humata::v1 as pb;
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
    attach.wait_until(|attach| attach.accepted.contains(&seq)).await;

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
        .send(ClientMessage::Resize(pb::TerminalResize { cols: 90, rows: 20 }))
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

    let tabs = list(&daemon).await;

    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].id.as_ref(), Some(&tab));
    assert_eq!(tabs[0].kind(), pb::tab::Kind::Shell);
    assert!(tabs[0].machine.is_some());
    assert!(tabs[0].end_offset >= attach.cursor);
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
    let refused = Attach::try_start(&daemon, open(&tab, 0)).await.err().unwrap();
    assert_eq!(not_found(&refused), pb::EntityKind::Tab);
}

#[tokio::test(flavor = "multi_thread")]
async fn attaching_past_the_end_is_refused() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;

    let refused = Attach::try_start(&daemon, open(&tab, u64::MAX)).await.err().unwrap();

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
        .wait_until(|attach| attach.states.last().is_some_and(|state| state.other_writers == 1))
        .await;

    assert_eq!(second.states[0].other_writers, 1);
    drop(second);
    first
        .wait_until(|attach| attach.states.last().is_some_and(|state| state.other_writers == 0))
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
