//! A tab's attach read as the terminal component's frames, and the
//! component's frames sent back over the attach.

use humata2_proto::humata::v1 as pb;
use humata2_terminal_link::{Inbound, LinkError, Outgoing, open, outbound};
use pb::attach_client_message::Message as ClientMessage;
use pb::attach_server_message::Message as ServerMessage;
use pb::terminal_state::Writable;
use terminal_wire::{ClientFrame, SNAPSHOT_ENGINE, ServerFrame};

const ROUTE: u64 = 7;

fn message(message: ServerMessage) -> pb::AttachServerMessage {
    pb::AttachServerMessage {
        message: Some(message),
    }
}

fn piece(offset: u64, vt: &[u8], more: bool) -> pb::AttachServerMessage {
    message(ServerMessage::Snapshot(pb::TerminalSnapshot {
        offset,
        vt: vt.to_vec(),
        cols: 80,
        rows: 24,
        more,
        engine: SNAPSHOT_ENGINE.to_owned(),
        terminal_uuid: "tab-1-gen-1".to_owned(),
        ..Default::default()
    }))
}

fn state(writable: Writable, cols: u32, rows: u32) -> pb::AttachServerMessage {
    message(ServerMessage::State(pb::TerminalState {
        writable: writable as i32,
        cols,
        rows,
        cell_width: 8,
        cell_height: 16,
        ..Default::default()
    }))
}

fn output(offset: u64, data: &[u8]) -> pb::AttachServerMessage {
    message(ServerMessage::Output(pb::TerminalOutput {
        offset,
        data: data.to_vec(),
        ..Default::default()
    }))
}

/// A link with a screen installed and input granted.
fn installed() -> Inbound {
    let mut link = Inbound::new(ROUTE);
    link.map(state(Writable::Yes, 80, 24)).expect("state");
    link.map(piece(10, b"screen", false)).expect("snapshot");
    link
}

#[test]
fn the_screen_comes_first_then_input_is_granted() {
    let mut link = Inbound::new(ROUTE);
    let early = link.map(state(Writable::Yes, 80, 24)).expect("state");
    assert!(early.is_empty(), "nothing reaches the replica before its screen");

    let frames = link.map(piece(10, b"screen", false)).expect("snapshot");
    assert!(matches!(
        &frames[..],
        [
            ServerFrame::Snapshot { id: ROUTE, offset: 10, cols: 80, rows: 24, live: true, exited: None, bytes, engine },
            ServerFrame::Control { id: ROUTE, controller: true },
        ] if bytes == b"screen" && engine == SNAPSHOT_ENGINE
    ));
    assert_eq!(link.terminal_uuid(), Some("tab-1-gen-1"));
}

#[test]
fn a_screen_in_pieces_arrives_as_one() {
    let mut link = Inbound::new(ROUTE);
    assert!(link.map(piece(10, b"scr", true)).expect("first").is_empty());
    let frames = link.map(piece(10, b"een", false)).expect("last");
    assert!(matches!(&frames[..], [ServerFrame::Snapshot { bytes, .. }] if bytes == b"screen"));
}

#[test]
fn output_follows_the_screen_in_order() {
    let mut link = installed();
    let frames = link.map(output(10, b"abc")).expect("output");
    assert!(matches!(
        &frames[..],
        [ServerFrame::Output { id: ROUTE, offset: 10, bytes }] if bytes == b"abc"
    ));
}

#[test]
fn output_before_any_screen_is_refused() {
    let mut link = Inbound::new(ROUTE);
    assert!(matches!(
        link.map(output(0, b"abc")),
        Err(LinkError::OutputBeforeSnapshot)
    ));
}

#[test]
fn a_peer_without_checkpoints_is_refused_and_stays_refused() {
    let mut link = Inbound::new(ROUTE);
    let old = message(ServerMessage::Snapshot(pb::TerminalSnapshot {
        offset: 3,
        vt: b"abc".to_vec(),
        cols: 80,
        rows: 24,
        ..Default::default()
    }));
    assert!(matches!(link.map(old), Err(LinkError::PeerVersionUnsupported)));
    assert!(matches!(
        link.map(output(3, b"def")),
        Err(LinkError::PeerVersionUnsupported)
    ));
}

#[test]
fn a_screen_from_another_engine_is_refused() {
    let mut link = Inbound::new(ROUTE);
    let other = message(ServerMessage::Snapshot(pb::TerminalSnapshot {
        engine: "other/9".to_owned(),
        cols: 80,
        rows: 24,
        ..Default::default()
    }));
    assert!(matches!(link.map(other), Err(LinkError::EngineMismatch { .. })));
}

#[test]
fn compressed_pieces_nobody_asked_for_are_refused() {
    let mut link = installed();
    let compressed = message(ServerMessage::Output(pb::TerminalOutput {
        offset: 10,
        data: b"x".to_vec(),
        codec: pb::ChunkCodec::Zstd as i32,
        ..Default::default()
    }));
    assert!(matches!(
        link.map(compressed),
        Err(LinkError::UnexpectedCodec { .. })
    ));
}

#[test]
fn a_written_input_is_confirmed_by_its_sequence() {
    let mut link = installed();
    let frames = link
        .map(message(ServerMessage::InputAccepted(pb::InputAccepted { input_seq: 4 })))
        .expect("accepted");
    assert!(matches!(&frames[..], [ServerFrame::InputWritten { id: ROUTE, seq: 4 }]));
}

#[test]
fn a_new_size_is_announced_once() {
    let mut link = installed();
    let frames = link.map(state(Writable::Yes, 100, 30)).expect("state");
    assert!(matches!(
        &frames[..],
        [ServerFrame::Resized { id: ROUTE, cols: 100, rows: 30, cell_width: 8, cell_height: 16 }]
    ));
    assert!(link.map(state(Writable::Yes, 100, 30)).expect("same").is_empty());
}

#[test]
fn moving_takes_input_away_and_gives_it_back() {
    let mut link = installed();
    let frames = link.map(state(Writable::Moving, 80, 24)).expect("moving");
    assert!(matches!(&frames[..], [ServerFrame::Control { controller: false, .. }]));
    let frames = link.map(state(Writable::Yes, 80, 24)).expect("moved");
    assert!(matches!(&frames[..], [ServerFrame::Control { controller: true, .. }]));
}

#[test]
fn a_read_only_attach_never_gets_input() {
    let mut link = Inbound::new(ROUTE);
    link.map(state(Writable::ReadOnlyAttach, 80, 24)).expect("state");
    let frames = link.map(piece(0, b"", false)).expect("snapshot");
    assert!(matches!(&frames[1..], [ServerFrame::Control { controller: false, .. }]));
}

#[test]
fn an_exit_is_reported_with_its_code() {
    let mut link = installed();
    let exited = message(ServerMessage::State(pb::TerminalState {
        writable: Writable::Exited as i32,
        exit_code: 3,
        cols: 80,
        rows: 24,
        cell_width: 8,
        cell_height: 16,
        ..Default::default()
    }));
    let frames = link.map(exited).expect("exited");
    assert!(matches!(&frames[..], [ServerFrame::Exited { id: ROUTE, code: Some(3) }]));
}

#[test]
fn the_attach_asks_for_checkpoints_from_the_start() {
    let tab = pb::TabId {
        value: "tab-1".to_owned(),
    };
    let first = open(None, tab, 120, 40, false, 1 << 20);
    let Some(ClientMessage::Open(open)) = first.message else {
        panic!("the first message opens the attach");
    };
    assert_eq!(open.checkpoint_engine, SNAPSHOT_ENGINE);
    assert_eq!(open.from_offset, 0);
    assert_eq!((open.cols, open.rows), (120, 40));
}

#[test]
fn the_replicas_frames_become_attach_messages() {
    let sent = |frame| match outbound(frame).expect("maps") {
        Outgoing::Send(message) => message.message,
        other => panic!("expected a message, got {other:?}"),
    };
    assert!(matches!(
        sent(ClientFrame::Input { id: ROUTE, seq: 2, bytes: b"ls\r".to_vec() }),
        Some(ClientMessage::Input(pb::TerminalInput { input_seq: 2, data })) if data == b"ls\r"
    ));
    assert!(matches!(
        sent(ClientFrame::Resize { id: ROUTE, cols: 90, rows: 20, cell_width: 7, cell_height: 15 }),
        Some(ClientMessage::Resize(pb::TerminalResize { cols: 90, rows: 20, cell_width: 7, cell_height: 15 }))
    ));
    assert!(matches!(
        sent(ClientFrame::Ack { id: ROUTE, offset: 99 }),
        Some(ClientMessage::Ack(pb::OutputAck { offset: 99 }))
    ));
}

#[test]
fn asking_for_a_fresh_screen_reopens_and_detaching_closes() {
    assert!(matches!(
        outbound(ClientFrame::SnapshotRequest { id: ROUTE }),
        Ok(Outgoing::Reopen)
    ));
    assert!(matches!(outbound(ClientFrame::Detach { id: ROUTE }), Ok(Outgoing::Close)));
    assert!(matches!(
        outbound(ClientFrame::TakeControl { id: ROUTE }),
        Ok(Outgoing::Nothing)
    ));
}

#[test]
fn host_commands_are_not_sent_over_an_attach() {
    assert!(matches!(
        outbound(ClientFrame::Stop { id: ROUTE }),
        Err(LinkError::NotOnAttach { .. })
    ));
}
